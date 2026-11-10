//! `opensdk opencli` — write the OpenCLI document a CLI target generates from.
//!
//! `generate --lang rust-cli` runs the OpenAPI → OpenCLI converter and hands the
//! result straight to the backend, so the intermediate document is never seen.
//! This command stops after the converter: it writes the OpenCLI JSON (with the
//! CLI target's converter options from sdk.json) and merges hand-written
//! OpenCLI fragments into it (`--declare`), so a CLI whose surface is part
//! generated and part hand-written has ONE document describing all of it.
//! `generate --spec <that document>` then drives the backend from it.
//!
//! # Fragment merge rules
//!
//! Each `--declare` file is an OpenCLI-shaped JSON (or YAML) object, applied in
//! the order given:
//!
//! * root `options`, root `exitCodes` and `info.description` REPLACE what the
//!   document has;
//! * `commands` APPEND in fragment order — except a fragment command whose
//!   `name` equals a top-level command already in the document (the
//!   `rootCommand` wrapper, or a command an earlier fragment added). That one is
//!   merged field-wise: its `description`, `aliases` and `hidden` replace the
//!   existing ones and its children are left alone; any other field on it is an
//!   error, because there is no merge rule that would not silently drop it.
//!
//! Any other root key (besides `opencli` and `$schema`, which are ignored, and
//! `info`, of which only `description` is read) is an error — a misspelt
//! `exitcodes` would otherwise vanish.

use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};

use opensdk_config::merge_publish_targets;

use crate::cli_targets::{
    convert_to_opencli, converter_bag_with_defaults, is_cli_target, load_cli_source,
    split_cli_options, CliSource,
};
use crate::config::ResolvedConfig;
use crate::error::{Error, Result};
use crate::paths;

/// Inputs for `opensdk opencli`.
#[derive(Debug, Clone, Default)]
pub struct OpenCliCommandOptions {
    /// The OpenAPI document (yaml/json). An OpenCLI document is accepted too:
    /// the converter is skipped and only the fragments are merged.
    pub spec: String,
    /// The CLI target whose converter options apply (`go-cli` | `rust-cli`).
    /// Unset: the first CLI target declared in the config, else none.
    pub lang: Option<String>,
    /// OpenCLI fragments merged in order.
    pub declare: Vec<String>,
    /// Write here instead of stdout.
    pub output: Option<String>,
    /// Defaults the converter's `cliName` (overridden by the section's own).
    pub sdk_name: Option<String>,
}

/// The CLI target `opencli` reads its converter options from.
fn target_lang(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
) -> Result<Option<String>> {
    if let Some(lang) = opts.lang.as_deref() {
        let lang = lang.to_lowercase();
        if !is_cli_target(Some(&lang)) {
            return Err(Error::msg(format!(
                "--lang must be a CLI target (go-cli, rust-cli), got \"{lang}\""
            )));
        }
        return Ok(Some(lang));
    }
    Ok(config.and_then(|c| {
        c.declared_languages()
            .into_iter()
            .find(|l| is_cli_target(Some(l)))
    }))
}

/// Build the merged OpenCLI document (what `opencli` writes).
pub fn build_opencli(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
    cwd: &Path,
) -> Result<Value> {
    let lang = target_lang(opts, config)?;
    let lang_label = lang
        .clone()
        .unwrap_or_else(|| "go-cli, rust-cli".to_string());
    let spec = paths::resolve_str(cwd, &opts.spec);

    let mut doc = match load_cli_source(&spec, &lang_label)? {
        CliSource::OpenCli(doc) => doc,
        CliSource::OpenApi(api) => {
            let empty = Map::new();
            let converter = match lang.as_deref() {
                Some(lang) => {
                    let bag = config
                        .and_then(|c| c.emitter_options_for(lang))
                        .unwrap_or(&empty);
                    split_cli_options(lang, bag)?.converter
                }
                None => Map::new(),
            };
            let publish = merge_publish_targets(&[
                config.and_then(|c| c.publish.as_ref()),
                lang.as_deref()
                    .and_then(|l| config.and_then(|c| c.target_for(l)))
                    .and_then(|t| t.publish.as_ref()),
            ]);
            let sdk_name = opts
                .sdk_name
                .clone()
                .or_else(|| config.and_then(|c| c.sdk_name.clone()));
            let converter =
                converter_bag_with_defaults(converter, sdk_name.as_deref(), publish.as_ref());
            convert_to_opencli(&api, converter, &lang_label)?
        }
    };

    for path in &opts.declare {
        let resolved = paths::resolve_str(cwd, path);
        let fragment = oas_doc::read_spec(&resolved)
            .map_err(|e| Error::msg(format!("--declare {path}: {e}")))?;
        merge_fragment(&mut doc, &fragment, path)?;
    }
    Ok(doc)
}

