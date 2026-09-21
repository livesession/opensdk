//! `x-cli` names two different things on two different documents, and this is
//! the guard that keeps them from ever meeting:
//!
//! - on an **OpenAPI** doc it is the CLI-generation hint block read by
//!   `openapi2opencli` (the sibling of `x-sdk`);
//! - on an **OpenSDK IR** doc it means "this SDK spawns a binary"
//!   (`opensdk_cli_common::is_cli_spec`), which puts all seven emitters into
//!   CLI mode.
//!
//! Reusing the name is safe only because Stage A builds a typed IR and drops
//! unknown root keys. If that ever changed — a passthrough of root extensions,
//! say — an OpenAPI doc carrying CLI hints would silently generate seven
//! process-spawning SDKs. This test is cheap; that failure would not be.

#[test]
fn an_openapi_x_cli_does_not_survive_into_the_ir() {
    let doc = serde_json::json!({
        "openapi": "3.0.0",
        "info": { "title": "t", "version": "1" },
        "x-cli": { "grammar": "verb-noun" },
        "paths": { "/a": { "get": { "operationId": "getA",
            "responses": { "200": { "description": "ok" } } } } }
    });
    let ir = openapi2opensdk::openapi2opensdk(&doc, None).unwrap();
    let v = serde_json::to_value(&ir).unwrap();
    assert!(
        v.get("x-cli").is_none(),
        "Stage A leaked x-cli into the IR: {v:#}"
    );
    assert!(!opensdk_cli_common::is_cli_spec(&v));
}
