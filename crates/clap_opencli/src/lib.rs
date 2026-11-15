//! Describe a [clap](https://docs.rs/clap) CLI as an
//! [OpenCLI](https://github.com/spectreconsole/open-cli) document.
//!
//! [`from_command`] reads a clap command tree, builder or derive, and maps its
//! commands, positionals and options. What clap cannot know is declared next
//! to it: exit codes, examples, environment variables, and what an
//! interactive session accepts. [`Builder::build`] returns an
//! [`opencli_model::Document`]; [`snapshot::assert_file`] keeps a committed
//! copy current in a test.
//!
//! ```
//! use clap::{CommandFactory, Parser};
//! use clap_opencli::{EnvVar, Hidden, Options};
//!
//! /// An example CLI.
//! #[derive(Parser)]
//! #[command(name = "acme", version = "1.0.0")]
//! struct Cli {
//!     /// What to ask
//!     prompt: Option<String>,
//!     /// Print the answer and exit
//!     #[arg(short, long)]
//!     print: bool,
//! }
//!
//! let doc = clap_opencli::from_command(&Cli::command())
//!     .options(Options { hidden: Hidden::Omit })
//!     .exit_code(0, "Success")
//!     .exit_code(2, "Usage error")
//!     .example("acme 'hello'")
//!     .env(EnvVar::new("ACME_COLOR", "When to color output").values(["always", "never"]))
//!     .build()
//!     .unwrap();
//! assert_eq!(doc.info.title, "acme");
//! assert_eq!(doc.arguments[0].name, "prompt");
//! assert_eq!(doc.options[0].name, "print");
//! assert_eq!(doc.options[0].aliases, ["p"]);
//! ```
//!
//! # The mapping
//!
//! The tree is read after [`clap::Command::build`], so defaults are filled in,
//! and compared with the unbuilt tree to find what clap added. clap's `help`
//! subcommand and every argument whose action is help or version are skipped
//! (by action, so an argument a CLI names `version` stays).
//!
//! * **Commands:** the name, the visible aliases, `long_about` else `about`
//!   as the description, `hidden`.
//! * **The root:** `info.title` is the bin name, else the name; `info.version`
//!   is the version; `info.summary` is `about`, and `info.description` is
//!   `long_about` when it differs. [`Builder::title`] and its siblings
//!   override them.
//! * **Positionals** become `arguments`, in index order: the first value name,
//!   lowercased (else the id); `required`; the `arity` unless it is exactly
//!   one value (no maximum when unbounded); the non-hidden possible values as
//!   `acceptedValues`; the help heading as `group`; `long_help` else `help`
//!   as the description; `hidden`.
//! * **Options:** the long name; as aliases, the short and the visible long
//!   and short aliases. An OpenCLI option is named by its long name, and a
//!   one-character alias is read as a short: an option with only a short
//!   (`-x`) is an error, since its name would read `--x`, and a
//!   one-character long alias (`--y`, which would read `-y`) is left out. An
//!   option taking values has one argument per value name (else one, named
//!   like a positional, with the option's arity); a flag (`SetTrue`, `SetFalse`,
//!   `Count`) has none and no default. `Append` and `Count` add the metadata
//!   `repeatable`. A global is `recursive: true` where it is declared; the
//!   copies clap spreads to the subcommands are not written.
//! * **Metadata:** `default` (one string, or a list for several values)
//!   unless the default is hidden from help; `env` with the `env` feature
//!   (and an `x-environment` entry on the root, unless the argument is
//!   hidden); `conflictsWith`, the spellings (`--name`, `<name>`) of the
//!   arguments an argument declares a conflict with, groups unrolled;
//!   `exclusive` and `last` when set.
//! * **Hidden** commands and arguments are marked `hidden: true`
//!   ([`Hidden::Mark`], the default) or left out ([`Hidden::Omit`]).
//! * **Not mapped:** `requires` (clap has no getter for it), whether an
//!   argument group is required, one-character long aliases, value hints,
//!   external subcommands.
//!
//! clap 4.5 is the oldest version supported, and every 4.x after it.
//!
//! # Features
//!
//! * `env`: map `Arg::env` (it enables clap's own `env` feature). Off by
//!   default, so the crate leaves a CLI's clap features as they are.

mod walk;

pub mod snapshot;

pub use opencli_model;
pub use opencli_model::{Document, EnvVar, Error, ExitCode, KeyBinding, XPassthrough};

use opencli_model::{Command, XInteractive};
use serde_json::Value;

use crate::walk::Walk;

/// What to do with a command or argument clap hides from help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hidden {
    /// Keep it, marked `hidden: true`: a docs site decides.
    #[default]
    Mark,
    /// Leave it out of the document.
    Omit,
}

/// How [`from_command`] reads the tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    pub hidden: Hidden,
}

