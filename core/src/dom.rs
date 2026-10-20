use js_sys::{Function, Object, Reflect};
use serde_json::json;
use std::cell::RefCell;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{
    Element, Event, EventInit, HtmlElement, HtmlInputElement, HtmlSelectElement,
    HtmlTextAreaElement, Node, ShadowRoot,
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
        label(element),
        truncate(element.text_content().unwrap_or_default(), 1000)
    ])
    .to_string()
}
fn associated_label(element: &Element) -> Option<String> {
    let id = element.get_attribute("id")?;
    let root = element.get_root_node();
    let labels = if let Some(doc) = root.dyn_ref::<web_sys::Document>() {
        doc.query_selector_all("label").ok()?
    } else {
        root.dyn_ref::<ShadowRoot>()?
            .query_selector_all("label")
            .ok()?
    };
    (0..labels.length())
        .filter_map(|i| labels.item(i))
        .filter_map(|n| n.dyn_into::<Element>().ok())
        .find(|label| label.get_attribute("for").as_deref() == Some(id.as_str()))
        .and_then(|label| label.text_content())
}
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn parent(element: &Element) -> Option<Element> {
    element.parent_element().or_else(|| {
        element
            .get_root_node()
            .dyn_ref::<ShadowRoot>()
            .map(ShadowRoot::host)
    })
}
fn style_visible(element: &Element, controls: bool) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let mut current = Some(element.clone());
    while let Some(node) = current {
        if controls
            && (node.has_attribute("inert")
                || node.get_attribute("aria-hidden").as_deref() == Some("true"))
        {
            return false;
        }
        if let Ok(Some(style)) = window.get_computed_style(&node) {
            if style.get_property_value("display").unwrap_or_default() == "none"
                || matches!(
                    style
                        .get_property_value("visibility")
                        .unwrap_or_default()
                        .as_str(),
                    "hidden" | "collapse"
                )
            {
                return false;
            }
        }
        current = parent(&node);
    }
    true
}
fn visible(element: &Element) -> bool {
    let rect = element.get_bounding_client_rect();
    rect.width() > 0.0 && rect.height() > 0.0 && style_visible(element, true)
}
fn referenced_text(element: &Element, attribute: &str) -> Option<String> {
    let ids = element.get_attribute(attribute)?;
    let root = element.get_root_node();
    let mut parts = Vec::new();
    for id in ids.split_whitespace() {
        let target = root
            .dyn_ref::<web_sys::Document>()
            .and_then(|d| d.get_element_by_id(id))
            .or_else(|| {
                root.dyn_ref::<ShadowRoot>()
                    .and_then(|s| s.get_element_by_id(id))
            });
        if let Some(target) = target {
            parts.push(target.text_content().unwrap_or_default());
        }
    }
    let text = normalize(&parts.join(" "));
    (!text.is_empty()).then_some(text)
}
fn label(element: &Element) -> String {
    let value = referenced_text(element, "aria-labelledby")
        .or_else(|| {
            element
                .get_attribute("aria-label")
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| associated_label(element))
        .or_else(|| {
            element
                .closest("label")
                .ok()
                .flatten()
                .and_then(|l| l.text_content())
        })
        .or_else(|| element.get_attribute("placeholder"))
        .or_else(|| element.get_attribute("title"))
        .or_else(|| {
            element
                .dyn_ref::<HtmlInputElement>()
                .filter(|i| matches!(i.type_().as_str(), "button" | "submit" | "reset"))
                .map(HtmlInputElement::value)
        })
        .or_else(|| element.text_content())
        .unwrap_or_default();
    truncate(normalize(&value), 180)
}
fn role(element: &Element) -> String {
    if let Some(role) = element.get_attribute("role").filter(|r| !r.is_empty()) {
        return role;
    }
    match element.tag_name().as_str() {
        "A" => "link",
        "BUTTON" | "SUMMARY" => "button",
        "SELECT" => "combobox",
        "TEXTAREA" => "textbox",
        "INPUT" => match element.get_attribute("type").as_deref().unwrap_or("text") {
            "checkbox" => "checkbox",
            "radio" => "radio",
            "button" | "submit" | "reset" => "button",
            "range" => "slider",
            _ => "textbox",
        },
        _ if element.has_attribute("contenteditable") => "textbox",
        _ => "generic",
    }
    .into()
}
fn interactive(element: &Element) -> bool {
    element.matches("a[href],button,input,textarea,select,summary,[contenteditable]:not([contenteditable=false]),[tabindex]").unwrap_or(false)
        || matches!(role(element).as_str(), "button"|"link"|"checkbox"|"radio"|"switch"|"tab"|"menuitem"|"menuitemcheckbox"|"menuitemradio"|"option"|"combobox"|"textbox"|"searchbox"|"slider"|"spinbutton"|"treeitem")
}
fn append_children(node: &Node, stack: &mut Vec<Node>) {
    let children = node.child_nodes();
    for index in (0..children.length()).rev() {
        if let Some(child) = children.item(index) {
            stack.push(child);
        }
    }
}
// A bounded composed-tree walk reaches open shadow roots without iframe or origin access.
fn page_nodes(root: &Node) -> (Vec<Element>, bool) {
    let mut stack = vec![root.clone()];
    let mut elements = Vec::new();
    let mut visited = 0;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 20_000 {
            return (elements, true);
        }
        if let Some(element) = node.dyn_ref::<Element>() {
            if let Some(shadow) = element.shadow_root() {
                append_children(shadow.as_ref(), &mut stack);
            }
            elements.push(element.clone());
        }
        append_children(&node, &mut stack);
    }
    (elements, false)
}
// Omit a control's own text only when its complete normalized text is already its
// returned label. Never deduplicate unrelated equal words elsewhere on the page.
fn context_text(root: &Node, represented: &[Element]) -> String {
    let mut stack = vec![(root.clone(), false)];
    let mut text = String::new();
    let mut visited = 0;
    while let Some((node, boundary)) = stack.pop() {
        if boundary {
            text.push('\n');
            continue;
        }
        visited += 1;
        if visited > 20_000 || text.len() >= 64_000 {
            break;
        }
        if node.node_type() == Node::TEXT_NODE {
            let value = node.text_content().unwrap_or_default();
            if !value.is_empty() {
                text.push_str(&value);
            }
            continue;
        }
        if let Some(element) = node.dyn_ref::<Element>() {
            if matches!(element.tag_name().as_str(), "SCRIPT" | "STYLE" | "NOSCRIPT")
                || !style_visible(element, false)
            {
                continue;
            }
            if represented.iter().any(|item| item == element) {
                text.push('\n');
                continue;
            }
            let block = web_sys::window()
                .and_then(|w| w.get_computed_style(element).ok().flatten())
                .is_some_and(|style| {
                    matches!(
                        style
                            .get_property_value("display")
                            .unwrap_or_default()
                            .as_str(),
                        "block"
                            | "flex"
                            | "grid"
                            | "table"
                            | "table-row"
                            | "table-cell"
                            | "list-item"
                            | "flow-root"
                    )
                });
            if block || element.tag_name() == "BR" {
                text.push('\n');
            }
            if block {
                stack.push((node.clone(), true));
            }
            if let Some(shadow) = element.shadow_root() {
                let children = shadow.child_nodes();
                for index in (0..children.length()).rev() {
                    if let Some(child) = children.item(index) {
                        stack.push((child, false));
                    }
                }
            }
        }
        let children = node.child_nodes();
        for index in (0..children.length()).rev() {
            if let Some(child) = children.item(index) {
                stack.push((child, false));
            }
        }
    }
    truncate(
        text.lines()
            .map(normalize)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        16_000,
    )
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
    let (nodes, scan_limited) = page_nodes(doc.as_ref());
    CURRENT.with(|current| {
        let mut state=current.borrow_mut();
        if state.nonce.is_empty() {state.nonce=web_sys::window().unwrap().crypto()?.random_uuid();}
        state.epoch+=1;state.elements.clear();
        let mut elements=Vec::new(); let mut represented=Vec::new();
        for element in nodes {
            if elements.len()>=250 {break;}
            if !interactive(&element) || !visible(&element) || element.get_attribute("type").as_deref()==Some("hidden") {continue;}
            let label=label(&element);
            let kind=element.get_attribute("type").unwrap_or_default();
            let mut item=json!({"reference":format!("{}:{}:{}",state.nonce,state.epoch,state.elements.len()),"tag":element.tag_name().to_lowercase(),"label":label,"type":kind,"role":role(&element),"disabled":element.matches(":disabled").unwrap_or(false) || element.get_attribute("aria-disabled").as_deref()==Some("true")});
            for (attribute,key) in [("aria-checked","checked"),("aria-selected","selected"),("aria-expanded","expanded"),("aria-pressed","pressed"),("aria-readonly","readonly"),("aria-required","required")] {
                if let Some(value)=element.get_attribute(attribute) { item[key]=match value.as_str(){"true"=>json!(true),"false"=>json!(false),_=>json!(value)}; }
            }
            if element.has_attribute("readonly") {item["readonly"]=json!(true);}
            if element.has_attribute("required") {item["required"]=json!(true);}
            if let Some(description)=referenced_text(&element,"aria-describedby") {item["description"]=json!(truncate(description,500));}
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
            if !label.is_empty() && normalize(&element.text_content().unwrap_or_default()) == label {
                represented.push(element.clone());
            }
            elements.push(item);let identity=fingerprint(&element);state.elements.push((element,identity));
        }
        let body=doc.body().map(|b|truncate(b.inner_text(),16_000)).unwrap_or_default();
        let context = doc.body().map(|b| context_text(b.as_ref(), &represented)).unwrap_or_default();
        Ok(json!({"title":doc.title(),"url":doc.url()?,"text":body,"context":context,"elements":elements,"scan_limited":scan_limited,"scope":"top-level document and open shadow roots; visible interactive elements","untrusted_content":true}).to_string())
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
    if element.matches(":disabled").unwrap_or(false)
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
            if element.get_attribute("aria-readonly").as_deref() == Some("true") {
                return Err(err("Input is read-only"));
            }
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
            } else if element
                .get_attribute("contenteditable")
                .is_some_and(|s| matches!(s.as_str(), "true" | "plaintext-only" | ""))
            {
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
