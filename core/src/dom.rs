use js_sys::{Function, Object, Reflect};
use serde_json::json;
use std::cell::RefCell;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{
    Element, Event, EventInit, HtmlElement, HtmlInputElement, HtmlSelectElement,
    HtmlTextAreaElement,
};

#[derive(Default)]
struct Snapshot {
    nonce: String,
    epoch: u64,
    elements: Vec<(Element, String)>,
}
thread_local! { static CURRENT: RefCell<Snapshot> = RefCell::new(Snapshot::default()); }
fn err(message: &str) -> JsValue {
    JsValue::from_str(message)
}
fn document() -> Result<web_sys::Document, JsValue> {
    web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| err("Page has no document"))
}
fn truncate(value: String, n: usize) -> String {
    value.chars().take(n).collect()
}
fn fingerprint(element: &Element) -> String {
    json!([
        element.tag_name(),
        element.get_attribute("type"),
        element.get_attribute("href"),
        element.get_attribute("aria-label"),
        element.get_attribute("role"),
        truncate(element.text_content().unwrap_or_default(), 1000)
    ])
    .to_string()
}
fn associated_label(doc: &web_sys::Document, element: &Element) -> Option<String> {
    let id = element.get_attribute("id")?;
    let labels = doc.query_selector_all("label").ok()?;
    (0..labels.length())
        .filter_map(|i| labels.item(i))
        .filter_map(|n| n.dyn_into::<Element>().ok())
        .find(|label| label.get_attribute("for").as_deref() == Some(id.as_str()))
        .and_then(|label| label.text_content())
}
fn visible(element: &Element) -> bool {
    let rect = element.get_bounding_client_rect();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return false;
    }
    web_sys::window()
        .and_then(|w| w.get_computed_style(element).ok().flatten())
        .is_some_and(|s| {
            s.get_property_value("visibility").unwrap_or_default() != "hidden"
                && s.get_property_value("display").unwrap_or_default() != "none"
        })
}
fn events(element: &Element) -> Result<(), JsValue> {
    let init = EventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    for name in ["input", "change"] {
        element.dispatch_event(&Event::new_with_event_init_dict(name, &init)?)?;
    }
    Ok(())
}
fn native_value(element: &Element, value: &str) -> Result<(), JsValue> {
    // Call the prototype setter so controlled React inputs receive a real value change.
    let proto = Object::get_prototype_of(element.as_ref());
    let descriptor = Object::get_own_property_descriptor(&proto, &JsValue::from_str("value"));
    let setter = Reflect::get(&descriptor, &JsValue::from_str("set"))?.dyn_into::<Function>()?;
    setter.call1(element.as_ref(), &JsValue::from_str(value))?;
    events(element)
}

#[wasm_bindgen]
pub fn snapshot() -> Result<String, JsValue> {
    let doc = document()?;
    let nodes=doc.query_selector_all("a[href],button,input,textarea,select,[role=button],[role=link],[contenteditable=true],[tabindex]")?;
    CURRENT.with(|current| {
        let mut state=current.borrow_mut();
        if state.nonce.is_empty() {state.nonce=web_sys::window().unwrap().crypto()?.random_uuid();}
        state.epoch+=1;state.elements.clear();
        let mut elements=Vec::new();
        for index in 0..nodes.length() {
            if elements.len()>=250 {break;}
            let Some(node)=nodes.item(index) else {continue;};
            let Ok(element)=node.dyn_into::<Element>() else {continue;};
            if !visible(&element) || element.get_attribute("type").as_deref()==Some("hidden") {continue;}
            let label=element.get_attribute("aria-label")
                .or_else(||element.get_attribute("placeholder"))
                .or_else(||element.get_attribute("title"))
                .or_else(||associated_label(&doc,&element))
                .or_else(||element.closest("label").ok().flatten().and_then(|l|l.text_content()))
                .or_else(||element.text_content()).unwrap_or_default();
            let kind=element.get_attribute("type").unwrap_or_default();
            let mut item=json!({"reference":format!("{}:{}:{}",state.nonce,state.epoch,state.elements.len()),"tag":element.tag_name().to_lowercase(),"label":truncate(label.split_whitespace().collect::<Vec<_>>().join(" "),180),"type":kind,"disabled":element.has_attribute("disabled") || element.get_attribute("aria-disabled").as_deref()==Some("true")});
            if let Some(input)=element.dyn_ref::<HtmlInputElement>() {
                let sensitive=format!("{} {} {}",input.type_(),input.name(),input.autocomplete()).to_lowercase().contains("password");
                if !sensitive && input.type_()!="file" {item["value"]=json!(truncate(input.value(),1000));}
                if matches!(input.type_().as_str(),"checkbox"|"radio") {item["checked"]=json!(input.checked());}
            }
            if let Some(area)=element.dyn_ref::<HtmlTextAreaElement>() {item["value"]=json!(truncate(area.value(),1000));}
            if let Some(select)=element.dyn_ref::<HtmlSelectElement>() {
                let options=select.query_selector_all("option")?;
                let mut values=Vec::new();
                for i in 0..options.length().min(80) {
                    if let Some(option)=options.item(i).and_then(|n|n.dyn_into::<Element>().ok()) {values.push(json!({"value":option.get_attribute("value").unwrap_or_else(||option.text_content().unwrap_or_default()),"label":truncate(option.text_content().unwrap_or_default(),120)}));}
                }
                item["options"]=json!(values);
                item["value"]=json!(select.value());
            }
            if let Some(href)=element.get_attribute("href") {item["href"]=json!(truncate(href,500));}
            elements.push(item);let identity=fingerprint(&element);state.elements.push((element,identity));
        }
        let body=doc.body().map(|b|truncate(b.inner_text(),16_000)).unwrap_or_default();
        Ok(json!({"title":doc.title(),"url":doc.url()?,"text":body,"elements":elements,"scope":"top-level document; only visible interactive elements","untrusted_content":true}).to_string())
    })
}

