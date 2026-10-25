//! Does the generated Ruby SDK actually COMPILE?
//!
//! The byte-exact goldens (`parity.rs`, `docs.rs`) prove the emitter produces the
//! same output it always did — but a syntax error emitted CONSISTENTLY matches its
//! golden perfectly, and a regenerated golden simply blesses it. This tier is the
//! independent check that does not depend on the golden being right.
//!
//! Ported from the deleted `@xyd-js/opensdk-ci`; recover the original with
//! `git show 4c9ae3b1^:packages/xyd-opensdk-ci/src/compile-smoke.ts`.
//!
//! Skips when ruby is absent so `cargo test --workspace` stays offline. Set
//! `XYD_SMOKE_RUBY=1` to turn a missing toolchain into a FAILURE — that is what CI
//! uses, so the tier cannot silently stop running there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use opensdk_e2e::{compile_smoke, toolchain_available};
use opensdk_ruby::generate_ruby;
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("__fixtures__")
}

fn write_project(files: &BTreeMap<String, String>, dir: &Path) {
    for (rel, contents) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create dir");
        }
        std::fs::write(&path, contents).expect("write generated file");
    }
}

/// Generate `case` into a fresh temp dir and compile it.
fn compile_case(case: &str) {
    let input = fixtures_dir().join(case).join("input.json");
    let ir: Value = serde_json::from_str(
        &std::fs::read_to_string(&input)
            .unwrap_or_else(|e| panic!("read {}: {e}", input.display())),
    )
    .expect("parse IR");

    let dir = std::env::temp_dir().join(format!("xyd_rb_smoke_{case}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_project(&generate_ruby(&ir), &dir);

    let result = compile_smoke("ruby", &dir);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(true) => {}
        Ok(false) => panic!("ruby vanished mid-test"),
        Err(e) => panic!("[{case}] generated Ruby does not compile:\n{e}"),
    }
}

/// An absent toolchain is a SKIP locally but a FAILURE under the gate — otherwise
/// the whole tier could stop running in CI and nothing would say so.
fn gated_skip() -> bool {
    if toolchain_available("ruby") {
        return false;
    }
    if std::env::var("XYD_SMOKE_RUBY").as_deref() == Ok("1") {
        panic!("XYD_SMOKE_RUBY=1 but ruby is not installed");
    }
    eprintln!("ruby absent — compile smoke SKIPPED");
    true
}

#[test]
fn basic_sdk_compiles() {
    if gated_skip() {
        return;
    }
    compile_case("1.basic");
}

#[test]
fn wire_sdk_compiles() {
    if gated_skip() {
        return;
    }
    compile_case("2.wire");
}

#[test]
fn unions_sdk_compiles() {
    if gated_skip() {
        return;
    }
    compile_case("3.unions");
}

#[test]
fn sdk_behavior_overrides_compile() {
    if gated_skip() {
        return;
    }
    compile_case("10.sdk-behavior");
}

// The default-headers runtime path (sdk.defaultHeaders) + an un-paginated list
// envelope must compile too — a seam that only ever ran under the golden check
// would be a syntax error waiting to be blessed.
#[test]
fn default_headers_sdk_compiles() {
    if gated_skip() {
        return;
    }
    compile_case("12.default-headers");
}

// The knob the Client exposes when the SDK carries default headers: a
// per-client `default_headers:` must reach the Transport, win over the built-in
// entry of the same name and keep the others. `ruby -c` cannot see that, so this
// one RUNS the generated code.
#[test]
fn default_headers_knob_reaches_the_transport() {
    if gated_skip() {
        return;
    }
    let input = fixtures_dir().join("12.default-headers").join("input.json");
    let ir: Value = serde_json::from_str(
        &std::fs::read_to_string(&input)
            .unwrap_or_else(|e| panic!("read {}: {e}", input.display())),
    )
    .expect("parse IR");
    let dir = std::env::temp_dir().join("xyd_rb_smoke_12_knob");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    write_project(&generate_ruby(&ir), &dir);

    let script = r#"
require "widgetco"
client = Widgetco::Client.new(api_key: "k", default_headers: { "X-Some-Version" => "9.9", "X-Extra" => "e" })
headers = client.instance_variable_get(:@transport).instance_variable_get(:@default_headers)
expected = { "X-Some-Version" => "9.9", "X-Release" => "2024-10-01", "X-Extra" => "e" }
abort("default headers: #{headers.inspect}") unless headers == expected
plain = Widgetco::Client.new(api_key: "k").instance_variable_get(:@transport).instance_variable_get(:@default_headers)
abort("built-in defaults: #{plain.inspect}") unless plain == Widgetco::DEFAULT_HEADERS
"#;
    let out = std::process::Command::new("ruby")
        .args(["-Ilib", "-e", script])
        .current_dir(&dir)
        .output()
        .expect("run ruby");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the default_headers: knob is not wired:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
