//! `src/cli/generate.ts` — the main command (single `--lang` and multi-target).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

use opensdk_config::{merge_publish_targets, PublishTarget};
use opensdk_core::behavior::{deep_merge, normalize_behavior_shorthands};
use opensdk_core::emitter::{GeneratedFile, WriteMode as CoreWriteMode};
use opensdk_framework::{write_project, FileEntry, FileMap, WriteMode, WriteProjectOptions};

use crate::cli_targets::{generate_cli_target, is_cli_target, CliTargetOptions};
use crate::config::ResolvedConfig;
use crate::error::{Error, Result};
use crate::grouping::{converter_options, ConverterInputs};
use crate::paths;
use crate::registry::{get_emitter, resolve_lang};
use crate::write_report::report_write_result;

/// `mergeBehaviorOverrides(...layers)` — deep-merge behavior layers, skipping
/// absent ones; arrays and scalars replace. Each layer's shorthands
/// (`pagination: "none"`) are expanded first, so a shorthand in one layer and
/// the object form in the next MERGE instead of one replacing the other.
///
/// Not in `opensdk_config` (which only ports `config.ts`), so it is folded
/// here over `opensdk_core::behavior::deep_merge` — the same primitive the
/// TypeScript's own `deepMerge` is.
pub fn merge_behavior_overrides(layers: &[Option<&Value>]) -> Value {
    let mut out = Value::Object(Map::new());
    for layer in layers.iter().flatten() {
        let mut layer = (*layer).clone();
        normalize_behavior_shorthands(&mut layer);
        out = deep_merge(&out, &layer);
    }
    out
}

/// Override the IR's package identity (`spec.info`) from a merged publish target.
///
/// The converter already fills `version`/`contact`/`license` from the OpenAPI
/// `info`; a publish block's identity fields win over those. Returns a NEW value
/// (never mutates), so per-language calls in [`generate_targets`] don't leak.
pub fn apply_publish_identity(info: &Value, publish: Option<&PublishTarget>) -> Value {
    let Some(publish) = publish else {
        return info.clone();
    };
    let mut next = match info.as_object() {
        Some(o) => o.clone(),
        // `{ ...info }` on a non-object yields `{}` in JS.
        None => Map::new(),
    };
    let set = |m: &mut Map<String, Value>, k: &str, v: &Option<String>| {
        if let Some(v) = v.as_ref().filter(|s| !s.is_empty()) {
            m.insert(k.to_string(), Value::String(v.clone()));
        }
    };
    set(&mut next, "version", &publish.version);
    set(&mut next, "homepage", &publish.homepage);
    set(&mut next, "repository", &publish.repository);
    if let Some(author) = publish.author.as_ref().filter(|s| !s.is_empty()) {
        let mut contact = next
            .get("contact")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        contact.insert("name".into(), Value::String(author.clone()));
        next.insert("contact".into(), Value::Object(contact));
    }
    if let Some(license) = publish.license.as_ref().filter(|s| !s.is_empty()) {
        let mut lic = next
            .get("license")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        lic.insert("identifier".into(), Value::String(license.clone()));
        next.insert("license".into(), Value::Object(lic));
    }
    Value::Object(next)
}

/// Adapt an emitter's `path -> GeneratedFile` map to the framework's `FileMap`.
fn to_file_map(files: BTreeMap<String, GeneratedFile>) -> FileMap {
    files
        .into_iter()
        .map(|(path, f)| {
            let write_mode = match f.write_mode {
                None | Some(CoreWriteMode::Overwrite) => WriteMode::Overwrite,
                Some(CoreWriteMode::SkipIfExists) => WriteMode::SkipIfExists,
                Some(CoreWriteMode::MergeJson) => WriteMode::MergeJson,
            };
            (
                path,
                FileEntry {
                    content: f.content,
                    write_mode,
                },
            )
        })
        .collect()
}

