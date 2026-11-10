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

/// The merged read whose halves are NESTED (sharing `orgId`) and whose query
/// params differ.
///
/// Both shapes produced broken output before: the shared positional was emitted
/// twice, so clap panicked on the duplicate name; and the collection's `--limit`
/// was sent on the item request while the item's own `--include` was not
/// declared at all.
#[test]
fn merged_read_nested() {
    run_case("10.merged-read-nested");
}

/// Root `options` + `exitCodes`: the root seam. The options are clap args on
/// the root command (`recursive` → `.global(true)`), parsing goes through
/// `try_get_matches` + `CliOverrides::print_usage_error`, and
/// `CliOverrides::before_dispatch` sees the root matches before any command
/// runs. Documents without root options keep the base output (fixtures 1-10).
#[test]
fn root_options() {
    run_case("11.root-options");
}

/// `x-openapi.defaultHeaders`: `config::DEFAULT_HEADERS`, the context headers
/// they seed (sent when a request lacks them), the one-`Authorization` rule in
/// `send()`, and the request bodies — `--body @file` / `@-` / inline, and the
/// declared `Content-Type` when it is not `application/json`.
#[test]
fn default_headers() {
    run_case("12.default-headers");
}

/// `baseURL` (the spelling the sdk.json CLI sections use) and `baseUrl` both
/// reach `config.rs`; before the alias, `baseURL` passed the option split and
/// was then silently dropped.
#[test]
fn the_base_url_option_takes_both_spellings() {
    let spec: Value = serde_json::json!({
        "opencli": "1.0.0",
        "info": { "title": "widgets", "version": "1.0.0" },
        "x-openapi": { "servers": ["https://api.widgets.test"] }
    });
    for key in ["baseURL", "baseUrl"] {
        let options =
            serde_json::from_value(serde_json::json!({ key: "https://override.test" })).unwrap();
        let files = flatten(&opencli2rust(&spec, Some(options)));
        assert!(
            files["src/gen/runtime/config.rs"]
                .contains(r#"const DEFAULT_BASE_URL: &str = "https://override.test";"#),
            "{key} did not reach config.rs"
        );
    }
}

/// The root seam is switched on by any one of its three triggers and by
/// nothing else; a document without them keeps the base runtime.
#[test]
fn the_root_seam_has_three_triggers() {
    let base = serde_json::json!({
        "opencli": "1.0.0",
        "info": { "title": "widgets", "version": "1.0.0" },
        "commands": [{ "name": "ping", "x-openapi": { "method": "get", "path": "/ping" } }]
    });
    let seam = |spec: &Value| {
        flatten(&opencli2rust(spec, None))["src/gen/runtime/overrides.rs"]
            .contains("fn before_dispatch(")
    };
    assert!(!seam(&base));
    let mut empty = base.clone();
    empty["options"] = serde_json::json!([]);
    empty["exitCodes"] = serde_json::json!([]);
    empty["x-openapi"] = serde_json::json!({ "defaultHeaders": {} });
    assert!(!seam(&empty), "empty triggers are no trigger");
    for (key, value) in [
        ("options", serde_json::json!([{ "name": "verbose" }])),
        ("exitCodes", serde_json::json!([{ "code": 3 }])),
        (
            "x-openapi",
            serde_json::json!({ "defaultHeaders": { "X-A": "1" } }),
        ),
    ] {
        let mut spec = base.clone();
        spec[key] = value;
        assert!(seam(&spec), "{key} should switch the seam on");
    }
}

/// A recursive root option cannot also be required (clap rejects a required
/// global argument): it is emitted global and optional.
#[test]
fn a_required_recursive_root_option_is_emitted_optional() {
    let spec = serde_json::json!({
        "opencli": "1.0.0",
        "info": { "title": "widgets", "version": "1.0.0" },
        "options": [{ "name": "profile", "required": true, "recursive": true, "arguments": [{ "name": "name" }] }],
        "commands": [{ "name": "ping", "x-openapi": { "method": "get", "path": "/ping" } }]
    });
    let cli = flatten(&opencli2rust(&spec, None))["src/gen/cli.rs"].clone();
    let arg = &cli[cli.find(r#"Arg::new("profile")"#).unwrap()..];
    let arg = &arg[..arg.find(".subcommand_required").unwrap()];
    assert!(arg.contains(".global(true)"), "{arg}");
    assert!(!arg.contains(".required(true)"), "{arg}");
}
