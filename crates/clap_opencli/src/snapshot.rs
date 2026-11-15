//! A committed OpenCLI document, checked in a test.
//!
//! A CLI commits its reference document next to its code and keeps it current
//! with one test:
//!
//! ```no_run
//! # fn cli() -> clap::Command { clap::Command::new("acme").version("1.0.0") }
//! // In a test of the CLI's crate:
//! let doc = clap_opencli::from_command(&cli()).build().unwrap();
//! clap_opencli::snapshot::assert_file("opencli.json", &doc);
//! ```
//!
//! The test fails while the file differs from the document the code
//! describes. Run it with `OPENCLI_UPDATE=1` to rewrite the file, then review
//! the diff.

use std::path::Path;

use opencli_model::Document;

/// The environment variable that makes [`assert_file`] rewrite the file.
pub const UPDATE_VAR: &str = "OPENCLI_UPDATE";

/// Assert that the file at `path` holds `doc`, as [`Document::to_json`]
/// writes it. With `OPENCLI_UPDATE=1` in the environment the file is
/// (re)written instead, its directory created when missing.
///
/// A relative `path` is read from the current directory, which `cargo test`
/// sets to the package's root. Line endings are compared as `\n`, so a
/// checkout that converts them still passes.
///
/// # Panics
///
/// When the file is missing or differs (the message names the first line that
/// differs, or says the files differ only in the newlines at their end), or
/// when it cannot be read or written.
#[track_caller]
pub fn assert_file(path: impl AsRef<Path>, doc: &Document) {
    let update = std::env::var_os(UPDATE_VAR).is_some_and(|v| v == "1");
    if let Err(message) = check(path.as_ref(), &doc.to_json(), update) {
        panic!("{message}");
    }
}

/// Compare (or, with `update`, write) `path` against `expected`.
pub(crate) fn check(path: &Path, expected: &str, update: bool) -> Result<(), String> {
    let shown = path.display();
    if update {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{shown}: {e}"))?;
        }
        let current = std::fs::read_to_string(path).ok();
        if current.as_deref() != Some(expected) {
            std::fs::write(path, expected).map_err(|e| format!("{shown}: {e}"))?;
        }
        return Ok(());
    }
    let committed = match std::fs::read_to_string(path) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "{shown}: no OpenCLI document yet; run the test with {UPDATE_VAR}=1 to write it"
            ));
        }
        Err(e) => return Err(format!("{shown}: {e}")),
    };
    if committed == expected {
        return Ok(());
    }
    let rewrite = format!("run the test with {UPDATE_VAR}=1 to rewrite it, then review the diff");
    // `str::lines` would hide this one: it ignores a final newline.
    let body = |text: &str| text.trim_end_matches('\n').len();
    if committed[..body(&committed)] == expected[..body(expected)] {
        let newlines = |text: &str| text.len() - body(text);
        return Err(format!(
            "{shown} is out of date: it differs only in the newlines at its end (committed: {}, \
             generated: {});\n{rewrite}",
            newlines(&committed),
            newlines(expected)
        ));
    }
    let mut committed_lines = committed.lines();
    let mut expected_lines = expected.lines();
    let mut line = 1;
    let shown_line = |l: Option<&str>| match l.map(str::trim) {
        None => "(end of file)".to_string(),
        Some("") => "(an empty line)".to_string(),
        Some(l) => l.to_string(),
    };
    let (was, now) = loop {
        match (committed_lines.next(), expected_lines.next()) {
            (Some(a), Some(b)) if a == b => line += 1,
            // The same lines: a carriage return `str::lines` drops.
            (None, None) => {
                return Err(format!(
                    "{shown} is out of date: it differs only in its line endings;\n{rewrite}"
                ))
            }
            (a, b) => break (shown_line(a), shown_line(b)),
        }
    };
    Err(format!(
        "{shown} is out of date at line {line}:\n  committed: {was}\n  generated: {now}\n{rewrite}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clap-opencli-snapshot-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_missing_file_is_written_on_update_and_an_error_otherwise() {
        let dir = scratch("missing");
        let path = dir.join("nested").join("opencli.json");
        let err = check(&path, "{}\n", false).unwrap_err();
        assert!(err.contains("no OpenCLI document yet"), "{err}");
        assert!(err.contains("OPENCLI_UPDATE=1"), "{err}");
        check(&path, "{}\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
        check(&path, "{}\n", false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_difference_names_its_first_line_and_crlf_is_not_one() {
        let dir = scratch("differs");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencli.json");
        std::fs::write(&path, "{\r\n  \"a\": 1,\r\n  \"b\": 2\r\n}\r\n").unwrap();
        check(&path, "{\n  \"a\": 1,\n  \"b\": 2\n}\n", false).unwrap();
        let err = check(&path, "{\n  \"a\": 1,\n  \"b\": 3\n}\n", false).unwrap_err();
        assert!(err.contains("out of date at line 3"), "{err}");
        assert!(err.contains("committed: \"b\": 2"), "{err}");
        assert!(err.contains("generated: \"b\": 3"), "{err}");
        let err = check(&path, "{\n  \"a\": 1,\n  \"b\": 2\n}\n\n", false).unwrap_err();
        assert!(
            err.contains("differs only in the newlines at its end"),
            "{err}"
        );
        // A line added at the end, or a blank line inside, is named.
        let err = check(&path, "{\n  \"a\": 1,\n  \"b\": 2\n}\n{}\n", false).unwrap_err();
        assert!(err.contains("out of date at line 5"), "{err}");
        assert!(err.contains("committed: (end of file)"), "{err}");
        assert!(err.contains("generated: {}"), "{err}");
        let err = check(&path, "{\n\n  \"a\": 1,\n  \"b\": 2\n}\n", false).unwrap_err();
        assert!(err.contains("out of date at line 2"), "{err}");
        assert!(err.contains("generated: (an empty line)"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_difference_only_in_the_final_newline_is_named() {
        let dir = scratch("newline");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencli.json");
        let expected = "{\n  \"a\": 1\n}\n";
        // Saved by an editor that strips the final newline...
        std::fs::write(&path, expected.trim_end()).unwrap();
        let err = check(&path, expected, false).unwrap_err();
        assert!(
            err.starts_with(&format!(
                "{} is out of date: it differs only in the newlines at its end (committed: 0, \
                 generated: 1);",
                path.display()
            )),
            "{err}"
        );
        assert!(err.contains("OPENCLI_UPDATE=1"), "{err}");
        // ...or one that adds a blank line, with CRLF endings.
        std::fs::write(&path, "{\r\n  \"a\": 1\r\n}\r\n\r\n").unwrap();
        let err = check(&path, expected, false).unwrap_err();
        assert!(err.contains("(committed: 2, generated: 1)"), "{err}");
        // A carriage return left over (`\r\r\n`) is a line ending too.
        std::fs::write(&path, "{\r\r\n  \"a\": 1\n}\n").unwrap();
        let err = check(&path, expected, false).unwrap_err();
        assert!(err.contains("differs only in its line endings"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