/// Emit an IR through one language's emitter to disk (or print on dry-run).
// Three of these are write-lifecycle booleans travelling together; the crate's
// existing convention for this shape is the allow, not a params struct.
#[allow(clippy::too_many_arguments)]
fn emit_to_disk(
    ir: &Value,
    lang: &str,
    emitter_options: &Map<String, Value>,
    output: &str,
    dry_run: bool,
    merge: bool,
    format: bool,
    cwd: &Path,
) -> Result<()> {
    let emitter = get_emitter(lang)?; // resolves aliases (typescript -> node, ...)
    let options = Value::Object(emitter_options.clone());
    let files = to_file_map((emitter.generate_files)(ir, &options));
    if dry_run {
        let mut names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        names.sort_unstable();
        for p in names {
            println!("{p}");
        }
        return Ok(());
    }
    let out_dir = paths::resolve(cwd, output);
    // BEFORE write_project — see cli/src/format.rs for why after is wrong.
    let mut files = files;
    if format {
        let changed = crate::format::format_file_map(&mut files)?;
        println!("Formatted {changed} file(s)");
    }
    let result = write_project(
        &files,
        &out_dir,
        &WriteProjectOptions {
            generator: None,
            merge,
        },
    )
    .map_err(|e| Error::msg(format!("write_project: {e}")))?;
    report_write_result(files.len(), output, &result);
    Ok(())
}

/// Options for `opensdk generate --lang <x>` (single target).
#[derive(Debug, Clone, Default)]
pub struct GenerateCommandOptions {
    pub inputs: ConverterInputs,
    pub spec: String,
    pub lang: String,
    pub output: String,
    pub dry_run: bool,
    /// Opt OUT of the emitted self-test suite (`{ tests: false }` on the bag).
    pub no_tests: bool,
    /// Language-specific option bag for the active emitter (from config).
    pub emitter_options: Option<Map<String, Value>>,
    /// Publish identity (global + per-language, pre-merged by the caller).
    pub publish: Option<PublishTarget>,
    /// 3-way merge hand-edits into regenerated files instead of overwriting.
    pub merge: bool,
    /// Run the language formatter over the file map before writing it.
    pub format: bool,
}

/// `opensdk generate --lang <x>` — single target.
pub fn generate_command(opts: &GenerateCommandOptions, cwd: &Path) -> Result<()> {
    // CLI output targets (go-cli/rust-cli) branch off BEFORE the OpenSDK IR:
    // they consume the raw OpenAPI doc via the OpenCLI pipeline. This single
    // seam also covers chain targets — the chain loop calls this same function.
    if is_cli_target(Some(&opts.lang)) {
        return generate_cli_target(
            &CliTargetOptions {
                spec: opts.spec.clone(),
                lang: opts.lang.clone(),
                output: opts.output.clone(),
                sdk_name: opts.inputs.sdk_name.clone(),
                options: opts.emitter_options.clone(),
                publish: opts.publish.clone(),
                mount_rules: opts.inputs.mount_rules.clone(),
                operation_hints: opts.inputs.operation_hints.clone(),
                grouping_file: opts.inputs.grouping.clone(),
                dry_run: opts.dry_run,
                merge: opts.merge,
                format: opts.format,
            },
            cwd,
        );
    }
    // `inputs.sdk` is the caller's merged behavior (global ⊕ the language's
    // layer). The converter applies it to a document; a pre-parsed IR gets it
    // stamped onto its `sdk` block instead, so both sources honour the config.
    let converter = converter_options(&opts.inputs, cwd)?;
    let source = load_source(&opts.spec, &converter)?;
    let mut ir = ir_for_language(&source, &converter, opts.inputs.sdk.as_ref(), None)?;
    let info = apply_publish_identity(
        ir.get("info").unwrap_or(&Value::Null),
        opts.publish.as_ref(),
    );
    set_key(&mut ir, "info", info);

    let mut emitter_options = opts.emitter_options.clone().unwrap_or_default();
    if opts.no_tests {
        emitter_options.insert("tests".into(), Value::Bool(false));
    }
    emit_to_disk(
        &ir,
        &opts.lang,
        &emitter_options,
        &opts.output,
        opts.dry_run,
        opts.merge,
        opts.format,
        cwd,
    )
}

/// Set a key on an IR object, preserving the existing key position.
fn set_key(ir: &mut Value, key: &str, value: Value) {
    if let Some(obj) = ir.as_object_mut() {
        obj.insert(key.to_string(), value);
    }
}

