//! Does the generated runtime DO what the goldens say it emits?
//!
//! The byte goldens (`parity.rs`) pin the generated source; this tier builds two
//! of the committed fixture crates and runs the binaries against a local
//! server that records every request, header by header:
//!
//!   * `12.default-headers` — the spec's default headers are sent unless the
//!     request sets them, `--body @file` / `@-` / inline, the declared
//!     `Content-Type`, and the JSON body never doubling as a query parameter;
//!   * `11.root-options` — with a `src/custom/mod.rs` that turns the root
//!     `--token` into an `Authorization` header in `before_dispatch`: exactly
//!     ONE `Authorization` reaches the server (the root option's, not also the
//!     environment's), and `print_usage_error` chooses a usage error's exit code.
//!
//! Gated like the compile smokes: `XYD_SMOKE_RUST=1 cargo test -p opencli2rust
//! --test runtime_smoke`. The crates pull reqwest/tokio/clap from crates.io, so a
//! dependency that cannot be fetched SKIPS (an environment problem) while any
//! compile error FAILS.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

fn gated() -> bool {
    std::env::var("XYD_SMOKE_RUST").is_err()
}

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Recorded {
    /// Every value of one header (case-insensitive), in arrival order.
    fn all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// A local server that records every request and answers `{"ok":true}`.
struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Recorded>>>,
    server: Arc<tiny_http::Server>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("bind"));
        let port = server.server_addr().to_ip().expect("ip").port();
        let seen: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let (srv, slot) = (Arc::clone(&server), Arc::clone(&seen));
        let handle = std::thread::spawn(move || {
            for mut req in srv.incoming_requests() {
                let mut body = Vec::new();
                let _ = req.as_reader().read_to_end(&mut body);
                let rec = Recorded {
                    method: req.method().as_str().to_string(),
                    url: req.url().to_string(),
                    headers: req
                        .headers()
                        .iter()
                        .map(|h| {
                            (
                                h.field.as_str().as_str().to_string(),
                                h.value.as_str().to_string(),
                            )
                        })
                        .collect(),
                    body: String::from_utf8_lossy(&body).into_owned(),
                };
                slot.lock().unwrap().push(rec);
                let json =
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
                let _ = req
                    .respond(tiny_http::Response::from_string("{\"ok\":true}").with_header(json));
            }
        });
        Server {
            port,
            seen,
            server,
            handle: Some(handle),
        }
    }

    /// The requests recorded since the last call.
    fn take(&self) -> Vec<Recorded> {
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn fixture_output(case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("__fixtures__")
        .join(case)
        .join("output")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn shared_target_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("opencli2rust-runtime-smoke-target");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Registry/network phrasing only; any rustc diagnostic is a real failure.
fn is_dependency_failure(stderr: &str) -> bool {
    if stderr.contains("error[") {
        return false;
    }
    [
        "failed to get ",
        "failed to download",
        "failed to fetch",
        "network failure",
        "could not connect",
        "unable to get packages from source",
        "registry index was not found",
    ]
    .iter()
    .any(|p| stderr.contains(p))
}

/// Copy a fixture crate (with optional file overrides), build it, and return
/// the binary — or `None` when its dependencies cannot be fetched.
fn build(case: &str, overrides: &[(&str, &str)]) -> Option<PathBuf> {
    let project = std::env::temp_dir().join(format!(
        "opencli2rust-runtime-smoke-{}-{case}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project);
    copy_tree(&fixture_output(case), &project);
    for (rel, content) in overrides {
        std::fs::write(project.join(rel), content).unwrap();
    }
    // Both fixtures are a `widgets` package, and cargo hashes a root path
    // package by its name and version, not its location: in one target dir the
    // second fixture would be judged fresh and reuse the first one's binary.
    // A per-fixture package name keeps them apart (the binary stays `widgets`).
    let manifest = project.join("Cargo.toml");
    let toml = std::fs::read_to_string(&manifest).unwrap();
    let package = format!("name = \"widgets-{}\"", case.replace('.', "-"));
    std::fs::write(&manifest, toml.replacen("name = \"widgets\"", &package, 1)).unwrap();
    // Both fixtures build a `widgets` binary into the shared target dir, so two
    // builds must not overlap (the tests run in parallel): one at a time, and
    // each copies its binary out before the next.
    static BUILD: Mutex<()> = Mutex::new(());
    let _one_at_a_time = BUILD.lock().unwrap_or_else(|e| e.into_inner());
    let target = shared_target_dir();
    let out = Command::new("cargo")
        .args(["build", "-q", "--bin", "widgets"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("run cargo (is it on PATH?)");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        if is_dependency_failure(&stderr) {
            eprintln!("runtime_smoke: SKIP {case} — dependencies unavailable:\n{stderr}");
            return None;
        }
        panic!("{case}: the generated crate does not build:\n{stderr}");
    }
    // Keep a copy: both fixtures build a `widgets` binary into the shared dir.
    // The project itself is no longer needed (its build lives in the target).
    let bin = std::env::temp_dir().join(format!(
        "opencli2rust-runtime-smoke-{}-{case}-bin",
        std::process::id()
    ));
    std::fs::copy(target.join("debug").join("widgets"), &bin).unwrap();
    let _ = std::fs::remove_dir_all(&project);
    Some(bin)
}

struct Run {
    code: i32,
    stderr: String,
}

fn run(bin: &Path, server: &Server, args: &[&str], stdin: Option<&str>) -> Run {
    let mut child = Command::new(bin)
        .args(args)
        .env(
            "WIDGETS_BASE_URL",
            format!("http://127.0.0.1:{}", server.port),
        )
        .env("WIDGETS_API_KEY", "env-token")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the generated binary");
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(input) = stdin {
            pipe.write_all(input.as_bytes()).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Run a command that must succeed with exactly one request; return it.
fn one(bin: &Path, server: &Server, args: &[&str], stdin: Option<&str>) -> Recorded {
    let r = run(bin, server, args, stdin);
    assert_eq!(r.code, 0, "{args:?} failed:\n{}", r.stderr);
    let mut seen = server.take();
    assert_eq!(seen.len(), 1, "{args:?}: {seen:#?}");
    seen.remove(0)
}

#[test]
fn default_headers_bodies_and_content_types() {
    if gated() {
        eprintln!("runtime_smoke: skipped (set XYD_SMOKE_RUST=1)");
        return;
    }
    let Some(bin) = build("12.default-headers", &[]) else {
        return;
    };
    let server = Server::start();

    let r = one(&bin, &server, &["widgets", "list"], None);
    assert_eq!((r.method.as_str(), r.url.as_str()), ("GET", "/widgets"));
    assert_eq!(r.all("x-api-version"), ["2024-10-01"]);
    assert_eq!(r.all("x-client"), ["widgets-cli"]);
    assert_eq!(r.all("authorization"), ["Bearer env-token"]);

    // The request's own header wins over the default, and is not doubled.
    let r = one(
        &bin,
        &server,
        &["widgets", "list", "--api-version", "2030-01-01"],
        None,
    );
    assert_eq!(r.all("x-api-version"), ["2030-01-01"]);

    // An inline JSON body: the runtime's default content type, no `?body=`.
    let r = one(
        &bin,
        &server,
        &["widgets", "create", "--body", r#"{"name":"w"}"#],
        None,
    );
    assert_eq!((r.method.as_str(), r.url.as_str()), ("POST", "/widgets"));
    assert_eq!(r.all("content-type"), ["application/json"]);
    assert_eq!(r.body, r#"{"name":"w"}"#);

    // `@file`, with the binding's declared media type.
    let csv = std::env::temp_dir().join(format!("opencli2rust-smoke-{}.csv", std::process::id()));
    std::fs::write(&csv, "a,b\n1,2\n").unwrap();
    let at_file = format!("@{}", csv.display());
    let r = one(
        &bin,
        &server,
        &["widgets", "import", "--body", &at_file],
        None,
    );
    assert_eq!(r.url, "/widgets/import");
    assert_eq!(r.all("content-type"), ["text/csv"]);
    assert_eq!(r.body, "a,b\n1,2\n");

    // `@-`: stdin.
    let r = one(
        &bin,
        &server,
        &["widgets", "import", "--body", "@-"],
        Some("x,y\n"),
    );
    assert_eq!(r.body, "x,y\n");

    // A header flag beside the declared media type.
    let r = one(
        &bin,
        &server,
        &[
            "widgets",
            "update",
            "w1",
            "--if-match",
            "abc",
            "--body",
            r#"{"a":1}"#,
        ],
        None,
    );
    assert_eq!(
        (r.method.as_str(), r.url.as_str()),
        ("PATCH", "/widgets/w1")
    );
    assert_eq!(r.all("content-type"), ["application/merge-patch+json"]);
    assert_eq!(r.all("if-match"), ["abc"]);

    // An unreadable `@file` fails before any request.
    let missing = std::env::temp_dir().join("opencli2rust-smoke-missing.csv");
    let r = run(
        &bin,
        &server,
        &[
            "widgets",
            "import",
            "--body",
            &format!("@{}", missing.display()),
        ],
        None,
    );
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("--body"), "{}", r.stderr);
    assert!(server.take().is_empty());
    std::fs::remove_file(&csv).ok();
    std::fs::remove_file(&bin).ok();
}

/// The hand-written half of a CLI with root options: the root `--token` becomes
/// the credential, and a usage error exits 64.
const ROOT_CUSTOM: &str = r#"use std::process::ExitCode;

use clap::ArgMatches;

use crate::gen::runtime::{CliOverrides, Context, CustomCommands};

pub struct Custom;

impl CliOverrides for Custom {
    fn before_dispatch(&self, ctx: &mut Context, root: &ArgMatches) {
        if let Some(token) = root.get_one::<String>("token") {
            ctx.headers.push(("Authorization".into(), format!("Bearer {token}")));
        }
    }

    fn print_usage_error(&self, err: &clap::Error) -> ExitCode {
        let _ = err.print();
        if err.use_stderr() {
            ExitCode::from(64)
        } else {
            ExitCode::SUCCESS
        }
    }
}

pub fn register(commands: &mut CustomCommands) {
    let _ = commands;
}

pub fn overrides() -> Custom {
    Custom
}
"#;

#[test]
fn root_options_reach_before_dispatch_and_authorization_is_sent_once() {
    if gated() {
        eprintln!("runtime_smoke: skipped (set XYD_SMOKE_RUST=1)");
        return;
    }
    let Some(bin) = build("11.root-options", &[("src/custom/mod.rs", ROOT_CUSTOM)]) else {
        return;
    };
    let server = Server::start();

    let r = one(&bin, &server, &["widgets", "list"], None);
    assert_eq!(r.all("authorization"), ["Bearer env-token"]);

    // The root option's credential REPLACES the environment's — before the
    // subcommand and after it (the option is global).
    let r = one(
        &bin,
        &server,
        &["--token", "cli-token", "widgets", "list"],
        None,
    );
    assert_eq!(r.all("authorization"), ["Bearer cli-token"]);
    let r = one(
        &bin,
        &server,
        &["widgets", "list", "--token", "late-token"],
        None,
    );
    assert_eq!(r.all("authorization"), ["Bearer late-token"]);

    // `-o` is accepted anywhere; an unknown value is a usage error → 64.
    assert_eq!(
        run(&bin, &server, &["widgets", "list", "-o", "table"], None).code,
        0
    );
    server.take();
    assert_eq!(
        run(&bin, &server, &["widgets", "list", "-o", "yaml"], None).code,
        64
    );
    assert_eq!(run(&bin, &server, &["--bogus"], None).code, 64);
    assert_eq!(run(&bin, &server, &["--help"], None).code, 0);
    assert!(server.take().is_empty());
    std::fs::remove_file(&bin).ok();
}
