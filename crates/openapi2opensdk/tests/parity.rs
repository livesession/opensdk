//! Tier-1 fixture parity for the OpenAPI → OpenSDK IR converter.
//! Every fixture runs — the converter is pure data-in/data-out with no
//! JS-closure post-processing, so there is no skip-list.
//!
//! A fixture may carry an `options.json` beside its `input.json`: the converter
//! options bag (`sdkName`, `operationHints`, `sdkBehavior`, ...) exactly as the
//! CLI assembles it. Fixtures 1–9 have none (the TypeScript-minted oracles were
//! captured with no options); 10+ exercise the option-driven paths.
//!
//! `XYD_BLESS=1` writes `output.json` instead of checking it — mint a new
//! fixture's oracle with it, then review the diff. Never set it in CI.

use openapi2opensdk::{openapi2opensdk_from_json_file, Options};
use parity_kit::canon;
use serde_json::Value;

fn run_case(name: &str) {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("__fixtures__");
    let case = fixtures.join(name);
    let input = case.join("input.json");
    assert!(input.exists(), "{name}: no input.json");

    let options: Option<Options> = match std::fs::read_to_string(case.join("options.json")) {
        Ok(raw) => Some(
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{name}: bad options.json: {e}")),
        ),
        Err(_) => None,
    };
    let spec = openapi2opensdk_from_json_file(input.to_str().unwrap(), options)
        .unwrap_or_else(|e| panic!("{name}: convert failed: {e}"));
    let actual = serde_json::to_value(&spec).expect("serialize");

    if std::env::var("XYD_BLESS").as_deref() == Ok("1") {
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(case.join("output.json"), format!("{pretty}\n")).unwrap();
        eprintln!("{name}: BLESSED output.json");
        return;
    }

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
                truncate(&a.to_string()),
                truncate(&b.to_string())
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn truncate(s: &str) -> String {
    if s.len() <= 220 {
        s.to_string()
    } else {
        let mut end = 220;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

macro_rules! parity_case {
    ($test:ident, $name:literal) => {
        #[test]
        fn $test() {
            run_case($name);
        }
    };
}

parity_case!(basic, "1.basic");
parity_case!(allof, "2.allof");
parity_case!(wire_name, "3.wire-name");
parity_case!(discriminator, "4.discriminator");
parity_case!(const_literal, "5.const-literal");
parity_case!(offset_pagination, "6.offset-pagination");
parity_case!(allof_envelope, "7.allof-envelope");
parity_case!(idempotency, "8.idempotency");
parity_case!(x_open_sdk, "9.x-open-sdk");
// Option-driven paths (each carries an `options.json`):
// 10 — `operationHints["GET /gadgets"].pagination: false` keeps the nested
//      `{ data, list }` envelope as the declared response type while the
//      sibling `GET /widgets` (top-level `has_more`) is still auto-paginated;
//      `sdkBehavior.defaultHeaders` + `headersFromExtension` resolve against
//      `info.x-release` into `sdk.defaultHeaders`.
// 11 — `sdkBehavior.pagination: "none"` (the string shorthand) turns the
//      heuristic off for every operation; the `pagination: true` hint on
//      `GET /widgets` forces it back on for that one.
parity_case!(pagination_hint, "10.pagination-hint");
parity_case!(pagination_none, "11.pagination-none");
