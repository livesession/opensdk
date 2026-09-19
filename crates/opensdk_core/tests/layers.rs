//! Is this repo still self-contained?
//!
//! These crates were extracted from github.com/livesession/xyd, which consumes
//! them back as a submodule. The extraction is done; the invariant it rested on
//! is permanent: **no crate here may path-depend on anything outside this repo.**
//! One `path = "../../xyd/crates/xyd_uniform"` added in a hurry builds fine on a
//! developer's machine — where xyd happens to sit next door — and fails in every
//! standalone clone and in CI.
//!
//! Two crates exist ONLY because of this rule:
//!
//! * `oas_doc` — `DocCtx` plus spec loading, split out of xyd's `xyd_openapi`.
//!   The deref engine is shared production code, so it lives here and xyd depends
//!   on it through the submodule.
//! * `parity_kit` — a vendored copy of xyd's fixture-parity comparator, because
//!   the original depends on `xyd_uniform` (xyd's docs data model, which did not
//!   come along). That copy's drift check lives on the XYD side, not here: there
//!   is no `xyd_uniform` in this repo to compare against.
//!
//! Checked by RESOLVING each declared path against the filesystem rather than by
//! pattern-matching the string. The earlier version classified `../<name>` as an
//! intra-repo edge and anything deeper as an escape — true only while every crate
//! sat at the same depth under `crates/`. `cli/` sits at the repo root and
//! legitimately reaches its dependencies as `../crates/<name>`, a shape that rule
//! would have called a leak.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/crates/opensdk_core — TWO pops reach the root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every workspace member's manifest: `crates/*` plus the root-level `cli`.
///
/// Mirrors `members` in the workspace manifest. If that list grows a third entry
/// this must grow with it, or the new crate goes unchecked.
fn member_manifests(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let crates = root.join("crates");
    let entries = std::fs::read_dir(&crates)
        .unwrap_or_else(|e| panic!("read {}: {e}", crates.display()))
        .filter_map(Result::ok);
    for e in entries {
        let manifest = e.path().join("Cargo.toml");
        if manifest.is_file() {
            out.push((e.file_name().to_string_lossy().into_owned(), manifest));
        }
    }
    let cli = root.join("cli/Cargo.toml");
    assert!(
        cli.is_file(),
        "no cli crate at {} — it is a workspace member listed separately from \
         the crates/* glob, so a move that forgets it drops the binary out of \
         --workspace entirely, silently",
        cli.display()
    );
    out.push(("cli".to_string(), cli));
    out
}

/// Every `path = "…"` a manifest declares, in any dependency section.
///
/// Sections are not distinguished on purpose: a dev-dependency escaping the repo
/// breaks a standalone clone exactly as hard as a real one, because the tests
/// have to build there too.
fn declared_paths(manifest: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with('#') {
            continue; // a commented-out dep is not a dep
        }
        let Some(i) = line.find("path") else { continue };
        // `path` must be a KEY, not the tail of one: `serde_json_path = "0.7"`
        // would otherwise parse as a path dep on "0.7".
        if i > 0 {
            let prev = line.as_bytes()[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'-' {
                continue;
            }
        }
        let rest = &line[i + 4..];
        let Some(j) = rest.find('"') else { continue };
        let rest = &rest[j + 1..];
        let Some(k) = rest.find('"') else { continue };
        let p = &rest[..k];
        // In-crate targets (`[[bin]] path = "src/bin/regen.rs"`) never leave the
        // crate. Only a `../` prefix reaches out, and only those are interesting.
        if p.starts_with("../") {
            out.push(p.to_string());
        }
    }
    out
}

#[test]
fn no_crate_depends_on_anything_outside_this_repo() {
    let root = repo_root();
    let members = member_manifests(&root);
    let mut edges = 0usize;
    let mut leaks = Vec::new();

    for (name, manifest) in &members {
        let dir = manifest.parent().expect("manifest dir");
        for p in declared_paths(manifest) {
            // Resolve for real. A path that cannot be canonicalized does not
            // exist — itself a leak, and one cargo would report only at build
            // time, in whichever clone lacks the neighbour.
            match dir.join(&p).canonicalize() {
                Ok(target) if target.starts_with(&root) => edges += 1,
                Ok(target) => leaks.push(format!(
                    "  {name} -> {p}  (resolves to {}, outside this repo)",
                    target.display()
                )),
                Err(e) => leaks.push(format!("  {name} -> {p}  (does not resolve: {e})")),
            }
        }
    }

    // The floor. Without it, a parsing change that silently matches nothing
    // leaves `leaks` empty and this test reports a green it never earned —
    // which is precisely how the by-hand version of this check once fooled me.
    assert!(
        edges >= 40,
        "only {edges} intra-repo path deps resolved across {} crates — the \
         manifest parser is broken, so an empty leak list proves nothing",
        members.len()
    );

    assert!(
        leaks.is_empty(),
        "{} dependency edge(s) leave this repo — a standalone clone cannot \
         build with these:\n{}",
        leaks.len(),
        leaks.join("\n")
    );
}
