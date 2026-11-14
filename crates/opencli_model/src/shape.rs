//! The two document shapes.
//!
//! The chain reads and writes the FLAT shape of the OpenCLI draft: the root
//! command's arguments, options, subcommands, exit codes, examples,
//! `interactive` and metadata sit on the document itself. The upstream draft
//! has since moved them under a root `command` object. A document in that
//! shape is read by lifting the root command's fields to the root.

use serde_json::{Map, Value};

use crate::Error;

/// The root-command fields the flat shape keeps on the document itself.
const LIFTED: &[&str] = &[
    "arguments",
    "options",
    "commands",
    "exitCodes",
    "examples",
    "interactive",
    "metadata",
];

/// The root command's own fields that have no place on a flat document: the
/// CLI's name is `info.title`, and a root has no aliases and cannot be hidden.
const DROPPED: &[&str] = &["name", "aliases", "hidden"];

/// Lift a root-`command` document to the flat shape; any other value is
/// returned as it is.
///
/// The root command's arguments, options, subcommands, exit codes, examples,
/// `interactive`, metadata and every other key (`x-*` included) move to the
/// root, after the keys already there. Its `description` becomes
/// `info.description` when `info` has none, and its `name` becomes
/// `info.title` when `info` has none; its `aliases` and `hidden` are dropped.
/// A document with both a root `command` and one of the lifted fields at the
/// root is an error: it is not clear which one describes the root command.
pub fn lift_root_command(doc: Value) -> Result<Value, Error> {
    let Value::Object(root) = doc else {
        return Ok(doc);
    };
    if !root.contains_key("command") {
        return Ok(Value::Object(root));
    }
    if let Some(key) = LIFTED.iter().find(|k| root.contains_key(**k)) {
        return Err(Error(format!(
            "the document has both a root `command` and a root `{key}`; the flat shape \
             keeps the root command's fields at the root, the upstream shape under \
             `command`, never both"
        )));
    }

    let mut out = Map::new();
    let mut command = None;
    for (key, value) in root {
        if key == "command" {
            command = Some(value);
        } else {
            out.insert(key, value);
        }
    }
    let Some(Value::Object(command)) = command else {
        return Err(Error("the root `command` must be an object".into()));
    };

    let info = out
        .entry("info")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(info) = info.as_object_mut() {
        for (from, to) in [("description", "description"), ("name", "title")] {
            if let Some(value) = command.get(from) {
                info.entry(to).or_insert_with(|| value.clone());
            }
        }
    }
    for (key, value) in command {
        if key != "description" && !DROPPED.contains(&key.as_str()) {
            out.insert(key, value);
        }
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_flat_document_is_returned_unchanged() {
        let doc = json!({
            "opencli": "1.0.0",
            "info": { "title": "acme", "version": "1" },
            "x-openapi": {},
            "commands": [{ "name": "a" }]
        });
        let lifted = lift_root_command(doc.clone()).unwrap();
        assert_eq!(
            serde_json::to_string(&lifted).unwrap(),
            serde_json::to_string(&doc).unwrap()
        );
    }

    #[test]
    fn the_root_command_moves_to_the_root() {
        let doc = json!({
            "opencli": "0.1",
            "command": {
                "name": "acme",
                "aliases": ["a"],
                "hidden": false,
                "description": "The acme CLI.",
                "options": [{ "name": "verbose" }],
                "commands": [{ "name": "ask" }],
                "x-environment": [{ "name": "ACME_COLOR", "description": "Color output." }]
            },
            "info": { "title": "acme", "version": "1.0.0" }
        });
        let lifted = lift_root_command(doc).unwrap();
        assert_eq!(
            lifted,
            json!({
                "opencli": "0.1",
                "info": { "title": "acme", "version": "1.0.0", "description": "The acme CLI." },
                "options": [{ "name": "verbose" }],
                "commands": [{ "name": "ask" }],
                "x-environment": [{ "name": "ACME_COLOR", "description": "Color output." }]
            })
        );
        let keys: Vec<&String> = lifted.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["opencli", "info", "options", "commands", "x-environment"]
        );
    }

    #[test]
    fn info_keeps_its_own_description_and_title() {
        let lifted = lift_root_command(json!({
            "opencli": "0.1",
            "info": { "title": "acme", "version": "1", "description": "Kept." },
            "command": { "name": "other", "description": "Dropped." }
        }))
        .unwrap();
        assert_eq!(lifted["info"]["title"], json!("acme"));
        assert_eq!(lifted["info"]["description"], json!("Kept."));
        // Without an info, the command's name titles the CLI.
        let lifted =
            lift_root_command(json!({ "opencli": "0.1", "command": { "name": "acme" } })).unwrap();
        assert_eq!(lifted["info"]["title"], json!("acme"));
    }

    #[test]
    fn both_shapes_at_once_and_a_non_object_command_are_errors() {
        let err = lift_root_command(json!({
            "opencli": "0.1",
            "info": { "title": "a", "version": "1" },
            "command": { "name": "a" },
            "options": []
        }))
        .unwrap_err();
        assert!(
            err.0.contains("both a root `command` and a root `options`"),
            "{err}"
        );
        assert!(lift_root_command(json!({ "opencli": "0.1", "command": [] })).is_err());
    }
}
