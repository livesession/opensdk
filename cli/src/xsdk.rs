//! `src/cli/xsdk.ts` + the `embedXSdk` half of `@xyd-js/opensdk-uniform`.
//!
//! The CI/CD-side spec enricher: read a RAW OpenAPI spec, compute every
//! operation's per-language SDK artifacts (signature, usage sample,
//! request/response type reference) through the emitters' DOCS capabilities,
//! and write the spec back with `x-sdk` extensions embedded (root + per
//! operation). Ship the result as your published OpenAPI and a docs site
//! renders SDK-native docs from it WITHOUT running the generator.
//!
//! Only `embedXSdk` is ported from `opensdk-uniform` — the rest of that package
//! is the DOCS-side reader (`attachSdkExamples` / `attachSdkFromSpec`), which
//! consumes Uniform `Reference`s and belongs to the docs pipeline, not the CLI.
//!
//! The heavy lifting is already Rust: `EmitterFns::docs_map(spec, options)` is
//! the port of `prepareSdk` + `operationDocs`, keyed identically
//! (`"<httpmethod-lowercase> <path>"`).
//!
//! # The docs pass sees what `generate` sees
//!
//! A usage sample is only right when it names the package a reader installs,
//! the class it exports, the credential variable the client reads and the
//! method the SDK actually has. All four come from the same sdk.json `generate`
//! consumes, so `xsdk` takes `--config` (and `--sdk-name` / `--grouping`) and
//! threads it through [`EmbedXSdkOptions`]:
//!
//! * the CONVERTER bag (`sdkName`, `mountRules`, `operationHints`, the global
//!   `behavior`) shapes the IR — the resource tree, the action names, the
//!   `<SDKNAME>_API_KEY` variable, which lists are paginated;
//! * each language's SECTION (`packageName`, `modulePath`, `envVar`, ...) is
//!   the options bag its docs capability receives, exactly as the generator's
//!   emitter receives it;
//! * a language's `behavior` override is re-stamped onto the IR's `sdk` block
//!   before its docs pass, the way the multi-target `generate` does it.
//!
//! The same contract applies to any other caller of `docs_map` (the docs
//! engine's generate mode): convert with the converter bag, then call the
//! language's docs with that language's section.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};

use opensdk_core::emitter::{resolve_language, EmitterFns, OperationDocs};

use crate::config::ResolvedConfig;
use crate::error::{Error, Result};
use crate::generate::{restamp_behavior, LoadedSource};
use crate::grouping::{converter_options, ConverterInputs};

/// One SDK language in the switcher. `id` is the public `x-sdk` key (the coder
/// highlight id — `typescript`/`csharp`, NOT the canonical emitter id).
struct SdkLang {
    id: &'static str,
    emitter: &'static EmitterFns,
}

/// The SDK languages, in switcher order (`SDK_LANGS`).
fn sdk_langs() -> Vec<SdkLang> {
    vec![
        SdkLang {
            id: "go",
            emitter: &opensdk_go::EMITTER,
        },
        SdkLang {
            id: "python",
            emitter: &opensdk_python::EMITTER,
        },
        SdkLang {
            id: "typescript",
            emitter: &opensdk_node::EMITTER,
        },
        SdkLang {
            id: "ruby",
            emitter: &opensdk_ruby::EMITTER,
        },
        SdkLang {
            id: "java",
            emitter: &opensdk_java::EMITTER,
        },
        SdkLang {
            id: "csharp",
            emitter: &opensdk_dotnet::EMITTER,
        },
    ]
}

const XSDK_KEY: &str = "x-sdk";

const HTTP_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// What the docs pass is told about the SDK it documents — the identity and
/// options `generate` would use for the same sdk.json. Everything is optional:
/// the default is the no-config run, whose output is byte-identical to what
/// `xsdk` produced before it took a config.
#[derive(Debug, Clone, Default)]
pub struct EmbedXSdkOptions {
    /// The converter options bag (`sdkName`, `mountRules`, `operationHints`,
    /// `sdkBehavior`) — the same object `generate` hands `openapi2opensdk`.
    pub converter: Map<String, Value>,
    /// Each language's emitter options (its sdk.json section minus
    /// `output`/`behavior`/`publish`), keyed by CANONICAL emitter id (`node`,
    /// `dotnet`, ...). A language without a section runs with no options.
    pub emitter_options: BTreeMap<String, Map<String, Value>>,
    /// Each language's `behavior` override, keyed by canonical emitter id. It
    /// is deep-merged over the converter's resolved `sdk` block before that
    /// language's docs pass.
    pub behaviors: BTreeMap<String, Value>,
}

