//! The typed OpenCLI document: the flat 0.1-draft shape this chain reads and
//! writes, with every key a type does not name kept in its `extensions`.

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::shape::lift_root_command;
use crate::Error;

/// The `opencli` string this chain writes.
///
/// It is the chain's own `"1.0.0"`, not the specification draft's version:
/// every committed golden carries it, and switching is a separate, chain-wide
/// change.
pub const OPENCLI_VERSION: &str = "1.0.0";

/// The keys an object does not type, in the order they were read: `x-*`
/// extensions, and any key a later OpenCLI draft adds. They are written back
/// after the typed fields (the root's `x-openapi` right after `info`, where
/// the converter writes it).
///
/// Never put a key a type names (`options`, `x-interactive`, ...) here: it
/// would be written twice.
pub type Extensions = Map<String, Value>;

/// The root `x-openapi` key: the converter's HTTP binding.
pub(crate) const X_OPENAPI: &str = "x-openapi";

/// An OpenCLI document.
///
/// Reading accepts the flat shape and the upstream root-`command` shape (whose
/// root command is lifted to the root, see [`crate::lift_root_command`]);
/// writing always produces the flat shape, keys in canonical order. Lists
/// left empty are not written.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    pub opencli: String,
    pub info: Info,
    pub conventions: Option<Conventions>,
    /// The root command's own positional arguments.
    pub arguments: Vec<Argument>,
    pub options: Vec<CliOption>,
    pub commands: Vec<Command>,
    pub exit_codes: Vec<ExitCode>,
    pub examples: Vec<String>,
    pub interactive: Option<bool>,
    pub metadata: Vec<Metadata>,
    pub x_interactive: Option<XInteractive>,
    pub x_environment: Vec<EnvVar>,
    pub extensions: Extensions,
}

/// What [`Document`] reads once the root-`command` shape is lifted. A mirror
/// rather than a derive on `Document` itself, so that every way of reading a
/// document (`serde_json::from_str::<Document>` included) lifts first.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlatDocument {
    opencli: String,
    info: Info,
    #[serde(default)]
    conventions: Option<Conventions>,
    #[serde(default)]
    arguments: Vec<Argument>,
    #[serde(default)]
    options: Vec<CliOption>,
    #[serde(default)]
    commands: Vec<Command>,
    #[serde(default)]
    exit_codes: Vec<ExitCode>,
    #[serde(default)]
    examples: Vec<String>,
    #[serde(default)]
    interactive: Option<bool>,
    #[serde(default)]
    metadata: Vec<Metadata>,
    #[serde(default, rename = "x-interactive")]
    x_interactive: Option<XInteractive>,
    #[serde(default, rename = "x-environment")]
    x_environment: Vec<EnvVar>,
    #[serde(flatten)]
    extensions: Extensions,
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let value = Value::deserialize(deserializer)?;
        let flat = lift_root_command(value).map_err(|e| D::Error::custom(e.0))?;
        let d: FlatDocument = serde_json::from_value(flat).map_err(D::Error::custom)?;
        Ok(Document {
            opencli: d.opencli,
            info: d.info,
            conventions: d.conventions,
            arguments: d.arguments,
            options: d.options,
            commands: d.commands,
            exit_codes: d.exit_codes,
            examples: d.examples,
            interactive: d.interactive,
            metadata: d.metadata,
            x_interactive: d.x_interactive,
            x_environment: d.x_environment,
            extensions: d.extensions,
        })
    }
}

