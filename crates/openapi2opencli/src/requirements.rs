//! Per-command security requirements (`includeSecurity`).
//!
//! Kept apart from `security.rs`, which maps the document's security SCHEMES
//! (how a credential is sent); this module reads which schemes — and which
//! scopes — an operation REQUIRES.

use serde_json::Value;

use crate::jsrt::js_object_keys;
use crate::model::SecurityRequirement;

/// The requirements that apply to `operation`: its own `security` when it has
/// the key (an empty list there means "no credential needed"), else the
/// document's. Each requirement object contributes one entry per scheme, in
/// document order; scope lists are copied as written. An empty requirement
/// object (`{}`, "optional") contributes nothing.
pub fn operation_requirements(doc: &Value, operation: &Value) -> Vec<SecurityRequirement> {
    let reqs = operation.get("security").or_else(|| doc.get("security"));
    let Some(reqs) = reqs.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for req in reqs {
        let Some(map) = req.as_object() else {
            continue;
        };
        for scheme in js_object_keys(map) {
            let scopes = map[scheme.as_str()]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.push(SecurityRequirement {
                scheme: scheme.clone(),
                scopes,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(scheme: &str, scopes: &[&str]) -> SecurityRequirement {
        SecurityRequirement {
            scheme: scheme.to_string(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn the_operation_overrides_the_document() {
        let doc = json!({ "security": [{ "oauth": ["widgets:read"] }] });
        let op = json!({ "security": [{ "oauth": ["widgets:write"] }, { "key": [] }] });
        assert_eq!(
            operation_requirements(&doc, &op),
            vec![req("oauth", &["widgets:write"]), req("key", &[])]
        );
        assert_eq!(
            operation_requirements(&doc, &json!({})),
            vec![req("oauth", &["widgets:read"])]
        );
    }

    #[test]
    fn an_empty_list_on_the_operation_means_none_and_does_not_inherit() {
        let doc = json!({ "security": [{ "oauth": [] }] });
        assert!(operation_requirements(&doc, &json!({ "security": [] })).is_empty());
        assert!(operation_requirements(&doc, &json!({ "security": [{}] })).is_empty());
        assert!(operation_requirements(&json!({}), &json!({})).is_empty());
    }
}
