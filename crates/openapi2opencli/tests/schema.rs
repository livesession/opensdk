//! Every OpenCLI document the repo commits validates against the OpenCLI JSON
//! Schema, and survives a trip through the typed model.
//!
//! The canonical/byte goldens prove the converter is deterministic, not that
//! its output is a valid OpenCLI document: a field emitted in the wrong shape
//! matches its golden perfectly, and the first to notice is a consumer that
//! validates (as `x-openapi.security` once showed — the schema requires each
//! entry's `type`, and a `{ scheme, scopes }` pair has none).
//!
//! The schema is `opencli_model::SCHEMA_JSON`:
//! `crates/opencli_model/schema/opencli-spec.json`, vendored VERBATIM from the
//! apitoolchain repository, `packages/apitoolchain-opencli/opencli-spec.json`
//! at commit 1e4714b459f30570220886dee0955d33a32c9784 (MIT). It is the OpenCLI
//! schema with the `x-openapi` binding declared; its objects are open (no
//! `additionalProperties: false`), so extensions such as the root
//! `x-openapi.defaultHeaders` or a command's `whenArgsPresent` validate as
//! additional properties. Refresh it by copying the file again, never by
//! editing it there.
//!
//! Every document is also validated against
//! `opencli_model::EXTENSIONS_SCHEMA_JSON`, the contract of the generic
//! extensions (`x-interactive`, `x-environment`, `x-passthrough`).
//!
//! Validated: the converter's fixture outputs, the OpenCLI inputs of both CLI
//! backends' fixtures, the documents `opensdk opencli` writes
//! (`cli/__fixtures__/opencli`), the component documents it mounts, and the
//! binary's own reference document (`cli/opencli.json`). `clap_opencli`
//! validates its goldens in its own tests.

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
    let schema: Value = serde_json::from_str(opencli_model::SCHEMA_JSON).expect("schema JSON");
    jsonschema::draft202012::new(&schema).expect("the vendored schema compiles")
}

fn extensions_validator() -> jsonschema::Validator {
    let schema: Value =
        serde_json::from_str(opencli_model::EXTENSIONS_SCHEMA_JSON).expect("schema JSON");
    jsonschema::draft202012::new(&schema).expect("the extensions schema compiles")
}

/// Validate each document against both schemas, returning `<path>: <error>
/// at <pointer>` lines.
fn violations(docs: &[PathBuf]) -> Vec<String> {
    let validators = [validator(), extensions_validator()];
    let mut out = Vec::new();
    for path in docs {
        let doc = read_json(path);
        for validator in &validators {
            for error in validator.iter_errors(&doc) {
                out.push(format!(
                    "{}: {} at {}",
                    path.display(),
                    error,
                    error.instance_path()
                ));
            }
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
    let problems = violations(&docs);
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
    assert_valid(case_files(&dir, "output.json"), 3);
}

/// The component documents `--mount` reads (one of them in the upstream
/// root-`command` shape, which the base schema's open root accepts).
#[test]
fn the_mounted_component_documents_are_valid_opencli() {
    let dir = crate_dir().join("../../cli/__fixtures__/opencli/3.mount/components");
    let mut docs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    docs.sort();
    assert_valid(docs, 2);
}

/// The `opensdk` binary's own reference document, which `clap_opencli` builds
/// from its clap tree (`cli/tests/reference.rs` keeps it current).
#[test]
fn the_binarys_reference_document_is_valid_opencli() {
    assert_valid(vec![crate_dir().join("../../cli/opencli.json")], 1);
}

/// Every committed document, parsed into the typed model and written back,
/// is the same JSON (serde_json's map equality ignores key order: the model
/// writes canonical order, the converter its own). A field the model dropped
/// or retyped fails here.
#[test]
fn every_committed_document_round_trips_through_the_typed_model() {
    let crates = crate_dir().join("..");
    let mut docs = case_files(&crate_dir().join("__fixtures__"), "output.json");
    docs.extend(case_files(
        &crates.join("opencli2rust/__fixtures__"),
        "input.json",
    ));
    docs.extend(case_files(
        &crates.join("opencli2go/__fixtures__"),
        "input.json",
    ));
    docs.extend(case_files(
        &crate_dir().join("../../cli/__fixtures__/opencli"),
        "output.json",
    ));
    docs.push(crate_dir().join("../../cli/opencli.json"));
    assert!(docs.len() >= 32, "found only {} documents", docs.len());
    for path in &docs {
        let doc = read_json(path);
        let typed = opencli_model::Document::from_value(doc.clone())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(typed.to_value(), doc, "{}", path.display());
    }
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
