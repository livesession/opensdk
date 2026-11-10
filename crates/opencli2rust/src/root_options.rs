//! Root options must not collide with anything clap also registers.
//!
//! A root option is an argument of the root command, and a `recursive` one is
//! `.global(true)`: clap copies it onto every subcommand. Two arguments sharing
//! an id, a long or a short at one level is a clap debug-build panic; a global
//! sharing an id with a subcommand's own flag is worse in release, where the
//! subcommand's value silently answers as the root option. Both are decided
//! here, at generation time, with both sides named.

use serde_json::Value;

use crate::model::{build_root_flags, FlagModel};

/// The spellings of one argument as clap registers it.
struct Spelled {
    /// How a message names it: `root option --token`, `option --limit of
    /// \`widgets list\``, ...
    label: String,
    id: String,
    longs: Vec<String>,
    short: Option<String>,
}

impl Spelled {
    fn option(label: String, name: &str, aliases: &[String]) -> Self {
        let mut longs = vec![name.to_string()];
        longs.extend(aliases.iter().filter(|a| a.chars().count() > 1).cloned());
        Spelled {
            label,
            id: name.to_string(),
            longs,
            // The first one-character alias is the one rendered as `.short`.
            short: aliases.iter().find(|a| a.chars().count() == 1).cloned(),
        }
    }

    fn of_root(flag: &FlagModel) -> Self {
        Self::option(
            format!("root option `--{}`", flag.flag_name),
            &flag.flag_name,
            &flag.aliases,
        )
    }

    /// The first spelling `self` and `other` share, rendered as a flag.
    fn shared(&self, other: &Spelled) -> Option<String> {
        if self.id == other.id {
            return Some(format!("the argument id `{}`", self.id));
        }
        if let Some(long) = self.longs.iter().find(|l| other.longs.contains(l)) {
            return Some(format!("`--{long}`"));
        }
        match (&self.short, &other.short) {
            (Some(a), Some(b)) if a == b => Some(format!("`-{a}`")),
            _ => None,
        }
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Every option and positional of every command in the tree, labelled with
/// its command path.
fn command_arguments(commands: &[Value], path: &[String], out: &mut Vec<Spelled>) {
    for command in commands {
        let name = command.get("name").and_then(Value::as_str).unwrap_or("");
        let mut here = path.to_vec();
        here.push(name.to_string());
        let at = here.join(" ");
        for opt in command
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let opt_name = opt.get("name").and_then(Value::as_str).unwrap_or("");
            out.push(Spelled::option(
                format!("option `--{opt_name}` of `{at}`"),
                opt_name,
                &string_list(opt.get("aliases")),
            ));
        }
        for arg in command
            .get("arguments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let arg_name = arg.get("name").and_then(Value::as_str).unwrap_or("");
            out.push(Spelled {
                label: format!("argument `<{arg_name}>` of `{at}`"),
                id: arg_name.to_string(),
                longs: Vec::new(),
                short: None,
            });
        }
        if let Some(subs) = command.get("commands").and_then(Value::as_array) {
            command_arguments(subs, &here, out);
        }
    }
}

/// The arguments clap adds by itself: `-h/--help`, and `-V/--version` when the
/// root command has a version.
fn builtins(has_version: bool) -> Vec<Spelled> {
    let mut out = vec![Spelled {
        label: "clap's built-in `-h/--help`".to_string(),
        id: "help".to_string(),
        longs: vec!["help".to_string()],
        short: Some("h".to_string()),
    }];
    if has_version {
        out.push(Spelled {
            label: "clap's built-in `-V/--version`".to_string(),
            id: "version".to_string(),
            longs: vec!["version".to_string()],
            short: Some("V".to_string()),
        });
    }
    out
}

/// Check the document's root options against clap's built-ins, each other,
/// and — for the global ones — every command's options and positionals.
pub fn check(spec: &Value, has_version: bool) -> Result<(), String> {
    let flags = build_root_flags(spec);
    let roots: Vec<(Spelled, bool)> = flags
        .iter()
        .map(|f| (Spelled::of_root(f), f.global))
        .collect();
    let clash = |a: &Spelled, b: &Spelled, what: String| {
        format!(
            "{} and {} both use {what} — rename one, or change its aliases",
            a.label, b.label
        )
    };

    let builtins = builtins(has_version);
    for (i, (root, global)) in roots.iter().enumerate() {
        for builtin in &builtins {
            if let Some(what) = root.shared(builtin) {
                return Err(clash(root, builtin, what));
            }
        }
        for (other, _) in &roots[i + 1..] {
            if let Some(what) = root.shared(other) {
                return Err(clash(root, other, what));
            }
        }
        if !global {
            continue;
        }
        let mut args = Vec::new();
        let commands = spec
            .get("commands")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        command_arguments(commands, &[], &mut args);
        for arg in &args {
            if let Some(what) = root.shared(arg) {
                return Err(clash(root, arg, what));
            }
        }
    }
    Ok(())
}
