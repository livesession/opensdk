//! Mounting a component's OpenCLI document into a host's.
//!
//! A host CLI that runs a component CLI as one of its commands (`acme agent
//! ...` handing every argument to an agent CLI) can describe the component in
//! its own reference document: [`mount`] places the component's document at
//! one command of the host's.
//!
//! # The rules
//!
//! * **M1 Input.** The component is an OpenCLI document (the root-`command`
//!   shape is lifted when it is read). Its `conventions` must equal the
//!   host's, absent meaning the defaults.
//! * **M2 Placement.** The component goes at the top level, or under the
//!   command [`MountOptions::under`] names. A command of that name may already
//!   exist only as a passthrough stub (`x-passthrough` with no arguments,
//!   options or subcommands), which the mount fills; the filled command keeps
//!   the stub's name, aliases, `hidden`, `x-passthrough` and description.
//!   Otherwise the command is appended, and its name must not be a spelling
//!   (name or alias) of a sibling. Mounting the same command twice is an
//!   error.
//! * **M3 What moves.** The component's root arguments, options, commands,
//!   exit codes, examples, `interactive`, metadata and `x-*` keys move onto the
//!   command. Its `info.description`, else `info.summary`, describes the
//!   command when the stub has no description. `info.version` becomes the
//!   metadata entry [`MountOptions::version_metadata`]; the title, contact and
//!   license are dropped, and so is a `$schema` key. An `x-openapi` binding
//!   anywhere in the component is an error: the component is documented,
//!   never generated from. So is any other root key that is not `x-*`: it
//!   would have no place on a command.
//! * **M4 Collisions.** Every recursive option of the host that is in scope at
//!   the command (the root's and its ancestors') is compared with every option
//!   in the mounted subtree, by every spelling. A match is an error naming
//!   both, unless the component's option is in
//!   [`MountOptions::drop_options`]: a dropped option is removed from the
//!   whole subtree and reported. A drop that matches no option is an error.
//! * **M5 Exit codes.** The mounted command's exit codes replace the host
//!   root's for its subtree, which is the nearest-ancestor rule docs apply.
//! * **M6 No filtering.** Nothing else is removed: what is hidden is the
//!   component's decision. The result is written in canonical key order.
//! * **M7 Report.** [`MountReport`] displays as one line:
//!   `mounted <path> <version>: dropped --<option>, ...`.

use crate::model::{CliOption, Command, Conventions, Document, Metadata, X_OPENAPI};
use crate::Error;

/// How [`mount`] places a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountOptions {
    /// The path of the host command to mount under (names, from the root).
    /// Empty: the top level.
    pub under: Vec<String>,
    /// Names of the component's options to remove from the mounted subtree,
    /// wherever they appear, with or without their dashes. Each must match at
    /// least one option; a name given twice is dropped once.
    pub drop_options: Vec<String>,
    /// The metadata entry that records the component's `info.version`.
    pub version_metadata: String,
}

impl Default for MountOptions {
    fn default() -> Self {
        MountOptions {
            under: Vec::new(),
            drop_options: Vec::new(),
            version_metadata: "componentVersion".to_string(),
        }
    }
}

/// What [`mount`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountReport {
    /// The mounted command's path (names, from the root).
    pub path: Vec<String>,
    /// The component's `info.version`.
    pub version: String,
    /// The options dropped from the subtree, by name.
    pub dropped: Vec<String>,
    /// Whether a passthrough stub was filled (else the command was appended).
    pub filled_stub: bool,
}

impl std::fmt::Display for MountReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mounted {} {}", self.path.join(" "), self.version)?;
        if !self.dropped.is_empty() {
            let dropped: Vec<String> = self.dropped.iter().map(|o| format!("--{o}")).collect();
            write!(f, ": dropped {}", dropped.join(", "))?;
        }
        Ok(())
    }
}

