//! The mapping from a built clap command tree to OpenCLI commands, arguments
//! and options.
//!
//! The walk reads a clone of the command after [`clap::Command::build`] (so
//! every default is filled in and every global is spread to the subcommands)
//! next to the same command unbuilt (so a global can be told apart from the
//! copies clap made of it). What clap added is skipped: its `help` subcommand
//! tree, and every argument whose action is help or version, whatever its id.

use clap::{Arg, ArgAction, Command as ClapCommand};
use opencli_model::{Argument, Arity, CliOption, Command, EnvVar, Metadata};

use crate::{Hidden, Options};

/// What one clap command contributes: its positionals, options and
/// subcommands.
#[derive(Default)]
pub(crate) struct Parts {
    pub arguments: Vec<Argument>,
    pub options: Vec<CliOption>,
    pub commands: Vec<Command>,
}

/// One walk over a command tree.
pub(crate) struct Walk<'o> {
    options: &'o Options,
    /// The environment variables the arguments read (the `env` feature), in
    /// walk order, one entry per name.
    pub environment: Vec<EnvVar>,
}

impl<'o> Walk<'o> {
    pub fn new(options: &'o Options) -> Self {
        Walk {
            options,
            environment: Vec::new(),
        }
    }

    /// The parts of `built`, a command of a built tree. `unbuilt` is the same
    /// command before the build, when it existed then (a command added by
    /// `Command::defer` did not); `inherited` holds the ids of the globals an
    /// ancestor declares.
    pub fn parts(
        &mut self,
        built: &ClapCommand,
        unbuilt: Option<&ClapCommand>,
        inherited: &[String],
    ) -> Parts {
        let mut parts = Parts::default();
        let mut positionals = Vec::new();
        let mut declared_globals = inherited.to_vec();

        for arg in built.get_arguments() {
            if is_help_or_version(arg) || is_spread_copy(arg, unbuilt, inherited) {
                continue;
            }
            // Recorded even when it is left out below: its copies further
            // down are still copies.
            if arg.is_global_set() {
                declared_globals.push(arg.get_id().as_str().to_string());
            }
            if arg.is_hide_set() && self.options.hidden == Hidden::Omit {
                continue;
            }
            if arg.is_positional() {
                positionals.push((
                    arg.get_index().unwrap_or(usize::MAX),
                    self.argument(built, arg),
                ));
            } else {
                parts.options.push(self.option(built, arg));
            }
        }
        // `sort_by_key` is stable: two positionals without an index (an
        // unbuilt tree never reaches here) keep their declaration order.
        positionals.sort_by_key(|(index, _)| *index);
        parts.arguments = positionals.into_iter().map(|(_, a)| a).collect();

        for sub in built.get_subcommands() {
            if is_help_subcommand(built, unbuilt, sub) {
                continue;
            }
            if sub.is_hide_set() && self.options.hidden == Hidden::Omit {
                continue;
            }
            let unbuilt_sub = unbuilt.and_then(|u| u.find_subcommand(sub.get_name()));
            parts
                .commands
                .push(self.command(sub, unbuilt_sub, &declared_globals));
        }
        parts
    }

    fn command(
        &mut self,
        built: &ClapCommand,
        unbuilt: Option<&ClapCommand>,
        inherited: &[String],
    ) -> Command {
        let parts = self.parts(built, unbuilt, inherited);
        Command {
            name: built.get_name().to_string(),
            aliases: built.get_visible_aliases().map(str::to_string).collect(),
            description: built
                .get_long_about()
                .or_else(|| built.get_about())
                .map(|s| s.to_string()),
            arguments: parts.arguments,
            options: parts.options,
            commands: parts.commands,
            hidden: self.hidden(built.is_hide_set()),
            ..Default::default()
        }
    }

    /// A positional argument.
    fn argument(&mut self, owner: &ClapCommand, arg: &Arg) -> Argument {
        let name = value_names(arg)
            .into_iter()
            .next()
            .unwrap_or_else(|| arg.get_id().as_str().to_string());
        let mut metadata = Vec::new();
        self.common_metadata(owner, arg, &mut metadata);
        Argument {
            name,
            required: arg.is_required_set().then_some(true),
            description: description(arg),
            arity: arity(arg),
            accepted_values: accepted_values(arg),
            group: arg.get_help_heading().map(str::to_string),
            hidden: self.hidden(arg.is_hide_set()),
            metadata,
            ..Default::default()
        }
    }

