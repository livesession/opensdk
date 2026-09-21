//! `chain.json` discovery + validation (`src/chain.ts`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::sources::resolve_path;
use crate::yaml;

/// Chain filenames tried (in order) when no explicit path is given.
///
/// `sdk.json` is last and is accepted ONLY when it is actually chain-shaped —
/// see [`is_chain_shaped`]. Most `sdk.json` files are the per-language
/// declarative config and have no `sources`/`targets` at all; claiming those as
/// chains would break every existing `opensdk run` that relies on finding a real
/// chain.json, and would turn a "no chain file" message into a confusing shape
/// error. Including it lets a project keep ONE config file at its root that
/// declares both what to generate and where from.
const CHAIN_NAMES: [&str; 3] = ["chain.json", ".chain/chain.json", "sdk.json"];

/// Does this document declare a chain (a non-empty `sources` AND `targets`)?
///
/// Both are required: `resolve_chain` rejects a document missing either, so a
/// file with only one of them is not a chain that could run — and treating it as
/// one would replace a clear config error with a chain error about the other key.
pub fn is_chain_shaped(doc: &Value) -> bool {
    let non_empty = |k: &str| {
        doc.get(k)
            .and_then(Value::as_object)
            .is_some_and(|m| !m.is_empty())
    };
    non_empty("sources") && non_empty("targets")
}

fn file_is_chain_shaped(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| {
            let name = path.to_string_lossy();
            if name.ends_with(".yaml") || name.ends_with(".yml") {
                yaml::from_str(&raw).ok()
            } else {
                serde_json::from_str(&raw).ok()
            }
        })
        .as_ref()
        .is_some_and(is_chain_shaped)
}

/// Locate a chain file: an explicit path (must exist) or the conventional names in `cwd`.
///
/// An EXPLICIT path is taken at its word — `--chain sdk.json` has always worked,
/// because `resolve_chain` reads whatever filename it is given and validates the
/// shape. Only the implicit `sdk.json` fallback is shape-gated.
pub fn detect_chain(cwd: &Path, explicit_path: Option<&str>) -> Option<PathBuf> {
    if let Some(explicit) = explicit_path {
        let resolved = resolve_path(cwd, Path::new(explicit));
        return resolved.exists().then_some(resolved);
    }
    CHAIN_NAMES.iter().find_map(|rel| {
        let p = resolve_path(cwd, Path::new(rel));
        if !p.exists() {
            return None;
        }
        if *rel == "sdk.json" && !file_is_chain_shaped(&p) {
            return None;
        }
        Some(p)
    })
}

/// Load + validate a `chain.json` (json, or yaml by extension).
///
/// Returns the document verbatim — the TS does no shape coercion, and downstream code
/// (`runChain`) reads it as-is — so key order and unknown fields survive.
/// Throws on a bad shape or a dangling source ref.
pub fn resolve_chain(chain_path: &str, cwd: &Path) -> Result<Value> {
    let abs = resolve_path(cwd, Path::new(chain_path));
    if !abs.exists() {
        return Err(Error::msg(format!(
            "Chain file not found: {}",
            abs.display()
        )));
    }
    let raw = std::fs::read_to_string(&abs).map_err(|e| Error::io(&abs, &e))?;

    let name = abs.to_string_lossy();
    let doc: Value = if name.ends_with(".yaml") || name.ends_with(".yml") {
        yaml::from_str(&raw).map_err(|e| Error::parse(&abs, e))?
    } else {
        serde_json::from_str(&raw).map_err(|e| Error::parse(&abs, e))?
    };

    let Some(root) = doc.as_object() else {
        return Err(Error::msg(format!(
            "Invalid chain file {}: expected an object",
            abs.display()
        )));
    };

    let sources = root.get("sources").and_then(Value::as_object);
    if sources.is_none_or(serde_json::Map::is_empty) {
        return Err(Error::msg("chain.json needs at least one `sources` entry"));
    }
    let targets = root.get("targets").and_then(Value::as_object);
    if targets.is_none_or(serde_json::Map::is_empty) {
        return Err(Error::msg("chain.json needs at least one `targets` entry"));
    }
    let (sources, targets) = (sources.expect("checked"), targets.expect("checked"));

    for (name, t) in targets {
        if !truthy(t.get("target")) {
            return Err(Error::msg(format!(
                "target \"{name}\" is missing `target` (the language)"
            )));
        }
        let Some(source) = t.get("source") else {
            return Err(Error::msg(format!("target \"{name}\" is missing `source`")));
        };
        if !truthy(Some(source)) {
            return Err(Error::msg(format!("target \"{name}\" is missing `source`")));
        }
        // `doc.sources[t.source]` — a non-string key is coerced by the property lookup.
        let key = match source {
            Value::String(s) => s.clone(),
            other => js_property_key(other),
        };
        if !truthy(sources.get(&key)) {
            return Err(Error::msg(format!(
                "target \"{name}\" references unknown source \"{key}\" (declare it under `sources`)"
            )));
        }
    }

    Ok(doc)
}

