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
}
