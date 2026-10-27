//! The docs capabilities vs `docs.json` — captured from the TypeScript emitter
//! before the port, so it is a true oracle for what it captured. Every fixture
//! dir carrying BOTH an `input.json` (the committed OpenSDK IR) and a
//! `docs.json` is a case; `generate_java_usage` must reproduce `usage` byte for
//! byte and `generate_java_type_reference` must serialize to `typeReference`
//! exactly.
//!
//! Options: a fixture renders with the bag in its `docs.options.json` when it
//! carries one, else with the suite default `{ "nestedDepth": 0 }`. The 0.2.2
//! type reference grew `nested` (the transitive type view), `schemaRef` and the
//! through-array `refTypeName`; the corpus goldens were re-minted at depth 0,
//! so they stay the one-level capture plus those small additive keys — the
//! nested view of a 242-operation corpus is megabytes of golden per language
//! with no oracle behind it. `13.nested-envelope` carries `docs.options.json`
//! `{}` and renders at the default depth: the one full `nested` golden.
//!
//! `XYD_BLESS=1` re-mints every `docs.json` (`usage` + `typeReference`; `chain`
//! and `action` are kept) instead of checking it. Never set it in CI.

use serde_json::Value;
use std::path::{Path, PathBuf};

use opensdk_java::{generate_java_type_reference, generate_java_usage};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("__fixtures__")
}

/// The emitter options a fixture's golden was minted with: its
/// `docs.options.json`, else the suite default — no nested type view.
fn docs_options(dir: &Path) -> Value {
    match std::fs::read_to_string(dir.join("docs.options.json")) {
        Ok(raw) => serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{}: bad docs.options.json: {e}", dir.display())),
        Err(_) => serde_json::json!({ "nestedDepth": 0 }),
    }
}

/// `XYD_BLESS=1`: rewrite the golden's `usage` and `typeReference` in place
/// (`chain` and `action` stay), in the captured file's own layout.
fn bless_golden(dir: &Path, golden: &Value, usage: String, type_reference: Value) {
    let mut out = golden.clone();
    out["usage"] = Value::String(usage);
    out["typeReference"] = type_reference;
    let pretty = serde_json::to_string_pretty(&out).unwrap();
    std::fs::write(dir.join("docs.json"), format!("{pretty}\n")).unwrap();
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

fn is_case(dir: &Path) -> bool {
    dir.join("input.json").is_file() && dir.join("docs.json").is_file()
}

/// Every dir carrying BOTH input.json and docs.json, at either depth (the small
/// hand-written fixtures and the per-operation corpus entries).
fn cases() -> Vec<(String, PathBuf)> {
    let root = fixtures_dir();
    let mut out = Vec::new();
    for entry in sorted_dirs(&root) {
        let name = entry.file_name().unwrap().to_string_lossy().to_string();
        if is_case(&entry) {
            out.push((name, entry));
            continue;
        }
        for sub in sorted_dirs(&entry) {
            if is_case(&sub) {
                let subname = sub.file_name().unwrap().to_string_lossy();
                out.push((format!("{name}/{subname}"), sub));
            }
        }
    }
    out
}

/// The SAME method the golden describes: walk the resource tree by `chain` and
/// take its first method (what the TS `firstMethod` resolves to).
fn method_at<'a>(spec: &'a Value, chain: &[String]) -> Option<&'a Value> {
    let mut resources: &[Value] = spec.get("resources")?.as_array()?;
    let mut current: Option<&Value> = None;
    for name in chain {
        let found = resources
            .iter()
            .find(|r| r.get("name").and_then(Value::as_str) == Some(name.as_str()))?;
        current = Some(found);
        resources = found
            .get("resources")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
    }
    current?.get("methods")?.as_array()?.first()
}

/// The first line at which two strings differ, rendered for a failure message.
fn first_diff(got: &str, want: &str) -> String {
    let gl: Vec<&str> = got.lines().collect();
    let wl: Vec<&str> = want.lines().collect();
    for i in 0..gl.len().max(wl.len()) {
        let (g, w) = (gl.get(i), wl.get(i));
        if g != w {
            return format!(
                "line {}:\n      got:  {:?}\n      want: {:?}",
                i + 1,
                g.unwrap_or(&"<eof>"),
                w.unwrap_or(&"<eof>")
            );
        }
    }
    "trailing whitespace/newline differs".to_string()
}

#[test]
fn docs_match_the_typescript_golden() {
    let cases = cases();
    assert!(
        !cases.is_empty(),
        "no docs.json found — the oracle would pass vacuously"
    );
    // A floor, not an equality: the corpus may grow, but a silently shrinking
    // one would quietly narrow the oracle without failing anything.
    assert!(
        cases.len() >= 248,
        "only {} docs fixtures found (expected >= 248) — the corpus shrank",
        cases.len()
    );

    let bless = std::env::var("XYD_BLESS").as_deref() == Ok("1");
    let mut failures: Vec<String> = Vec::new();
    for (id, dir) in &cases {
        let spec: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("input.json")).unwrap())
                .unwrap();
        let golden: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("docs.json")).unwrap()).unwrap();
        let chain: Vec<String> = golden["chain"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let Some(method) = method_at(&spec, &chain) else {
            failures.push(format!("{id}: no method at chain {chain:?}"));
            continue;
        };

        let options = docs_options(dir);
        let usage = generate_java_usage(&spec, &chain, method, &options);
        let tr = serde_json::to_value(generate_java_type_reference(
            &spec, &chain, method, &options,
        ))
        .unwrap();
        if bless {
            bless_golden(dir, &golden, usage, tr);
            continue;
        }

        let want_usage = golden["usage"].as_str().unwrap();
        if usage != want_usage {
            failures.push(format!(
                "{id}: usage differs at {}",
                first_diff(&usage, want_usage)
            ));
        }
        if tr != golden["typeReference"] {
            failures.push(format!(
                "{id}: typeReference differs\n      got:  {}\n      want: {}",
                tr, golden["typeReference"]
            ));
        }
    }

    if bless {
        eprintln!("BLESSED {} docs.json", cases.len());
        return;
    }

    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn the_oracle_covers_every_committed_fixture() {
    // Guards against a silently-shrinking corpus: every fixture dir with an
    // `input.json` was frozen with a `docs.json`, so the counts must agree.
    let frozen = cases().len();
    let mut with_input = 0;
    for entry in sorted_dirs(&fixtures_dir()) {
        if entry.join("input.json").is_file() {
            with_input += 1;
            continue;
        }
        with_input += sorted_dirs(&entry)
            .iter()
            .filter(|d| d.join("input.json").is_file())
            .count();
    }
    assert_eq!(frozen, with_input, "{frozen} goldens vs {with_input} IRs");
    assert!(frozen >= 248, "corpus shrank to {frozen}");
}
