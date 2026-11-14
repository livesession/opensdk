//! The typed model: what it reads, what it writes, and that `x-*` keys survive
//! the trip.

use opencli_model::{
    Argument, CliOption, Command, Document, EnvVar, ExitCode, KeyBinding, Metadata, XInteractive,
    XPassthrough,
};
use serde_json::{json, Value};

fn keys(v: &Value) -> Vec<&str> {
    v.as_object().unwrap().keys().map(String::as_str).collect()
}

/// A document touching every typed field, with an `x-*` key on every object.
fn everything() -> Value {
    json!({
        "opencli": "1.0.0",
        "info": {
            "title": "acme",
            "version": "1.2.3",
            "description": "The acme CLI.",
            "summary": "acme",
            "contact": { "name": "Acme", "url": "https://acme.test", "email": "cli@acme.test", "x-c": 1 },
            "license": { "name": "MIT", "identifier": "MIT", "url": "https://acme.test/license", "x-l": 1 },
            "x-info": true
        },
        "conventions": { "groupOptions": false, "optionSeparator": "=", "x-conv": 1 },
        "x-openapi": { "servers": ["https://api.acme.test"] },
        "arguments": [{ "name": "prompt", "description": "What to ask", "x-arg": 1 }],
        "options": [{
            "name": "output",
            "aliases": ["o"],
            "required": false,
            "description": "Output format",
            "group": "Display",
            "recursive": true,
            "hidden": false,
            "arguments": [{
                "name": "format",
                "required": true,
                "arity": { "minimum": 1, "maximum": 1, "x-arity": 1 },
                "acceptedValues": ["json", "table"]
            }],
            "metadata": [{ "name": "default", "value": "table", "x-meta": 1 }],
            "x-opt": 1
        }],
        "commands": [{
            "name": "agent",
            "aliases": ["ag"],
            "description": "Run the agent CLI.",
            "arguments": [{ "name": "args" }],
            "options": [{ "name": "print", "aliases": ["p"] }],
            "commands": [{ "name": "resume", "hidden": true }],
            "exitCodes": [{ "code": 3, "description": "Not signed in" }],
            "examples": ["acme agent -p hello"],
            "interactive": true,
            "hidden": false,
            "metadata": [{ "name": "componentVersion", "value": "0.4.0" }],
            "x-passthrough": { "program": "acme-agent", "x-pt": 1 },
            "x-interactive": {
                "commands": [{ "name": "model", "arguments": [{ "name": "name" }] }],
                "keys": [{ "keys": ["ctrl+c"], "description": "Quit", "context": "always", "x-key": 1 }]
            },
            "x-environment": [{ "name": "ACME_AGENT_HOME", "description": "Where it keeps state.", "x-env": 1 }],
            "x-openapi": { "method": "get", "path": "/ping" },
            "x-cmd": 1
        }],
        "exitCodes": [{ "code": 0, "description": "Success", "x-exit": 1 }],
        "examples": ["acme ask hello"],
        "interactive": false,
        "metadata": [{ "name": "componentVersion", "value": "1" }],
        "x-interactive": { "keys": [{ "keys": ["esc"], "description": "Back" }] },
        "x-environment": [{ "name": "ACME_COLOR", "description": "When to color", "values": ["always", "never"] }],
        "x-root": { "nested": [1, 2] }
    })
}

#[test]
fn every_field_and_every_extension_round_trips() {
    let doc = Document::from_value(everything()).unwrap();
    // Typed where typed...
    assert_eq!(doc.info.title, "acme");
    assert_eq!(
        doc.options[0].arguments[0].accepted_values,
        ["json", "table"]
    );
    let agent = &doc.commands[0];
    assert_eq!(
        agent.x_passthrough.as_ref().unwrap().program.as_deref(),
        Some("acme-agent")
    );
    assert_eq!(
        agent.x_interactive.as_ref().unwrap().commands[0].name,
        "model"
    );
    assert_eq!(doc.x_environment[0].values, ["always", "never"]);
    // ...and every untyped key kept where it was found.
    assert_eq!(doc.extensions["x-root"], json!({ "nested": [1, 2] }));
    assert_eq!(agent.extensions["x-cmd"], json!(1));
    assert_eq!(doc.options[0].extensions["x-opt"], json!(1));
    // serde_json's map equality ignores order: the content is identical.
    assert_eq!(doc.to_value(), everything());
    assert_eq!(Document::from_json(&doc.to_json()).unwrap(), doc);
}

