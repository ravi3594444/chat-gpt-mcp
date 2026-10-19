use serde_json::Value;

/// Validate at the browser boundary as well as at the MCP boundary.
pub fn check_command(input: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(input).map_err(|_| "Invalid command JSON")?;
    let action = value["action"].as_str().ok_or("Missing action")?;
    if !matches!(
        action,
        "tabs" | "snapshot" | "click" | "fill" | "select" | "scroll" | "navigate" | "screenshot"
    ) {
        return Err("Unsupported browser action".into());
    }
    if action != "tabs"
        && value["tab_id"]
            .as_u64()
            .filter(|id| *id > 0 && *id <= i32::MAX as u64)
            .is_none()
    {
        return Err("A valid tab_id is required".into());
    }
    if matches!(action, "click" | "fill" | "select") {
        let reference = value["reference"]
            .as_str()
            .ok_or("Use a reference from a fresh snapshot")?;
        if reference.is_empty() || reference.len() > 120 {
            return Err("Invalid element reference".into());
        }
    }
    if action == "fill"
        && value["text"]
            .as_str()
            .filter(|s| s.len() <= 20_000)
            .is_none()
    {
        return Err("Text must be a string under 20,000 bytes".into());
    }
    if action == "select"
        && value["value"]
            .as_str()
            .filter(|s| s.len() <= 2_000)
            .is_none()
    {
        return Err("A select value is required".into());
    }
    if action == "navigate" {
        let url = url::Url::parse(value["url"].as_str().ok_or("Missing URL")?)
            .map_err(|_| "Invalid URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Only HTTP(S) URLs without embedded credentials are supported".into());
        }
    }
    if action == "scroll"
        && value["pixels"]
            .as_i64()
            .filter(|n| n.unsigned_abs() <= 5_000)
            .is_none()
    {
        return Err("Scroll pixels must be between -5000 and 5000".into());
    }
    Ok(value)
}

#[cfg(target_arch = "wasm32")]
mod dom;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn validate_command(input: &str) -> Result<String, wasm_bindgen::JsValue> {
    check_command(input)
        .map(|v| v.to_string())
        .map_err(|e| wasm_bindgen::JsValue::from_str(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_privileged_and_executable_navigation() {
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "chrome://settings",
            "https://name:secret@example.com",
        ] {
            assert!(check_command(
                &serde_json::json!({"action":"navigate","tab_id":1,"url":url}).to_string()
            )
            .is_err());
        }
    }
    #[test]
    fn validates_real_action_boundaries() {
        assert!(check_command(
            r#"{"action":"fill","tab_id":1,"reference":"r:1:2","text":"hello"}"#
        )
        .is_ok());
        assert!(check_command(r#"{"action":"eval","tab_id":1}"#).is_err());
        assert!(check_command(r#"{"action":"scroll","tab_id":1,"pixels":6000}"#).is_err());
        assert!(check_command(r#"{"action":"click","tab_id":0,"reference":"x"}"#).is_err());
    }
}
