//! The mapping, pinned: each case builds a clap CLI (builder or derive), maps
//! it, and compares the document with `__fixtures__/<n>.<case>.json`.
//!
//! `XYD_BLESS=1 cargo test -p clap_opencli --all-features` rewrites the
//! goldens (never set it in CI); review the diff before committing. Every
//! golden is also read back through the typed model and validated against
//! the base OpenCLI schema and the extensions schema.
//!
//! The `env` case needs the crate's `env` feature: run the suite once with
//! `--features env` too (CI does).

use std::path::PathBuf;

use clap::builder::PossibleValue;
use clap::{Arg, ArgAction, ArgGroup, Args, Command, CommandFactory, Parser, Subcommand};
use clap_opencli::{
    from_command, interactive_from_command, Document, EnvVar, Hidden, KeyBinding, Options,
    XPassthrough,
};
use opencli_model::{EXTENSIONS_SCHEMA_JSON, SCHEMA_JSON};
use serde_json::{json, Value};

/// Every golden, by file stem. A golden without a case here, or a case
/// without a golden, fails `every_golden_has_a_case_and_validates`.
const CASES: &[&str] = &[
    "1.flags-and-values",
    "2.positionals-and-arity",
    "3.globals",
    "4.hidden-mark",
    "5.hidden-omit",
    "6.aliases",
    "7.defaults-and-conflicts",
    "8.env",
    "9.help-heading-groups",
    "10.no-help-no-version",
    "11.parent-with-args",
    "12.extras",
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("__fixtures__")
}

/// Compare `doc` with the golden `name` (or rewrite it under `XYD_BLESS=1`).
#[track_caller]
fn golden(name: &str, doc: &Document) {
    assert!(CASES.contains(&name), "{name} is not listed in CASES");
    let path = fixtures().join(format!("{name}.json"));
    if std::env::var("XYD_BLESS").as_deref() == Ok("1") {
        std::fs::create_dir_all(fixtures()).unwrap();
        std::fs::write(&path, doc.to_json()).unwrap();
        return;
    }
    clap_opencli::snapshot::assert_file(&path, doc);
    // The typed model reads the golden back as the same document.
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        &Document::from_json(&text).unwrap(),
        doc,
        "{name}: round trip"
    );
}

// ── 1. flags and values ─────────────────────────────────────────────────────

#[test]
fn flags_and_values() {
    let cli = Command::new("acme")
        .version("1.2.3")
        .about("An example CLI")
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::Count)
                .help("More output; repeat for more"),
        )
        .arg(
            Arg::new("quiet")
                .short('q')
                .long("quiet")
                .action(ArgAction::SetTrue)
                .help("Less output"),
        )
        .arg(
            Arg::new("color")
                .long("no-color")
                .action(ArgAction::SetFalse)
                .help("Never color the output"),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("FILE")
                .help("Where to write the result"),
        )
        .arg(
            Arg::new("tag")
                .long("tag")
                .action(ArgAction::Append)
                .value_name("TAG")
                .help("A tag; repeat the option for more"),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_parser([
                    PossibleValue::new("json"),
                    PossibleValue::new("text"),
                    PossibleValue::new("legacy").hide(true),
                ])
                .help("The output format"),
        )
        .arg(
            Arg::new("size")
                .long("size")
                .value_names(["WIDTH", "HEIGHT"])
                .help("The width and the height"),
        )
        .arg(
            Arg::new("level")
                .long("level")
                .num_args(0..=1)
                .default_missing_value("1")
                .help("The level; the value is optional"),
        )
        .arg(
            Arg::new("include")
                .long("include")
                .num_args(1..)
                .value_name("PATH")
                .help("Paths to include"),
        )
        .arg(
            Arg::new("x")
                .short('x')
                .action(ArgAction::SetTrue)
                .help("A short-only flag"),
        );
    golden("1.flags-and-values", &from_command(&cli).build().unwrap());
}

// ── 2. positionals and arity ────────────────────────────────────────────────