impl Serialize for Document {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("opencli", &self.opencli)?;
        map.serialize_entry("info", &self.info)?;
        if let Some(conventions) = &self.conventions {
            map.serialize_entry("conventions", conventions)?;
        }
        if let Some(binding) = self.extensions.get(X_OPENAPI) {
            map.serialize_entry(X_OPENAPI, binding)?;
        }
        if !self.arguments.is_empty() {
            map.serialize_entry("arguments", &self.arguments)?;
        }
        if !self.options.is_empty() {
            map.serialize_entry("options", &self.options)?;
        }
        if !self.commands.is_empty() {
            map.serialize_entry("commands", &self.commands)?;
        }
        if !self.exit_codes.is_empty() {
            map.serialize_entry("exitCodes", &self.exit_codes)?;
        }
        if !self.examples.is_empty() {
            map.serialize_entry("examples", &self.examples)?;
        }
        if let Some(interactive) = self.interactive {
            map.serialize_entry("interactive", &interactive)?;
        }
        if !self.metadata.is_empty() {
            map.serialize_entry("metadata", &self.metadata)?;
        }
        if let Some(x) = &self.x_interactive {
            map.serialize_entry("x-interactive", x)?;
        }
        if !self.x_environment.is_empty() {
            map.serialize_entry("x-environment", &self.x_environment)?;
        }
        for (key, value) in &self.extensions {
            if key != X_OPENAPI {
                map.serialize_entry(key, value)?;
            }
        }
        map.end()
    }
}

impl Document {
    /// An empty document for a CLI titled `title` at `version`.
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        Document {
            opencli: OPENCLI_VERSION.to_string(),
            info: Info::new(title, version),
            ..Default::default()
        }
    }

    /// Read a document from JSON (either shape).
    pub fn from_value(value: Value) -> Result<Self, Error> {
        serde_json::from_value(value).map_err(|e| Error(format!("not an OpenCLI document: {e}")))
    }

    /// Read a document from JSON text (either shape).
    pub fn from_json(text: &str) -> Result<Self, Error> {
        serde_json::from_str(text).map_err(|e| Error(format!("not an OpenCLI document: {e}")))
    }

    /// The document as JSON, keys in canonical order.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("an OpenCLI document serializes")
    }

    /// The document as pretty-printed JSON with a trailing newline: the bytes
    /// the chain writes to disk.
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("an OpenCLI document serializes")
        )
    }

    /// The command at `path` (names, from the root), if there is one.
    pub fn command(&self, path: &[&str]) -> Option<&Command> {
        let (first, rest) = path.split_first()?;
        let mut current = self.commands.iter().find(|c| c.name == *first)?;
        for name in rest {
            current = current.commands.iter().find(|c| c.name == *name)?;
        }
        Some(current)
    }

    /// The command at `path`, mutably.
    pub fn command_mut(&mut self, path: &[&str]) -> Option<&mut Command> {
        let (first, rest) = path.split_first()?;
        let mut current = self.commands.iter_mut().find(|c| c.name == *first)?;
        for name in rest {
            current = current.commands.iter_mut().find(|c| c.name == *name)?;
        }
        Some(current)
    }
}

/// `info`: what the CLI is.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Info {
    pub title: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<Contact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Info {
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        Info {
            title: title.into(),
            version: version.into(),
            ..Default::default()
        }
    }
}

/// `conventions`: how the CLI's options are spelled.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conventions {
    /// Whether short options may be grouped (`-ab`). Default: true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_options: Option<bool>,
    /// What separates an option from its value. Default: a space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_separator: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Conventions {
    /// The conventions with the specification's defaults filled in, for
    /// comparing two documents (an absent `conventions` means the defaults).
    pub fn effective(conventions: Option<&Conventions>) -> (bool, String) {
        let group = conventions.and_then(|c| c.group_options).unwrap_or(true);
        let separator = conventions
            .and_then(|c| c.option_separator.clone())
            .unwrap_or_else(|| " ".to_string());
        (group, separator)
    }
}

