//! Presentation only: DOM access and element identity checks remain in Wasm.
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    #[default]
    Compact,
    Full,
}

#[derive(Clone, Copy, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRequest {
    pub tab_id: u32,
    /// text (default): compact readable lines; json: the compatible structured response.
    #[serde(default)]
    pub format: Format,
    /// Optional case-insensitive text to find in page text, labels, values or options.
    pub query: Option<String>,
    /// compact (default) or full field values/options. Prefer full with a query.
    #[serde(default)]
    pub detail: Detail,
    /// Text characters to return: default 4000 compact / 16000 full; 0 for controls only.
    pub max_chars: Option<usize>,
    /// Controls to return: default 250 in both modes; use a lower limit to page.
    pub max_elements: Option<usize>,
    /// Start character in the matching text; use next_text_offset from the previous result.
    #[serde(default)]
    pub text_offset: usize,
    /// Start control in the matching list; use next_element_offset from the previous result.
    #[serde(default)]
    pub element_offset: usize,
}

impl SnapshotRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_chars.is_some_and(|n| n > 16_000)
            || self.max_elements.is_some_and(|n| n == 0 || n > 250)
            || self.text_offset > 32_000
            || self.element_offset > 250
            || self.query.as_ref().is_some_and(|s| s.chars().count() > 200)
        {
            return Err("Snapshot limits: max_chars 0..16000, max_elements 1..250, text_offset 0..32000, element_offset 0..250, query up to 200 characters.".into());
        }
        Ok(())
    }
}

struct TabReferences {
    generation: u64,
    elements: HashMap<String, String>,
}

pub struct References {
    next_reference: u64,
    generation: u64,
    tabs: HashMap<u32, TabReferences>,
}

impl Default for References {
    fn default() -> Self {
        Self {
            // Decimal references tokenize efficiently. A random 48-bit starting
            // point preserves the old namespace's restart separation; never
            // reuse an identifier within this server instance.
            next_reference: rand::random::<u64>() & ((1u64 << 48) - 1),
            generation: 0,
            tabs: HashMap::new(),
        }
    }
}

impl References {
    pub fn resolve(&self, tab_id: u32, reference: &str) -> Result<String, String> {
        self.tabs
            .get(&tab_id)
            .and_then(|tab| tab.elements.get(reference))
            .cloned()
            .ok_or_else(|| {
                "Unknown or stale element reference. Take a fresh snapshot of this tab.".into()
            })
    }

    pub fn present(&mut self, raw: Value, request: &SnapshotRequest) -> Value {
        self.generation += 1;
        let full = matches!(request.detail, Detail::Full);
        let max_chars = request
            .max_chars
            .unwrap_or(if full { 16_000 } else { 4_000 });
        let max_elements = request.max_elements.unwrap_or(250);
        let query = request.query.as_deref().unwrap_or("").trim().to_lowercase();
        let context = !full && matches!(request.format, Format::Text) && query.is_empty();
        let source_text = if context {
            raw["context"].as_str().or_else(|| raw["text"].as_str())
        } else {
            raw["text"].as_str()
        }
        .unwrap_or_default();
        let filtered_text = matching_text(source_text, &query);
        let text_total = filtered_text.chars().count();
        let text: String = filtered_text
            .chars()
            .skip(request.text_offset)
            .take(max_chars)
            .collect();
        let text_end = request.text_offset + text.chars().count();
        let source_elements = raw["elements"].as_array().cloned().unwrap_or_default();
        let matching: Vec<_> = source_elements
            .iter()
            .filter(|e| matches_element(e, &query))
            .collect();
        let mut references = HashMap::new();
        let mut elements = Vec::new();
        for element in matching
            .iter()
            .skip(request.element_offset)
            .take(max_elements)
        {
            let mut element = (*element).clone();
            let Some(object) = element.as_object_mut() else {
                continue;
            };
            let Some(original) = object
                .get("reference")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            self.next_reference += 1;
            let reference = format!("e{}", self.next_reference);
            object.insert("reference".into(), json!(reference));
            if !full {
                compact_element(object);
            }
            references.insert(reference, original);
            elements.push(element);
        }
        let element_end = request.element_offset + elements.len();
        let mut result = json!({
            "title":raw["title"], "url":raw["url"], "text":text, "elements":elements,
            "text_total":text_total, "elements_total":matching.len(), "untrusted_content":true
        });
        if context && raw["context"].is_string() {
            result["text_source"] =
                json!("context; control text is represented in the control list");
        }
        if !query.is_empty() {
            result["query"] = json!(request.query.as_deref().unwrap_or_default().trim());
        }
        if max_chars > 0 && text_end < text_total {
            result["next_text_offset"] = json!(text_end);
        }
        if max_chars == 0 && text_total > 0 {
            result["text_omitted"] = json!(true);
        }
        if element_end < matching.len() {
            result["next_element_offset"] = json!(element_end);
        }
        // The existing extension caps extraction before this presentation layer.
        // Do not imply pagination can recover data beyond those source limits.
        let mut source_limits = Vec::new();
        if source_text.chars().count() >= 16_000
            || raw["text"]
                .as_str()
                .is_some_and(|s| s.chars().count() >= 16_000)
        {
            source_limits.push("text");
        }
        if source_elements.len() >= 250 {
            source_limits.push("elements");
        }
        if raw["scan_limited"].as_bool() == Some(true) {
            source_limits.push("dom");
        }
        if source_elements
            .iter()
            .any(|e| e["options"].as_array().is_some_and(|o| o.len() >= 80))
        {
            source_limits.push("options");
        }
        if !source_limits.is_empty() {
            result["source_limit_possible"] = json!(source_limits);
        }
        self.tabs.insert(
            request.tab_id,
            TabReferences {
                generation: self.generation,
                elements: references,
            },
        );
        if self.tabs.len() > 64 {
            if let Some(oldest) = self
                .tabs
                .iter()
                .min_by_key(|(_, t)| t.generation)
                .map(|(id, _)| *id)
            {
                self.tabs.remove(&oldest);
            }
        }
        result
    }
}

