//! The CI job that holds clap at the floor (`clap-floor` in
//! `.github/workflows/ci.yml`) must run the oldest clap this crate's manifest
//! admits: a pin above it would let a getter added since go unnoticed, and
//! break a consumer locked on an older release.

use std::path::Path;

/// The oldest release a caret requirement admits: `4.5` is `4.5.0`.
fn oldest(requirement: &str) -> String {
    let mut parts: Vec<&str> = requirement.trim_start_matches('^').split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

/// The version requirement of the first `clap = ...` line after `[section]`.
fn clap_requirement(manifest: &str, section: &str) -> String {
    let line = manifest
        .lines()
        .skip_while(|l| l.trim() != section)
        .find(|l| l.starts_with("clap = "))
        .unwrap_or_else(|| panic!("no clap under {section}"));
    let version = line
        .split("version = \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no version in `{line}`"));
    version.to_string()
}

#[test]
fn the_ci_floor_is_the_oldest_clap_the_manifest_admits() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Ok(ci) = std::fs::read_to_string(crate_dir.join("../../.github/workflows/ci.yml")) else {
        // Outside the repository (a packaged crate): nothing to keep in step.
        return;
    };
    let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
    let requirement = clap_requirement(&manifest, "[dependencies]");
    assert_eq!(
        clap_requirement(&manifest, "[dev-dependencies]"),
        requirement,
        "the tests build against the same clap requirement as the crate"
    );
    let pin = format!("cargo update -p clap --precise {}", oldest(&requirement));
    assert!(
        ci.lines().any(|l| l.trim() == pin),
        "the clap floor job should run `{pin}` (the manifest requires clap {requirement})"
    );
}

#[test]
fn the_oldest_release_of_a_requirement() {
    assert_eq!(oldest("4.5"), "4.5.0");
    assert_eq!(oldest("^4.5.20"), "4.5.20");
    assert_eq!(oldest("4"), "4.0.0");
}
