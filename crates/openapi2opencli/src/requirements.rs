//! Per-command security requirements (`includeSecurity`).
//!
//! `security.rs` maps the document's security SCHEMES (how a credential is
//! sent); this module reads which schemes — and which scopes — an operation
//! REQUIRES, and describes each one with that same mapping, so a command's
//! `x-openapi.security` entries are `XOpenApiSecurity` objects like the root's.

use serde_json::Value;

use crate::jsrt::js_object_keys;
use crate::model::{CommandSecurity, Security};
use crate::security::security_schemes_to_x_openapi;

/// Every security scheme of the document by its `components.securitySchemes`
/// key, each described by the root mapping (`security_schemes_to_x_openapi`).
///
/// The mapping returns its entries in key order, skipping a scheme without a
/// `type`; the keys are enumerated the same way and paired with them.
pub fn scheme_descriptors(
    doc: &Value,
    cli_name: &str,
    auth_env_var: Option<&str>,
) -> Vec<(String, Security)> {
    let keys: Vec<String> = doc
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .and_then(Value::as_object)
        .map(|schemes| {
            js_object_keys(schemes)
                .into_iter()
                .filter(|k| schemes[k.as_str()].get("type").is_some())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mapped = security_schemes_to_x_openapi(doc, cli_name, auth_env_var);
    debug_assert_eq!(keys.len(), mapped.len(), "one descriptor per typed scheme");
    keys.into_iter().zip(mapped).collect()
}

/// The requirements that apply to `operation`: its own `security` when it has
/// the key (an empty list there means "no credential needed"), else the
/// document's. Each requirement object contributes one entry per scheme, in
/// document order, with the scopes copied as written. An empty requirement
/// object (`{}`, "optional") contributes nothing, and a scheme name that is not
/// a key of `components.securitySchemes` is skipped with a warning — there is
/// no descriptor to give it.
pub fn operation_requirements(
    doc: &Value,
    operation: &Value,
    schemes: &[(String, Security)],
) -> Vec<CommandSecurity> {
    let reqs = operation.get("security").or_else(|| doc.get("security"));
    let Some(reqs) = reqs.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for req in reqs {
        let Some(map) = req.as_object() else {
            continue;
        };
        for name in js_object_keys(map) {
            let Some((_, scheme)) = schemes.iter().find(|(key, _)| key == name) else {
                eprintln!(
                    "warning: a security requirement names `{name}`, which is not a security \\
                     scheme of the document; it is left out of the command's security"
                );
                continue;
            };
            let scopes = map[name.as_str()]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.push(CommandSecurity {
                scheme: scheme.clone(),
                reference: name.clone(),
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

    fn doc(security: Value) -> Value {
        json!({
            "security": security,
            "components": { "securitySchemes": {
                "token": { "type": "http", "scheme": "bearer" },
                "key": { "type": "apiKey", "in": "header", "name": "X-Api-Key" },
                "untyped": { "description": "no type, so no descriptor" }
            } }
        })
    }

    fn reqs(doc: &Value, op: &Value) -> Value {
        let schemes = scheme_descriptors(doc, "acme", None);
        serde_json::to_value(operation_requirements(doc, op, &schemes)).unwrap()
    }

    #[test]
    fn each_requirement_carries_the_schemes_descriptor_its_key_and_its_scopes() {
        let d = doc(json!([{ "token": ["widgets:read"] }]));
        let op = json!({ "security": [{ "token": ["widgets:write"] }, { "key": [] }] });
        assert_eq!(
            reqs(&d, &op),
            json!([
                {
                    "type": "http",
                    "kind": "bearer",
                    "envVar": "ACME_API_KEY",
                    "scheme": "bearer",
                    "ref": "token",
                    "scopes": ["widgets:write"]
                },
                {
                    "type": "apiKey",
                    "kind": "apiKey-header",
                    "envVar": "ACME_API_KEY",
                    "in": "header",
                    "name": "X-Api-Key",
                    "ref": "key",
                    "scopes": []
                }
            ])
        );
        // No `security` key on the operation: the document's applies.
        assert_eq!(reqs(&d, &json!({}))[0]["scopes"], json!(["widgets:read"]));
    }

    #[test]
    fn an_empty_list_on_the_operation_means_none_and_does_not_inherit() {
        let d = doc(json!([{ "token": [] }]));
        assert_eq!(reqs(&d, &json!({ "security": [] })), json!([]));
        assert_eq!(reqs(&d, &json!({ "security": [{}] })), json!([]));
        assert_eq!(reqs(&json!({}), &json!({})), json!([]));
    }

    #[test]
    fn an_undeclared_scheme_name_is_left_out() {
        let d = doc(json!([{ "nope": [] }, { "untyped": [] }, { "key": [] }]));
        let out = reqs(&d, &json!({}));
        assert_eq!(out.as_array().unwrap().len(), 1);
        assert_eq!(out[0]["ref"], json!("key"));
    }
}