impl EmbedXSdkOptions {
    /// Assemble the options from the command's converter inputs and the
    /// resolved config: the converter bag through [`converter_options`] (so a
    /// `--grouping` file overrides the config's rules field by field, exactly
    /// as it does for `generate`), the sections and behaviors from the config.
    pub fn from_config(
        inputs: &ConverterInputs,
        config: Option<&ResolvedConfig>,
        cwd: &Path,
    ) -> Result<Self> {
        let mut out = EmbedXSdkOptions {
            converter: converter_options(inputs, cwd)?,
            ..Default::default()
        };
        if let Some(config) = config {
            for (lang, options) in &config.emitter_options {
                out.emitter_options.insert(lang.clone(), options.clone());
            }
            for (lang, target) in &config.targets {
                if let Some(behavior) = target.behavior.as_ref() {
                    out.behaviors.insert(lang.clone(), behavior.clone());
                }
            }
        }
        Ok(out)
    }

    fn options_for(&self, canonical: &str) -> Value {
        self.emitter_options
            .get(canonical)
            .map(|m| Value::Object(m.clone()))
            .unwrap_or(Value::Null)
    }
}

/// The result of embedding: the enriched doc, how many operations got an
/// `x-sdk`, and which languages were embedded.
#[derive(Debug, Clone)]
pub struct EmbedXSdkResult {
    pub doc: Value,
    pub operations: usize,
    pub languages: Vec<String>,
}