    /// An option.
    fn option(&mut self, owner: &ClapCommand, arg: &Arg) -> CliOption {
        let long = arg.get_long();
        let short = arg.get_short();
        let name = match (long, short) {
            (Some(long), _) => long.to_string(),
            (None, Some(short)) => short.to_string(),
            // clap calls an argument without a long or a short a positional.
            (None, None) => arg.get_id().as_str().to_string(),
        };
        let mut aliases = Vec::new();
        if let (Some(_), Some(short)) = (long, short) {
            aliases.push(short.to_string());
        }
        aliases.extend(
            arg.get_visible_aliases()
                .unwrap_or_default()
                .into_iter()
                .map(str::to_string),
        );
        aliases.extend(
            arg.get_visible_short_aliases()
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.to_string()),
        );

        let mut metadata = Vec::new();
        self.common_metadata(owner, arg, &mut metadata);
        if matches!(arg.get_action(), ArgAction::Append | ArgAction::Count) {
            // After `default` and `env`, before the relations.
            let at = metadata
                .iter()
                .position(|m| m.name != "default" && m.name != "env")
                .unwrap_or(metadata.len());
            metadata.insert(at, Metadata::new("repeatable", true));
        }

        CliOption {
            name,
            aliases,
            required: arg.is_required_set().then_some(true),
            description: description(arg),
            group: arg.get_help_heading().map(str::to_string),
            recursive: arg.is_global_set().then_some(true),
            hidden: self.hidden(arg.is_hide_set()),
            arguments: option_values(arg),
            metadata,
            ..Default::default()
        }
    }

    /// The metadata both kinds share, in order: `default`, `env`,
    /// `conflictsWith`, `exclusive`, `last`. An option inserts `repeatable`
    /// after `env`.
    fn common_metadata(&mut self, owner: &ClapCommand, arg: &Arg, out: &mut Vec<Metadata>) {
        let takes_values = arg.get_action().takes_values();
        if takes_values && !arg.is_hide_default_value_set() {
            let defaults: Vec<String> = arg
                .get_default_values()
                .iter()
                .map(|v| v.to_string_lossy().into_owned())
                .collect();
            match defaults.len() {
                0 => {}
                1 => out.push(Metadata::new("default", defaults[0].clone())),
                _ => out.push(Metadata::new("default", defaults)),
            }
        }
        #[cfg(feature = "env")]
        self.env(arg, out);

        let conflicts = self.conflicts(owner, arg);
        if !conflicts.is_empty() {
            out.push(Metadata::new("conflictsWith", conflicts));
        }
        if arg.is_exclusive_set() {
            out.push(Metadata::new("exclusive", true));
        }
        if arg.is_last_set() {
            out.push(Metadata::new("last", true));
        }
    }

    /// The `env` metadata and the root `x-environment` entry of an argument
    /// that reads a variable. A variable whose help clap hides (`hide_env`)
    /// is left out, and a hidden argument's variable stays off the root list.
    #[cfg(feature = "env")]
    fn env(&mut self, arg: &Arg, out: &mut Vec<Metadata>) {
        let Some(name) = arg.get_env() else { return };
        if arg.is_hide_env_set() {
            return;
        }
        let name = name.to_string_lossy().into_owned();
        out.push(Metadata::new("env", name.clone()));
        if arg.is_hide_set() || self.environment.iter().any(|e| e.name == name) {
            return;
        }
        let description = arg
            .get_help()
            .or_else(|| arg.get_long_help())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Sets {}.", spelling(arg)));
        self.environment
            .push(EnvVar::new(name, description).values(accepted_values(arg)));
    }

    /// The spellings of the arguments `arg` conflicts with, groups unrolled,
    /// as declared on `arg` (clap applies them both ways when parsing).
    fn conflicts(&self, owner: &ClapCommand, arg: &Arg) -> Vec<String> {
        // `get_arg_conflicts_with` reads a global's conflicts across the
        // subcommands and cannot unroll a group there; the declaring command
        // holds every id it names (clap asserts that when it builds), so ask
        // it as for a plain argument.
        let plain = arg.clone().global(false);
        let mut out: Vec<String> = Vec::new();
        for other in owner.get_arg_conflicts_with(&plain) {
            if is_help_or_version(other)
                || (other.is_hide_set() && self.options.hidden == Hidden::Omit)
            {
                continue;
            }
            let s = spelling(other);
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    fn hidden(&self, hidden: bool) -> Option<bool> {
        (hidden && self.options.hidden == Hidden::Mark).then_some(true)
    }
}

/// clap's own `--help` and `--version` (and any argument a CLI gives one of
/// their actions): a docs site documents those itself.
fn is_help_or_version(arg: &Arg) -> bool {
    matches!(
        arg.get_action(),
        ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
    )
}

/// Whether `arg` is a copy clap spread from an ancestor's global: a global
/// the ancestor declares that the command did not declare itself.
fn is_spread_copy(arg: &Arg, unbuilt: Option<&ClapCommand>, inherited: &[String]) -> bool {
    let id = arg.get_id();
    arg.is_global_set()
        && inherited.iter().any(|g| g == id.as_str())
        && !unbuilt.is_some_and(|u| u.get_arguments().any(|a| a.get_id() == id))
}

/// Whether `sub` is the `help` subcommand clap adds to `parent`.
fn is_help_subcommand(
    parent: &ClapCommand,
    unbuilt: Option<&ClapCommand>,
    sub: &ClapCommand,
) -> bool {
    sub.get_name() == "help"
        && !parent.is_disable_help_subcommand_set()
        && !unbuilt.is_some_and(|u| u.find_subcommand("help").is_some())
}

/// The value names, lowercased.
fn value_names(arg: &Arg) -> Vec<String> {
    arg.get_value_names()
        .unwrap_or_default()
        .iter()
        .map(|n| n.as_str().to_lowercase())
        .collect()
}

/// `long_help`, else `help`.
fn description(arg: &Arg) -> Option<String> {
    arg.get_long_help()
        .or_else(|| arg.get_help())
        .map(|s| s.to_string())
}

/// The arity, unless it is exactly one value.
fn arity(arg: &Arg) -> Option<Arity> {
    let range = arg.get_num_args()?;
    let (min, max) = (range.min_values(), range.max_values());
    if (min, max) == (1, 1) {
        return None;
    }
    Some(Arity {
        minimum: Some(min as u64),
        maximum: (max != usize::MAX).then_some(max as u64),
        ..Default::default()
    })
}

/// The possible values clap lists in help (the hidden ones never).
fn accepted_values(arg: &Arg) -> Vec<String> {
    arg.get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| v.get_name().to_string())
        .collect()
}

/// An option's values: one argument per value name, or one argument with the
/// option's arity. A flag (an action taking no value) has none.
fn option_values(arg: &Arg) -> Vec<Argument> {
    if !arg.get_action().takes_values() {
        return Vec::new();
    }
    let names = value_names(arg);
    let accepted = accepted_values(arg);
    if names.len() > 1 {
        return names
            .into_iter()
            .map(|name| Argument {
                name,
                accepted_values: accepted.clone(),
                ..Default::default()
            })
            .collect();
    }
    let name = names
        .into_iter()
        .next()
        .unwrap_or_else(|| arg.get_id().as_str().to_string());
    vec![Argument {
        name,
        arity: arity(arg),
        accepted_values: accepted,
        ..Default::default()
    }]
}

/// How an argument is typed: `--long`, `-s`, or `<name>` for a positional.
fn spelling(arg: &Arg) -> String {
    if let Some(long) = arg.get_long() {
        format!("--{long}")
    } else if let Some(short) = arg.get_short() {
        format!("-{short}")
    } else {
        let name = value_names(arg)
            .into_iter()
            .next()
            .unwrap_or_else(|| arg.get_id().as_str().to_string());
        format!("<{name}>")
    }
}
