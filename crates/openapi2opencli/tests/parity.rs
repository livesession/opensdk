//! Tier-1 fixture parity for the OpenAPI → OpenCLI converter. The 5 fixtures
//! are pre-flattened OpenAPI (no $ref/allOf), so read_spec + convert matches
//! the JS `deferencedOpenAPI → openapi2opencli`. (-2.complex.openai is the
//! separate conformance oracle, not a golden fixture.)

use openapi2opencli::openapi2opencli_from_file;
use parity_kit::canon;
use serde_json::Value;

fn run_case(name: &str) {
    // Fixtures live IN the crate (relocated when the TS package was deleted);
    // In-crate since the A5 sweep — the old xyd_parity::fixtures_dir resolved
    // ../../packages/<pkg>/__fixtures__, which no longer exists.
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("__fixtures__");
    let case = fixtures.join(name);
    let input = case.join("input.yaml");
    assert!(input.exists(), "{name}: no input.yaml");

    // Converter options come from an OPTIONAL `options.json` beside the input.
    // The five original cases have none, so they still convert with `None` and
    // stay byte-identical; a case that exercises an option is self-describing
    // rather than needing a parallel table in this file.
    let options_path = case.join("options.json");
    let options = options_path.exists().then(|| {
        let raw = std::fs::read_to_string(&options_path)
            .unwrap_or_else(|e| panic!("{name}: reading options.json: {e}"));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{name}: options.json is not valid Options: {e}"))
    });

    let spec = openapi2opencli_from_file(input.to_str().unwrap(), options)
        .unwrap_or_else(|e| panic!("{name}: convert failed: {e}"));
    let actual = serde_json::to_value(&spec).expect("serialize");
    let oracle: Value = parity_kit::read_oracle(&case);

    if std::env::var("XYD_PARITY_DUMP").as_deref() == Ok("1") {
        std::fs::write(
            case.join("output.rust.json"),
            serde_json::to_string_pretty(&actual).unwrap(),
        )
        .unwrap();
    }

    let diffs = canon::diff_paths(&actual, &oracle, 12);
    assert!(
        diffs.is_empty(),
        "{name}: PARITY FAILED — first divergences:\n{}",
        diffs
            .iter()
            .map(|(p, a, b)| format!(
                "  at {p}\n    rust:   {}\n    oracle: {}",
                trunc(a),
                trunc(b)
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn trunc(v: &Value) -> String {
    let s = v.to_string();
    if s.len() <= 200 {
        s
    } else {
        let mut e = 200;
        while !s.is_char_boundary(e) {
            e -= 1;
        }
        format!("{}…", &s[..e])
    }
}

macro_rules! c {
    ($t:ident, $n:literal) => {
        #[test]
        fn $t() {
            run_case($n);
        }
    };
}
c!(basic, "1.basic");
c!(crud, "2.crud");
c!(nested, "3.nested");
c!(body_flatten, "4.body-flatten");
c!(responses, "5.responses");
// `rootCommand` wraps the whole tree under one parent. Same spec as 2.crud, so
// the only difference between the two goldens IS the wrapper — which is what
// makes this readable as a diff.
c!(root_command, "6.root-command");

// Verb-first placement with kubectl-style reads. Pins, in one golden:
//   - list + retrieve collapse into ONE command, singular canonical, plural
//     alias, and a second binding under `whenArgsPresent`
//   - that command is ALSO a parent (`get sdk targets`), i.e. a runnable node
//   - `apis` needs an override or it collides with itself, so the override is
//     part of the fixture rather than a footnote
//   - `usage` and other already-singular nouns pass through untouched
c!(grammar_verb_noun, "7.grammar-verb-noun");

// The spec configuring its own CLI through a root `x-cli` block, with NO
// converter options passed. Pins that every option is reachable from the spec
// (name, grammar and the override table all take effect here) — the block
// deserializes into the same `Options` type, so this covers the mechanism
// rather than a chosen subset of keys.
c!(x_cli_root, "8.x-cli-root");

// Per-operation `x-cli`, and the case it exists for: MIXING. The root block
// puts the document in verb-noun, and one pair of operations opts back into
// noun-verb, so both word orders live in one tree — no path-glob matcher, no
// second dialect. The same fixture pins each explicit override (group, verb,
// aliases, hidden, description, ignore) and the path-item → operation ladder.
c!(x_cli_operation, "9.x-cli-operation");

// A `$ref`'d request-body schema still flattens into per-field options. This
// regressed silently once: `ctx.resolve()` on the requestBody unwraps a ref in
// the object SLOT, not one nested under `content.<media>.schema`, so a normal
// spec produced a single `--body '<json>'` where four flags belonged. It was
// invisible because the converter used to be handed pre-dereferenced input.
c!(ref_body_flattens, "10.ref-body-flattens");
