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

use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};

use opencli_model::merge_fragment;

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
    Ok(doc)
}

/// `opensdk opencli`: build the document and write it (stdout or `--output`).
pub fn opencli_command(
    opts: &OpenCliCommandOptions,
    config: Option<&ResolvedConfig>,
    cwd: &Path,
) -> Result<()> {
    let doc = build_opencli(opts, config, cwd)?;
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
