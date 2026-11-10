//! Every OpenCLI document the repo commits validates against the OpenCLI JSON
//! Schema.
//!
//! The canonical/byte goldens prove the converter is deterministic, not that
//! its output is a valid OpenCLI document: a field emitted in the wrong shape
//! matches its golden perfectly, and the first to notice is a consumer that
//! validates (as `x-openapi.security` once showed — the schema requires each
//! entry's `type`, and a `{ scheme, scopes }` pair has none).
//!
//! `opencli-spec.json` beside this file is vendored VERBATIM from the
//! apitoolchain repository, `packages/apitoolchain-opencli/opencli-spec.json`
//! at commit 1e4714b459f30570220886dee0955d33a32c9784 (MIT). It is the OpenCLI
//! schema with the `x-openapi` binding declared; its objects are open (no
//! `additionalProperties: false`), so extensions such as the root
//! `x-openapi.defaultHeaders` or a command's `whenArgsPresent` validate as
//! additional properties. Refresh it by copying the file again, never by
//! editing it here.
//!
//! Validated: the converter's fixture outputs, the OpenCLI inputs of both CLI
//! backends' fixtures, and the documents `opensdk opencli` writes
//! (`cli/__fixtures__/opencli`).

use std::path::{Path, PathBuf};

use serde_json::Value;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    let raw =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// `<dir>/<case>/<file>` for every case directory that has it, sorted.
fn case_files(dir: &Path, file: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path().join(file))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
}

fn validator() -> jsonschema::Validator {
    let schema = read_json(&crate_dir().join("tests/opencli-spec.json"));
    jsonschema::draft202012::new(&schema).expect("the vendored schema compiles")
}

/// Validate each document, returning `<path>: <error> at <pointer>` lines.
fn violations(validator: &jsonschema::Validator, docs: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    for path in docs {
        let doc = read_json(path);
        for error in validator.iter_errors(&doc) {
            out.push(format!(
                "{}: {} at {}",
                path.display(),
                error,
                error.instance_path()
            ));
        }
    }
    out
}

fn assert_valid(docs: Vec<PathBuf>, at_least: usize) {
    assert!(
        docs.len() >= at_least,
        "found {} document(s), expected at least {at_least} — a moved fixture directory \
         would make this test vacuously green",
        docs.len()
    );
    let problems = violations(&validator(), &docs);
    assert!(
        problems.is_empty(),
        "{} schema violation(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn the_converters_fixture_outputs_are_valid_opencli() {
    assert_valid(
        case_files(&crate_dir().join("__fixtures__"), "output.json"),
        14,
    );
}

#[test]
fn the_cli_backends_fixture_inputs_are_valid_opencli() {
    let crates = crate_dir().join("..");
    let mut docs = case_files(&crates.join("opencli2rust/__fixtures__"), "input.json");
    docs.extend(case_files(
        &crates.join("opencli2go/__fixtures__"),
        "input.json",
    ));
    assert_valid(docs, 15);
}

#[test]
fn the_documents_opensdk_opencli_writes_are_valid_opencli() {
    let dir = crate_dir().join("../../cli/__fixtures__/opencli");
    assert_valid(case_files(&dir, "output.json"), 2);
}

/// The guard bites: the per-command security shape this test was written for.
#[test]
fn a_security_entry_without_type_is_rejected() {
    let doc = serde_json::json!({
        "opencli": "1.0.0",
        "info": { "title": "t", "version": "1" },
        "commands": [{
            "name": "list",
            "x-openapi": {
                "method": "get",
                "path": "/w",
                "security": [{ "scheme": "token", "scopes": [] }]
            }
        }]
    });
    assert!(!validator().is_valid(&doc));
}