/// Render all extracted action targets and their state without repeating JSON keys.
/// JSON remains available for clients that parse the original response structure.
pub fn render_text(snapshot: &Value) -> String {
    use std::fmt::Write;
    let quote = |value: &Value| value.to_string();
    let mut output = format!(
        "Page: {}\nURL: {}\nUntrusted page content.\n",
        quote(&snapshot["title"]),
        quote(&snapshot["url"])
    );
    if let Some(text) = snapshot["text"].as_str().filter(|s| !s.is_empty()) {
        output.push_str(if snapshot.get("text_source").is_some() {
            "Text (control labels below):\n"
        } else {
            "Text:\n"
        });
        output.push_str(text);
        output.push('\n');
    }
    output.push_str("Controls:\n");
    if let Some(elements) = snapshot["elements"].as_array() {
        for element in elements {
            let role = element["role"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    match (
                        element["tag"].as_str().unwrap_or(""),
                        element["type"].as_str().unwrap_or(""),
                    ) {
                        ("a", _) => "link",
                        ("textarea", _) => "textbox",
                        ("select", _) => "combobox",
                        ("input", "checkbox") => "checkbox",
                        ("input", "radio") => "radio",
                        ("input", "button" | "submit" | "reset") | ("button" | "summary", _) => {
                            "button"
                        }
                        ("input", _) => "textbox",
                        (tag, _) => tag,
                    }
                });
            let _ = write!(
                output,
                "- {} {} [ref={}]",
                role,
                quote(&element["label"]),
                element["reference"].as_str().unwrap_or("")
            );
            for key in [
                "type",
                "value",
                "checked",
                "disabled",
                "readonly",
                "required",
                "expanded",
                "selected",
                "pressed",
                "description",
                "href",
                "value_truncated",
                "options_truncated",
                "options_count",
            ] {
                if let Some(value) = element.get(key) {
                    if key == "type" && value.as_str() == Some("") {
                        continue;
                    }
                    let _ = write!(output, " {key}={value}");
                }
            }
            if let Some(options) = element.get("options") {
                let _ = write!(output, " options={options}");
            }
            output.push('\n');
        }
    }
    for key in [
        "query",
        "next_text_offset",
        "next_element_offset",
        "text_omitted",
        "source_limit_possible",
    ] {
        if let Some(value) = snapshot.get(key) {
            let _ = writeln!(output, "{key}={value}");
        }
    }
    output
}

fn compact_element(element: &mut Map<String, Value>) {
    if element.get("disabled") == Some(&json!(false)) {
        element.remove("disabled");
    }
    if element.get("type").and_then(Value::as_str) == Some("") {
        element.remove("type");
    }
    if let Some(value) = element.get("value").and_then(Value::as_str) {
        if value.chars().count() > 160 {
            element.insert(
                "value".into(),
                json!(value.chars().take(160).collect::<String>()),
            );
            element.insert("value_truncated".into(), json!(true));
        }
    }
    if let Some(options) = element.get("options").and_then(Value::as_array) {
        let count = options.len();
        let compact: Vec<_> = options
            .iter()
            .filter(|o| o["value"].as_str().is_some_and(|v| v.len() <= 256))
            .take(5)
            .cloned()
            .collect();
        if compact.len() < count {
            element.insert("options_count".into(), json!(count));
            element.insert("options_truncated".into(), json!(true));
        }
        element.insert("options".into(), json!(compact));
    }
}

fn matches_element(element: &Value, query: &str) -> bool {
    query.is_empty()
        || ["label", "value", "href"].iter().any(|key| {
            element[key]
                .as_str()
                .is_some_and(|s| s.to_lowercase().contains(query))
        })
        || element["options"].as_array().is_some_and(|options| {
            options.iter().any(|o| {
                ["label", "value"].iter().any(|key| {
                    o[key]
                        .as_str()
                        .is_some_and(|s| s.to_lowercase().contains(query))
                })
            })
        })
}

