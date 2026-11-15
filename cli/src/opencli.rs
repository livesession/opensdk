//! `opensdk opencli` — write the OpenCLI document a CLI target generates from.
//!
//! `generate --lang rust-cli` runs the OpenAPI → OpenCLI converter and hands the
//! result straight to the backend, so the intermediate document is never seen.
//! This command stops after the converter: it writes the OpenCLI JSON (with the
//! CLI target's converter options from sdk.json) and merges hand-written
//! OpenCLI fragments into it (`--declare`), so a CLI whose surface is part
//! generated and part hand-written has ONE document describing all of it.
//! `generate --spec <that document>` then drives the backend from it.
//!
//! Each `--declare` file is an OpenCLI-shaped JSON (or YAML) object, merged in
//! the order given by [`opencli_model::merge_fragment`], whose docs state the
//! merge rules.
//!
//! Each `--mount NAME=DOC` then places a component CLI's OpenCLI document at
//! the command NAME (`parent/name` mounts under `parent`), by
//! [`opencli_model::mount`], whose docs state the mount rules; `--mount-drop
//! NAME:OPTION` drops one of the component's options from that mount. A
//! document with mounts is written in the model's canonical key order, and
//! each mount reports one line on stderr.

use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};

use opencli_model::{merge_fragment, mount, Document, MountOptions, MountReport};

use opensdk_config::merge_publish_targets;

use crate::cli_targets::{
    convert_to_opencli, converter_bag_with_defaults, is_cli_target, load_cli_source,
    split_cli_options, CliSource,
};
use crate::config::ResolvedConfig;
use crate::error::{Error, Result};
use crate::paths;

/// Inputs for `opensdk opencli`.
#[derive(Debug, Clone, Default)]
pub struct OpenCliCommandOptions {
    /// The OpenAPI document (yaml/json). An OpenCLI document is accepted too:
    /// the converter is skipped and only the fragments are merged.
    pub spec: String,
    /// The CLI target whose converter options apply (`go-cli` | `rust-cli`).
    /// Unset: the first CLI target declared in the config, else none.
    pub lang: Option<String>,
    /// OpenCLI fragments merged in order.
    pub declare: Vec<String>,
    /// Write here instead of stdout.
    pub output: Option<String>,
    /// Defaults the converter's `cliName` (overridden by the section's own).
    pub sdk_name: Option<String>,
    /// Component documents mounted after the fragments, in order:
    /// `NAME=DOCUMENT`, where NAME is the command (`parent/name` mounts under
    /// `parent`).
    pub mount: Vec<String>,
    /// `NAME:OPTION`: drop the component's OPTION from the NAME mount.
    pub mount_drop: Vec<String>,
}

/// One `--mount`, with the `--mount-drop`s that name it.
struct MountSpec<'a> {
    /// As given (`tools/sync`): how errors and drops name it.
    flag: &'a str,
    under: Vec<String>,
    name: String,
    document: &'a str,
    drops: Vec<String>,
}

/// Pair each `--mount NAME=DOCUMENT` with its `--mount-drop NAME:OPTION`s.
fn mount_specs<'a>(mounts: &'a [String], drops: &'a [String]) -> Result<Vec<MountSpec<'a>>> {
    let mut specs: Vec<MountSpec> = Vec::new();
    for entry in mounts {
        let parsed = entry.split_once('=').and_then(|(flag, document)| {
            let segments: Vec<String> = flag.split('/').map(str::to_string).collect();
            (!document.is_empty() && segments.iter().all(|s| !s.is_empty()))
                .then_some((flag, segments, document))
        });
        let Some((flag, mut segments, document)) = parsed else {
            return Err(Error::msg(format!(
                "--mount {entry}: expected NAME=DOCUMENT (NAME may be parent/name)"
            )));
        };
        if specs.iter().any(|s| s.flag == flag) {
            return Err(Error::msg(format!("--mount {flag} is given twice")));
        }
        let name = segments.pop().expect("split yields one segment at least");
        specs.push(MountSpec {
            flag,
            under: segments,
            name,
            document,
            drops: Vec::new(),
        });
    }
    for entry in drops {
        let Some((flag, option)) = entry
            .split_once(':')
            .filter(|(f, o)| !f.is_empty() && !o.is_empty())
        else {
            return Err(Error::msg(format!(
                "--mount-drop {entry}: expected NAME:OPTION"
            )));
        };
        let Some(spec) = specs.iter_mut().find(|s| s.flag == flag) else {
            return Err(Error::msg(format!(
                "--mount-drop {entry}: there is no --mount {flag}=..."
            )));
        };
        spec.drops.push(option.to_string());
    }
    Ok(specs)
}

