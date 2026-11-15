//! The two exported schemas compile, a document using every extension passes
//! both, and the extensions schema rejects what its contract rules out.

use opencli_model::{EXTENSIONS_SCHEMA_JSON, SCHEMA_JSON};
use serde_json::{json, Value};

fn validator(schema: &str) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(schema).expect("the schema is JSON");
    jsonschema::draft202012::new(&schema).expect("the schema compiles")
}

fn errors(schema: &str, doc: &Value) -> Vec<String> {
    validator(schema)
        .iter_errors(doc)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect()
}

fn valid() -> Value {
    json!({
        "opencli": "1.0.0",
        "info": { "title": "acme", "version": "1.0.0" },
        "options": [{ "name": "profile", "recursive": true, "arguments": [{ "name": "name" }] }],
        "commands": [{
            "name": "agent",
            "description": "Run the agent CLI: every argument after `agent` is its own.",
            "x-passthrough": { "program": "acme-agent" },
            "commands": [{
                "name": "resume",
                "x-environment": [{ "name": "ACME_AGENT_HOME", "description": "Where it keeps state." }]
            }]
        }],
        "interactive": true,
        "x-interactive": {
            "commands": [
                { "name": "model", "description": "Switch the model", "arguments": [{ "name": "name" }] },
                { "name": "clear", "aliases": ["new"] }
            ],
            "keys": [
                { "keys": ["ctrl+c"], "description": "Interrupt, then quit", "context": "always" },
                { "keys": ["esc"], "description": "Close the picker", "context": "picker" }
            ]
        },
        "x-environment": [
            { "name": "ACME_COLOR", "description": "When to color output", "values": ["always", "never", "auto"] }
        ]
    })
}

#[test]
fn a_document_with_every_extension_passes_both_schemas() {
    let doc = valid();
    assert_eq!(errors(SCHEMA_JSON, &doc), Vec::<String>::new());
    assert_eq!(errors(EXTENSIONS_SCHEMA_JSON, &doc), Vec::<String>::new());
}

/// Breaks one rule of the extensions schema in a valid document.
type Breaker = fn(&mut Value);

#[test]
fn the_extensions_schema_rejects_what_its_contract_rules_out() {
    let cases: Vec<(&str, Breaker)> = vec![
        ("x-passthrough on the root", |d| {
            d["x-passthrough"] = json!({});
        }),
        ("x-passthrough that is not an object", |d| {
            d["commands"][0]["x-passthrough"] = json!(true);
        }),
        ("an empty program", |d| {
            d["commands"][0]["x-passthrough"] = json!({ "program": "" });
        }),
        ("an environment variable without a description", |d| {
            d["commands"][0]["commands"][0]["x-environment"] = json!([{ "name": "A" }]);
        }),
        ("a key binding without keys", |d| {
            d["x-interactive"]["keys"] = json!([{ "keys": [], "description": "x" }]);
        }),
        ("a session command without a name", |d| {
            d["x-interactive"]["commands"] = json!([{ "description": "x" }]);
        }),
        ("a session command that is a passthrough", |d| {
            d["x-interactive"]["commands"] = json!([{ "name": "shell", "x-passthrough": {} }]);
        }),
    ];
    for (what, break_it) in cases {
        let mut doc = valid();
        break_it(&mut doc);
        assert!(
            !errors(EXTENSIONS_SCHEMA_JSON, &doc).is_empty(),
            "{what} should be rejected"
        );
    }
}