/// Start describing `command` (the root of a CLI).
///
/// The command is cloned, so it can still be used to parse. Nothing is read
/// until [`Builder::build`].
pub fn from_command(command: &clap::Command) -> Builder {
    Builder {
        command: command.clone(),
        options: Options::default(),
        title: None,
        version: None,
        summary: None,
        description: None,
        extras: Extras::default(),
        at: Vec::new(),
    }
}

/// The commands of a REPL parsed with clap, for [`Builder::interactive_commands`]:
/// the subcommands of `command`, mapped like the commands of a CLI (hidden
/// ones marked). The command's own arguments are not part of the result.
///
/// # Errors
///
/// Where [`Builder::build`] fails on the tree: an option with only a short.
pub fn interactive_from_command(command: &clap::Command) -> Result<Vec<Command>, Error> {
    let mut built = command.clone();
    built.build();
    let options = Options::default();
    let root = vec![command.get_name().to_string()];
    Ok(Walk::new(&options)
        .parts(&built, Some(command), &[], &root)?
        .commands)
}

/// The extras a command can declare: everything clap does not know.
#[derive(Debug, Clone, Default)]
struct Extras {
    exit_codes: Vec<ExitCode>,
    examples: Vec<String>,
    environment: Vec<EnvVar>,
    interactive: Option<bool>,
    interactive_commands: Option<Vec<Command>>,
    keys: Option<Vec<KeyBinding>>,
    extensions: Vec<(String, Value)>,
}

/// The extension keys a typed field or method owns.
const TYPED_EXTENSIONS: &[(&str, &str)] = &[
    ("x-interactive", "interactive_commands and keys"),
    ("x-environment", "env"),
    ("x-passthrough", "passthrough, on a command"),
];

impl Extras {
    fn check(&self, at: &str) -> Result<(), Error> {
        for (key, _) in &self.extensions {
            if !key.starts_with("x-") {
                return Err(Error(format!(
                    "{at}: `{key}` is not an extension key; extension keys start with `x-`"
                )));
            }
            if let Some((_, method)) = TYPED_EXTENSIONS.iter().find(|(k, _)| k == key) {
                return Err(Error(format!(
                    "{at}: `{key}` has a typed form; set it with {method}"
                )));
            }
            if key == "x-openapi" {
                return Err(Error(format!(
                    "{at}: `x-openapi` is the HTTP binding a generated CLI reads; a clap CLI \
                     has none"
                )));
            }
        }
        Ok(())
    }

    fn x_interactive(&self) -> Option<XInteractive> {
        if self.interactive_commands.is_none() && self.keys.is_none() {
            return None;
        }
        Some(XInteractive {
            commands: self.interactive_commands.clone().unwrap_or_default(),
            keys: self.keys.clone().unwrap_or_default(),
            ..Default::default()
        })
    }
}

/// The extras methods [`Builder`] and [`At`] share.
macro_rules! extras_methods {
    () => {
        /// Add an exit code and what it means. Codes are written in the order
        /// they are added; a command's list replaces its ancestors' for its
        /// subtree.
        pub fn exit_code(mut self, code: i64, description: impl Into<String>) -> Self {
            self.extras
                .exit_codes
                .push(ExitCode::new(code, description));
            self
        }

        /// Add an example command line.
        pub fn example(mut self, example: impl Into<String>) -> Self {
            self.extras.examples.push(example.into());
            self
        }

        /// Add an environment variable to `x-environment`. On the root, one
        /// named like a variable an argument reads (the `env` feature)
        /// replaces that entry.
        pub fn env(mut self, var: EnvVar) -> Self {
            self.extras.environment.push(var);
            self
        }

        /// Set `interactive`: whether the command can prompt.
        pub fn interactive(mut self, interactive: bool) -> Self {
            self.extras.interactive = Some(interactive);
            self
        }

        /// Set `x-interactive.commands`: the commands a running session
        /// accepts, written without the prefix the session reads them with.
        /// See [`interactive_from_command`] for a REPL parsed with clap.
        pub fn interactive_commands(mut self, commands: impl IntoIterator<Item = Command>) -> Self {
            self.extras.interactive_commands = Some(commands.into_iter().collect());
            self
        }

        /// Set `x-interactive.keys`: the key bindings of a running session.
        pub fn keys(mut self, keys: impl IntoIterator<Item = KeyBinding>) -> Self {
            self.extras.keys = Some(keys.into_iter().collect());
            self
        }

        /// Set an extension key (`x-` keys only; the typed extensions have
        /// their own methods). A key set twice keeps the last value.
        pub fn extension(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
            self.extras.extensions.push((key.into(), value.into()));
            self
        }
    };
}

/// Describes one clap CLI; made by [`from_command`].
#[derive(Debug, Clone)]
#[must_use = "nothing is described until `build` is called"]
pub struct Builder {
    command: clap::Command,
    options: Options,
    title: Option<String>,
    version: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    extras: Extras,
    at: Vec<(Vec<String>, At)>,
}