#[wasm_bindgen]
pub fn operate(input: &str) -> Result<String, JsValue> {
    let value = crate::check_command(input).map_err(|e| err(&e))?;
    let action = value["action"].as_str().unwrap();
    if action == "snapshot" {
        return snapshot();
    }
    if action == "scroll" {
        web_sys::window()
            .ok_or_else(|| err("No window"))?
            .scroll_by_with_x_and_y(0.0, value["pixels"].as_i64().unwrap() as f64);
        return Ok(json!({"ok":true,"action":"scroll"}).to_string());
    }
    let reference = value["reference"]
        .as_str()
        .ok_or_else(|| err("Missing element reference"))?;
    let element = CURRENT.with(|current| {
        let state = current.borrow();
        let prefix = format!("{}:{}:", state.nonce, state.epoch);
        let index = reference
            .strip_prefix(&prefix)
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or_else(|| err("Stale reference. Take a fresh snapshot."))?;
        let (element, identity) = state
            .elements
            .get(index)
            .cloned()
            .ok_or_else(|| err("Unknown element reference"))?;
        if fingerprint(&element) != identity {
            return Err(err("Element meaning changed. Take a fresh snapshot."));
        }
        Ok(element)
    })?;
    if !element.is_connected() || !visible(&element) {
        return Err(err("Element changed or is hidden. Take a fresh snapshot."));
    }
    if element.has_attribute("disabled")
        || element.get_attribute("aria-disabled").as_deref() == Some("true")
    {
        return Err(err("Element is disabled"));
    }
    match action {
        "click" => {
            if element
                .get_attribute("type")
                .is_some_and(|t| matches!(t.as_str(), "password" | "file"))
            {
                return Err(err("Use password and file inputs manually"));
            }
            let html = element
                .dyn_ref::<HtmlElement>()
                .ok_or_else(|| err("Element cannot be clicked"))?;
            html.scroll_into_view();
            html.click();
        }
        "fill" => {
            let text = value["text"].as_str().unwrap();
            if let Some(input) = element.dyn_ref::<HtmlInputElement>() {
                if !matches!(
                    input.type_().as_str(),
                    "text" | "email" | "tel" | "url" | "search" | "number"
                ) {
                    return Err(err("This input type must be completed manually"));
                }
                if input.read_only() {
                    return Err(err("Input is read-only"));
                }
                input.focus()?;
                native_value(&element, text)?;
            } else if let Some(area) = element.dyn_ref::<HtmlTextAreaElement>() {
                if area.read_only() {
                    return Err(err("Text area is read-only"));
                }
                area.focus()?;
                native_value(&element, text)?;
            } else if element.get_attribute("contenteditable").as_deref() == Some("true") {
                element
                    .dyn_ref::<HtmlElement>()
                    .ok_or_else(|| err("Not editable"))?
                    .focus()?;
                element.set_text_content(Some(text));
                events(&element)?;
            } else {
                return Err(err("Element is not a supported text field"));
            }
        }
        "select" => {
            let select = element
                .dyn_ref::<HtmlSelectElement>()
                .ok_or_else(|| err("Element is not a select"))?;
            let desired = value["value"].as_str().unwrap();
            let options = select.query_selector_all("option")?;
            let exists = (0..options.length())
                .filter_map(|i| options.item(i))
                .filter_map(|n| n.dyn_into::<Element>().ok())
                .any(|e| {
                    !e.has_attribute("disabled")
                        && e.get_attribute("value")
                            .unwrap_or_else(|| e.text_content().unwrap_or_default())
                            == desired
                });
            if !exists {
                return Err(err("Option is missing or disabled"));
            }
            native_value(&element, desired)?;
        }
        _ => return Err(err("This action belongs to the extension worker")),
    }
    Ok(json!({"ok":true,"action":action,"instruction":"Read the page again to verify the outcome."}).to_string())
}
