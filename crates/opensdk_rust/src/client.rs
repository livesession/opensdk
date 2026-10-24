//! src/client.rs — port of client.ts.

use serde_json::Value;

use crate::naming::{pascal_case, snake_case};
use crate::rswriter::{braced, rs_doc, rs_string};

pub fn render_client_file(spec: &Value, env_var: &str) -> String {
    let empty = Vec::new();
    let resources = spec
        .get("resources")
        .and_then(|r| r.as_array())
        .unwrap_or(&empty);
    // With sdk.defaultHeaders the transport takes a per-client header list and
    // the builder grows `default_header`; without them the client is exactly
    // what it always was.
    let with_default_headers =
        !opensdk_core::behavior::default_headers(&opensdk_core::behavior::resolve_behavior(spec))
            .is_empty();
    let no_headers_arg = if with_default_headers {
        ", Vec::new()"
    } else {
        ""
    };

    let accessors: String = resources
        .iter()
        .map(|r| {
            let name = r.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let md = snake_case(name);
            let cls = pascal_case(name);
            braced(
                &format!("pub fn {}(&self) -> crate::{md}::{cls}", snake_case(name)),
                &format!("crate::{md}::{cls}::new(self.transport.clone())"),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let client_impl_members = vec![
        braced(
            "pub fn new(api_key: impl Into<String>) -> Self",
            &format!(
                "Client {{\n    transport: Arc::new(Transport::new(Some(api_key.into()), None, None{no_headers_arg})),\n}}"
            ),
        ),
        braced(
            "pub fn from_env() -> Self",
            &format!(
                "Client {{\n    transport: Arc::new(Transport::new(std::env::var({}).ok(), None, None{no_headers_arg})),\n}}",
                rs_string(env_var)
            ),
        ),
        braced("pub fn builder() -> ClientBuilder", "ClientBuilder::default()"),
        accessors,
    ];
    let client_impl_body = client_impl_members
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut builder_members = vec![
        braced(
            "pub fn api_key(mut self, api_key: impl Into<String>) -> Self",
            "self.api_key = Some(api_key.into());\nself",
        ),
        braced(
            "pub fn base_url(mut self, base_url: impl Into<String>) -> Self",
            "self.base_url = Some(base_url.into());\nself",
        ),
        braced(
            "pub fn timeout_ms(mut self, timeout_ms: u64) -> Self",
            "self.timeout_ms = Some(timeout_ms);\nself",
        ),
    ];
    if with_default_headers {
        builder_members.push(format!(
            "{}\n{}",
            rs_doc(Some(
                "Send `value` as the `name` header on every request (overrides the SDK's built-in default of the same name)."
            )),
            braced(
                "pub fn default_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self",
                "self.default_headers.push((name.into(), value.into()));\nself",
            )
        ));
    }
    builder_members.push(braced(
        "pub fn build(self) -> Client",
        if with_default_headers {
            "Client {\n    transport: Arc::new(Transport::new(self.api_key, self.base_url, self.timeout_ms, self.default_headers)),\n}"
        } else {
            "Client {\n    transport: Arc::new(Transport::new(self.api_key, self.base_url, self.timeout_ms)),\n}"
        },
    ));
    let builder_impl_body = builder_members.join("\n\n");
    let builder_fields = if with_default_headers {
        "    api_key: Option<String>,\n    base_url: Option<String>,\n    timeout_ms: Option<u64>,\n    default_headers: Vec<(String, String)>,"
    } else {
        "    api_key: Option<String>,\n    base_url: Option<String>,\n    timeout_ms: Option<u64>,"
    };

    let title = spec
        .get("info")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let doc = rs_doc(Some(&format!("Client is the {title} API client.")));

    format!(
        "use std::sync::Arc;\n\nuse crate::transport::Transport;\n\n{doc}\npub struct Client {{\n    transport: Arc<Transport>,\n}}\n\n{}\n\n/// A builder for base URL / timeout overrides.\n#[derive(Default)]\npub struct ClientBuilder {{\n{builder_fields}\n}}\n\n{}\n",
        braced("impl Client", &client_impl_body),
        braced("impl ClientBuilder", &builder_impl_body)
    )
}
