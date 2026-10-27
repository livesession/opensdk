//! The docs capabilities vs `docs.json` — captured from the TypeScript emitter
//! before the port, so it is a true oracle for what it captured. `usage` must
//! match BYTE-exactly and `typeReference` must serialize to the identical JSON
//! shape.
//!
//! One `docs.json` per operation sits next to the `input.json` IR it was built
//! from (the small hand-written fixtures at depth 1, the 242-operation OpenAI
//! corpus at depth 2), so a byte divergence names the exact operation.
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

use std::path::{Path, PathBuf};

use opensdk_ruby::{generate_ruby_type_reference, generate_ruby_usage};
use serde_json::Value;

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

/// Every dir carrying BOTH `input.json` and `docs.json`, at either depth.
fn cases() -> Vec<(String, PathBuf)> {
    let root = fixtures_dir();
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    let mut tops: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    tops.sort();
    for top in tops {
        if !top.is_dir() {
            continue;
        }
        let label = top.file_name().unwrap().to_string_lossy().to_string();
        if top.join("input.json").exists() {
            if top.join("docs.json").exists() {
                out.push((label, top));
            }
            continue;
        }
        let mut subs: Vec<PathBuf> = match std::fs::read_dir(&top) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => continue,
        };
        subs.sort();
        for sub in subs {
            if !sub.is_dir() {
                continue;
            }
            if sub.join("input.json").exists() && sub.join("docs.json").exists() {
                let name = sub.file_name().unwrap().to_string_lossy().to_string();
                out.push((format!("{label}/{name}"), sub));
            }
        }
    }
    out
}

/// The method the golden describes: walk the resource tree by `chain` (the
/// resource-NAME path) and take its first method — what `firstMethod` returns.
fn resolve_method(spec: &Value, chain: &[String]) -> Option<Value> {
    let mut level = spec.get("resources")?.as_array()?.clone();
    let mut current: Option<Value> = None;
    for name in chain {
        let found = level
            .iter()
            .find(|r| r.get("name").and_then(Value::as_str) == Some(name.as_str()))?
            .clone();
        level = found
            .get("resources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        current = Some(found);
    }
    current?
        .get("methods")
        .and_then(Value::as_array)
        .and_then(|m| m.first())
        .cloned()
}

/// The 1-based first differing line, with both sides, for a precise report.
fn first_diff(golden: &str, got: &str) -> String {
    let g: Vec<&str> = golden.split('\n').collect();
    let r: Vec<&str> = got.split('\n').collect();
    for i in 0..g.len().max(r.len()) {
        let a = g.get(i).copied().unwrap_or("<EOF>");
        let b = r.get(i).copied().unwrap_or("<EOF>");
        if a != b {
            return format!("first diff line {}: golden={a:?} rust={b:?}", i + 1);
        }
    }
    "(identical by line; length/tail differs)".to_string()
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
        let Some(method) = resolve_method(&spec, &chain) else {
            failures.push(format!(
                "{id}: could not resolve the method for chain {chain:?}"
            ));
            continue;
        };

        let options = docs_options(dir);
        let usage = generate_ruby_usage(&spec, &chain, &method, &options);
        let tr = serde_json::to_value(generate_ruby_type_reference(
            &spec, &chain, &method, &options,
        ))
        .unwrap();
        if bless {
            bless_golden(dir, &golden, usage, tr);
            continue;
        }

        let want_usage = golden["usage"].as_str().unwrap();
        if usage != want_usage {
            failures.push(format!("{id} [usage] {}", first_diff(want_usage, &usage)));
        }
        if tr != golden["typeReference"] {
            let want = serde_json::to_string_pretty(&golden["typeReference"]).unwrap();
            let got = serde_json::to_string_pretty(&tr).unwrap();
            failures.push(format!("{id} [typeReference] {}", first_diff(&want, &got)));
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

/// The one docs behaviour the goldens CANNOT cover: the real docs pipeline
/// passes `emitterOptions: {}`, so no committed `docs.json` exercises
/// `baseUrlEnv`. Without this, a `baseUrlEnv` that silently did nothing would
/// still pass the oracle above.
#[test]
fn base_url_env_is_snippet_only_and_off_by_default() {
    let (_, dir) = cases()
        .into_iter()
        .find(|(id, _)| id == "1.basic")
        .expect("1.basic fixture");
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("input.json")).unwrap()).unwrap();
    let golden: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("docs.json")).unwrap()).unwrap();
    let chain: Vec<String> = golden["chain"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let method = resolve_method(&spec, &chain).unwrap();

    let default = generate_ruby_usage(&spec, &chain, &method, &Value::Null);
    // An empty bag, and an empty-string value (falsy in JS), both stay default.
    for bag in [
        serde_json::json!({}),
        serde_json::json!({ "baseUrlEnv": "" }),
    ] {
        assert_eq!(
            generate_ruby_usage(&spec, &chain, &method, &bag),
            default,
            "emitterOptions {bag} must not alter the snippet"
        );
    }

    let with = generate_ruby_usage(
        &spec,
        &chain,
        &method,
        &serde_json::json!({ "baseUrlEnv": "RECORDER_BASE_URL" }),
    );
    assert!(
        with.contains(r#"base_url: ENV["RECORDER_BASE_URL"]"#),
        "baseUrlEnv never reached the client line:\n{with}"
    );
    // Only the client line moves: everything else is byte-identical.
    assert_eq!(
        with.replace(r#", base_url: ENV["RECORDER_BASE_URL"]"#, ""),
        default
    );

    // The type reference is options-independent (a docs-only base URL is not a
    // type), so the bag must not perturb it.
    let tr = |o: &Value| {
        serde_json::to_value(generate_ruby_type_reference(&spec, &chain, &method, o)).unwrap()
    };
    assert_eq!(
        tr(&serde_json::json!({ "baseUrlEnv": "RECORDER_BASE_URL" })),
        tr(&Value::Null)
    );
}