/// Mount `component` into `host` as the command `name` (see the module docs
/// for the rules). On an error `host` is unchanged.
pub fn mount(
    host: &mut Document,
    name: &str,
    component: &Document,
    options: &MountOptions,
) -> Result<MountReport, Error> {
    let mut path = options.under.clone();
    path.push(name.to_string());
    let at = path.join(" ");
    // How `opensdk opencli --mount` spells this mount, for the hints.
    let flag_name = path.join("/");
    let fail = |msg: String| Error(format!("mount {at}: {msg}"));

    if name.is_empty() {
        return Err(fail("the command name is empty".into()));
    }

    // M1: one set of conventions for one command line.
    if Conventions::effective(host.conventions.as_ref())
        != Conventions::effective(component.conventions.as_ref())
    {
        let show = |c: Option<&Conventions>| {
            let (group, separator) = Conventions::effective(c);
            format!("groupOptions {group}, optionSeparator {separator:?}")
        };
        return Err(fail(format!(
            "the component's conventions ({}) differ from the host's ({})",
            show(component.conventions.as_ref()),
            show(host.conventions.as_ref())
        )));
    }

    // M3: a component is documented, never generated from.
    if component.extensions.contains_key(X_OPENAPI) {
        return Err(fail(
            "the component carries an `x-openapi` binding at its root; a mounted \
             component is documentation, so it cannot bind HTTP requests"
                .into(),
        ));
    }
    if let Some(bound) = find_binding(&component.commands, &[]) {
        return Err(fail(format!(
            "the component's command `{bound}` carries an `x-openapi` binding; a mounted \
             component is documentation, so it cannot bind HTTP requests"
        )));
    }
    if component.extensions.contains_key("x-passthrough") {
        return Err(fail(
            "the component carries `x-passthrough` at its root; the marker belongs on the \
             host's command"
                .into(),
        ));
    }
    // Only `x-*` keys move (and `$schema` is the file's, not the CLI's): any
    // other root key would be silently lost on a command.
    if let Some(key) = component
        .extensions
        .keys()
        .find(|k| !k.starts_with("x-") && k.as_str() != "$schema")
    {
        return Err(fail(format!(
            "the component's root key `{key}` has no place on a command; only its arguments, \
             options, commands, exitCodes, examples, interactive, metadata and x-* keys move"
        )));
    }

    // M2: where it goes. The ancestors' recursive options are collected on
    // the way down (M4).
    let mut in_scope: Vec<(String, CliOption)> = host
        .options
        .iter()
        .filter(|o| o.is_recursive())
        .map(|o| ("the host's root".to_string(), o.clone()))
        .collect();
    let mut siblings = &host.commands;
    for (depth, segment) in options.under.iter().enumerate() {
        let parent_path = options.under[..=depth].join(" ");
        let Some(parent) = siblings.iter().find(|c| c.name == *segment) else {
            return Err(fail(format!("the host has no command `{parent_path}`")));
        };
        in_scope.extend(
            parent
                .options
                .iter()
                .filter(|o| o.is_recursive())
                .map(|o| (format!("the host's `{parent_path}`"), o.clone())),
        );
        siblings = &parent.commands;
    }
    let existing = siblings.iter().find(|c| c.name == name);
    let filled_stub = match existing {
        Some(command)
            if command
                .metadata
                .iter()
                .any(|m| m.name == options.version_metadata) =>
        {
            return Err(fail(format!(
                "`{at}` is already mounted (it has a `{}` metadata entry)",
                options.version_metadata
            )));
        }
        Some(command) if !command.is_passthrough_stub() => {
            return Err(fail(format!(
                "the host already has a command `{at}`, and only a passthrough stub (an \
                 `x-passthrough` marker with no arguments, options or subcommands) can be \
                 mounted into"
            )));
        }
        Some(_) => true,
        None => {
            if let Some(other) = siblings.iter().find(|c| c.spellings().any(|s| s == name)) {
                return Err(fail(format!(
                    "command `{name}` and command `{}` both answer to `{name}` (a name or an \
                     alias)",
                    other.name
                )));
            }
            false
        }
    };

    // The subtree that moves (M3), with the drops applied (M4).
    let mut mounted = Command {
        arguments: component.arguments.clone(),
        options: component.options.clone(),
        commands: component.commands.clone(),
        ..Default::default()
    };
    let mut dropped: Vec<String> = Vec::new();
    for option in &options.drop_options {
        let option = option.trim_start_matches('-');
        // A repeat (`profile` and `--profile` are one) was dropped already.
        if dropped.iter().any(|d| d == option) {
            continue;
        }
        if drop_option(&mut mounted, option) == 0 {
            return Err(fail(format!(
                "--mount-drop {flag_name}:{option} matches no option of the component"
            )));
        }
        dropped.push(option.to_string());
    }

    // M4: a host option in scope must not answer to a spelling the component
    // uses anywhere below the command.
    let mut component_options = Vec::new();
    collect_options(&mounted, &path, &mut component_options);
    for (owner, host_option) in &in_scope {
        let host_spellings = host_option.spellings();
        for (command_path, option) in &component_options {
            if let Some(spelling) = option
                .spellings()
                .into_iter()
                .find(|s| host_spellings.contains(s))
            {
                return Err(fail(format!(
                    "{owner} recursive option `--{}` and the component's option `--{}` of \
                     `{command_path}` both answer to `{spelling}`; drop the component's with \
                     --mount-drop {flag_name}:{}",
                    host_option.name, option.name, option.name
                )));
            }
        }
    }

    // Everything checked: fill the stub, or append the command.
    let siblings = siblings_mut(host, &options.under);
    let command = match siblings.iter_mut().position(|c| c.name == name) {
        Some(i) => &mut siblings[i],
        None => {
            siblings.push(Command::new(name));
            siblings.last_mut().expect("just pushed")
        }
    };
    command.arguments = mounted.arguments;
    command.options = mounted.options;
    command.commands = mounted.commands;
    if !component.exit_codes.is_empty() {
        command.exit_codes = component.exit_codes.clone();
    }
    if !component.examples.is_empty() {
        command.examples = component.examples.clone();
    }
    if component.interactive.is_some() {
        command.interactive = component.interactive;
    }
    command.metadata.extend(component.metadata.iter().cloned());
    command
        .metadata
        .retain(|m| m.name != options.version_metadata);
    command.metadata.push(Metadata::new(
        options.version_metadata.clone(),
        component.info.version.clone(),
    ));
    if component.x_interactive.is_some() {
        command.x_interactive = component.x_interactive.clone();
    }
    if !component.x_environment.is_empty() {
        command.x_environment = component.x_environment.clone();
    }
    for (key, value) in component
        .extensions
        .iter()
        .filter(|(k, _)| k.starts_with("x-"))
    {
        command.extensions.insert(key.clone(), value.clone());
    }
    if command.description.is_none() {
        command.description = component
            .info
            .description
            .clone()
            .or_else(|| component.info.summary.clone());
    }

    Ok(MountReport {
        path,
        version: component.info.version.clone(),
        dropped,
        filled_stub,
    })
}