fn matching_text(source: &str, query: &str) -> String {
    if query.is_empty() {
        return source.to_string();
    }
    let lines: Vec<_> = source.lines().collect();
    let mut included = vec![false; lines.len()];
    for (i, line) in lines.iter().enumerate() {
        if line.to_lowercase().contains(query) {
            for item in &mut included[i.saturating_sub(1)..=(i + 1).min(lines.len() - 1)] {
                *item = true;
            }
        }
    }
    let mut output = String::new();
    let mut previous = None;
    for (i, line) in lines.iter().enumerate().filter(|(i, _)| included[*i]) {
        if let Some(p) = previous {
            output.push_str(if i > p + 1 { "\n…\n" } else { "\n" });
        }
        output.push_str(line);
        previous = Some(i);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: Value) -> SnapshotRequest {
        serde_json::from_value(value).unwrap()
    }
    fn page(text: &str) -> Value {
        json!({"title":"Local fixture", "url":"https://fixture.example/", "text":text,
            "elements":[{"reference":"original:1:0", "tag":"button", "label":"Save", "type":"", "disabled":false}]})
    }

    #[test]
    fn text_pages_use_characters_and_search_keeps_context() {
        let mut refs = References::default();
        let raw = page(
            "Hidden prefix\nOther content\nBefore\nΚΌΣΜΟΣ 🧪\nAfter\nUnrelated\nHidden suffix",
        );
        let params = request(json!({"tab_id":1,"query":"κόσμος","max_chars":10}));
        let first = refs.present(raw.clone(), &params);
        assert_eq!(first["text"], "Before\nΚΌΣ");
        assert_eq!(first["next_text_offset"], 10);
        let second = refs.present(
            raw,
            &request(json!({"tab_id":1,"query":"κόσμος","max_chars":100,"text_offset":10})),
        );
        assert_eq!(second["text"], "ΜΟΣ 🧪\nAfter");
        assert!(second.get("next_text_offset").is_none());
        assert_eq!(first["elements_total"], 0);
    }

    #[test]
    fn references_are_tab_scoped_and_replaceable_with_bounded_retention() {
        let mut refs = References::default();
        let one = refs.present(page("One"), &request(json!({"tab_id":1})));
        let old = one["elements"][0]["reference"].as_str().unwrap();
        assert_eq!(refs.resolve(1, old).unwrap(), "original:1:0");
        assert!(refs.resolve(2, old).is_err());
        refs.present(page("Two"), &request(json!({"tab_id":2})));
        assert!(refs.resolve(1, old).is_ok());
        refs.present(page("Refresh"), &request(json!({"tab_id":1})));
        assert!(refs.resolve(1, old).is_err());
        for tab in 3..=66 {
            refs.present(page("New tab"), &request(json!({"tab_id":tab})));
        }
        assert_eq!(refs.tabs.len(), 64);
        assert!(!refs.tabs.contains_key(&1));
    }

    #[test]
    fn large_option_values_require_explicit_full_detail() {
        let mut refs = References::default();
        let mut raw = page("Plan");
        raw["elements"][0]["options"] = json!([{"label":"Long option", "value":"v".repeat(500)}]);
        raw["elements"][0]["value"] = json!("🧪".repeat(200));
        raw["elements"][0]["disabled"] = json!(true);
        let compact = refs.present(raw.clone(), &request(json!({"tab_id":1})));
        assert_eq!(
            compact["elements"][0]["value"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            160
        );
        assert_eq!(compact["elements"][0]["value_truncated"], true);
        assert_eq!(compact["elements"][0]["disabled"], true);
        assert_eq!(compact["elements"][0]["options_truncated"], true);
        let full = refs.present(raw, &request(json!({"tab_id":1,"detail":"full"})));
        assert_eq!(full["elements"][0]["options"][0]["value"], "v".repeat(500));
        assert_eq!(
            full["elements"][0]["value"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            200
        );
    }

    #[test]
    fn text_context_never_changes_json_full_reads_and_keeps_target_state() {
        let mut refs = References::default();
        let mut raw = page("Save\nApproved\nSave");
        raw["context"] = json!("Approved\nSave");
        raw["elements"][0]["role"] = json!("checkbox");
        raw["elements"][0]["checked"] = json!(false);
        raw["elements"][0]["readonly"] = json!(true);
        raw["scan_limited"] = json!(true);
        let compact = refs.present(raw.clone(), &request(json!({"tab_id":1})));
        assert_eq!(compact["text"], "Approved\nSave");
        let rendered = render_text(&compact);
        assert!(rendered.contains("checkbox \"Save\" [ref="));
        assert!(rendered.contains("checked=false"));
        assert!(rendered.contains("readonly=true"));
        assert!(rendered.contains("source_limit_possible=[\"dom\"]"));
        assert_eq!(
            refs.present(raw.clone(), &request(json!({"tab_id":1,"format":"json"})))["text"],
            raw["text"]
        );
        assert_eq!(
            refs.present(raw.clone(), &request(json!({"tab_id":1,"detail":"full"})))["text"],
            raw["text"]
        );
        assert_eq!(
            refs.present(raw.clone(), &request(json!({"tab_id":1,"query":"Save"})))["text"],
            raw["text"]
        );
    }
}