#[test]
fn positionals_and_arity() {
    // Declared out of order: the document lists them by index.
    let cli = Command::new("acme")
        .version("1.0.0")
        .arg(
            Arg::new("rest")
                .index(3)
                .last(true)
                .num_args(0..)
                .help("Passed on after `--`"),
        )
        .arg(
            Arg::new("mode")
                .index(1)
                .required(true)
                .value_parser(["fast", "safe"])
                .help("How to copy"),
        )
        .arg(
            Arg::new("files")
                .index(2)
                .value_name("FILE")
                .action(ArgAction::Append)
                .num_args(1..)
                .required(true)
                .help("The files to copy"),
        )
        .subcommand(
            Command::new("pair").about("Store a pair").arg(
                Arg::new("pair")
                    .value_names(["KEY", "VALUE"])
                    .required(true)
                    .help("The key and its value"),
            ),
        );
    golden(
        "2.positionals-and-arity",
        &from_command(&cli).build().unwrap(),
    );
}

// ── 3. globals declared at two levels (derive) ─────────────────────────────

/// Manage widgets.
#[derive(Parser)]
#[command(name = "acme", version = "2.0.0")]
struct GlobalsCli {
    /// The profile to use
    #[arg(long, short = 'P', global = true, value_name = "NAME")]
    profile: Option<String>,
    #[command(subcommand)]
    command: GlobalsTop,
}

#[derive(Subcommand)]
enum GlobalsTop {
    /// Change widgets
    Widgets(WidgetsArgs),
    /// Show the status
    Status,
}

#[derive(Args)]
struct WidgetsArgs {
    /// Print what would change, change nothing
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: WidgetsCommand,
}

#[derive(Subcommand)]
enum WidgetsCommand {
    /// List widgets
    List {
        /// Include archived widgets
        #[arg(long)]
        all: bool,
    },
    /// Delete a widget
    Delete {
        /// The widget's id
        id: String,
    },
}

#[test]
fn globals_are_recursive_where_declared() {
    GlobalsCli::command().debug_assert();
    let doc = from_command(&GlobalsCli::command()).build().unwrap();
    golden("3.globals", &doc);
    // The copies clap spreads are not written.
    for path in [
        &["status"][..],
        &["widgets", "list"],
        &["widgets", "delete"],
    ] {
        let command = doc.command(path).unwrap();
        assert!(
            command
                .options
                .iter()
                .all(|o| o.name != "profile" && o.name != "dry-run"),
            "{path:?} carries a spread copy"
        );
    }
}

// ── 4/5. hidden, marked and omitted ─────────────────────────────────────────

fn hidden_cli() -> Command {
    Command::new("acme")
        .version("1.0.0")
        .arg(
            Arg::new("debug")
                .long("debug")
                .action(ArgAction::SetTrue)
                .hide(true)
                .help("Print internal state"),
        )
        .arg(
            Arg::new("color")
                .long("color")
                .value_parser([
                    PossibleValue::new("always"),
                    PossibleValue::new("never"),
                    PossibleValue::new("legacy").hide(true),
                ])
                .conflicts_with("debug")
                .help("When to color output"),
        )
        .arg(Arg::new("token").hide(true).help("A hidden positional"))
        .arg(
            Arg::new("log-file")
                .long("log-file")
                .global(true)
                .hide(true)
                .help("Where to write the internal log"),
        )
        .subcommand(
            Command::new("run")
                .about("Run the task")
                .arg(Arg::new("target").help("What to run"))
                .arg(
                    Arg::new("trace")
                        .long("trace")
                        .action(ArgAction::SetTrue)
                        .hide(true),
                ),
        )
        .subcommand(
            Command::new("internal")
                .about("Maintenance commands")
                .hide(true)
                .subcommand(Command::new("dump").about("Dump the state")),
        )
}

#[test]
fn hidden_is_marked_by_default() {
    let doc = from_command(&hidden_cli()).build().unwrap();
    golden("4.hidden-mark", &doc);
    // A hidden global is written once, where it is declared.
    let log = doc.options.iter().find(|o| o.name == "log-file").unwrap();
    assert_eq!((log.recursive, log.hidden), (Some(true), Some(true)));
    let run = doc.command(&["run"]).unwrap();
    assert!(run.options.iter().all(|o| o.name != "log-file"));
}

/// Every option name in the document, at any depth.
fn all_option_names(doc: &Document) -> Vec<String> {
    fn walk(commands: &[opencli_model::Command], out: &mut Vec<String>) {
        for c in commands {
            out.extend(c.options.iter().map(|o| o.name.clone()));
            walk(&c.commands, out);
        }
    }
    let mut out: Vec<String> = doc.options.iter().map(|o| o.name.clone()).collect();
    walk(&doc.commands, &mut out);
    out
}

