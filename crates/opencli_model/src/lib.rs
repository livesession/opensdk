//! A typed model of [OpenCLI](https://github.com/spectreconsole/open-cli)
//! documents.
//!
//! * [`Document`] and the types under it model the flat OpenCLI draft shape
//!   this toolchain reads and writes. Every object keeps the keys it does not
//!   type in `extensions`, so `x-*` keys round-trip. Reading also accepts the
//!   upstream root-`command` shape ([`lift_root_command`]); writing is always
//!   flat, with keys in canonical order.
//! * Three generic extensions are typed: [`XInteractive`] (`x-interactive`,
//!   what an interactive session accepts), [`EnvVar`] (`x-environment`, the
//!   environment variables a command reads) and [`XPassthrough`]
//!   (`x-passthrough`, a command that hands its arguments to another
//!   program). [`EXTENSIONS_SCHEMA_JSON`] is their JSON Schema, and
//!   [`SCHEMA_JSON`] the base OpenCLI schema; validate a document against
//!   both with the validator of your choice.
//! * [`merge_fragment`] merges a hand-written OpenCLI fragment into a
//!   document (the `opensdk opencli --declare` step).
//!
//! ```
//! use opencli_model::{Command, Document, EnvVar};
//!
//! let mut doc = Document::new("acme", "1.0.0");
//! doc.commands.push(Command::new("ask"));
//! doc.x_environment.push(EnvVar::new("ACME_COLOR", "When to color output.").values(["always", "never"]));
//! let json = doc.to_json();
//! assert_eq!(Document::from_json(&json).unwrap(), doc);
//! ```

mod merge;
mod model;
mod shape;

pub use merge::merge_fragment;

pub use model::{
    Argument, Arity, CliOption, Command, Contact, Conventions, Document, EnvVar, ExitCode,
    Extensions, Info, KeyBinding, License, Metadata, XInteractive, XPassthrough, OPENCLI_VERSION,
};
pub use shape::lift_root_command;

/// The base OpenCLI JSON Schema (draft 2020-12), with the `x-openapi` binding
/// declared. Its objects are open, so extension keys validate as additional
/// properties.
pub const SCHEMA_JSON: &str = include_str!("../schema/opencli-spec.json");

/// The JSON Schema (draft 2020-12) of the three generic extensions:
/// `x-interactive`, `x-environment` and `x-passthrough`. It checks only those
/// keys, walking the command tree to find them, so a document is validated
/// against this schema AND [`SCHEMA_JSON`].
pub const EXTENSIONS_SCHEMA_JSON: &str = include_str!("../schema/opencli-extensions.schema.json");

/// An error reading, merging or mounting a document. The message names what
/// is wrong and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl Error {
    pub(crate) fn msg(message: impl Into<String>) -> Self {
        Error(message.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}
