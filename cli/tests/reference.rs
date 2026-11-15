//! The `opensdk` binary's own OpenCLI document, `cli/opencli.json`.
//!
//! It is built from the binary's clap tree by `clap_opencli`, with the exit
//! codes and examples declared here, so the committed file cannot drift from
//! the parser: change a flag and this test fails until the document is
//! rewritten. `OPENCLI_UPDATE=1 cargo test -p opensdk --test reference`
//! rewrites it; review the diff. `scripts/release.sh` bumps its
//! `info.version` with the manifest's.

use clap_opencli::Document;

fn reference() -> Document {
    clap_opencli::from_command(&opensdk::command::command())
        .exit_code(0, "Success; --help and --version exit 0 too")
        .exit_code(1, "An error or a usage error")
        .at(&["diff"], |c| {
            c.exit_code(0, "No change, or only safe changes (unless --fail-on any)")
                .exit_code(
                    1,
                    "A risky change, any change under --fail-on any, or an error",
                )
                .exit_code(2, "A breaking change")
        })
        .example("opensdk generate --spec openapi.yaml --lang go")
        .example("opensdk opencli --config sdk.json --lang rust-cli --output opencli.json")
        .example("opensdk diff old.yaml new.yaml --fail-on risky")
        .build()
        .expect("the opensdk command tree describes itself")
}

#[test]
fn the_committed_reference_document_is_current() {
    clap_opencli::snapshot::assert_file(
        concat!(env!("CARGO_MANIFEST_DIR"), "/opencli.json"),
        &reference(),
    );
}

#[test]
fn the_reference_document_carries_the_manifest_version() {
    assert_eq!(reference().info.version, env!("CARGO_PKG_VERSION"));
}
