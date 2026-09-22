//! Lenient JSON extraction for providers without enforced structured output.

use serde_json::Value;

/// Parse a JSON object from model text: plain JSON, fenced ```json blocks, or
/// the outermost `{...}` span inside prose.
pub fn extract_object(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    if let Some(start) = t.find("```") {
        let rest = &t[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        if let Some(end) = rest.find("```")
            && let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(rest[..end].trim())
        {
            return Some(v);
        }
    }
    let (s, e) = (t.find('{')?, t.rfind('}')?);
    if e <= s {
        return None;
    }
    match serde_json::from_str::<Value>(&t[s..=e]) {
        Ok(v @ Value::Object(_)) => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants() {
        assert!(extract_object("{\"a\":1}").is_some());
        assert!(extract_object("Sure!\n```json\n{\"a\": 1}\n```\n").is_some());
        assert!(extract_object("Here you go: {\"a\": {\"b\": 2}} hope it helps").is_some());
        assert!(extract_object("no json").is_none());
        assert!(extract_object("[1,2]").is_none());
    }
}