/// Options for `opensdk generate` with no `--lang` (multi target).
#[derive(Debug, Clone, Default)]
pub struct GenerateTargetsOptions {
    /// Run the language formatter over each file map before writing it.
    pub format: bool,
    pub inputs: ConverterInputs,
    pub spec: String,
    pub output: String,
    pub dry_run: bool,
    pub no_tests: bool,
    pub merge: bool,
}

/// `opensdk generate` (no `--lang`) — MULTI target.
///
/// Converts the spec once with the global behavior, then emits every declared
/// language from [`ir_for_language`]: the global conversion for a language
/// without a `behavior` layer, a second conversion with the merged block for one
/// that has a layer (so the keys the CONVERTER reads — `pagination.mode`,
/// `idempotency`, `headersFromExtension` — hold per language, exactly as on
/// `generate --lang`), or the stamped IR when `--spec` was a pre-parsed IR.
pub fn generate_targets(
    opts: &GenerateTargetsOptions,
    config: &ResolvedConfig,
    cwd: &Path,
) -> Result<()> {
    let langs = config.declared_languages();
    if langs.is_empty() {
        return Err(Error::msg(
            "No languages declared in the config. Add a language section (e.g. \"typescript\": \
             { \"output\": \"./sdk/ts\" }) or pass --lang.",
        ));
    }

    // CLI output targets don't consume the OpenSDK IR — generate them first, and
    // only run the converter when SDK languages remain (a CLI-only config must
    // not pay for — or fail on — the IR conversion).
    let sdk_langs: Vec<&String> = langs.iter().filter(|l| !is_cli_target(Some(l))).collect();
    for lang in langs.iter().filter(|l| is_cli_target(Some(l))) {
        let target = config.target_for(lang);
        generate_cli_target(
            &CliTargetOptions {
                spec: opts.spec.clone(),
                lang: lang.clone(),
                output: target
                    .and_then(|t| t.output.clone())
                    .unwrap_or_else(|| paths::join(&opts.output, lang)),
                sdk_name: opts.inputs.sdk_name.clone(),
                options: config.emitter_options_for(lang).cloned(),
                publish: merge_publish_targets(&[
                    config.publish.as_ref(),
                    target.and_then(|t| t.publish.as_ref()),
                ]),
                mount_rules: opts.inputs.mount_rules.clone(),
                operation_hints: opts.inputs.operation_hints.clone(),
                grouping_file: opts.inputs.grouping.clone(),
                dry_run: opts.dry_run,
                merge: target.and_then(|t| t.merge).unwrap_or(opts.merge),
                // Deliberately NOT read off `ResolvedTarget`: that struct's
                // projected shape is frozen by TypeScript-minted goldens, and a
                // per-section `format` would have to be added to it to be read
                // here. The command-level flag covers every target in the run,
                // which is what a chain target's `format` already sets.
                format: opts.format,
            },
            cwd,
        )?;
    }
    if sdk_langs.is_empty() {
        return Ok(());
    }

    let inputs = ConverterInputs {
        sdk: config.sdk.clone(),
        ..opts.inputs.clone()
    };
    let converter = converter_options(&inputs, cwd)?;
    let source = load_source(&opts.spec, &converter)?;
    for lang in sdk_langs {
        let target = config.target_for(lang);
        // A fresh IR per language: its `info` is the converter's, so a
        // per-language publish identity never leaks into the next language.
        let mut ir = ir_for_language(
            &source,
            &converter,
            config.sdk.as_ref(),
            target.and_then(|t| t.behavior.as_ref()),
        )?;
        let publish = merge_publish_targets(&[
            config.publish.as_ref(),
            target.and_then(|t| t.publish.as_ref()),
        ]);
        let info = apply_publish_identity(ir.get("info").unwrap_or(&Value::Null), publish.as_ref());
        set_key(&mut ir, "info", info);

        let output = target
            .and_then(|t| t.output.clone())
            .unwrap_or_else(|| paths::join(&opts.output, lang));
        let mut emitter_options = config
            .emitter_options_for(lang)
            .cloned()
            .unwrap_or_default();
        if opts.no_tests {
            emitter_options.insert("tests".into(), Value::Bool(false));
        }
        emit_to_disk(
            &ir,
            lang,
            &emitter_options,
            &output,
            opts.dry_run,
            target.and_then(|t| t.merge).unwrap_or(opts.merge),
            opts.format,
            cwd,
        )?;
    }
    Ok(())
}