/// A command (a subcommand of the root, or of another command).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<Argument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<CliOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Command>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit_codes: Vec<ExitCode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<Metadata>,
    #[serde(
        default,
        rename = "x-passthrough",
        skip_serializing_if = "Option::is_none"
    )]
    pub x_passthrough: Option<XPassthrough>,
    #[serde(
        default,
        rename = "x-interactive",
        skip_serializing_if = "Option::is_none"
    )]
    pub x_interactive: Option<XInteractive>,
    #[serde(
        default,
        rename = "x-environment",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub x_environment: Vec<EnvVar>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Command {
    pub fn new(name: impl Into<String>) -> Self {
        Command {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Whether this command is a passthrough stub: an `x-passthrough` marker
    /// with no arguments, options or subcommands of its own, which a mount
    /// may fill.
    pub fn is_passthrough_stub(&self) -> bool {
        self.x_passthrough.is_some()
            && self.arguments.is_empty()
            && self.options.is_empty()
            && self.commands.is_empty()
    }

    /// The spellings the command answers to: its name, then its aliases.
    pub fn spellings(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(self.aliases.iter().map(String::as_str))
    }
}

/// An option (`--name`). OpenCLI calls it `Option`; it is renamed here so it
/// does not shadow [`std::option::Option`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CliOption {
    /// The long name, without dashes (a one-character name is still a long
    /// one, `--x`). An option with only a short has no name in this shape.
    pub name: String,
    /// Other spellings, without dashes: one character is a short (`-o`),
    /// more is a long (`--out`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Accepted by the command it is declared on and every command below it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recursive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// The option's values; none for a flag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<Argument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<Metadata>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl CliOption {
    pub fn new(name: impl Into<String>) -> Self {
        CliOption {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Whether `recursive` is set.
    pub fn is_recursive(&self) -> bool {
        self.recursive == Some(true)
    }

    /// The spellings the option answers to, as typed: `--name`, then each
    /// alias (`-o` for one character, `--out` for more).
    pub fn spellings(&self) -> Vec<String> {
        std::iter::once(format!("--{}", self.name))
            .chain(self.aliases.iter().map(|a| {
                if a.chars().count() == 1 {
                    format!("-{a}")
                } else {
                    format!("--{a}")
                }
            }))
            .collect()
    }
}

/// A positional argument, or one value of an option.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Argument {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arity: Option<Arity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<Metadata>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Argument {
    pub fn new(name: impl Into<String>) -> Self {
        Argument {
            name: name.into(),
            ..Default::default()
        }
    }
}

/// How many values an argument takes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Arity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<u64>,
    /// Absent: no maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<u64>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// One exit code and what it means.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ExitCode {
    pub code: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl ExitCode {
    pub fn new(code: i64, description: impl Into<String>) -> Self {
        ExitCode {
            code,
            description: Some(description.into()),
            ..Default::default()
        }
    }
}

/// One `metadata` entry: a name and any JSON value.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl Metadata {
    pub fn new(name: impl Into<String>, value: impl Into<Value>) -> Self {
        Metadata {
            name: name.into(),
            value: Some(value.into()),
            ..Default::default()
        }
    }
}

/// `info.contact`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Contact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// `info.license`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct License {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// `x-interactive`: what a running interactive session accepts.
///
/// The commands are typed inside the session, not on the command line: they
/// are written without the prefix the session reads them with (`model`, not
/// `/model`), and a docs site makes no page for them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct XInteractive {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Command>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<KeyBinding>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

/// One key binding of an interactive session.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct KeyBinding {
    /// The keys that trigger it, any one of them (`ctrl+c`, `esc`).
    pub keys: Vec<String>,
    pub description: String,
    /// Where it applies, when not everywhere (a picker, the composer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl KeyBinding {
    pub fn new<K: Into<String>>(
        keys: impl IntoIterator<Item = K>,
        description: impl Into<String>,
    ) -> Self {
        KeyBinding {
            keys: keys.into_iter().map(Into::into).collect(),
            description: description.into(),
            ..Default::default()
        }
    }

    /// Where the binding applies.
    pub fn context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
}

/// One `x-environment` entry: an environment variable the CLI reads.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub description: String,
    /// The values it accepts, when it is not free-form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl EnvVar {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        EnvVar {
            name: name.into(),
            description: description.into(),
            ..Default::default()
        }
    }

    /// The values the variable accepts.
    pub fn values<V: Into<String>>(mut self, values: impl IntoIterator<Item = V>) -> Self {
        self.values = values.into_iter().map(Into::into).collect();
        self
    }
}

/// `x-passthrough`: the host hands every argument after this command,
/// unparsed, to another program.
///
/// The command's arguments, options and subcommands, if any, document that
/// program. Code generators never read them; docs render them as usual.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct XPassthrough {
    /// The program, when it is not named like the command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(flatten)]
    pub extensions: Extensions,
}

impl XPassthrough {
    /// A marker handing the arguments to `program`.
    pub fn program(program: impl Into<String>) -> Self {
        XPassthrough {
            program: Some(program.into()),
            ..Default::default()
        }
    }
}
