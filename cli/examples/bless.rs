//! Mint (or re-mint) an emitter fixture's goldens — DEV ONLY, never run in CI.
//!
//! The per-language parity suites compare byte for byte against committed
//! trees and have no write mode of their own (seven harnesses, seven shapes).
//! This example is the one place that writes them, through the same registry
//! `opensdk generate` and `opensdk xsdk` dispatch through:
//!
//! ```text
//! cargo run -p opensdk --example bless -- <lang> <fixture-dir>... [--docs-only] [--options '<json>']
//! ```
//!
//! For each fixture dir (one carrying an `input.json` OpenSDK IR) it writes:
//!
//! * `output/` — the full generated tree (replaced wholesale), unless
//!   `--docs-only`;
//! * `docs.json` — `{ chain, action, usage, typeReference }` for the FIRST
//!   method of the resource tree (the shape every `tests/docs.rs` reads), when
//!   the emitter has the docs capabilities (every language but Rust).
//!
//! `--options` is the emitter options bag (the language section of an
//! sdk.json), for probing; the committed fixtures are minted with none.
//!
//! Review the resulting diff before committing it: a golden is only as right as
//! the eyes that blessed it.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use opensdk::registry::get_emitter;
use opensdk_core::emitter::walk_methods;

fn main() {
    let mut lang: Option<String> = None;
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut docs_only = false;
    let mut options = Value::Null;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--docs-only" => docs_only = true,
            "--options" => {
                let raw = args.next().expect("--options takes a JSON object");
                options = serde_json::from_str(&raw).expect("--options is not valid JSON");
            }
            _ if lang.is_none() => lang = Some(arg),
            _ => dirs.push(PathBuf::from(arg)),
        }
    }
    let lang = lang.expect("usage: bless <lang> <fixture-dir>... [--docs-only] [--options <json>]");
    assert!(!dirs.is_empty(), "no fixture dirs given");
    let entry = get_emitter(&lang).unwrap_or_else(|e| panic!("{}", e.0));

    for dir in &dirs {
        bless_one(entry, dir, docs_only, &options);
    }
}

fn bless_one(
    entry: &opensdk::registry::EmitterEntry,
    dir: &Path,
    docs_only: bool,
    options: &Value,
) {
    let input = dir.join("input.json");
    let ir: Value = serde_json::from_str(
        &std::fs::read_to_string(&input).unwrap_or_else(|e| panic!("read {input:?}: {e}")),
    )
    .unwrap_or_else(|e| panic!("parse {input:?}: {e}"));

    if !docs_only {
        let out = dir.join("output");
        let _ = std::fs::remove_dir_all(&out);
        let files = (entry.generate_files)(&ir, options);
        for (rel, file) in &files {
            let path = out.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, &file.content).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        }
        eprintln!(
            "{}: wrote {} file(s) under {}",
            entry.language(),
            files.len(),
            out.display()
        );
    }

    let (Some(usage_fn), Some(tr_fn)) =
        (entry.fns.generate_usage, entry.fns.generate_type_reference)
    else {
        eprintln!(
            "{}: no docs capability — docs.json skipped",
            entry.language()
        );
        return;
    };
    // The first method, pre-order with a resource's own methods before its
    // children's — exactly `firstMethod` in every docs harness.
    let Some((chain, method)) = walk_methods(&ir).into_iter().next() else {
        eprintln!("{}: no methods — docs.json skipped", entry.language());
        return;
    };
    let golden = json!({
        "chain": chain,
        "action": method.get("action").cloned().unwrap_or(Value::Null),
        "usage": usage_fn(&ir, &chain, method, options),
        "typeReference": tr_fn(&ir, &chain, method, options),
    });
    let path = dir.join("docs.json");
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&golden).unwrap()),
    )
    .unwrap_or_else(|e| panic!("write {path:?}: {e}"));
    eprintln!("{}: wrote {}", entry.language(), path.display());
}