/// `opensdk opencli`: build the document and write it (stdout or `--output`).
pub fn opencli_command(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
    cwd: &Path,
) -> Result<()> {
    let doc = build_opencli(opts, config, cwd)?;
    let json = format!(
        "{}\n",
        serde_json::to_string_pretty(&doc).map_err(|e| Error::msg(e.to_string()))?
    );
    match opts.output.as_deref() {
        Some(output) => {
            let path = paths::resolve(cwd, output);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::msg(format!("create {}: {e}", parent.display())))?;
            }
            std::fs::write(&path, &json).map_err(|e| Error::msg(format!("write {output}: {e}")))?;
            println!("Wrote OpenCLI to {output}");
        }
        None => {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(json.as_bytes());
            let _ = stdout.flush();
        }
    }
    Ok(())
}

/// The root keys a fragment may carry.
const FRAGMENT_KEYS: &[&str] = &[
    "$schema",
    "opencli",
    "info",
    "options",
    "exitCodes",
    "commands",
];

/// The fields a fragment command may set on a command the document already has.
const MERGEABLE_COMMAND_KEYS: &[&str] = &["name", "description", "aliases", "hidden"];

/// Key order the merge inserts new keys by, matching what the converter emits.
const ROOT_ORDER: &[&str] = &[
    "opencli",
    "info",
    "x-openapi",
    "options",
    "commands",
    "exitCodes",
];
const INFO_ORDER: &[&str] = &[
    "title",
    "version",
    "description",
    "summary",
    "contact",
    "license",
];
const COMMAND_ORDER: &[&str] = &[
    "name",
    "aliases",
    "description",
    "arguments",
    "options",
    "commands",
    "hidden",
    "metadata",
    "x-openapi",
];

/// Set `key` on `map`, in place when present. A new key in `order` goes right
/// after the last key already there that `order` puts before it, else right
/// before the first one it puts after it, else at the end; a new key not in
/// `order` goes at the end. Keys `order` does not know keep their positions —
/// a leading `$schema` stays first.
fn set_ordered(map: &mut Map<String, Value>, key: &str, value: Value, order: &[&str]) {
    if let Some(slot) = map.get_mut(key) {
        *slot = value;
        return;
    }
    let rank = |k: &str| order.iter().position(|o| *o == k);
    let Some(own) = rank(key) else {
        map.insert(key.to_string(), value);
        return;
    };
    let keys: Vec<&String> = map.keys().collect();
    let after_predecessor = keys
        .iter()
        .rposition(|k| rank(k).is_some_and(|r| r < own))
        .map(|i| i + 1);
    let before_successor = || keys.iter().position(|k| rank(k).is_some_and(|r| r > own));
    let at = after_predecessor
        .or_else(before_successor)
        .unwrap_or(keys.len());
    let mut out = Map::new();
    for (i, (k, v)) in std::mem::take(map).into_iter().enumerate() {
        if i == at {
            out.insert(key.to_string(), value.clone());
        }
        out.insert(k, v);
    }
    if !out.contains_key(key) {
        out.insert(key.to_string(), value);
    }
    *map = out;
}

/// The spellings a top-level command answers to: its name, then its aliases.
fn spellings(command: &Value) -> Vec<&str> {
    command
        .get("name")
        .and_then(Value::as_str)
        .into_iter()
        .chain(
            command
                .get("aliases")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .collect()
}

/// The first of `wanted` some top-level command OTHER than `own` answers to,
/// with that command's name.
fn spelling_taken<'a>(
    commands: &'a [Value],
    own: &str,
    wanted: &[&str],
) -> Option<(String, &'a str)> {
    commands
        .iter()
        .filter(|c| c.get("name").and_then(Value::as_str) != Some(own))
        .find_map(|c| {
            let other = c.get("name").and_then(Value::as_str).unwrap_or_default();
            spellings(c)
                .into_iter()
                .find(|s| wanted.contains(s))
                .map(|s| (s.to_string(), other))
        })
}