#[test]
fn hidden_is_omitted_on_request() {
    let doc = from_command(&hidden_cli())
        .options(Options {
            hidden: Hidden::Omit,
        })
        .build()
        .unwrap();
    golden("5.hidden-omit", &doc);
    let names = all_option_names(&doc);
    assert!(!names.iter().any(|n| n == "log-file"), "{names:?}");
}

// ── 6. aliases ──────────────────────────────────────────────────────────────

#[test]
fn only_visible_aliases_are_written() {
    let cli = Command::new("acme")
        .version("1.0.0")
        .subcommand(
            Command::new("remove")
                .about("Remove an item")
                .visible_alias("rm")
                .alias("del")
                .arg(
                    Arg::new("force")
                        .short('f')
                        .long("force")
                        .visible_alias("yes")
                        .alias("assume-yes")
                        .visible_short_alias('y')
                        .short_alias('F')
                        .action(ArgAction::SetTrue)
                        .help("Do not ask"),
                ),
        )
        .subcommand(
            Command::new("list")
                .about("List the items")
                .visible_aliases(["ls", "l"]),
        );
    golden("6.aliases", &from_command(&cli).build().unwrap());
}

// ── 7. defaults and conflicts ───────────────────────────────────────────────

#[test]
fn defaults_and_conflicts() {
    let cli = Command::new("acme")
        .version("1.0.0")
        .arg(
            Arg::new("format")
                .long("format")
                .value_parser(["text", "json"])
                .default_value("text")
                .help("The output format"),
        )
        .arg(
            Arg::new("retries")
                .long("retries")
                .default_value("3")
                .hide_default_value(true)
                .help("How often to retry; the default is not shown"),
        )
        .arg(
            Arg::new("header")
                .long("header")
                .action(ArgAction::Append)
                .default_values(["accept: text/plain", "x-trace: off"])
                .help("A request header"),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .conflicts_with("format")
                .help("Shorthand for --format json"),
        )
        .arg(
            Arg::new("csv")
                .long("csv")
                .action(ArgAction::SetTrue)
                .help("Write CSV"),
        )
        .arg(
            Arg::new("tsv")
                .long("tsv")
                .action(ArgAction::SetTrue)
                .help("Write TSV"),
        )
        .group(ArgGroup::new("tables").args(["csv", "tsv"]))
        .arg(
            Arg::new("raw")
                .long("raw")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["tables", "file"])
                .help("Write the raw bytes"),
        )
        .arg(
            Arg::new("schema")
                .long("schema")
                .action(ArgAction::SetTrue)
                .exclusive(true)
                .help("Print the input schema and exit"),
        )
        .arg(
            Arg::new("file")
                .default_value("-")
                .help("The input; `-` reads stdin"),
        );
    golden(
        "7.defaults-and-conflicts",
        &from_command(&cli).build().unwrap(),
    );
}

// ── 8. env (the `env` feature) ──────────────────────────────────────────────

#[cfg(feature = "env")]
#[test]
fn env_reaches_the_metadata_and_the_root() {
    let cli = Command::new("acme")
        .version("1.0.0")
        .arg(
            Arg::new("color")
                .long("color")
                .env("ACME_COLOR")
                .value_parser(["always", "never", "auto"])
                .help("When to color output"),
        )
        .arg(
            Arg::new("token")
                .long("token")
                .env("ACME_TOKEN")
                .hide_env(true)
                .help("The access token; the variable is not shown"),
        )
        .arg(
            Arg::new("trace")
                .long("trace")
                .env("ACME_TRACE")
                .hide(true)
                .action(ArgAction::SetTrue),
        )
        .subcommand(
            Command::new("serve").about("Serve the site").arg(
                Arg::new("port")
                    .long("port")
                    .env("ACME_PORT")
                    .default_value("4000"),
            ),
        );
    let doc = from_command(&cli)
        .env(EnvVar::new("ACME_HOME", "Where the CLI keeps its state"))
        .build()
        .unwrap();
    golden("8.env", &doc);
}

// ── 9. help headings become groups ──────────────────────────────────────────

