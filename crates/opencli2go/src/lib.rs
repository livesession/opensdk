//! OpenCLI → buildable Go CLI project generator — Rust port of
//! `@xyd-js/opencli2go` (S6+ W7). Pure: an OpenCLI doc (`serde_json::Value`)
//! → a virtual file map `{ relativePath: contents }` of Go source, byte-exact
//! with the JS emitter. The `writeProject` fs step stays JS (out of scope).

mod command;
mod flags;
mod golit;
mod handler;
mod maincmd;
mod model;
mod naming;
mod runtime;

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

use command::render_resource_file;
use maincmd::render_main;
use naming::slug;
use runtime::{config_go, runtime_go};

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub module_path: Option<String>,
    pub bin_name: Option<String>,
    pub go_version: Option<String>,
    /// The base URL baked into the generated config (default: the document's
    /// first server). `baseURL` — the spelling the sdk.json CLI sections
    /// document — is accepted beside the camelCase `baseUrl`.
    #[serde(alias = "baseURL")]
    pub base_url: Option<String>,
}

/// A document the generator refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "opencli2go: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// Generate the Go CLI project file map from an OpenCLI document.
///
/// Panics where [`try_opencli2go`] returns an error. Prefer
/// [`try_opencli2go`].
pub fn opencli2go(spec: &Value, options: Option<Options>) -> BTreeMap<String, String> {
    try_opencli2go(spec, options).unwrap_or_else(|e| panic!("{e}"))
}

/// Generate the Go CLI project file map from an OpenCLI document, refusing
/// one this backend cannot generate yet: a command marked `x-passthrough`
/// (one that hands every argument after it, unparsed, to another program).
pub fn try_opencli2go(
    spec: &Value,
    options: Option<Options>,
) -> Result<BTreeMap<String, String>, Error> {
    if let Some(path) = find_passthrough(spec) {
        return Err(Error(format!(
            "{path} is marked `x-passthrough` (it hands every argument after it, unparsed, to \
             another program), which the Go backend does not generate yet; generate this \
             document with the rust-cli target, or drop the marker"
        )));
    }
    let options = options.unwrap_or_default();

    let title = spec
        .get("info")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("cli");
    let bin_name = options.bin_name.clone().unwrap_or_else(|| {
        let s = slug(title);
        if s.is_empty() {
            "cli".to_string()
        } else {
            s
        }
    });
    let module = options
        .module_path
        .clone()
        .unwrap_or_else(|| format!("example.com/{bin_name}"));
    let go_version = options
        .go_version
        .clone()
        .unwrap_or_else(|| "1.22".to_string());
    let base_url = options.base_url.clone().unwrap_or_else(|| {
        spec.get("x-openapi")
            .and_then(|x| x.get("servers"))
            .and_then(|s| s.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    });

    let mut files: BTreeMap<String, String> = BTreeMap::new();
    files.insert(
        "go.mod".to_string(),
        format!("module {module}\n\ngo {go_version}\n"),
    );

    let mut constructors: Vec<String> = Vec::new();
    if let Some(cmds) = spec.get("commands").and_then(|c| c.as_array()) {
        for top in cmds {
            let resource = render_resource_file(top, &module);
            files.insert(resource.path, resource.content);
            constructors.push(resource.constructor);
        }
    }

    files.insert(
        format!("cmd/{bin_name}/main.go"),
        render_main(spec, &bin_name, &module, &constructors),
    );
    files.insert("internal/runtime/runtime.go".to_string(), runtime_go());
    files.insert(
        "internal/runtime/config.go".to_string(),
        config_go(spec, &bin_name, &base_url),
    );

    Ok(files)
}

/// Where the document carries `x-passthrough`, first found: `the document
/// root`, or `command \`a b\``.
fn find_passthrough(spec: &Value) -> Option<String> {
    if spec.get("x-passthrough").is_some() {
        return Some("the document root".to_string());
    }
    fn walk(commands: &[Value], path: &[&str]) -> Option<String> {
        for command in commands {
            let name = command.get("name").and_then(Value::as_str).unwrap_or("");
            let mut here = path.to_vec();
            here.push(name);
            if command.get("x-passthrough").is_some() {
                return Some(format!("command `{}`", here.join(" ")));
            }
            if let Some(found) = command
                .get("commands")
                .and_then(Value::as_array)
                .and_then(|subs| walk(subs, &here))
            {
                return Some(found);
            }
        }
        None
    }
    walk(spec.get("commands")?.as_array()?, &[])
}
