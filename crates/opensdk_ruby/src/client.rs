//! Port of client.ts — `lib/<pkg>/client.rb` emission.

use serde_json::Value;

use crate::naming::snake_case;
use crate::service::resource_class_name;
use crate::writer::{block, indent, rb_comment, rb_string};

pub fn render_client_file(spec: &Value, module_name: &str, env_var: &str) -> String {
    let resources = spec
        .get("resources")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut members: Vec<String> = Vec::new();

    if !resources.is_empty() {
        let readers: Vec<String> = resources
            .iter()
            .map(|r| format!(":{}", snake_case(name_of(r))))
            .collect();
        members.push(format!("attr_reader {}", readers.join(", ")));
    }

    // `default_headers:` rides along only when the SDK carries sdk.defaultHeaders
    // (the knob to override them per client, handed straight to the Transport);
    // otherwise the constructor is the one it always was.
    let with_default_headers =
        !opensdk_core::behavior::default_headers(&opensdk_core::behavior::resolve_behavior(spec))
            .is_empty();
    let (ctor_param, transport_arg) = if with_default_headers {
        (
            ", default_headers: nil",
            ", default_headers: default_headers",
        )
    } else {
        ("", "")
    };
    let mut ctor = vec![
        format!("def initialize(api_key: nil, base_url: nil, timeout: nil{ctor_param})"),
        indent(&format!("api_key = ENV[{}] if api_key.nil?", rb_string(env_var))),
        indent(&format!(
            "@transport = {module_name}::Transport.new(api_key: api_key, base_url: base_url, timeout: timeout{transport_arg})"
        )),
    ];
    for r in &resources {
        let sn = snake_case(name_of(r));
        ctor.push(indent(&format!(
            "@{sn} = {module_name}::Resources::{}.new(@transport)",
            resource_class_name(&[name_of(r).to_string()])
        )));
    }
    ctor.push("end".to_string());
    members.push(ctor.join("\n"));

    let title = spec
        .get("info")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let doc = rb_comment(&format!("Client is the {title} API client."));
    let client_class = format!("{doc}\n{}", block("class Client", &members.join("\n\n")));
    format!(
        "{}\n",
        block(&format!("module {module_name}"), &client_class)
    )
}

fn name_of(v: &Value) -> &str {
    v.get("name").and_then(|n| n.as_str()).unwrap_or("")
}
