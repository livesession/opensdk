//! Converter options — port of types.ts `OpenApi2OpenCliOptions`.

use serde::Deserialize;

pub const DEFAULT_CUSTOM_ACTION_VERBS: [&str; 23] = [
    "cancel",
    "submit",
    "complete",
    "expire",
    "archive",
    "unarchive",
    "restore",
    "validate",
    "verify",
    "refund",
    "capture",
    "void",
    "pause",
    "resume",
    "start",
    "stop",
    "retry",
    "finalize",
    "confirm",
    "approve",
    "reject",
    "publish",
    "unpublish",
];

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct VerbMap {
    pub list_collection: Option<String>,
    pub get_item: Option<String>,
    pub create_collection: Option<String>,
    pub update_item: Option<String>,
    pub delete_item: Option<String>,
}

/// Which order a generated command reads in.
///
/// `NounVerb` is the historical shape and the default: the resource comes
/// first, the action last (`api sdks list`). `VerbNoun` puts the action first
/// (`api get sdks`), the way kubectl and PowerShell do.
#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Grammar {
    #[default]
    NounVerb,
    VerbNoun,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub cli_name: Option<String>,
    pub version: Option<String>,
    pub grouping: Option<String>,
    pub body_strategy: Option<String>,
    pub include_methods: Option<Vec<String>>,
    pub include_headers: Option<bool>,
    pub flag_case: Option<String>,
    pub action_aliases: Option<bool>,
    pub verb_map: Option<VerbMap>,
    pub custom_action_verbs: Option<Vec<String>>,
    pub include_paths: Option<Vec<String>>,
    pub max_body_depth: Option<u32>,
    pub auth_env_var: Option<String>,
    /// Wrap every generated command under one named parent, so `api get sdks`
    /// becomes `api <root> get sdks`.
    ///
    /// For a CLI whose generated surface is only part of what the binary does —
    /// the rest being hand-written commands — this keeps the two from competing
    /// for the top level. Unset (the default) emits the tree unwrapped, exactly
    /// as before.
    ///
    /// Note for the backends: both emit one source file per TOP-LEVEL command,
    /// so a wrapper collapses the whole CLI into a single generated file.
    pub root_command: Option<String>,
    /// Command grammar — `"noun-verb"` (default) or `"verb-noun"`.
    ///
    /// Do NOT feed verb-noun output to `opencli2opensdk`: it would produce SDK
    /// resources named `get`/`create` rather than the API's nouns.
    pub grammar: Option<Grammar>,
    /// Irregular singulars, keyed by the word or the whole kebab segment.
    ///
    /// Only consulted under `verb-noun`, where a resource name is singularized
    /// for single-item commands. The built-in ladder handles regular English and
    /// no-ops on anything else; this is the escape hatch for the rest.
    pub singular_overrides: Option<std::collections::BTreeMap<String, String>>,
}

impl Options {
    /// Field-wise merge with `self` winning: the converter config beats the
    /// spec's root `x-cli` block, which beats the built-in default.
    ///
    /// The precedence is the one `openapi2opensdk` already established for
    /// `x-open-sdk-*`: **specificity beats source**. A value written next to the
    /// thing it describes is more specific than one in a build config, EXCEPT
    /// that an explicit converter option is the operator overriding the spec on
    /// purpose — which is why config sits above the root block and (in the next
    /// slice) below the per-operation one.
    ///
    /// Written out field by field with NO `..base` rest pattern, deliberately:
    /// a rest pattern would silently take the base's value for any field added
    /// later, so a new option would appear to work while ignoring the spec.
    /// Spelled out, adding a field fails to compile until it is handled here.
    pub fn over(self, base: Options) -> Options {
        Options {
            cli_name: self.cli_name.or(base.cli_name),
            version: self.version.or(base.version),
            grouping: self.grouping.or(base.grouping),
            body_strategy: self.body_strategy.or(base.body_strategy),
            include_methods: self.include_methods.or(base.include_methods),
            include_headers: self.include_headers.or(base.include_headers),
            flag_case: self.flag_case.or(base.flag_case),
            action_aliases: self.action_aliases.or(base.action_aliases),
            verb_map: self.verb_map.or(base.verb_map),
            custom_action_verbs: self.custom_action_verbs.or(base.custom_action_verbs),
            include_paths: self.include_paths.or(base.include_paths),
            max_body_depth: self.max_body_depth.or(base.max_body_depth),
            auth_env_var: self.auth_env_var.or(base.auth_env_var),
            root_command: self.root_command.or(base.root_command),
            grammar: self.grammar.or(base.grammar),
            singular_overrides: self.singular_overrides.or(base.singular_overrides),
        }
    }
}

/// Read the document's root `x-cli` block as converter options.
///
/// The sibling of `x-sdk`, which `opensdk xsdk` already embeds in OpenAPI specs:
/// `x-sdk` carries SDK docs, `x-cli` carries CLI-generation hints, both written
/// next to the API they describe.
///
/// `x-cli` also names something else — a root block on an OpenSDK **IR** meaning
/// "this SDK spawns a binary" (`opensdk_cli_common::is_cli_spec`). That is a
/// different document: Stage A builds a typed IR and drops unknown root keys, so
/// an OpenAPI `x-cli` cannot reach it. `openapi2opensdk/tests/x_cli_namespace.rs`
/// is the guard that keeps it that way.
///
/// A malformed block warns and is ignored rather than failing the conversion:
/// this is spec-authored hint data, and a typo in it should not take down a
/// build that has a perfectly good converter config.
pub fn root_options(doc: &serde_json::Value) -> Options {
    let Some(block) = doc.get("x-cli") else {
        return Options::default();
    };
    match serde_json::from_value::<Options>(block.clone()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("warning: ignoring the root `x-cli` block — {e}");
            Options::default()
        }
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_wins_and_unset_fields_fall_through() {
        let base = root_options(&json!({
            "x-cli": { "cliName": "from-spec", "grammar": "verb-noun" }
        }));
        let config = Options {
            cli_name: Some("from-config".into()),
            ..Default::default()
        };
        let merged = config.over(base);
        assert_eq!(merged.cli_name.as_deref(), Some("from-config"));
        assert_eq!(
            merged.grammar,
            Some(Grammar::VerbNoun),
            "spec value survives"
        );
    }

    #[test]
    fn a_malformed_block_is_ignored_not_fatal() {
        let o = root_options(&json!({ "x-cli": { "grammar": "sideways" } }));
        assert!(o.grammar.is_none());
        assert!(o.cli_name.is_none());
    }

    #[test]
    fn a_document_without_the_block_is_unchanged() {
        assert!(root_options(&json!({ "openapi": "3.0.0" }))
            .cli_name
            .is_none());
    }
}