#[test]
fn keys_are_written_in_canonical_order() {
    let v = Document::from_value(everything()).unwrap().to_value();
    assert_eq!(
        keys(&v),
        [
            "opencli",
            "info",
            "conventions",
            "x-openapi",
            "arguments",
            "options",
            "commands",
            "exitCodes",
            "examples",
            "interactive",
            "metadata",
            "x-interactive",
            "x-environment",
            "x-root"
        ]
    );
    assert_eq!(
        keys(&v["info"]),
        [
            "title",
            "version",
            "description",
            "summary",
            "contact",
            "license",
            "x-info"
        ]
    );
    assert_eq!(
        keys(&v["commands"][0]),
        [
            "name",
            "aliases",
            "description",
            "arguments",
            "options",
            "commands",
            "exitCodes",
            "examples",
            "interactive",
            "hidden",
            "metadata",
            "x-passthrough",
            "x-interactive",
            "x-environment",
            "x-openapi",
            "x-cmd"
        ]
    );
    assert_eq!(
        keys(&v["options"][0]),
        [
            "name",
            "aliases",
            "required",
            "description",
            "group",
            "recursive",
            "hidden",
            "arguments",
            "metadata",
            "x-opt"
        ]
    );
    assert_eq!(
        keys(&v["options"][0]["arguments"][0]),
        ["name", "required", "arity", "acceptedValues"]
    );
}

#[test]
fn a_document_built_in_code_writes_flat_json_with_a_trailing_newline() {
    let mut doc = Document::new("acme", "1.0.0");
    let mut ask = Command::new("ask");
    ask.arguments.push(Argument::new("prompt"));
    let mut verbose = CliOption::new("verbose");
    verbose.recursive = Some(true);
    verbose.metadata.push(Metadata::new("repeatable", true));
    doc.options.push(verbose);
    doc.commands.push(ask);
    doc.exit_codes.push(ExitCode::new(0, "Success"));
    doc.x_interactive = Some(XInteractive {
        keys: vec![KeyBinding::new(["ctrl+c", "ctrl+d"], "Quit").context("composer")],
        ..Default::default()
    });
    doc.x_environment
        .push(EnvVar::new("ACME_COLOR", "When to color output").values(["always", "never"]));
    let text = doc.to_json();
    assert!(text.ends_with("}\n"), "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({
            "opencli": "1.0.0",
            "info": { "title": "acme", "version": "1.0.0" },
            "options": [{
                "name": "verbose",
                "recursive": true,
                "metadata": [{ "name": "repeatable", "value": true }]
            }],
            "commands": [{ "name": "ask", "arguments": [{ "name": "prompt" }] }],
            "exitCodes": [{ "code": 0, "description": "Success" }],
            "x-interactive": {
                "keys": [{ "keys": ["ctrl+c", "ctrl+d"], "description": "Quit", "context": "composer" }]
            },
            "x-environment": [{
                "name": "ACME_COLOR", "description": "When to color output", "values": ["always", "never"]
            }]
        })
    );
}

#[test]
fn the_root_command_shape_is_read_and_written_flat() {
    let doc: Document = serde_json::from_value(json!({
        "opencli": "0.1",
        "info": { "title": "acme", "version": "1.0.0" },
        "command": {
            "name": "acme",
            "description": "The acme CLI.",
            "options": [{ "name": "verbose" }],
            "commands": [{ "name": "ask" }],
            "x-interactive": { "commands": [{ "name": "clear" }] }
        }
    }))
    .unwrap();
    assert_eq!(doc.info.description.as_deref(), Some("The acme CLI."));
    assert_eq!(doc.options[0].name, "verbose");
    assert_eq!(doc.commands[0].name, "ask");
    assert!(doc.x_interactive.is_some());
    assert!(!doc.to_value().as_object().unwrap().contains_key("command"));
}

#[test]
fn a_malformed_typed_extension_is_an_error() {
    let mut bad = everything();
    bad["x-environment"] = json!([{ "name": "ACME_COLOR" }]);
    let err = Document::from_value(bad).unwrap_err();
    assert!(err.0.contains("description"), "{err}");
    let mut bad = everything();
    bad["commands"][0]["x-passthrough"] = json!(true);
    assert!(Document::from_value(bad).is_err());
}

#[test]
fn paths_find_commands_and_stubs_are_recognized() {
    let mut doc = Document::from_value(everything()).unwrap();
    assert_eq!(doc.command(&["agent", "resume"]).unwrap().name, "resume");
    assert!(doc.command(&["agent", "nope"]).is_none());
    assert!(doc.command(&[]).is_none());
    doc.command_mut(&["agent"]).unwrap().description = None;
    assert!(doc.commands[0].description.is_none());

    let mut stub = Command::new("agent");
    assert!(!stub.is_passthrough_stub());
    stub.x_passthrough = Some(XPassthrough::program("acme-agent"));
    assert!(stub.is_passthrough_stub());
    stub.commands.push(Command::new("resume"));
    assert!(!stub.is_passthrough_stub());

    let mut opt = CliOption::new("profile");
    opt.aliases = vec!["P".into(), "prof".into()];
    assert_eq!(opt.spellings(), ["--profile", "-P", "--prof"]);
}