/// Embed `x-sdk` artifacts into a RAW (un-dereferenced) OpenAPI document.
pub fn embed_xsdk(
    raw_doc: &Value,
    langs: Option<&[String]>,
    options: &EmbedXSdkOptions,
) -> Result<EmbedXSdkResult> {
    // `openapi2opensdk` needs `$ref` identity, so it runs on the RAW doc.
    let converter_opts = serde_json::from_value(Value::Object(options.converter.clone()))
        .map_err(|e| Error::msg(format!("bad converter options: {e}")))?;
    let ir = openapi2opensdk::openapi2opensdk(raw_doc, Some(converter_opts))
        .map_err(|_| Error::msg("embedXSdk: unsupported OpenAPI document (not 3.x / no paths)"))?;
    let ir = serde_json::to_value(&ir).map_err(|e| Error::msg(e.to_string()))?;

    let selected: Vec<SdkLang> = match langs {
        Some(wanted) => sdk_langs()
            .into_iter()
            .filter(|l| wanted.iter().any(|w| w == l.id))
            .collect(),
        None => sdk_langs(),
    };
    if selected.is_empty() {
        return Err(Error::msg(format!(
            "embedXSdk: no known SDK languages in [{}]",
            langs.map(|l| l.join(", ")).unwrap_or_default()
        )));
    }

    // One docs pass per language over the whole IR (the batch surface), keyed
    // `"<method> <path>"` exactly like the operation loop below looks it up.
    // Each language sees ITS section as the options bag and, when it carries a
    // behavior override, an IR whose `sdk` block has that override stamped on.
    let source = LoadedSource {
        ir: ir.clone(),
        doc: Some(raw_doc.clone()),
    };
    let mut docs: Vec<(&'static str, BTreeMap<String, OperationDocs>)> = Vec::new();
    for lang in &selected {
        let canonical = resolve_language(lang.id);
        let lang_options = options.options_for(&canonical);
        let map = match options.behaviors.get(&canonical) {
            Some(behavior) => {
                let mut ir_for_lang = ir.clone();
                restamp_behavior(&mut ir_for_lang, &source, Some(behavior));
                lang.emitter.docs_map(&ir_for_lang, &lang_options)
            }
            None => lang.emitter.docs_map(&ir, &lang_options),
        };
        if let Some(map) = map {
            docs.push((lang.id, map));
        }
    }

    let mut doc = raw_doc.clone();
    let mut operations = 0usize;

    let path_names: Vec<String> = doc
        .get("paths")
        .and_then(Value::as_object)
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();

    for spec_path in path_names {
        for method in HTTP_METHODS {
            let key = format!("{method} {spec_path}");
            let mut xop = Map::new();
            for (id, map) in &docs {
                let Some(found) = map.get(&key) else { continue };
                let mut entry = Map::new();
                if !found.usage.is_empty() {
                    entry.insert("usage".into(), Value::String(found.usage.clone()));
                }
                let tref = &found.type_reference;
                entry.insert("signature".into(), Value::String(tref.signature.clone()));
                let mut types = Map::new();
                types.insert(
                    "request".into(),
                    serde_json::to_value(&tref.request).map_err(|e| Error::msg(e.to_string()))?,
                );
                types.insert(
                    "response".into(),
                    serde_json::to_value(&tref.response).map_err(|e| Error::msg(e.to_string()))?,
                );
                entry.insert("types".into(), Value::Object(types));
                xop.insert((*id).to_string(), Value::Object(entry));
            }
            if xop.is_empty() {
                continue;
            }
            let Some(op) = doc
                .get_mut("paths")
                .and_then(|p| p.get_mut(&spec_path))
                .and_then(|pi| pi.get_mut(method))
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            op.insert(XSDK_KEY.into(), Value::Object(xop));
            operations += 1;
        }
    }

    let languages: Vec<String> = selected.iter().map(|l| l.id.to_string()).collect();
    if let Some(root) = doc.as_object_mut() {
        root.insert(
            XSDK_KEY.into(),
            serde_json::json!({ "languages": languages.clone() }),
        );
    }
    Ok(EmbedXSdkResult {
        doc,
        operations,
        languages,
    })
}

#[derive(Debug, Clone, Default)]
pub struct XsdkCommandOptions {
    /// OpenAPI spec path (yaml/json).
    pub spec: String,
    /// Write the enriched spec to a file; default stdout. Extension picks format.
    pub output: Option<String>,
    /// Restrict to a subset of SDK language ids (default: all six).
    pub langs: Option<Vec<String>>,
    /// Converter inputs: the command's `--sdk-name` / `--grouping` layered over
    /// the config (`sdkName`, `grouping`, `behavior`).
    pub inputs: ConverterInputs,
    /// The resolved sdk.json, for the per-language sections and behaviors.
    pub config: Option<ResolvedConfig>,
}

/// Read a spec, returning its text and whether it is JSON.
fn read_source(source: &str) -> Result<(String, bool)> {
    if source.starts_with("http://") || source.starts_with("https://") {
        // The TS uses `fetch`; this crate stays HTTP-free (the same call every
        // other ported converter makes its caller pre-fetch).
        return Err(Error::msg(format!(
            "Failed to fetch OpenAPI spec from {source}: remote specs are not supported by the \
             Rust `opensdk xsdk` — download the spec and pass a local path."
        )));
    }
    let content =
        std::fs::read_to_string(source).map_err(|e| Error::msg(format!("read {source}: {e}")))?;
    let json = source.ends_with(".json") || content.trim_start().starts_with('{');
    Ok((content, json))
}

fn serialize(doc: &Value, as_json: bool) -> Result<String> {
    if as_json {
        return Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(doc).map_err(|e| Error::msg(e.to_string()))?
        ));
    }
    // js-yaml `dump(doc, { noRefs: true, lineWidth: -1 })`. serde_yaml never
    // emits anchors/aliases for a `Value`, so `noRefs` is structural here; the
    // exact line breaking is NOT byte-identical to js-yaml (documented
    // divergence — the contract is that the output re-parses to the same doc
    // and carries no `&ref` anchors).
    serde_yaml::to_string(doc).map_err(|e| Error::msg(format!("yaml: {e}")))
}