#[test]
fn help_headings_become_groups() {
    let cli = Command::new("acme")
        .version("1.0.0")
        .arg(
            Arg::new("target")
                .help_heading("Inputs")
                .help("What to fetch"),
        )
        .next_help_heading("Output")
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Print JSON"),
        )
        .arg(
            Arg::new("pretty")
                .long("pretty")
                .action(ArgAction::SetTrue)
                .help("Indent the JSON"),
        )
        .next_help_heading("Network")
        .arg(
            Arg::new("timeout")
                .long("timeout")
                .value_name("SECONDS")
                .help("Give up after this long"),
        )
        .arg(
            Arg::new("proxy")
                .long("proxy")
                .value_name("URL")
                .help_heading("Advanced")
                .help("Send requests through a proxy"),
        );
    golden(
        "9.help-heading-groups",
        &from_command(&cli).build().unwrap(),
    );
}

// ── 10. help and version turned off ─────────────────────────────────────────

#[test]
fn help_and_version_args_are_matched_by_action() {
    let cli = Command::new("acme")
        .version("1.0.0")
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::SetTrue)
                .help("Print the version as JSON"),
        )
        // The CLI's own help flag is still a help flag: skipped.
        .arg(
            Arg::new("usage")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help("Print help"),
        )
        .subcommand(Command::new("run").about("Run it"));
    let doc = from_command(&cli).build().unwrap();
    golden("10.no-help-no-version", &doc);
    assert_eq!(doc.options.len(), 1);
    assert_eq!(doc.commands.len(), 1, "no help subcommand");
}

// ── 11. a parent with its own arguments and subcommands (derive) ────────────

/// An agent CLI for the terminal.
///
/// Start a session with a prompt, or resume one.
#[derive(Parser)]
#[command(name = "acme-agent", version = "0.4.0")]
struct AgentCli {
    /// The prompt to start with
    prompt: Option<String>,
    /// Print the answer and exit
    #[arg(short, long)]
    print: bool,
    /// The model to use
    #[arg(short, long, value_name = "NAME")]
    model: Option<String>,
    #[command(subcommand)]
    command: Option<AgentCommand>,
}

#[derive(Subcommand)]
enum AgentCommand {
    /// Resume a session
    #[command(visible_alias = "continue")]
    Resume {
        /// The session to resume
        session: Option<String>,
        /// Resume the most recent session
        #[arg(long, conflicts_with = "session")]
        last: bool,
    },
    /// Sign in
    Login,
}

#[test]
fn a_parent_keeps_its_own_arguments() {
    AgentCli::command().debug_assert();
    let doc = from_command(&AgentCli::command()).build().unwrap();
    golden("11.parent-with-args", &doc);
    assert_eq!(
        doc.info.summary.as_deref(),
        Some("An agent CLI for the terminal")
    );
}

// ── 12. the builder's extras (derive) ───────────────────────────────────────

/// The commands typed inside a session.
#[derive(Parser)]
#[command(name = "session", multicall = true)]
struct Repl {
    #[command(subcommand)]
    command: ReplCommand,
}

#[derive(Subcommand)]
enum ReplCommand {
    /// Switch the model
    Model {
        /// The model; without it, a picker opens
        name: Option<String>,
    },
    /// Start a new conversation
    #[command(visible_alias = "new")]
    Clear,
    /// Leave the session
    Quit,
}

fn extras_builder() -> clap_opencli::Builder {
    from_command(&AgentCli::command())
        .title("acme agent")
        .summary("Chat with an agent")
        .exit_code(0, "Success")
        .exit_code(1, "An error")
        .exit_code(2, "Usage error")
        .example("acme-agent 'what changed?'")
        .example("acme-agent -p 'summarize' < notes.txt")
        .env(EnvVar::new("ACME_AGENT_HOME", "Where sessions are kept"))
        .interactive(true)
        .interactive_commands(interactive_from_command(&Repl::command()))
        .keys([
            KeyBinding::new(["ctrl+c"], "Interrupt the answer; twice to quit"),
            KeyBinding::new(["esc"], "Close the picker").context("picker"),
        ])
        .extension("x-acme-channel", "stable")
        .at(&["resume"], |c| {
            c.example("acme-agent resume --last")
                .exit_code(0, "Resumed")
                .exit_code(3, "No session to resume")
        })
        .at(&["login"], |c| {
            c.description("Sign in through the browser")
                .passthrough(XPassthrough::program("acme-login"))
                .extension("x-acme-since", json!({ "version": "0.3.0" }))
        })
}

