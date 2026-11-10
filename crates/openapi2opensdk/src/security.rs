//! Security-scheme mapping — port of src/security.ts.

use serde_json::{Map, Value};

use crate::jsrt::{js_object_keys, screaming_snake_case};
use crate::model::Security;

/// Map one OpenAPI securityScheme object into the OpenSDK security shape.
pub fn map_security_scheme(
    value: Option<&Value>,
    scheme_name: Option<&str>,
    env_var: Option<&str>,
) -> Option<Security> {
    let scheme = value?.as_object()?;
    let scheme_type = scheme.get("type")?;

    let mut entry = Security {
        scheme_type: scheme_type.clone(),
        kind: "other".to_string(),
        scheme_name: scheme_name.map(|s| s.to_string()),
        env_var: env_var.map(|s| s.to_string()),
        scheme: None,
        bearer_format: None,
        location: None,
        name: None,
        flows: None,
        scopes: None,
    };

    match scheme_type.as_str() {
        Some("http") => {
            // JS `entry.scheme = scheme.scheme` — undefined assignment omits.
            entry.scheme = scheme.get("scheme").cloned();
            if let Some(bf) = scheme.get("bearerFormat") {
                // JS truthiness gate on bearerFormat.
                let truthy = match bf {
                    Value::String(s) => !s.is_empty(),
                    Value::Bool(b) => *b,
                    Value::Null => false,
                    Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
                    _ => true,
                };
                if truthy {
                    entry.bearer_format = Some(bf.clone());
                }
            }
            let s = scheme
                .get("scheme")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            entry.kind = if s == "basic" { "basic" } else { "bearer" }.to_string();
        }
        Some("apiKey") => {
            entry.location = scheme.get("in").cloned();
            entry.name = scheme.get("name").cloned();
            entry.kind = match scheme.get("in").and_then(|v| v.as_str()) {
                Some("query") => "apiKey-query",
                Some("cookie") => "apiKey-cookie",
                _ => "apiKey-header",
            }
            .to_string();
        }
        // RFC 6750: an OAuth 2.0 access token is sent as `Authorization: Bearer
        // <token>`, so at runtime an oauth2 scheme is a bearer credential. The
        // generated client does not run the flow; it sends the token it is
        // given. `scheme: "bearer"` keeps the identity `opensdk diff` compares
        // equal to an `http`/`bearer` scheme's.
        Some("oauth2") => {
            entry.kind = "bearer".to_string();
            entry.scheme = Some(Value::String("bearer".to_string()));
            let (flows, scopes) = oauth2_flows(scheme.get("flows"));
            entry.flows = flows;
            entry.scopes = scopes;
        }
        _ => {}
    }

    Some(entry)
}

/// The flow types (document order) and the merged scope catalogue of an
/// oauth2 scheme's `flows` object. A scope declared by several flows keeps the
/// first description. Each half is `None` when empty.
fn oauth2_flows(flows: Option<&Value>) -> (Option<Vec<String>>, Option<Map<String, Value>>) {
    let Some(flows) = flows.and_then(Value::as_object) else {
        return (None, None);
    };
    let mut names = Vec::new();
    let mut scopes = Map::new();
    for flow_name in js_object_keys(flows) {
        // `x-*` keys are specification extensions, not flows.
        if flow_name.starts_with("x-") {
            continue;
        }
        let Some(flow) = flows[flow_name].as_object() else {
            continue;
        };
        names.push(flow_name.clone());
        let Some(flow_scopes) = flow.get("scopes").and_then(Value::as_object) else {
            continue;
        };
        for scope in js_object_keys(flow_scopes) {
            if !scopes.contains_key(scope) {
                scopes.insert(scope.clone(), flow_scopes[scope].clone());
            }
        }
    }
    (
        (!names.is_empty()).then_some(names),
        (!scopes.is_empty()).then_some(scopes),
    )
}

/// The document's `components.securitySchemes` with a best-guess env var.
pub fn security_schemes(doc: &Value, sdk_name: &str, auth_env_var: Option<&str>) -> Vec<Security> {
    let Some(schemes) = doc
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .and_then(|s| s.as_object())
    else {
        return vec![];
    };

    let default_env = auth_env_var
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}_API_KEY", screaming_snake_case(sdk_name)));

    js_object_keys(schemes)
        .into_iter()
        .filter_map(|name| map_security_scheme(schemes.get(name), Some(name), Some(&default_env)))
        .collect()
}