/// JS truthiness for the `!t?.target` / `!doc.sources[...]` guards.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn js_property_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n
            .as_f64()
            .map(crate::jsnum::number_to_string)
            .unwrap_or_else(|| n.to_string()),
        other => crate::jsnum::stringify_pretty(other),
    }
}

#[cfg(test)]
mod detect_tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, body).unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opensdk-detect-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    const CHAIN: &str = r#"{"version":1,"sources":{"a":{"inputs":[{"location":"s.yaml"}]}},
                            "targets":{"t":{"target":"node","source":"a"}}}"#;
    // The overwhelmingly common sdk.json: per-language sections, no pipeline.
    const PLAIN_SDK: &str = r#"{"version":1,"api":"s.yaml","node":{"output":"out"}}"#;

    #[test]
    fn a_chain_shaped_sdk_json_is_discovered() {
        let d = tmp("shaped");
        write(&d, "sdk.json", CHAIN);
        assert_eq!(detect_chain(&d, None), Some(d.join("sdk.json")));
    }

    #[test]
    fn a_plain_sdk_json_is_not_claimed_as_a_chain() {
        // The compat guarantee. Claiming it would turn "no chain file" into a
        // shape error for every project that has an ordinary sdk.json.
        let d = tmp("plain");
        write(&d, "sdk.json", PLAIN_SDK);
        assert_eq!(detect_chain(&d, None), None);
    }

    #[test]
    fn chain_json_still_wins_over_a_chain_shaped_sdk_json() {
        let d = tmp("order");
        write(&d, "chain.json", CHAIN);
        write(&d, "sdk.json", CHAIN);
        assert_eq!(detect_chain(&d, None), Some(d.join("chain.json")));
    }

    #[test]
    fn half_a_chain_is_not_a_chain() {
        // `resolve_chain` rejects a document missing either half, so treating one
        // as a chain would swap a clear config error for a chain error about the
        // key that IS missing.
        for body in [
            r#"{"version":1,"sources":{"a":{"inputs":[]}}}"#,
            r#"{"version":1,"targets":{"t":{"target":"node","source":"a"}}}"#,
            r#"{"version":1,"sources":{},"targets":{}}"#,
        ] {
            let d = tmp("half");
            write(&d, "sdk.json", body);
            assert_eq!(detect_chain(&d, None), None, "not a chain: {body}");
        }
    }

    #[test]
    fn an_explicit_path_is_taken_at_its_word() {
        // `--chain sdk.json` has always worked because resolve_chain reads any
        // filename; only the IMPLICIT fallback is shape-gated.
        let d = tmp("explicit");
        write(&d, "sdk.json", PLAIN_SDK);
        assert_eq!(
            detect_chain(&d, Some("sdk.json")),
            Some(d.join("sdk.json")),
            "an explicit path must not be shape-gated"
        );
    }

    #[test]
    fn unparseable_json_is_not_mistaken_for_a_chain() {
        let d = tmp("broken");
        write(&d, "sdk.json", "{ not json");
        assert_eq!(detect_chain(&d, None), None);
    }
}