/// What `--spec` resolved to: the IR every emitter consumes and, when the
/// source was an OpenAPI document rather than a pre-parsed IR, that document.
///
/// The document is kept because a per-language `behavior` layer is applied by
/// running the converter AGAIN over it with the merged block
/// ([`ir_for_language`]); a pre-parsed IR has no document, so its layers are
/// stamped onto the IR instead.
#[derive(Debug, Clone)]
pub struct LoadedSource {
    pub ir: Value,
    /// `None` when `--spec` was an OpenSDK IR (`.json` carrying `opensdk`).
    pub doc: Option<Value>,
}

/// Accept either an OpenAPI spec (yaml/json) or an already-parsed OpenSDK IR json.
pub fn load_source(source: &str, options: &Map<String, Value>) -> Result<LoadedSource> {
    if source.ends_with(".json") {
        let raw = std::fs::read_to_string(source)
            .map_err(|e| Error::msg(format!("read {source}: {e}")))?;
        let doc: Value =
            serde_json::from_str(&raw).map_err(|e| Error::msg(format!("parse {source}: {e}")))?;
        if doc.get("opensdk").and_then(Value::as_str).is_some() {
            return Ok(LoadedSource { ir: doc, doc: None });
        }
        let ir = convert(&doc, options)?;
        return Ok(LoadedSource { ir, doc: Some(doc) });
    }
    let doc = oas_doc::read_spec(source).map_err(|e| Error::msg(e.to_string()))?;
    let ir = convert(&doc, options)?;
    Ok(LoadedSource { ir, doc: Some(doc) })
}

/// [`load_source`] for callers that want only the IR.
pub fn load_ir(source: &str, options: &Map<String, Value>) -> Result<Value> {
    load_source(source, options).map(|s| s.ir)
}

/// One language's IR from what `--spec` resolved to.
///
/// Every path applies the same effective behavior — the config's global block
/// with the language's `behavior` layer merged over it, the way `generate
/// --lang` and a chain target already have it — so a per-language layer may set
/// ANY key, the ones the converter reads (`pagination.mode`, `idempotency`,
/// `headersFromExtension`) included:
///
/// * an OpenAPI document and a language layer: the converter runs again with
///   the merged block (a conversion is cheap; the second pass is what makes a
///   per-language `pagination: "none"` or extension header real);
/// * a document and no layer: the global conversion is the language's;
/// * a pre-parsed IR: nothing to convert, the layers are stamped over the IR's
///   own `sdk` block ([`restamp_behavior`]).
pub fn ir_for_language(
    source: &LoadedSource,
    converter: &Map<String, Value>,
    global_behavior: Option<&Value>,
    lang_behavior: Option<&Value>,
) -> Result<Value> {
    match (source.doc.as_ref(), lang_behavior) {
        (Some(doc), Some(_)) => convert(
            doc,
            &converter_options_for_language(converter, lang_behavior),
        ),
        (Some(_), None) => Ok(source.ir.clone()),
        (None, _) => {
            let mut ir = source.ir.clone();
            restamp_behavior(&mut ir, global_behavior, lang_behavior);
            Ok(ir)
        }
    }
}

/// The converter bag for one language: `base` with its `sdkBehavior` replaced by
/// the global block ⊕ the language's layer (through [`merge_behavior_overrides`],
/// so every path merges the same way). `base` as is without a layer.
pub fn converter_options_for_language(
    base: &Map<String, Value>,
    lang_behavior: Option<&Value>,
) -> Map<String, Value> {
    let Some(layer) = lang_behavior else {
        return base.clone();
    };
    let mut out = base.clone();
    out.insert(
        "sdkBehavior".into(),
        merge_behavior_overrides(&[base.get("sdkBehavior"), Some(layer)]),
    );
    out
}