impl Builder {
    /// How the tree is read.
    pub fn options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }

    /// Override `info.title` (default: the bin name, else the name).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Override `info.version` (default: the command's version).
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Override `info.summary` (default: `about`).
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Override `info.description` (default: `long_about`, when it differs
    /// from `about`).
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    extras_methods!();

    /// Declare extras on the command at `path` (names from the root, never
    /// aliases). A path that names no command of the document makes
    /// [`Builder::build`] fail.
    pub fn at<S: AsRef<str>>(mut self, path: &[S], declare: impl FnOnce(At) -> At) -> Self {
        let path: Vec<String> = path.iter().map(|s| s.as_ref().to_string()).collect();
        self.at.push((path, declare(At::default())));
        self
    }

    /// Read the tree and write the document.
    ///
    /// # Errors
    ///
    /// When the command has no version and none is given with
    /// [`Builder::version`], when an option has only a short (`-x`: an
    /// OpenCLI option is named by its long name), when an
    /// [`at`](Builder::at) path names no command, and when an extension key
    /// is not an `x-` key, has a typed form, or is `x-openapi`.
    ///
    /// # Panics
    ///
    /// Where clap's own build does: a command clap considers malformed.
    pub fn build(self) -> Result<Document, Error> {
        self.extras.check("the root")?;
        for (path, at) in &self.at {
            at.extras.check(&format!("command `{}`", path.join(" ")))?;
        }

        let unbuilt = &self.command;
        let mut built = unbuilt.clone();
        built.build();

        let version = self
            .version
            .or_else(|| unbuilt.get_version().map(str::to_string))
            .ok_or_else(|| {
                Error(
                    "the command has no version: give it one (`Command::version`) or set it \
                     with `Builder::version`"
                        .to_string(),
                )
            })?;
        let title = self.title.unwrap_or_else(|| {
            unbuilt
                .get_bin_name()
                .unwrap_or_else(|| unbuilt.get_name())
                .to_string()
        });
        let mut doc = Document::new(title, version);
        let about = unbuilt.get_about().map(|s| s.to_string());
        let long_about = unbuilt
            .get_long_about()
            .map(|s| s.to_string())
            .filter(|l| Some(l) != about.as_ref());
        doc.info.summary = self.summary.or(about);
        doc.info.description = self.description.or(long_about);

        let mut walk = Walk::new(&self.options);
        let root = vec![unbuilt.get_name().to_string()];
        let parts = walk.parts(&built, Some(unbuilt), &[], &root)?;
        doc.arguments = parts.arguments;
        doc.options = parts.options;
        doc.commands = parts.commands;

        let extras = self.extras;
        doc.exit_codes = extras.exit_codes.clone();
        doc.examples = extras.examples.clone();
        doc.interactive = extras.interactive;
        doc.x_interactive = extras.x_interactive();
        let mut environment = walk.environment;
        for var in extras.environment {
            match environment.iter_mut().find(|e| e.name == var.name) {
                Some(existing) => *existing = var,
                None => environment.push(var),
            }
        }
        doc.x_environment = environment;
        for (key, value) in extras.extensions {
            doc.extensions.insert(key, value);
        }

        for (path, at) in self.at {
            let names: Vec<&str> = path.iter().map(String::as_str).collect();
            let Some(command) = doc.command_mut(&names) else {
                let hint = if self.options.hidden == Hidden::Omit {
                    " (hidden commands are left out under Hidden::Omit)"
                } else {
                    ""
                };
                return Err(Error(format!(
                    "at: the document has no command `{}`{hint}",
                    path.join(" ")
                )));
            };
            at.apply(command);
        }
        Ok(doc)
    }
}

/// The extras of one command, declared through [`Builder::at`].
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct At {
    extras: Extras,
    description: Option<String>,
    passthrough: Option<XPassthrough>,
}

impl At {
    /// Replace the command's description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Mark the command `x-passthrough`: it hands every argument after it,
    /// unparsed, to another program (which its subcommands, if any,
    /// document).
    pub fn passthrough(mut self, marker: XPassthrough) -> Self {
        self.passthrough = Some(marker);
        self
    }

    extras_methods!();

    fn apply(self, command: &mut Command) {
        if let Some(description) = self.description {
            command.description = Some(description);
        }
        if let Some(marker) = self.passthrough {
            command.x_passthrough = Some(marker);
        }
        let x_interactive = self.extras.x_interactive();
        let extras = self.extras;
        command.exit_codes.extend(extras.exit_codes);
        command.examples.extend(extras.examples);
        command.x_environment.extend(extras.environment);
        if extras.interactive.is_some() {
            command.interactive = extras.interactive;
        }
        if x_interactive.is_some() {
            command.x_interactive = x_interactive;
        }
        for (key, value) in extras.extensions {
            command.extensions.insert(key, value);
        }
    }
}