/// Merge one `--declare` fragment into the document. `origin` names the
/// fragment in error messages.
pub fn merge_fragment(doc: &mut Value, fragment: &Value, origin: &str) -> Result<()> {
    let fail = |msg: String| Error::msg(format!("--declare {origin}: {msg}"));
    let Some(frag) = fragment.as_object() else {
        return Err(fail("a fragment is a JSON object".into()));
    };
    let unknown: Vec<&str> = frag
        .keys()
        .map(String::as_str)
        .filter(|k| !FRAGMENT_KEYS.contains(k))
        .collect();
    if !unknown.is_empty() {
        return Err(fail(format!(
            "unknown key(s) {}; a fragment sets info.description, options, exitCodes and commands",
            unknown.join(", ")
        )));
    }
    let Some(root) = doc.as_object_mut() else {
        return Err(fail("the document is not a JSON object".into()));
    };

    if let Some(description) = frag.get("info").and_then(|i| i.get("description")) {
        if !description.is_string() {
            return Err(fail("info.description must be a string".into()));
        }
        let mut info = root
            .get("info")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        set_ordered(&mut info, "description", description.clone(), INFO_ORDER);
        set_ordered(root, "info", Value::Object(info), ROOT_ORDER);
    }
    for key in ["options", "exitCodes"] {
        if let Some(value) = frag.get(key) {
            if !value.is_array() {
                return Err(fail(format!("`{key}` must be an array")));
            }
            set_ordered(root, key, value.clone(), ROOT_ORDER);
        }
    }

    let Some(commands) = frag.get("commands") else {
        return Ok(());
    };
    let Some(commands) = commands.as_array() else {
        return Err(fail("`commands` must be an array".into()));
    };
    let mut existing: Vec<Value> = root
        .get("commands")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for command in commands {
        let Some(name) = command.get("name").and_then(Value::as_str) else {
            return Err(fail("every command needs a string `name`".into()));
        };
        let Some(fields) = command.as_object() else {
            return Err(fail(format!("command `{name}` is not an object")));
        };
        match existing
            .iter_mut()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(name))
        {
            Some(target) => {
                let extra: Vec<&str> = fields
                    .keys()
                    .map(String::as_str)
                    .filter(|k| !MERGEABLE_COMMAND_KEYS.contains(k))
                    .collect();
                if !extra.is_empty() {
                    return Err(fail(format!(
                        "command `{name}` already exists, so only description, aliases and \
                         hidden can be set on it (found {})",
                        extra.join(", ")
                    )));
                }
                let target = target.as_object_mut().expect("commands are objects");
                for key in ["description", "aliases", "hidden"] {
                    if let Some(value) = fields.get(key) {
                        set_ordered(target, key, value.clone(), COMMAND_ORDER);
                    }
                }
            }
            None => existing.push(command.clone()),
        }
        // Names and aliases share one namespace at the top level: clap
        // rejects two commands answering to one spelling.
        let wanted = spellings(command);
        if let Some((spelling, other)) = spelling_taken(&existing, name, &wanted) {
            return Err(fail(format!(
                "command `{name}` and command `{other}` both answer to `{spelling}` (a name or \
                 an alias)"
            )));
        }
    }
    set_ordered(root, "commands", Value::Array(existing), ROOT_ORDER);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(v: &Value) -> Vec<&str> {
        v.as_object().unwrap().keys().map(String::as_str).collect()
    }

    fn generated() -> Value {
        json!({
            "opencli": "1.0.0",
            "info": { "title": "acme", "version": "1.0.0" },
            "x-openapi": { "servers": ["https://api.acme.test"] },
            "commands": [
                { "name": "api", "commands": [{ "name": "widgets", "commands": [] }] }
            ]
        })
    }

    #[test]
    fn root_options_exit_codes_and_the_description_replace_in_opencli_order() {
        let mut doc = generated();
        merge_fragment(
            &mut doc,
            &json!({
                "exitCodes": [{ "code": 0, "description": "ok" }],
                "options": [{ "name": "verbose" }],
                "info": { "title": "ignored", "description": "The acme CLI." }
            }),
            "a.json",
        )
        .unwrap();
        assert_eq!(
            keys(&doc),
            [
                "opencli",
                "info",
                "x-openapi",
                "options",
                "commands",
                "exitCodes"
            ]
        );
        assert_eq!(keys(&doc["info"]), ["title", "version", "description"]);
        assert_eq!(doc["info"]["title"], json!("acme"));

        merge_fragment(&mut doc, &json!({ "options": [] }), "b.json").unwrap();
        assert_eq!(doc["options"], json!([]));
    }

    #[test]
    fn commands_append_and_a_matching_top_level_command_merges_its_fields() {
        let mut doc = generated();
        merge_fragment(
            &mut doc,
            &json!({ "commands": [
                { "name": "login", "description": "Sign in" },
                { "name": "api", "description": "Raw API calls", "aliases": ["a"], "hidden": false }
            ] }),
            "a.json",
        )
        .unwrap();
        let commands = doc["commands"].as_array().unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            keys(&commands[0]),
            ["name", "aliases", "description", "commands", "hidden"]
        );
        assert_eq!(commands[0]["commands"][0]["name"], json!("widgets"));
        assert_eq!(commands[1]["name"], json!("login"));
    }

    #[test]
    fn unknown_keys_keep_their_place_and_known_ones_follow_their_predecessor() {
        let mut doc = json!({
            "$schema": "https://example.test/opencli.json",
            "opencli": "1.0.0",
            "info": { "title": "t", "version": "1" },
            "commands": []
        });
        merge_fragment(
            &mut doc,
            &json!({ "exitCodes": [], "options": [] }),
            "a.json",
        )
        .unwrap();
        assert_eq!(
            keys(&doc),
            [
                "$schema",
                "opencli",
                "info",
                "options",
                "commands",
                "exitCodes"
            ]
        );
        // No known predecessor: before the first known successor.
        let mut map = json!({ "x-a": 1, "info": {}, "x-b": 2 });
        set_ordered(
            map.as_object_mut().unwrap(),
            "opencli",
            json!("1"),
            ROOT_ORDER,
        );
        assert_eq!(keys(&map), ["x-a", "opencli", "info", "x-b"]);
        // No known key at all: at the end.
        let mut map = json!({ "x-a": 1 });
        set_ordered(
            map.as_object_mut().unwrap(),
            "options",
            json!([]),
            ROOT_ORDER,
        );
        assert_eq!(keys(&map), ["x-a", "options"]);
    }

    #[test]
    fn a_name_or_alias_another_top_level_command_answers_to_is_an_error() {
        let mut doc = generated();
        merge_fragment(
            &mut doc,
            &json!({ "commands": [{ "name": "login", "aliases": ["signin"] }] }),
            "a.json",
        )
        .unwrap();
        // A new command named like an existing alias.
        let err = merge_fragment(
            &mut doc,
            &json!({ "commands": [{ "name": "signin" }] }),
            "b.json",
        )
        .unwrap_err();
        assert_eq!(
            err.0,
            "--declare b.json: command `signin` and command `login` both answer to `signin` (a \
             name or an alias)"
        );
        // A new command whose alias is an existing name.
        assert!(merge_fragment(
            &mut doc,
            &json!({ "commands": [{ "name": "auth", "aliases": ["api"] }] }),
            "c.json"
        )
        .is_err());
        // A merged alias equal to another command's name.
        assert!(merge_fragment(
            &mut doc,
            &json!({ "commands": [{ "name": "api", "aliases": ["login"] }] }),
            "d.json"
        )
        .is_err());
        // A command's own name among its aliases is not a clash.
        merge_fragment(
            &mut doc,
            &json!({ "commands": [{ "name": "api", "aliases": ["api", "raw"] }] }),
            "e.json",
        )
        .unwrap();
    }

    #[test]
    fn a_matching_command_cannot_carry_other_fields() {
        let mut doc = generated();
        let err = merge_fragment(
            &mut doc,
            &json!({ "commands": [{ "name": "api", "commands": [] }] }),
            "a.json",
        )
        .unwrap_err();
        assert_eq!(
            err.0,
            "--declare a.json: command `api` already exists, so only description, aliases and \
             hidden can be set on it (found commands)"
        );
    }

    #[test]
    fn unknown_root_keys_and_bad_shapes_are_errors() {
        let mut doc = generated();
        let err = merge_fragment(&mut doc, &json!({ "exitcodes": [] }), "a.json").unwrap_err();
        assert!(err.0.contains("unknown key(s) exitcodes"), "{}", err.0);
        let err = merge_fragment(&mut doc, &json!({ "options": {} }), "a.json").unwrap_err();
        assert!(err.0.contains("`options` must be an array"), "{}", err.0);
        let err =
            merge_fragment(&mut doc, &json!({ "commands": [{ "x": 1 }] }), "a.json").unwrap_err();
        assert!(err.0.contains("string `name`"), "{}", err.0);
        assert!(merge_fragment(&mut doc, &json!([]), "a.json").is_err());
    }

    #[test]
    fn a_document_without_commands_gains_them() {
        let mut doc = json!({ "opencli": "1.0.0", "info": { "title": "t", "version": "1" } });
        merge_fragment(
            &mut doc,
            &json!({ "commands": [{ "name": "version" }] }),
            "a",
        )
        .unwrap();
        assert_eq!(keys(&doc), ["opencli", "info", "commands"]);
    }
}
