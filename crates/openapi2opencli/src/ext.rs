//! Reading vendor extensions by a dotted path — shared by `namesFrom` (on an
//! operation) and `headersFromExtension` (on `info`, then the document root).

use serde_json::Value;

/// The value at `path` on `owner`.
///
/// The whole string is tried as ONE key first, so an extension whose own name
/// contains a dot (`x-a.b`) is found; then `path` is walked segment by segment
/// (`x-docs.path` → `owner["x-docs"]["path"]`). An empty path finds nothing.
pub fn lookup<'a>(owner: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return None;
    }
    if let Some(v) = owner.get(path) {
        return Some(v);
    }
    let mut cur = owner;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// A scalar rendered as header/name text: a string as is, a number or boolean
/// through its JSON spelling. Objects, arrays and `null` have no text.
pub fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_literal_key_wins_over_the_dotted_walk() {
        let owner = json!({ "x-a.b": "literal", "x-a": { "b": "walked" } });
        assert_eq!(lookup(&owner, "x-a.b"), Some(&json!("literal")));
        let owner = json!({ "x-a": { "b": "walked" } });
        assert_eq!(lookup(&owner, "x-a.b"), Some(&json!("walked")));
        assert_eq!(lookup(&owner, "x-a.c"), None);
        assert_eq!(lookup(&owner, ""), None);
    }

    #[test]
    fn only_scalars_have_text() {
        assert_eq!(scalar_text(&json!("v")).as_deref(), Some("v"));
        assert_eq!(scalar_text(&json!(3)).as_deref(), Some("3"));
        assert_eq!(scalar_text(&json!(true)).as_deref(), Some("true"));
        assert_eq!(scalar_text(&json!({})), None);
        assert_eq!(scalar_text(&Value::Null), None);
    }
}
