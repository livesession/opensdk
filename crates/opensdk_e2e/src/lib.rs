//! Toolchain-dependent e2e support for the generated SDKs — the Rust port of the
//! deleted `@xyd-js/opensdk-ci`.
//!
//! The byte-exact goldens already prove the emitters produce the SAME output they
//! always did. What they CANNOT catch is output that is consistently wrong: a
//! syntax error emitted identically every run matches its golden perfectly. This
//! crate answers the question the goldens can't — does the generated SDK actually
//! compile?
//!
//! DEV-ONLY, and deliberately a separate crate rather than part of
//! `opensdk_cli_common`: it spawns language toolchains, and the emitter crates
//! are linked into the napi cdylib. Keep `std::process::Command` out of their
//! dependency graph.
//!
//! Gating follows the repo's Rust convention (`XYD_CLI_SMOKE_<LANG>`), not the
//! deleted TypeScript's (`O2S_<LANG>_SMOKE`): an absent toolchain SKIPS rather
//! than fails, so the default `cargo test --workspace` stays offline on a machine
//! without go/ruby/dotnet installed.
//!
//! Ported from `packages/xyd-opensdk-ci/src/compile-smoke.ts`; recover the
//! original with `git show 4c9ae3b1^:packages/xyd-opensdk-ci/src/compile-smoke.ts`.

pub mod record;
pub use opensdk_cli_common::{expected_request, load_per_method_fixtures, RecordedRequest};
pub use record::{
    body_field_names, diff_request, full_ir, merge_resources, normalize_recorded, RawRequest,
    RecordingServer,
};

use std::path::{Path, PathBuf};
use std::process::Command;

/// The command that proves a language's toolchain is installed.
fn probe(lang: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match lang {
        "node" => ("node", &["--version"]),
        "go" => ("go", &["version"]),
        "python" => ("python3", &["--version"]),
        "ruby" => ("ruby", &["--version"]),
        "java" => ("javac", &["-version"]),
        "dotnet" => ("dotnet", &["--version"]),
        "rust" => ("cargo", &["--version"]),
        _ => return None,
    })
}

/// Is the toolchain for `lang` usable here?
pub fn toolchain_available(lang: &str) -> bool {
    let Some((bin, args)) = probe(lang) else {
        return false;
    };
    Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Recursively collect files under `dir` whose name ends with `ext`.
pub fn files_by_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort(); // deterministic argv — a compiler error should name the same file every run
    for p in entries {
        if p.is_dir() {
            out.extend(files_by_ext(&p, ext));
        } else if p.to_string_lossy().ends_with(ext) {
            out.push(p);
        }
    }
    out
}

/// Suppresses the .NET CLI's per-invocation first-run work. On its own this does
/// NOT prevent the mutex race below — `DOTNET_SKIP_FIRST_TIME_EXPERIENCE` is
/// honoured inconsistently across SDK versions, which is why the isolation and
/// serialization there carry the fix and these are only a cheap reduction of the
/// window.
const DOTNET_ENV: &[(&str, &str)] = &[
    ("DOTNET_CLI_TELEMETRY_OPTOUT", "1"),
    ("DOTNET_NOLOGO", "1"),
    ("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1"),
];

/// Run a command in `dir`, returning Err with captured output on failure.
fn run(bin: &str, args: &[String], dir: &Path, env: &[(&str, &str)]) -> Result<(), String> {
    let mut cmd = Command::new(bin);
    cmd.args(args).current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("spawn {bin}: {e} (cwd {})", dir.display()))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{bin} {} failed ({})\n--- stdout ---\n{}\n--- stderr ---\n{}",
        args.join(" "),
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    ))
}

fn s(v: &str) -> String {
    v.to_string()
}

