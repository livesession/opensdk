//! securitySchemesToXOpenApi — port of security.ts.

use serde_json::Value;

use crate::jsrt::{js_object_keys, screaming_snake_case};
use crate::model::Security;

pub fn security_schemes_to_x_openapi(
    doc: &Value,
    cli_name: &str,
    auth_env_var: Option<&str>,
) -> Vec<Security> {
    let Some(schemes) = doc
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .and_then(|s| s.as_object())
    else {
        return vec![];
    };

    let default_env = auth_env_var
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}_API_KEY", screaming_snake_case(cli_name)));

    let mut out = Vec::new();
    // JS `for (const value of Object.values(schemes))` — value order.
    for key in js_object_keys(schemes) {
        let scheme = &schemes[key];
        let Some(scheme_type) = scheme.get("type") else {
            continue;
        };

        let mut entry = Security {
            scheme_type: scheme_type.clone(),
            kind: "other".to_string(),
            env_var: default_env.clone(),
            scheme: None,
            bearer_format: None,
            location: None,
            name: None,
        };

        match scheme_type.as_str() {
            Some("http") => {
                entry.scheme = scheme.get("scheme").cloned();
                if let Some(bf) = scheme.get("bearerFormat") {
                    let truthy = crate::jsrt::truthy(Some(bf));
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
            // RFC 6750: an OAuth 2.0 access token is sent as `Authorization:
            // Bearer <token>`, so the generated CLI reads it from the same
            // variable and attaches it the way it attaches an `http`/`bearer`
            // credential. The CLI does not run the flow.
            Some("oauth2") => {
                entry.scheme = Some(Value::String("bearer".to_string()));
                entry.kind = "bearer".to_string();
            }
            _ => {}
        }

        out.push(entry);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map_one(scheme: Value) -> Value {
        let doc = json!({ "components": { "securitySchemes": { "auth": scheme } } });
        let out = security_schemes_to_x_openapi(&doc, "acme", None);
        assert_eq!(out.len(), 1);
        serde_json::to_value(&out[0]).unwrap()
    }

    #[test]
    fn oauth2_with_an_authorization_code_flow_is_a_bearer_scheme() {
        assert_eq!(
            map_one(json!({
                "type": "oauth2",
                "flows": { "authorizationCode": {
                    "authorizationUrl": "https://auth.example.com/authorize",
                    "tokenUrl": "https://auth.example.com/token",
                    "scopes": { "widgets:read": "Read widgets" }
                } }
            })),
            json!({
                "type": "oauth2",
                "kind": "bearer",
                "envVar": "ACME_API_KEY",
                "scheme": "bearer"
            })
        );
    }

    #[test]
    fn http_bearer_is_unchanged() {
        assert_eq!(
            map_one(json!({ "type": "http", "scheme": "bearer", "bearerFormat": "JWT" })),
            json!({
                "type": "http",
                "kind": "bearer",
                "envVar": "ACME_API_KEY",
                "scheme": "bearer",
                "bearerFormat": "JWT"
            })
        );
    }

    #[test]
    fn api_key_is_unchanged() {
        assert_eq!(
            map_one(json!({ "type": "apiKey", "in": "header", "name": "X-Api-Key" })),
            json!({
                "type": "apiKey",
                "kind": "apiKey-header",
                "envVar": "ACME_API_KEY",
                "in": "header",
                "name": "X-Api-Key"
            })
        );
    }

    #[test]
    fn an_unrecognized_type_stays_other() {
        assert_eq!(
            map_one(json!({ "type": "openIdConnect" }))["kind"],
            json!("other")
        );
    }
}
