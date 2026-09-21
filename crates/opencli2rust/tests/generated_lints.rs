//! Generated Rust must survive `-D warnings`, and the goldens cannot tell us so.
//!
//! A byte-exact golden proves the generator is DETERMINISTIC, not that what it
//! emits is acceptable Rust — a consistently-emitted lint error matches its
//! golden perfectly. The fixture output trees are also not workspace members, so
//! `cargo clippy --workspace` never compiles them.
//!
//! That gap has already cost once. The merged-read branch emitted
//! `method: method` — valid Rust, rejected by clippy's `redundant_field_names`.
//! Every golden passed; apitoolchain, which builds its generated crate under
//! `cargo clippy --workspace --all-targets -- -D warnings`, went red.
//!
//! Running real clippy here would need a toolchain and a network fetch per
//! fixture. This instead greps the committed trees for the specific patterns
//! that are deny-by-default or commonly denied — cheap, offline, and aimed at
//! the failure that actually happened rather than at lint coverage in general.

use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("__fixtures__")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// `field: field` — clippy::redundant_field_names.
fn redundant_field_names(line: &str) -> Option<String> {
    let t = line.trim().trim_end_matches(',');
    let (lhs, rhs) = t.split_once(':')?;
    let (lhs, rhs) = (lhs.trim(), rhs.trim());
    if lhs.is_empty() || lhs != rhs {
        return None;
    }
    // Identifiers only — skip type ascriptions (`x: Vec<T>`) and paths (`a::b`).
    if !lhs
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    Some(format!("redundant field name `{lhs}: {rhs}`"))
}

#[test]
fn generated_rust_has_no_deny_by_default_lint_trips() {
    let mut files = Vec::new();
    rust_files(&fixtures_dir(), &mut files);
    assert!(
        !files.is_empty(),
        "no generated .rs files found — this test would be vacuously green"
    );

    let mut problems: Vec<String> = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        for (i, line) in src.lines().enumerate() {
            if let Some(what) = redundant_field_names(line) {
                problems.push(format!("{}:{}: {what}", f.display(), i + 1));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "generated Rust would fail `-D warnings` in {} place(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}