#[test]
fn the_builders_extras() {
    golden("12.extras", &extras_builder().build().unwrap());
}

#[test]
fn a_repl_parsed_with_clap_gives_the_session_commands() {
    let commands = interactive_from_command(&Repl::command());
    let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["model", "clear", "quit"],
        "clap's help subcommand is skipped"
    );
    assert_eq!(commands[1].aliases, ["new"]);
    assert_eq!(commands[0].arguments[0].name, "name");
}

// ── errors ──────────────────────────────────────────────────────────────────

#[test]
fn an_unknown_path_is_an_error() {
    let err = extras_builder()
        .at(&["resume", "nope"], |c| c.example("x"))
        .build()
        .unwrap_err();
    assert_eq!(err.0, "at: the document has no command `resume nope`");
    // An alias is not a path.
    let err = extras_builder()
        .at(&["continue"], |c| c.example("x"))
        .build()
        .unwrap_err();
    assert_eq!(err.0, "at: the document has no command `continue`");
    // A command left out is not one either.
    let err = from_command(&hidden_cli())
        .options(Options {
            hidden: Hidden::Omit,
        })
        .at(&["internal"], |c| c.example("x"))
        .build()
        .unwrap_err();
    assert_eq!(
        err.0,
        "at: the document has no command `internal` (hidden commands are left out under \
         Hidden::Omit)"
    );
}

#[test]
fn extension_keys_are_checked() {
    let cli = || from_command(&hidden_cli());
    let err = |b: clap_opencli::Builder| b.build().unwrap_err().0;
    assert_eq!(
        err(cli().extension("acme", 1)),
        "the root: `acme` is not an extension key; extension keys start with `x-`"
    );
    assert_eq!(
        err(cli().extension("x-environment", json!([]))),
        "the root: `x-environment` has a typed form; set it with env"
    );
    assert!(
        err(cli().extension("x-openapi", json!({}))).contains("`x-openapi` is the HTTP binding")
    );
    assert_eq!(
        err(cli().at(&["run"], |c| c.extension("x-passthrough", json!({})))),
        "command `run`: `x-passthrough` has a typed form; set it with passthrough, on a command"
    );
}

#[test]
fn a_command_without_a_version_needs_one() {
    let cli = Command::new("acme").arg(Arg::new("x").long("x"));
    let err = from_command(&cli).build().unwrap_err();
    assert!(err.0.starts_with("the command has no version"), "{err}");
    let doc = from_command(&cli).version("0.0.1").build().unwrap();
    assert_eq!(doc.info.version, "0.0.1");
}

#[test]
fn the_command_is_left_as_it_was() {
    let cli = AgentCli::command();
    let _ = from_command(&cli).build().unwrap();
    // Still unbuilt: building it again would spread nothing twice.
    assert!(cli.get_subcommands().all(|s| s.get_name() != "help"));
    cli.try_get_matches_from(["acme-agent", "-p", "hi"])
        .unwrap();
}

// ── every golden ────────────────────────────────────────────────────────────

fn validator(schema: &str) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(schema).unwrap();
    jsonschema::draft202012::new(&schema).unwrap()
}

#[test]
fn every_golden_has_a_case_and_validates() {
    let base = validator(SCHEMA_JSON);
    let extensions = validator(EXTENSIONS_SCHEMA_JSON);
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(fixtures()).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            path.extension().and_then(|e| e.to_str()),
            Some("json"),
            "{stem}: only <n>.<case>.json files belong in __fixtures__"
        );
        assert!(CASES.contains(&stem.as_str()), "{stem}.json has no case");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for (name, v) in [("base", &base), ("extensions", &extensions)] {
            let errors: Vec<String> = v
                .iter_errors(&doc)
                .map(|e| format!("{e} at {}", e.instance_path()))
                .collect();
            assert!(errors.is_empty(), "{stem}: the {name} schema: {errors:#?}");
        }
        seen.push(stem);
    }
    seen.sort();
    let mut expected: Vec<String> = CASES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(seen, expected, "a case without a golden");
}
