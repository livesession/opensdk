//! `opensdk opencli` and `generate --spec <opencli.json>`.
//!
//! Each `__fixtures__/opencli/<case>/` holds an OpenAPI document, the sdk.json
//! whose CLI section supplies the converter options, an `args.json` (`lang`,
//! `declare`, and optionally `mount` and `mountDrop`) and `output.json` — the
//! BYTES `opencli --output` writes.
//! `XYD_BLESS=1` rewrites the goldens instead of checking them.

use std::path::{Path, PathBuf};

use serde_json::Value;

use opensdk::command::main_with;
use opensdk::config::resolve_config;
use opensdk::generate::{generate_command, GenerateCommandOptions};
use opensdk::opencli::{opencli_command, OpenCliCommandOptions};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("__fixtures__/opencli")
}

fn tmp(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "opensdk-rs-opencli-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A case's `args.json`: `lang`, `declare`, and the optional `mount` /
/// `mountDrop` lists.
struct CaseArgs {
    lang: String,
    declare: Vec<String>,
    mount: Vec<String>,
    mount_drop: Vec<String>,
}

fn args_of(case: &Path) -> CaseArgs {
    let args: Value =
        serde_json::from_str(&std::fs::read_to_string(case.join("args.json")).unwrap()).unwrap();
    let list = |key: &str| -> Vec<String> {
        args.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    };
    CaseArgs {
        lang: args["lang"].as_str().unwrap().to_string(),
        declare: list("declare"),
        mount: list("mount"),
        mount_drop: list("mountDrop"),
    }
}

/// Run `opencli` for a case through the library entry point, into `out`.
fn emit(case: &Path, out: &Path) {
    let args = args_of(case);
    let config = resolve_config(case, None).unwrap().expect("sdk.json");
    opencli_command(
        &OpenCliCommandOptions {
            spec: config.spec.clone().expect("api"),
            lang: Some(args.lang),
            declare: args.declare,
            output: Some(out.to_string_lossy().to_string()),
            sdk_name: None,
            mount: args.mount,
            mount_drop: args.mount_drop,
        },
        Some(&config),
        case,
    )
    .unwrap();
}

fn check_case(name: &str) {
    let case = fixtures().join(name);
    let dir = tmp(name);
    let out = dir.join("opencli.json");
    emit(&case, &out);
    let actual = std::fs::read_to_string(&out).unwrap();
    let golden = case.join("output.json");
    if std::env::var("XYD_BLESS").as_deref() == Ok("1") {
        std::fs::write(&golden, &actual).unwrap();
        eprintln!("blessed opencli/{name}");
    } else {
        let want = std::fs::read_to_string(&golden)
            .unwrap_or_else(|e| panic!("{name}: no golden ({e}) — run with XYD_BLESS=1"));
        assert_eq!(
            actual, want,
            "{name}: the emitted OpenCLI differs from output.json"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_cli_sections_converter_options_reach_the_document() {
    check_case("1.converter-options");
}

#[test]
fn fragments_merge_into_the_generated_document() {
    check_case("2.declare");
}

/// Converter, then the fragments, then the mounts: a passthrough stub filled
/// with `--profile` dropped, and a component in the root-`command` shape
/// appended under `api`.
#[test]
fn components_mount_into_the_merged_document() {
    check_case("3.mount");
}

/// The options `mount_case` passes: 3.mount's, with `mount` and `mount_drop`
/// replaced.
fn mount_options(case: &Path, mount: &[&str], mount_drop: &[&str]) -> OpenCliCommandOptions {
    let args = args_of(case);
    let config = resolve_config(case, None).unwrap().expect("sdk.json");
    OpenCliCommandOptions {
        spec: config.spec.clone().expect("api"),
        lang: Some(args.lang),
        declare: args.declare,
        mount: mount.iter().map(|s| s.to_string()).collect(),
        mount_drop: mount_drop.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

/// `build_opencli` on 3.mount with other mounts: the error, if any.
fn mount_error(dir: &Path, mount: &[&str], mount_drop: &[&str]) -> String {
    let case = fixtures().join("3.mount");
    let config = resolve_config(&case, None).unwrap().expect("sdk.json");
    // Component paths are relative to `dir`, where the broken ones are written.
    let mount: Vec<String> = mount
        .iter()
        .map(|m| match m.split_once('=') {
            Some((name, doc)) if !doc.is_empty() => {
                let doc = if dir.join(doc).exists() {
                    dir.join(doc)
                } else {
                    case.join(doc)
                };
                format!("{name}={}", doc.display())
            }
            _ => m.to_string(),
        })
        .collect();
    let mount: Vec<&str> = mount.iter().map(String::as_str).collect();
    opensdk::opencli::build_opencli(
        &mount_options(&case, &mount, mount_drop),
        Some(&config),
        &case,
    )
    .err()
    .map(|e| e.0)
    .unwrap_or_else(|| "no error".into())
}

#[test]
fn every_mount_rule_has_its_error() {
    let dir = tmp("mount-errors");
    let agent = "components/agent.opencli.json";
    let agent_doc: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures().join("3.mount").join(agent)).unwrap(),
    )
    .unwrap();
    let write = |name: &str, change: &dyn Fn(&mut Value)| {
        let mut doc = agent_doc.clone();
        change(&mut doc);
        std::fs::write(dir.join(name), serde_json::to_string(&doc).unwrap()).unwrap();
        format!("agent={name}")
    };
    let drop = ["agent:profile"];

    // The flags themselves.
    assert_eq!(
        mount_error(&dir, &["agent"], &[]),
        "--mount agent: expected NAME=DOCUMENT (NAME may be parent/name)"
    );
    assert_eq!(
        mount_error(&dir, &["agent=a.json", "agent=b.json"], &[]),
        "--mount agent is given twice"
    );
    assert_eq!(
        mount_error(&dir, &["agent=x.json"], &["agent"]),
        "--mount-drop agent: expected NAME:OPTION"
    );
    assert_eq!(
        mount_error(&dir, &["agent=x.json"], &["other:profile"]),
        "--mount-drop other:profile: there is no --mount other=..."
    );
    // M1: not an OpenCLI document; other conventions.
    let not_opencli = write("not-opencli.json", &|d| {
        d.as_object_mut().unwrap().remove("info");
    });
    let e = mount_error(&dir, &[&not_opencli], &drop);
    assert!(
        e.starts_with("--mount agent: ") && e.contains("not-opencli.json: not an OpenCLI document"),
        "{e}"
    );
    let conventions = write("conventions.json", &|d| {
        d["conventions"] = serde_json::json!({ "groupOptions": false });
    });
    assert!(mount_error(&dir, &[&conventions], &drop).contains(
        "mount agent: the component's conventions (groupOptions false, optionSeparator \" \") \
         differ from the host's (groupOptions true, optionSeparator \" \")"
    ));
    // M2: a host command that is not a stub, a missing parent, twice.
    assert!(
        mount_error(&dir, &[&format!("api={agent}")], &["api:profile"]).contains(
            "mount api: the host already has a command `api`, and only a passthrough stub"
        )
    );
    assert_eq!(
        mount_error(&dir, &[&format!("nope/agent={agent}")], &[]),
        "mount nope agent: the host has no command `nope`"
    );
    assert_eq!(
        mount_error(&dir, &[&format!("ag={agent}")], &["ag:profile"]),
        "mount ag: command `ag` and command `agent` both answer to `ag` (a name or an alias)"
    );
    // M3: an HTTP binding.
    let bound = write("bound.json", &|d| {
        d["commands"][0]["x-openapi"] = serde_json::json!({ "method": "get", "path": "/" });
    });
    assert!(mount_error(&dir, &[&bound], &drop)
        .contains("the component's command `resume` carries an `x-openapi` binding"));
    // M4: a collision without the drop, by the long name and by an alias; a
    // drop that matches nothing.
    assert_eq!(
        mount_error(&dir, &[&format!("agent={agent}")], &[]),
        "mount agent: the host's root recursive option `--profile` and the component's option \
         `--profile` of `agent` both answer to `--profile`; drop the component's with \
         --mount-drop agent:profile"
    );
    let short = write("short.json", &|d| {
        d["commands"][0]["options"][0]["aliases"] = serde_json::json!(["P"]);
    });
    assert!(mount_error(&dir, &[&short], &drop).contains(
        "recursive option `--profile` and the component's option `--last` of `agent resume` \
         both answer to `-P`"
    ));
    assert_eq!(
        mount_error(
            &dir,
            &[&format!("agent={agent}")],
            &["agent:profile", "agent:nope"]
        ),
        "mount agent: --mount-drop agent:nope matches no option of the component"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The binary itself: the same bytes as the golden, and one report line per
/// mount on stderr (M7).
#[test]
fn the_argv_layer_mounts_and_reports_each_mount_on_stderr() {
    let case = fixtures().join("3.mount");
    let dir = tmp("mount-argv");
    let out = dir.join("opencli.json");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_opensdk"))
        .current_dir(&case)
        .args([
            "opencli",
            "--config",
            "sdk.json",
            "--declare",
            "fragments/root.json",
            "--mount",
            "agent=components/agent.opencli.json",
            "--mount-drop",
            "agent:profile",
            "--mount",
            "api/sync=components/sync.opencli.json",
            "--output",
        ])
        .arg(&out)
        .output()
        .expect("run opensdk");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert_eq!(
        stderr,
        "mounted agent 0.4.0: dropped --profile\nmounted api sync 1.2.0\n"
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        std::fs::read_to_string(case.join("output.json")).unwrap()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Every generated file, relative to `root`, with its bytes.
fn tree(root: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, rel: &str, out: &mut Vec<(String, String)>) {
        let dir = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_string();
            let child = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if e.path().is_dir() {
                walk(root, &child, out);
            } else {
                out.push((
                    child.clone(),
                    std::fs::read_to_string(root.join(&child)).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out.sort();
    out
}

fn generate_rust_cli(case: &Path, spec: &str, out: &Path) {
    let config = resolve_config(case, None).unwrap().expect("sdk.json");
    generate_command(
        &GenerateCommandOptions {
            spec: spec.to_string(),
            lang: "rust-cli".into(),
            output: out.to_string_lossy().to_string(),
            emitter_options: config.emitter_options_for("rust-cli").cloned(),
            ..Default::default()
        },
        case,
    )
    .unwrap();
}

#[test]
fn generating_from_the_emitted_document_equals_generating_from_the_openapi() {
    // The document `opencli` writes is exactly what `generate` hands the
    // backend, so with no fragments the two routes produce the same tree.
    let case = fixtures().join("1.converter-options");
    let dir = tmp("same-tree");
    let doc = dir.join("opencli.json");
    emit(&case, &doc);
    generate_rust_cli(
        &case,
        &case.join("openapi.yaml").to_string_lossy(),
        &dir.join("direct"),
    );
    generate_rust_cli(&case, &doc.to_string_lossy(), &dir.join("via-opencli"));
    let direct = tree(&dir.join("direct"));
    assert!(!direct.is_empty());
    assert_eq!(direct, tree(&dir.join("via-opencli")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_merged_document_drives_the_rust_backend() {
    let case = fixtures().join("2.declare");
    let dir = tmp("merged");
    let doc = dir.join("opencli.json");
    emit(&case, &doc);
    generate_rust_cli(&case, &doc.to_string_lossy(), &dir.join("cli"));
    let cli = std::fs::read_to_string(dir.join("cli/src/gen/cli.rs")).unwrap();
    // The fragment's root options are global clap args, and the root seam is on.
    assert!(cli.contains(r#"Arg::new("output")"#), "{cli}");
    assert!(cli.contains(".global(true)"), "{cli}");
    assert!(
        cli.contains("o.before_dispatch(&mut ctx, &matches);"),
        "{cli}"
    );
    // The wrapper took the fragment's description and alias; the hand-written
    // commands are leaves bound through the actions seam.
    let api = std::fs::read_to_string(dir.join("cli/src/gen/cmd/api.rs")).unwrap();
    assert!(api.contains(r#".visible_alias("raw")"#), "{api}");
    assert!(api.contains(r#".about("Call the API directly")"#), "{api}");
    let commands = std::fs::read_to_string(dir.join("cli/src/gen/runtime/commands.rs")).unwrap();
    assert!(commands.contains("fn login("), "{commands}");
    assert!(commands.contains("fn logout("), "{commands}");
    // The resolved default headers reach the runtime config.
    let config = std::fs::read_to_string(dir.join("cli/src/gen/runtime/config.rs")).unwrap();
    assert!(
        config.contains(r#"("X-Api-Version", "2024-10-01")"#),
        "{config}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_opencli_document_is_refused_for_an_sdk_language() {
    let case = fixtures().join("1.converter-options");
    let dir = tmp("sdk-refused");
    let doc = dir.join("opencli.json");
    emit(&case, &doc);
    let err = generate_command(
        &GenerateCommandOptions {
            spec: doc.to_string_lossy().to_string(),
            lang: "go".into(),
            output: dir.join("go").to_string_lossy().to_string(),
            ..Default::default()
        },
        &dir,
    )
    .unwrap_err();
    assert!(err.0.contains("is an OpenCLI document"), "{}", err.0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_mixed_config_refuses_an_opencli_document_before_writing_anything() {
    let case = fixtures().join("1.converter-options");
    let dir = tmp("mixed-refused");
    let doc = dir.join("opencli.json");
    emit(&case, &doc);
    let mut config = resolve_config(&case, None).unwrap().expect("sdk.json");
    config
        .emitter_options
        .push(("go".to_string(), serde_json::Map::new()));
    let out = dir.join("out");
    let err = opensdk::generate::generate_targets(
        &opensdk::generate::GenerateTargetsOptions {
            spec: doc.to_string_lossy().to_string(),
            output: out.to_string_lossy().to_string(),
            ..Default::default()
        },
        &config,
        &dir,
    )
    .unwrap_err();
    assert!(err.0.contains("is an OpenCLI document"), "{}", err.0);
    assert!(!out.exists(), "a CLI target was written before the refusal");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_argv_layer_writes_the_same_bytes_and_repeats_declare_in_order() {
    let case = fixtures().join("2.declare");
    let dir = tmp("argv");
    let out = dir.join("nested/opencli.json");
    let argv: Vec<String> = [
        "opensdk",
        "opencli",
        "--config",
        "sdk.json",
        "--lang",
        "rust-cli",
        "--declare",
        "fragments/root.json",
        "--declare",
        "fragments/auth.json",
        "--output",
        out.to_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // `--spec` omitted: the config's `api` is the document.
    assert_eq!(main_with(argv, &case), 0);
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        std::fs::read_to_string(case.join("output.json")).unwrap()
    );

    // A fragment that misspells a root key fails the command (exit 1).
    let bad = dir.join("bad.json");
    std::fs::write(&bad, r#"{ "exitcodes": [] }"#).unwrap();
    let argv: Vec<String> = [
        "opensdk",
        "opencli",
        "--config",
        "sdk.json",
        "--declare",
        bad.to_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(main_with(argv, &case), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn without_a_cli_section_the_converter_runs_with_its_defaults() {
    // No --lang and no CLI section: the document is the plain conversion,
    // named from --sdk-name.
    let case = fixtures().join("1.converter-options");
    let dir = tmp("defaults");
    let out = dir.join("opencli.json");
    opencli_command(
        &OpenCliCommandOptions {
            spec: case.join("openapi.yaml").to_string_lossy().to_string(),
            output: Some(out.to_string_lossy().to_string()),
            sdk_name: Some("gizmo".into()),
            ..Default::default()
        },
        None,
        &dir,
    )
    .unwrap();
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(doc["info"]["title"], serde_json::json!("gizmo"));
    // Derived names: the version segment is a group without pathPrefixStrip.
    assert_eq!(doc["commands"][0]["name"], serde_json::json!("v3"));
    assert!(doc["x-openapi"].get("defaultHeaders").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_root_option_colliding_with_a_clap_builtin_fails_the_rust_generation() {
    let dir = tmp("collide");
    let doc = dir.join("opencli.json");
    std::fs::write(
        &doc,
        serde_json::to_string(&serde_json::json!({
            "opencli": "1.0.0",
            "info": { "title": "widgets", "version": "1.0.0" },
            "options": [{ "name": "verbose", "aliases": ["h"], "recursive": true }],
            "commands": [{ "name": "ping", "x-openapi": { "method": "get", "path": "/ping" } }]
        }))
        .unwrap(),
    )
    .unwrap();
    let out = dir.join("cli");
    let err = generate_command(
        &GenerateCommandOptions {
            spec: doc.to_string_lossy().to_string(),
            lang: "rust-cli".into(),
            output: out.to_string_lossy().to_string(),
            ..Default::default()
        },
        &dir,
    )
    .unwrap_err();
    assert_eq!(
        err.0,
        "opencli2rust: root option `--verbose` and clap's built-in `-h/--help` both use `-h` — \
         rename one, or change its aliases"
    );
    assert!(!out.exists());
    std::fs::remove_dir_all(&dir).ok();
}