/// Stamp the config's behavior onto a pre-parsed IR's `sdk` block: the global
/// block, then the language's layer, deep-merged over whatever the IR carries
/// (shorthands expanded first). Emitters read `sdk` through `resolve_behavior`
/// (defaults ⊕ `sdk`), so an override-only block and a fully populated one emit
/// the same bytes.
///
/// `headersFromExtension` is resolved against the OpenAPI document, which a
/// pre-parsed IR no longer has: an entry the IR does not already carry a value
/// for (resolved when it was parsed with the same config) stays unsent, and is
/// named on stderr rather than dropped silently. An explicit `null` under
/// `defaultHeaders` is a cancellation, not a missing value.
pub fn restamp_behavior(
    ir: &mut Value,
    global_behavior: Option<&Value>,
    lang_behavior: Option<&Value>,
) {
    let base = ir
        .get("sdk")
        .filter(|s| s.is_object())
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let layers = merge_behavior_overrides(&[global_behavior, lang_behavior]);
    let behavior = deep_merge(&base, &layers);
    let unresolved: Vec<&str> = behavior
        .get("headersFromExtension")
        .and_then(Value::as_object)
        .map(|from_ext| {
            from_ext
                .keys()
                .filter(|header| {
                    behavior
                        .get("defaultHeaders")
                        .and_then(|h| h.get(header.as_str()))
                        .is_none()
                })
                .map(String::as_str)
                .collect()
        })
        .unwrap_or_default();
    if !unresolved.is_empty() {
        eprintln!(
            "Warning: headersFromExtension is resolved against the OpenAPI document, and --spec \
             is a pre-parsed IR — no value for {}. Parse the document with the same config, or \
             generate from the document.",
            unresolved.join(", ")
        );
    }
    set_key(ir, "sdk", behavior);
}

fn convert(doc: &Value, options: &Map<String, Value>) -> Result<Value> {
    let converter_options = serde_json::from_value(Value::Object(options.clone()))
        .map_err(|e| Error::msg(format!("bad converter options: {e}")))?;
    let spec = openapi2opensdk::openapi2opensdk(doc, Some(converter_options))
        .map_err(|e| Error::msg(e.to_string()))?;
    serde_json::to_value(&spec).map_err(|e| Error::msg(format!("serialize IR: {e}")))
}