/// The CLI target `opencli` reads its converter options from.
fn target_lang(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
) -> Result<Option<String>> {
    if let Some(lang) = opts.lang.as_deref() {
        let lang = lang.to_lowercase();
        if !is_cli_target(Some(&lang)) {
            return Err(Error::msg(format!(
                "--lang must be a CLI target (go-cli, rust-cli), got \"{lang}\""
            )));
        }
        return Ok(Some(lang));
    }
    Ok(config.and_then(|c| {
        c.declared_languages()
            .into_iter()
            .find(|l| is_cli_target(Some(l)))
    }))
}

/// Build the merged OpenCLI document (what `opencli` writes).
pub fn build_opencli(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
    cwd: &Path,
) -> Result<Value> {
    build_opencli_reported(opts, config, cwd).map(|(doc, _)| doc)
}

/// [`build_opencli`], with what each `--mount` did.
pub fn build_opencli_reported(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
    cwd: &Path,
) -> Result<(Value, Vec<MountReport>)> {
    // Parsed first: a malformed flag fails before any document is read.
    let mounts = mount_specs(&opts.mount, &opts.mount_drop)?;
    let lang = target_lang(opts, config)?;
    let lang_label = lang
        .clone()
        .unwrap_or_else(|| "go-cli, rust-cli".to_string());
    let spec = paths::resolve_str(cwd, &opts.spec);

    let mut doc = match load_cli_source(&spec, &lang_label)? {
        CliSource::OpenCli(doc) => doc,
        CliSource::OpenApi(api) => {
            let empty = Map::new();
            let converter = match lang.as_deref() {
                Some(lang) => {
                    let bag = config
                        .and_then(|c| c.emitter_options_for(lang))
                        .unwrap_or(&empty);
                    split_cli_options(lang, bag)?.converter
                }
                None => Map::new(),
            };
            let publish = merge_publish_targets(&[
                config.and_then(|c| c.publish.as_ref()),
                lang.as_deref()
                    .and_then(|l| config.and_then(|c| c.target_for(l)))
                    .and_then(|t| t.publish.as_ref()),
            ]);
            let sdk_name = opts
                .sdk_name
                .clone()
                .or_else(|| config.and_then(|c| c.sdk_name.clone()));
            let converter =
                converter_bag_with_defaults(converter, sdk_name.as_deref(), publish.as_ref());
            convert_to_opencli(&api, converter, &lang_label)?
        }
    };

    for path in &opts.declare {
        let resolved = paths::resolve_str(cwd, path);
        let fragment = oas_doc::read_spec(&resolved)
            .map_err(|e| Error::msg(format!("--declare {path}: {e}")))?;
        merge_fragment(&mut doc, &fragment, path).map_err(|e| Error::msg(e.0))?;
    }
    if mounts.is_empty() {
        return Ok((doc, Vec::new()));
    }

    let mut host = Document::from_value(doc)
        .map_err(|e| Error::msg(format!("--mount: the host document is {e}")))?;
    let mut reports = Vec::new();
    for spec in mounts {
        let resolved = paths::resolve_str(cwd, spec.document);
        let raw = oas_doc::read_spec(&resolved)
            .map_err(|e| Error::msg(format!("--mount {}: {e}", spec.flag)))?;
        let component = Document::from_value(raw)
            .map_err(|e| Error::msg(format!("--mount {}: {}: {e}", spec.flag, spec.document)))?;
        let options = MountOptions {
            under: spec.under,
            drop_options: spec.drops,
            ..Default::default()
        };
        let report =
            mount(&mut host, &spec.name, &component, &options).map_err(|e| Error::msg(e.0))?;
        reports.push(report);
    }
    Ok((host.to_value(), reports))
}

/// `opensdk opencli`: build the document and write it (stdout or `--output`).
pub fn opencli_command(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
    cwd: &Path,
) -> Result<()> {
    let (doc, reports) = build_opencli_reported(opts, config, cwd)?;
    for report in &reports {
        eprintln!("{report}");
    }
    let json = format!(
        "{}\n",
        serde_json::to_string_pretty(&doc).map_err(|e| Error::msg(e.to_string()))?
    );
    match opts.output.as_deref() {
        Some(output) => {
            let path = paths::resolve(cwd, output);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::msg(format!("create {}: {e}", parent.display())))?;
            }
            std::fs::write(&path, &json).map_err(|e| Error::msg(format!("write {output}: {e}")))?;
            println!("Wrote OpenCLI to {output}");
        }
        None => {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(json.as_bytes());
            let _ = stdout.flush();
        }
    }
    Ok(())
}
