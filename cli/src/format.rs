//! Format generated files IN MEMORY, before they are written.
//!
//! WHY BEFORE, NOT AFTER. `.sdk/sdk.lock` records a hash of the PRISTINE
//! generated content — `opensdk_framework`'s own header says so: the lock holds
//! "the PRISTINE generated content — NOT any post-fmt bytes". Format after the
//! write and every file's on-disk hash disagrees with its lock entry, which
//! quietly breaks three things at once:
//!
//!   - the stale-prune guard stops pruning, because every orphan now looks
//!     locally modified;
//!   - no regeneration is ever a byte-stable no-op, because every file differs
//!     from its record and is rewritten;
//!   - `--merge` 3-way-merges each file against an UNFORMATTED base, so the
//!     whole tree reads as hand-edited.
//!
//! That is the state a post-write `cargo fmt` leaves behind — including the one
//! in `opencli2rust`'s `regen` bin and the one consumers wrap around this CLI.
//! Formatting the map first makes the lock hash exactly what lands on disk, so
//! none of the three can arise rather than each needing its own workaround.
//!
//! It also lives HERE, in the binary, for the reason `exec.rs` already records:
//! `opensdk_framework` is the pure write/merge lifecycle and is linked into the
//! `@xyd-js/native` cdylib, where a `std::process::Command` dependency does not
//! belong. The emitter crates are pure for the same reason.

use std::io::Write;
use std::process::{Command, Stdio};

use opensdk_framework::write::FileMap;

use crate::error::{Error, Result};

/// Format every file in `files` that a known formatter claims.
///
/// Returns how many files the formatter actually changed. Opt-in: callers only
/// reach this when the target asked for it, so a missing formatter is a hard
/// error rather than a silent skip — a build that quietly stopped formatting is
/// exactly the drift this feature exists to end.
pub fn format_file_map(files: &mut FileMap) -> Result<usize> {
    let rust_paths: Vec<String> = files
        .iter()
        .filter(|(p, _)| p.ends_with(".rs"))
        .map(|(p, _)| p.clone())
        .collect();
    if rust_paths.is_empty() {
        return Ok(0);
    }

    let edition = rust_edition(files);
    let mut changed = 0usize;
    for (path, entry) in files.iter_mut() {
        if !path.ends_with(".rs") {
            continue;
        }
        let formatted = rustfmt(&entry.content, &edition)
            .map_err(|e| Error::msg(format!("rustfmt {path}: {e}")))?;
        if formatted != entry.content {
            entry.content = formatted;
            changed += 1;
        }
    }
    Ok(changed)
}

/// The edition to format under, read from the Cargo.toml the generator emitted.
///
/// This is not a nicety: rustfmt defaults to edition 2015, under which `async
/// fn` is a syntax error — and the generated clients are async throughout. Get
/// this wrong and every file fails to parse, which is why the fallback is a
/// modern edition rather than rustfmt's own default.
fn rust_edition(files: &FileMap) -> String {
    files
        .iter()
        .find(|(p, _)| p == "Cargo.toml" || p.ends_with("/Cargo.toml"))
        .and_then(|(_, e)| {
            e.content.lines().find_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("edition")?.trim_start();
                let rest = rest.strip_prefix('=')?.trim();
                rest.trim_matches('"').to_string().into()
            })
        })
        .unwrap_or_else(|| "2021".to_string())
}

/// Run `rustfmt` over `source`, via stdin.
///
/// stdin rather than a path on purpose: given a path, rustfmt follows `mod`
/// declarations and reformats files the caller never named — which would reach
/// outside the generated set and touch user-owned files like the `SkipIfExists`
/// `src/custom/mod.rs` scaffold. Reading from stdin formats exactly one file and
/// nothing else.
fn rustfmt(source: &str, edition: &str) -> std::result::Result<String, String> {
    let mut child = Command::new("rustfmt")
        .args(["--emit", "stdout", "--edition", edition, "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "rustfmt not found on PATH (install it with `rustup component add rustfmt`)"
                    .to_string()
            } else {
                e.to_string()
            }
        })?;

    child
        .stdin
        .take()
        .ok_or_else(|| "could not open rustfmt stdin".to_string())?
        .write_all(source.as_bytes())
        .map_err(|e| e.to_string())?;

    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.trim().to_string());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensdk_framework::write::{FileEntry, WriteMode};

    /// `FileEntry` derives neither `PartialEq` nor `Debug`, and adding them to
    /// another crate to satisfy a test here would be the tail wagging the dog.
    fn snapshot(files: &FileMap) -> Vec<(String, String)> {
        files
            .iter()
            .map(|(p, e)| (p.clone(), e.content.clone()))
            .collect()
    }

    fn map(entries: &[(&str, &str)]) -> FileMap {
        entries
            .iter()
            .map(|(p, c)| {
                (
                    (*p).to_string(),
                    FileEntry {
                        content: (*c).to_string(),
                        write_mode: WriteMode::Overwrite,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_map_with_no_rust_is_untouched_and_never_spawns_a_formatter() {
        // The early return matters beyond speed: a node-only target must not
        // fail on a machine without rustfmt.
        let mut files = map(&[("package.json", "{}\n"), ("src/index.ts", "export {}\n")]);
        let before: Vec<(String, String)> = snapshot(&files);
        assert_eq!(format_file_map(&mut files).unwrap(), 0);
        assert_eq!(snapshot(&files), before);
    }

    #[test]
    fn the_edition_comes_from_the_generated_manifest() {
        let files = map(&[(
            "Cargo.toml",
            "[package]\nname = \"x\"\nedition = \"2024\"\n",
        )]);
        assert_eq!(rust_edition(&files), "2024");
    }

    #[test]
    fn the_edition_falls_back_to_a_modern_one_not_rustfmts_2015_default() {
        // rustfmt's own default is 2015, under which the generated `async fn`
        // does not parse. A wrong fallback here fails every file.
        assert_eq!(rust_edition(&map(&[("Cargo.toml", "[package]\n")])), "2021");
        assert_eq!(rust_edition(&map(&[])), "2021");
    }

    #[test]
    fn formatting_is_idempotent_and_reports_only_real_changes() {
        if Command::new("rustfmt").arg("--version").output().is_err() {
            return; // no rustfmt on this machine; the gated tiers cover it
        }
        let mut files = map(&[("src/lib.rs", "pub fn  a( ) ->u8{1}\n")]);
        assert_eq!(format_file_map(&mut files).unwrap(), 1);
        // A second pass changes nothing — which is what makes a regeneration a
        // byte-stable no-op instead of an endless diff.
        assert_eq!(format_file_map(&mut files).unwrap(), 0);
    }

    #[test]
    fn async_fn_formats_rather_than_failing_to_parse() {
        if Command::new("rustfmt").arg("--version").output().is_err() {
            return;
        }
        let mut files = map(&[
            ("Cargo.toml", "[package]\nedition = \"2021\"\n"),
            ("src/main.rs", "pub async fn  go( )  {  }\n"),
        ]);
        assert!(format_file_map(&mut files).is_ok(), "async fn must parse");
    }
}