pub fn xsdk_command(opts: &XsdkCommandOptions, cwd: &Path) -> Result<()> {
    let (content, json) = read_source(&opts.spec)?;
    let doc: Value = if json {
        serde_json::from_str(&content).map_err(|e| Error::msg(format!("json: {e}")))?
    } else {
        oas_doc::parse_spec(&content, &opts.spec).map_err(|e| Error::msg(e.to_string()))?
    };

    let embed = EmbedXSdkOptions::from_config(&opts.inputs, opts.config.as_ref(), cwd)?;
    let result = embed_xsdk(&doc, opts.langs.as_deref(), &embed)?;

    let out_json = match opts.output.as_deref() {
        Some(output) => output.ends_with(".json"),
        None => json,
    };
    let out = serialize(&result.doc, out_json)?;

    match opts.output.as_deref() {
        Some(output) => {
            std::fs::write(output, &out).map_err(|e| Error::msg(format!("write {output}: {e}")))?;
            println!(
                "[xsdk] embedded {} for {} operation(s) → {output}",
                result.languages.join(", "),
                result.operations
            );
        }
        None => {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(out.as_bytes());
            let _ = stdout.flush();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::normalize_sdk_json;
    use serde_json::json;

    #[test]
    fn the_switcher_order_is_the_public_language_list() {
        let ids: Vec<&str> = sdk_langs().iter().map(|l| l.id).collect();
        assert_eq!(
            ids,
            ["go", "python", "typescript", "ruby", "java", "csharp"]
        );
    }

    #[test]
    fn unknown_only_languages_fail_loudly() {
        let doc = serde_json::json!({
            "openapi": "3.0.0",
            "info": { "title": "t", "version": "1" },
            "paths": { "/pets": { "get": { "operationId": "listPets", "responses": {} } } }
        });
        let err = embed_xsdk(
            &doc,
            Some(&["cobol".to_string()]),
            &EmbedXSdkOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.0, "embedXSdk: no known SDK languages in [cobol]");
    }

    #[test]
    fn a_non_openapi3_document_is_rejected() {
        let err = embed_xsdk(
            &serde_json::json!({ "swagger": "2.0" }),
            None,
            &EmbedXSdkOptions::default(),
        )
        .unwrap_err();
        assert_eq!(
            err.0,
            "embedXSdk: unsupported OpenAPI document (not 3.x / no paths)"
        );
    }

    #[test]
    fn remote_specs_are_rejected_with_a_pointer_to_a_local_path() {
        let err = read_source("https://example.com/openapi.json").unwrap_err();
        assert!(
            err.0.contains("Failed to fetch OpenAPI spec from"),
            "{}",
            err.0
        );
    }

    #[test]
    fn options_from_a_config_key_sections_by_canonical_id_and_carry_the_converter_bag() {
        let config = normalize_sdk_json(
            &json!({
                "version": 1,
                "sdkName": "Widgetco",
                "behavior": { "retry": { "maxRetries": 1 } },
                "grouping": { "operationHints": { "GET /gadgets": { "pagination": false } } },
                "typescript": { "packageName": "@acme/widgets", "output": "./ts",
                                "behavior": { "retry": { "maxRetries": 5 } } },
                "csharp": { "namespace": "Acme.Widgets" },
                "go": { "modulePath": "github.com/acme/widgets-go" }
            }),
            "/proj/sdk.json",
        )
        .unwrap();
        let inputs = ConverterInputs {
            sdk_name: config.sdk_name.clone(),
            mount_rules: config.mount_rules.clone(),
            operation_hints: config.operation_hints.clone(),
            grouping: None,
            sdk: config.sdk.clone(),
        };
        let embed =
            EmbedXSdkOptions::from_config(&inputs, Some(&config), Path::new("/proj")).unwrap();
        // The converter bag is the one `generate` would build.
        assert_eq!(embed.converter["sdkName"], json!("Widgetco"));
        assert_eq!(
            embed.converter["operationHints"]["GET /gadgets"]["pagination"],
            json!(false)
        );
        assert_eq!(
            embed.converter["sdkBehavior"]["retry"]["maxRetries"],
            json!(1)
        );
        // Sections are keyed by the EMITTER id, so the `typescript`/`csharp`
        // switcher ids resolve to them through `resolve_language`.
        assert_eq!(
            embed.options_for(&resolve_language("typescript"))["packageName"],
            json!("@acme/widgets")
        );
        assert_eq!(
            embed.options_for(&resolve_language("csharp"))["namespace"],
            json!("Acme.Widgets")
        );
        assert_eq!(
            embed.options_for("go")["modulePath"],
            json!("github.com/acme/widgets-go")
        );
        // A language without a section runs with no options — `Null`, which the
        // emitters read as "no overrides", not an empty object.
        assert_eq!(embed.options_for("python"), Value::Null);
        // Only languages with a `behavior` override get a re-stamp.
        assert_eq!(embed.behaviors.keys().collect::<Vec<_>>(), vec!["node"]);
        // No config at all: an empty bag, no sections, no behaviors.
        let bare = EmbedXSdkOptions::from_config(&ConverterInputs::default(), None, Path::new("/"))
            .unwrap();
        assert!(
            bare.converter.is_empty()
                && bare.emitter_options.is_empty()
                && bare.behaviors.is_empty()
        );
    }
}