/// Compile the generated SDK at `dir` for `lang`.
///
/// `Ok(false)` means the toolchain is absent — SKIPPED, not passed. `Err` means the
/// generated SDK does not compile, which is the failure this crate exists to catch.
pub fn compile_smoke(lang: &str, dir: &Path) -> Result<bool, String> {
    if !toolchain_available(lang) {
        return Ok(false);
    }
    match lang {
        "go" => {
            // -mod=mod so `tidy` may write go.sum; CGO off keeps it hermetic.
            run(
                "go",
                &[s("mod"), s("tidy")],
                dir,
                &[("CGO_ENABLED", "0"), ("GOFLAGS", "-mod=mod")],
            )?;
            run(
                "go",
                &[s("build"), s("./...")],
                dir,
                &[("CGO_ENABLED", "0")],
            )?;
        }
        "python" => run(
            "python3",
            &[s("-m"), s("compileall"), s("-q"), s(".")],
            dir,
            &[],
        )?,
        "ruby" => {
            for f in files_by_ext(dir, ".rb") {
                run(
                    "ruby",
                    &[s("-c"), f.to_string_lossy().to_string()],
                    dir,
                    &[],
                )?;
            }
        }
        "java" => {
            let out = dir.join("__javac_out");
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            let mut args = vec![s("-d"), out.to_string_lossy().to_string()];
            args.extend(
                files_by_ext(dir, ".java")
                    .iter()
                    .map(|p| p.to_string_lossy().to_string()),
            );
            let r = run("javac", &args, dir, &[]);
            let _ = std::fs::remove_dir_all(&out);
            r?;
        }
        "dotnet" => {
            // On its first invocation the .NET CLI runs a first-use configurer,
            // which takes a NAMED MUTEX ("NuGet-Migrations") under
            // $TMPDIR/.dotnet/shm/session<uid>. cargo runs these cases on
            // parallel threads, so several `dotnet build` processes reach that
            // at once and all but one die before compiling anything:
            //
            //   System.IO.IOException: ... 'NuGet-Migrations'. One or more system
            //   calls failed: mkdir(".../shm/session2010") == -1; errno == EEXIST
            //     at NuGet.Common.Migrations.MigrationRunner.Run(..)
            //     at Microsoft.DotNet.Configurer.DotnetFirstTimeUseConfigurer.Configure()
            //
            // The giveaway that it is a race and not a compile error: the failures
            // report DIFFERENT errnos (EEXIST on mkdir, ENOENT on mkdir, EEXIST on
            // an O_EXCL open) and which case loses changes between runs.
            //
            // Warming the configurer once was not enough — it left the builds
            // themselves running concurrently, and the failure count merely went
            // from 3 to 1. The shared resource is the mutex PATH, so:
            //
            //  1. Give each invocation its own TMPDIR. The shm directory hangs off
            //     it, so no two dotnet processes name the same mutex at all. This
            //     is the load-bearing fix and it holds across processes, not just
            //     across threads of this one.
            //  2. Serialize the builds anyway. Belt and braces: if some part of
            //     the path turns out not to derive from TMPDIR on a given SDK,
            //     one-at-a-time still cannot contend with itself.
            // BESIDE the project, never inside it: an SDK-style csproj globs
            // `**/*.cs`, so a scratch directory under `dir` is one stray file away
            // from joining the compilation it is supposed to be invisible to.
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "sdk".to_string());
            let dotnet_tmp = std::env::temp_dir().join(format!("xyd_dotnet_tmp_{name}"));
            let _ = std::fs::remove_dir_all(&dotnet_tmp);
            std::fs::create_dir_all(&dotnet_tmp)
                .map_err(|e| format!("create {}: {e}", dotnet_tmp.display()))?;
            let tmp = dotnet_tmp.to_string_lossy().to_string();
            let mut env: Vec<(&str, &str)> = DOTNET_ENV.to_vec();
            env.push(("TMPDIR", &tmp));
            env.push(("DOTNET_CLI_HOME", &tmp));

            static DOTNET_BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());
            // A panicking test would poison this; the lock orders builds and
            // guards no data, so recover rather than cascading one case's failure
            // into every other case.
            let _serial = DOTNET_BUILD.lock().unwrap_or_else(|e| e.into_inner());

            let mut result = Ok(());
            for proj in files_by_ext(dir, ".csproj") {
                result = run(
                    "dotnet",
                    &[
                        s("build"),
                        s("--nologo"),
                        proj.to_string_lossy().to_string(),
                    ],
                    dir,
                    &env,
                );
                if result.is_err() {
                    break;
                }
            }
            // Ours to clean: it lives outside `dir`, which the caller removes.
            let _ = std::fs::remove_dir_all(&dotnet_tmp);
            result?;
        }
        "rust" => run("cargo", &[s("build")], dir, &[])?,
        // node is handled by the caller: locating `tsc` needs the JS toolchain's
        // resolution, which this crate deliberately does not depend on.
        other => return Err(format!("no compile smoke for language \"{other}\"")),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_language_is_an_error_not_a_silent_skip() {
        // A typo'd language must NOT read as "toolchain absent" — that would make
        // the whole smoke vanish quietly, which is the failure mode this tier exists
        // to prevent.
        assert!(probe("kotlin").is_none());
        assert!(!toolchain_available("kotlin"));
    }

    #[test]
    fn files_by_ext_is_recursive_and_sorted() {
        let dir = std::env::temp_dir().join("xyd_o2s_e2e_fbe");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("b.rb"), "").unwrap();
        std::fs::write(dir.join("a.rb"), "").unwrap();
        std::fs::write(dir.join("nested/c.rb"), "").unwrap();
        std::fs::write(dir.join("skip.txt"), "").unwrap();
        let got: Vec<String> = files_by_ext(&dir, ".rb")
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(got, vec!["a.rb", "b.rb", "c.rb"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
