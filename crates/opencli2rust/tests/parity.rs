//! Tier-1 golden parity: opencli2rust(input.json) === the committed output/ tree,
//! byte-exact per file. Inline harness (this worktree lacks xyd_parity — see the
//! standalone Cargo.toml note). The env-gated cargo/e2e smokes are out of scope.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use opencli2rust::{flatten, opencli2rust};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    // Fixtures live IN the crate (relocated when the TS package was deleted).
    Path::new(env!("CARGO_MANIFEST_DIR")).join("__fixtures__")
}

fn list_tree(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let p = entry.path();
        if p.is_dir() {
            list_tree(&p, base, out);
        } else {
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read_to_string(&p).unwrap());
        }
    }
}

fn run_case(name: &str) {
    let case = fixtures_dir().join(name);
    let spec: Value =
        serde_json::from_str(&std::fs::read_to_string(case.join("input.json")).unwrap()).unwrap();
    let generated = flatten(&opencli2rust(&spec, None));

    let out_dir = case.join("output");

    // `XYD_BLESS=1` rewrites the golden tree instead of checking it — the same
    // gate the other crates use. Never set it in CI: it would turn every
    // regression into a passing test that quietly rewrites its own expectation.
    if std::env::var("XYD_BLESS").as_deref() == Ok("1") {
        let _ = std::fs::remove_dir_all(&out_dir);
        for (rel, content) in &generated {
            let p = out_dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        eprintln!("blessed {name} ({} files)", generated.len());
        return;
    }

    let mut expected: BTreeMap<String, String> = BTreeMap::new();
    list_tree(&out_dir, &out_dir, &mut expected);

    // Same set of paths.
    let gen_keys: Vec<&String> = generated.keys().collect();
    let exp_keys: Vec<&String> = expected.keys().collect();
    assert_eq!(gen_keys, exp_keys, "{name}: file set differs");

    // Byte-exact content per file (report the first divergence with context).
    for (rel, content) in &generated {
        let want = &expected[rel];
        if content != want {
            let first = content
                .lines()
                .zip(want.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b);
            panic!(
                "{name}/{rel}: content differs (gen {} bytes, want {} bytes){}",
                content.len(),
                want.len(),
                match first {
                    Some((i, (a, b))) =>
                        format!("\n  line {}:\n    gen:  {a:?}\n    want: {b:?}", i + 1),
                    None => String::new(),
                }
            );
        }
    }
}

#[test]
fn basic() {
    run_case("1.basic");
}
#[test]
fn crud() {
    run_case("2.crud");
}
#[test]
fn nested() {
    run_case("3.nested");
}
#[test]
fn body_flatten() {
    run_case("4.body-flatten");
}
#[test]
fn local_tool() {
    run_case("6.local-tool");
}
#[test]
fn mixed() {
    run_case("7.mixed");
}

/// A command that is BOTH runnable and a parent — the shape a kubectl-style
/// grammar produces (`get sdks` lists, `get sdk <id>` retrieves, `get sdk
/// targets <id>` is a child of the same node).
///
/// It used to be unrepresentable: the `commands` branch won and the node's
/// `x-openapi` binding was silently dropped, so the command existed in `--help`
/// but could never be invoked. This fixture is the regression guard for that.
#[test]
fn runnable_parent() {
    run_case("8.runnable-parent");
}

/// The merged read command: ONE command carrying TWO HTTP bindings
/// (`x-openapi.whenArgsPresent`), chosen by whether its optional positional was
/// supplied — `get sdks` lists, `get sdk <id>` retrieves.
///
/// Neither backend had a fixture for this path, and that gap has already cost
/// once: the branch emitted `method: method`, which is valid Rust but trips
/// clippy's `redundant_field_names`. Every golden passed, then a downstream
/// repo building the generated crate under `-D warnings` went red.
#[test]
fn merged_read() {
    run_case("9.merged-read");
}