/// The path of the first command at or below `commands` with an `x-openapi`
/// binding.
fn find_binding(commands: &[Command], path: &[&str]) -> Option<String> {
    for command in commands {
        let mut here = path.to_vec();
        here.push(&command.name);
        if command.extensions.contains_key(X_OPENAPI) {
            return Some(here.join(" "));
        }
        if let Some(found) = find_binding(&command.commands, &here) {
            return Some(found);
        }
    }
    None
}

/// Remove every option named `name` from `command` and its subtree; how many
/// went.
fn drop_option(command: &mut Command, name: &str) -> usize {
    let before = command.options.len();
    command.options.retain(|o| o.name != name);
    let mut removed = before - command.options.len();
    for sub in &mut command.commands {
        removed += drop_option(sub, name);
    }
    removed
}

/// Every option of `command` and its subtree, with its command's path.
fn collect_options(command: &Command, path: &[String], out: &mut Vec<(String, CliOption)>) {
    let at = path.join(" ");
    out.extend(command.options.iter().map(|o| (at.clone(), o.clone())));
    for sub in &command.commands {
        let mut here = path.to_vec();
        here.push(sub.name.clone());
        collect_options(sub, &here, out);
    }
}

/// The subcommand list of the host command at `path` (the root's when empty).
/// The path was checked to exist.
fn siblings_mut<'a>(host: &'a mut Document, path: &[String]) -> &'a mut Vec<Command> {
    let mut siblings = &mut host.commands;
    for segment in path {
        let i = siblings
            .iter()
            .position(|c| c.name == *segment)
            .expect("checked above");
        siblings = &mut siblings[i].commands;
    }
    siblings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ExitCode, XPassthrough};
    use serde_json::json;

    fn host() -> Document {
        Document::from_value(json!({
            "opencli": "1.0.0",
            "info": { "title": "acme", "version": "2.0.0" },
            "options": [
                { "name": "profile", "aliases": ["P"], "recursive": true, "arguments": [{ "name": "name" }] },
                { "name": "quiet", "aliases": ["q"] }
            ],
            "commands": [
                {
                    "name": "agent",
                    "aliases": ["ag"],
                    "description": "Run the agent CLI.",
                    "hidden": false,
                    "x-passthrough": { "program": "acme-agent" }
                },
                {
                    "name": "tools",
                    "options": [{ "name": "dry-run", "recursive": true }],
                    "commands": [{ "name": "list" }]
                }
            ],
            "exitCodes": [{ "code": 0, "description": "Success" }]
        }))
        .unwrap()
    }

    fn component() -> Document {
        Document::from_value(json!({
            "opencli": "1.0.0",
            "info": {
                "title": "acme-agent",
                "version": "0.4.0",
                "summary": "An agent CLI.",
                "description": "An agent CLI for the terminal.",
                "license": { "name": "MIT" }
            },
            "arguments": [{ "name": "prompt" }],
            "options": [
                { "name": "print", "aliases": ["p"] },
                { "name": "profile", "arguments": [{ "name": "name" }] }
            ],
            "commands": [{
                "name": "resume",
                "options": [{ "name": "last" }, { "name": "profile", "hidden": true }]
            }],
            "exitCodes": [{ "code": 3, "description": "Not signed in" }],
            "examples": ["acme-agent -p hello"],
            "interactive": true,
            "x-interactive": { "commands": [{ "name": "model" }] },
            "x-environment": [{ "name": "ACME_AGENT_HOME", "description": "Where it keeps state." }],
            "x-extra": { "a": 1 }
        }))
        .unwrap()
    }

    fn drop_profile() -> MountOptions {
        MountOptions {
            drop_options: vec!["profile".into()],
            ..Default::default()
        }
    }

    #[test]
    fn a_stub_is_filled_and_keeps_its_own_identity() {
        let mut doc = host();
        let report = mount(&mut doc, "agent", &component(), &drop_profile()).unwrap();
        assert_eq!(report.to_string(), "mounted agent 0.4.0: dropped --profile");
        assert!(report.filled_stub);
        let agent = doc.command(&["agent"]).unwrap();
        assert_eq!(agent.aliases, ["ag"]);
        assert_eq!(agent.description.as_deref(), Some("Run the agent CLI."));
        assert_eq!(agent.hidden, Some(false));
        assert_eq!(
            agent.x_passthrough,
            Some(XPassthrough::program("acme-agent"))
        );
        assert_eq!(agent.arguments[0].name, "prompt");
        let names: Vec<&str> = agent.options.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["print"]);
        let resume: Vec<&str> = agent.commands[0]
            .options
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(resume, ["last"]);
        assert_eq!(agent.exit_codes, [ExitCode::new(3, "Not signed in")]);
        assert_eq!(agent.examples, ["acme-agent -p hello"]);
        assert_eq!(agent.interactive, Some(true));
        assert_eq!(agent.metadata, [Metadata::new("componentVersion", "0.4.0")]);
        assert!(agent.x_interactive.is_some());
        assert_eq!(agent.x_environment[0].name, "ACME_AGENT_HOME");
        assert_eq!(agent.extensions["x-extra"], json!({ "a": 1 }));
        // The host root's own exit codes stay (M5 is the docs' nearest rule).
        assert_eq!(doc.exit_codes[0].code, 0);
    }

    #[test]
    fn a_new_command_is_appended_and_described_by_the_component() {
        let mut doc = host();
        let options = MountOptions {
            under: vec!["tools".into()],
            ..drop_profile()
        };
        let report = mount(&mut doc, "agent", &component(), &options).unwrap();
        assert_eq!(
            report.to_string(),
            "mounted tools agent 0.4.0: dropped --profile"
        );
        assert!(!report.filled_stub);
        let agent = doc.command(&["tools", "agent"]).unwrap();
        assert_eq!(
            agent.description.as_deref(),
            Some("An agent CLI for the terminal.")
        );
        assert!(agent.x_passthrough.is_none());
        assert_eq!(doc.command(&["tools"]).unwrap().commands.len(), 2);
    }

    #[test]
    fn a_drop_given_twice_is_dropped_once() {
        let mut doc = host();
        let twice = MountOptions {
            drop_options: vec!["profile".into(), "--profile".into(), "profile".into()],
            ..Default::default()
        };
        let report = mount(&mut doc, "agent", &component(), &twice).unwrap();
        assert_eq!(report.to_string(), "mounted agent 0.4.0: dropped --profile");
        assert_eq!(report.dropped, ["profile"]);
        // A name that matches nothing is still an error after a repeat.
        let mut doc = host();
        let unknown = MountOptions {
            drop_options: vec!["profile".into(), "profile".into(), "nope".into()],
            ..Default::default()
        };
        assert_eq!(
            err(&mut doc, "agent", &component(), &unknown),
            "mount agent: --mount-drop agent:nope matches no option of the component"
        );
    }

    #[test]
    fn a_report_without_drops_is_one_short_line() {
        let mut doc = host();
        let mut c = component();
        c.options.retain(|o| o.name != "profile");
        c.commands[0].options.retain(|o| o.name != "profile");
        assert_eq!(
            mount(&mut doc, "agent", &c, &MountOptions::default())
                .unwrap()
                .to_string(),
            "mounted agent 0.4.0"
        );
    }

    fn err(host: &mut Document, name: &str, c: &Document, o: &MountOptions) -> String {
        let before = host.clone();
        let e = mount(host, name, c, o).unwrap_err().0;
        assert_eq!(*host, before, "a failed mount changed the host");
        e
    }

    #[test]
    fn every_rule_has_its_error() {
        let mut doc = host();
        // M4: the collision names both sides, and a short alias collides too.
        assert_eq!(
            err(&mut doc, "agent", &component(), &MountOptions::default()),
            "mount agent: the host's root recursive option `--profile` and the component's \
             option `--profile` of `agent` both answer to `--profile`; drop the component's \
             with --mount-drop agent:profile"
        );
        let mut c = component();
        c.options[0].aliases = vec!["P".into()];
        assert!(err(&mut doc, "agent", &c, &drop_profile()).contains("both answer to `-P`"));
        // The ancestors' recursive options are in scope too.
        let mut c = component();
        c.commands[0].options.push(CliOption::new("dry-run"));
        let under_tools = MountOptions {
            under: vec!["tools".into()],
            ..drop_profile()
        };
        assert!(err(&mut doc, "agent", &c, &under_tools)
            .contains("the host's `tools` recursive option `--dry-run`"));
        // M4: a drop matching nothing.
        let unknown = MountOptions {
            drop_options: vec!["profile".into(), "nope".into()],
            ..Default::default()
        };
        assert_eq!(
            err(&mut doc, "agent", &component(), &unknown),
            "mount agent: --mount-drop agent:nope matches no option of the component"
        );
        // M1: conventions.
        let mut c = component();
        c.conventions = Some(Conventions {
            option_separator: Some("=".into()),
            ..Default::default()
        });
        assert!(err(&mut doc, "agent", &c, &drop_profile())
            .contains("the component's conventions (groupOptions true, optionSeparator \"=\")"));
        // ...but the defaults spelled out equal an absent block.
        let mut c = component();
        c.conventions = Some(Conventions {
            group_options: Some(true),
            option_separator: Some(" ".into()),
            ..Default::default()
        });
        mount(&mut doc.clone(), "agent", &c, &drop_profile()).unwrap();
        // M3: an HTTP binding anywhere.
        let mut c = component();
        c.commands[0]
            .extensions
            .insert("x-openapi".into(), json!({ "method": "get", "path": "/" }));
        assert!(err(&mut doc, "agent", &c, &drop_profile())
            .contains("the component's command `resume` carries an `x-openapi` binding"));
        // M3: a root key with no place on a command; `$schema` is dropped.
        let mut c = component();
        c.extensions
            .insert("$schema".into(), json!("https://acme.test/s.json"));
        let mut copy = doc.clone();
        mount(&mut copy, "agent", &c, &drop_profile()).unwrap();
        assert!(!copy
            .command(&["agent"])
            .unwrap()
            .extensions
            .contains_key("$schema"));
        c.extensions.insert("future".into(), json!(1));
        assert!(err(&mut doc, "agent", &c, &drop_profile())
            .contains("the component's root key `future` has no place on a command"));
        // M2: a missing parent, a taken spelling, a command that is not a stub.
        let nowhere = MountOptions {
            under: vec!["tools".into(), "nope".into()],
            ..drop_profile()
        };
        assert_eq!(
            err(&mut doc, "agent", &component(), &nowhere),
            "mount tools nope agent: the host has no command `tools nope`"
        );
        assert_eq!(
            err(&mut doc, "ag", &component(), &drop_profile()),
            "mount ag: command `ag` and command `agent` both answer to `ag` (a name or an alias)"
        );
        assert!(err(&mut doc, "tools", &component(), &drop_profile())
            .contains("only a passthrough stub"));
        // M2: twice.
        mount(&mut doc, "agent", &component(), &drop_profile()).unwrap();
        assert!(err(&mut doc, "agent", &component(), &drop_profile())
            .contains("`agent` is already mounted"));
    }
}