/// `resolveLanguage` re-export for the command layer (single-target config lookup).
pub fn canonical(lang: &str) -> String {
    resolve_lang(lang)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn publish_identity_is_a_no_op_without_a_publish_block() {
        let info = json!({ "title": "t", "version": "1.0.0" });
        assert_eq!(apply_publish_identity(&info, None), info);
    }

    #[test]
    fn publish_identity_overrides_and_appends_in_typescript_order() {
        let info = json!({ "title": "t", "version": "1.0.0", "contact": { "email": "a@b.c" } });
        let publish = PublishTarget {
            version: Some("2.0.0".into()),
            homepage: Some("https://acme.dev".into()),
            repository: Some("https://github.com/acme/acme".into()),
            author: Some("Acme".into()),
            license: Some("MIT".into()),
            ..Default::default()
        };
        let out = apply_publish_identity(&info, Some(&publish));
        let keys: Vec<&str> = out
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        // Existing keys keep their slot; new ones append in assignment order.
        assert_eq!(
            keys,
            [
                "title",
                "version",
                "contact",
                "homepage",
                "repository",
                "license"
            ]
        );
        assert_eq!(out["version"], json!("2.0.0"));
        // contact merges (the existing email survives).
        assert_eq!(out["contact"], json!({ "email": "a@b.c", "name": "Acme" }));
        assert_eq!(out["license"], json!({ "identifier": "MIT" }));
    }

    #[test]
    fn empty_publish_fields_are_falsy_and_skipped() {
        let info = json!({ "version": "1.0.0" });
        let publish = PublishTarget {
            version: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            apply_publish_identity(&info, Some(&publish))["version"],
            json!("1.0.0")
        );
    }

    #[test]
    fn behavior_overrides_deep_merge_with_later_layers_winning() {
        let global = json!({ "retry": { "maxRetries": 3, "backoffMs": 100 } });
        let per_lang = json!({ "retry": { "maxRetries": 7 } });
        assert_eq!(
            merge_behavior_overrides(&[Some(&global), Some(&per_lang)]),
            json!({ "retry": { "maxRetries": 7, "backoffMs": 100 } })
        );
        // No layers at all → `{}` (which the emitters re-merge over defaults).
        assert_eq!(merge_behavior_overrides(&[None, None]), json!({}));
    }

    #[test]
    fn behavior_overrides_expand_the_pagination_shorthand_in_every_layer() {
        // A shorthand in one layer and the object form in the next must MERGE:
        // unexpanded, the object would replace the string and lose the mode.
        let global = json!({ "pagination": "none" });
        let per_lang = json!({ "pagination": { "autoPageDelayMs": 5 } });
        assert_eq!(
            merge_behavior_overrides(&[Some(&global), Some(&per_lang)]),
            json!({ "pagination": { "mode": "none", "autoPageDelayMs": 5 } })
        );
        assert_eq!(
            merge_behavior_overrides(&[Some(&per_lang), Some(&global)]),
            json!({ "pagination": { "autoPageDelayMs": 5, "mode": "none" } })
        );
    }

    #[test]
    fn the_language_bag_merges_its_layer_over_the_global_sdk_behavior() {
        let mut base = Map::new();
        base.insert("sdkName".into(), json!("Widgetco"));
        base.insert(
            "sdkBehavior".into(),
            json!({ "retry": { "maxRetries": 3 }, "pagination": "auto" }),
        );
        let layer = json!({ "pagination": "none", "retry": { "maxRetries": 7 } });
        let bag = converter_options_for_language(&base, Some(&layer));
        assert_eq!(bag["sdkName"], json!("Widgetco"));
        assert_eq!(bag["sdkBehavior"]["retry"]["maxRetries"], json!(7));
        assert_eq!(bag["sdkBehavior"]["pagination"]["mode"], json!("none"));
        // Without a layer the bag is the global one, untouched.
        assert_eq!(converter_options_for_language(&base, None), base);
        // A layer over a bag with no global block is the layer alone.
        let bare = converter_options_for_language(&Map::new(), Some(&layer));
        assert_eq!(bare["sdkBehavior"]["pagination"]["mode"], json!("none"));
    }

    #[test]
    fn restamping_a_pre_parsed_ir_layers_global_then_language_over_its_sdk_block() {
        let mut ir = json!({
            "opensdk": "1.0.0",
            "sdk": {
                "retry": { "maxRetries": 1, "backoff": { "initialDelayMs": 9 } },
                "defaultHeaders": { "X-Release": "2024-10-01" }
            }
        });
        let global = json!({
            "retry": { "maxRetries": 3 },
            "pagination": "none",
            "defaultHeaders": { "X-Some-Version": "1.0" },
            "headersFromExtension": { "X-Release": "x-release" }
        });
        let lang = json!({ "retry": { "maxRetries": 7 } });
        restamp_behavior(&mut ir, Some(&global), Some(&lang));
        let sdk = &ir["sdk"];
        // Language over global over the IR; untouched IR keys survive.
        assert_eq!(sdk["retry"]["maxRetries"], json!(7));
        assert_eq!(sdk["retry"]["backoff"]["initialDelayMs"], json!(9));
        assert_eq!(sdk["pagination"]["mode"], json!("none"));
        // The IR's already-resolved extension header is kept beside the static one.
        assert_eq!(sdk["defaultHeaders"]["X-Release"], json!("2024-10-01"));
        assert_eq!(sdk["defaultHeaders"]["X-Some-Version"], json!("1.0"));
        // No layers, no `sdk`: an empty block, never a panic.
        let mut bare = json!({ "opensdk": "1.0.0" });
        restamp_behavior(&mut bare, None, None);
        assert_eq!(bare["sdk"], json!({}));
    }

    #[test]
    fn canonical_resolves_aliases() {
        assert_eq!(canonical("typescript"), "node");
    }
}