/// Map a per-operation `security` requirement list into the OpenSDK shape.
pub fn security_requirements(doc: &Value, requirements: &[Value]) -> Vec<Security> {
    let empty = serde_json::Map::new();
    let schemes = doc
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .and_then(|s| s.as_object())
        .unwrap_or(&empty);

    let mut out = Vec::new();
    for requirement in requirements {
        let Some(req) = requirement.as_object() else {
            continue;
        };
        for scheme_name in js_object_keys(req) {
            if let Some(entry) =
                map_security_scheme(schemes.get(scheme_name), Some(scheme_name), None)
            {
                out.push(entry);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(scheme: Value) -> Value {
        let entry = map_security_scheme(Some(&scheme), Some("auth"), Some("ACME_API_KEY"))
            .expect("a scheme with a type maps");
        serde_json::to_value(entry).unwrap()
    }

    #[test]
    fn oauth2_with_an_authorization_code_flow_is_a_bearer_scheme() {
        let entry = map(json!({
            "type": "oauth2",
            "flows": {
                "authorizationCode": {
                    "authorizationUrl": "https://auth.example.com/authorize",
                    "tokenUrl": "https://auth.example.com/token",
                    "scopes": { "widgets:read": "Read widgets", "widgets:write": "Write widgets" }
                }
            }
        }));
        assert_eq!(
            entry,
            json!({
                "type": "oauth2",
                "kind": "bearer",
                "schemeName": "auth",
                "envVar": "ACME_API_KEY",
                "scheme": "bearer",
                "flows": ["authorizationCode"],
                "scopes": { "widgets:read": "Read widgets", "widgets:write": "Write widgets" }
            })
        );
        // Key order is part of the IR's byte shape: the new keys come last.
        let keys: Vec<&String> = entry.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "type",
                "kind",
                "schemeName",
                "envVar",
                "scheme",
                "flows",
                "scopes"
            ]
        );
    }

    #[test]
    fn oauth2_merges_scopes_across_flows_first_description_wins() {
        let entry = map(json!({
            "type": "oauth2",
            "flows": {
                "clientCredentials": {
                    "tokenUrl": "https://auth.example.com/token",
                    "scopes": { "b": "B from client credentials", "a": "A" }
                },
                "authorizationCode": {
                    "authorizationUrl": "https://auth.example.com/authorize",
                    "tokenUrl": "https://auth.example.com/token",
                    "scopes": { "b": "B from authorization code", "c": "C" }
                },
                "x-vendor": { "scopes": { "z": "an extension, not a flow" } }
            }
        }));
        assert_eq!(entry["kind"], json!("bearer"));
        assert_eq!(
            entry["flows"],
            json!(["clientCredentials", "authorizationCode"])
        );
        assert_eq!(
            entry["scopes"],
            json!({ "b": "B from client credentials", "a": "A", "c": "C" })
        );
        let order: Vec<&String> = entry["scopes"].as_object().unwrap().keys().collect();
        assert_eq!(order, ["b", "a", "c"]);
    }

    #[test]
    fn oauth2_without_flows_or_scopes_omits_both_keys() {
        let no_flows = json!({ "type": "oauth2" });
        let empty_flows = json!({ "type": "oauth2", "flows": {} });
        let no_scopes = json!({ "type": "oauth2", "flows": { "implicit": {
            "authorizationUrl": "https://auth.example.com/authorize",
            "scopes": {}
        } } });
        for scheme in [&no_flows, &empty_flows, &no_scopes] {
            let entry = map(scheme.clone());
            assert_eq!(entry["kind"], json!("bearer"), "{scheme}");
            assert_eq!(entry["scheme"], json!("bearer"), "{scheme}");
            assert!(entry.get("scopes").is_none(), "{scheme}");
        }
        assert!(map(no_flows).get("flows").is_none());
        assert!(map(empty_flows).get("flows").is_none());
        assert_eq!(map(no_scopes)["flows"], json!(["implicit"]));
    }

    #[test]
    fn http_bearer_is_unchanged() {
        assert_eq!(
            map(json!({ "type": "http", "scheme": "bearer", "bearerFormat": "JWT" })),
            json!({
                "type": "http",
                "kind": "bearer",
                "schemeName": "auth",
                "envVar": "ACME_API_KEY",
                "scheme": "bearer",
                "bearerFormat": "JWT"
            })
        );
        assert_eq!(
            map(json!({ "type": "http", "scheme": "basic" }))["kind"],
            json!("basic")
        );
    }

    #[test]
    fn api_key_is_unchanged() {
        assert_eq!(
            map(json!({ "type": "apiKey", "in": "header", "name": "X-Api-Key" })),
            json!({
                "type": "apiKey",
                "kind": "apiKey-header",
                "schemeName": "auth",
                "envVar": "ACME_API_KEY",
                "in": "header",
                "name": "X-Api-Key"
            })
        );
        assert_eq!(
            map(json!({ "type": "apiKey", "in": "query", "name": "key" }))["kind"],
            json!("apiKey-query")
        );
        assert_eq!(
            map(json!({ "type": "apiKey", "in": "cookie", "name": "sid" }))["kind"],
            json!("apiKey-cookie")
        );
    }

    #[test]
    fn an_unrecognized_type_stays_other() {
        let entry = map(json!({ "type": "mutualTLS" }));
        assert_eq!(entry["kind"], json!("other"));
        assert!(entry.get("scheme").is_none());
    }

    #[test]
    fn the_document_level_list_maps_oauth2_with_the_default_env_var() {
        let doc = json!({ "components": { "securitySchemes": {
            "oauth": { "type": "oauth2", "flows": { "clientCredentials": {
                "tokenUrl": "https://auth.example.com/token", "scopes": { "read": "Read" } } } }
        } } });
        let entries = serde_json::to_value(security_schemes(&doc, "Acme", None)).unwrap();
        assert_eq!(entries[0]["kind"], json!("bearer"));
        assert_eq!(entries[0]["schemeName"], json!("oauth"));
        assert_eq!(entries[0]["envVar"], json!("ACME_API_KEY"));
    }
}
