//! deriveTarget — method+path → command-tree placement + action verb.
//! Port of action.ts. Simpler custom-action rule than opensdk (only the
//! customVerbs membership test, no after-param heuristic).

use serde_json::Value;

use crate::jsrt::{kebab_case, singularize_segment, split_words};
use crate::options::{Grammar, OperationXCli, Options, DEFAULT_CUSTOM_ACTION_VERBS};

struct Segment {
    is_param: bool,
    value: String,
}

fn parse_path(path: &str) -> Vec<Segment> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|seg| {
            if seg.len() >= 2 && seg.starts_with('{') && seg.ends_with('}') {
                Segment {
                    is_param: true,
                    value: seg[1..seg.len() - 1].to_string(),
                }
            } else {
                Segment {
                    is_param: false,
                    value: seg.to_string(),
                }
            }
        })
        .collect()
}

pub struct DerivedTarget {
    pub resource_path: Vec<String>,
    pub action: String,
    pub aliases: Vec<String>,
    pub path_param_names: Vec<String>,
    /// The `namesFrom` value that named this command, when one did. `None` for
    /// every derived name, which is what keeps documents without the option
    /// byte-identical.
    pub names_from: Option<String>,
}

struct Verbs {
    list_collection: String,
    get_item: String,
    create_collection: String,
    update_item: String,
    delete_item: String,
}

fn verbs(options: &Options) -> Verbs {
    let vm = options.verb_map.as_ref();
    let pick = |f: Option<&String>, d: &str| f.cloned().unwrap_or_else(|| d.to_string());
    Verbs {
        list_collection: pick(vm.and_then(|m| m.list_collection.as_ref()), "list"),
        get_item: pick(vm.and_then(|m| m.get_item.as_ref()), "retrieve"),
        create_collection: pick(vm.and_then(|m| m.create_collection.as_ref()), "create"),
        update_item: pick(vm.and_then(|m| m.update_item.as_ref()), "update"),
        delete_item: pick(vm.and_then(|m| m.delete_item.as_ref()), "delete"),
    }
}

fn leading_verb(operation_id: Option<&str>) -> Option<String> {
    split_words(operation_id?).into_iter().next()
}

/// The operation's `namesFrom` value: the extension at the configured dotted
/// path, when it is a non-blank string. `None` when the option is unset.
pub fn names_from_value(operation: &Value, options: &Options) -> Option<String> {
    let path = options.names_from.as_deref().filter(|p| !p.is_empty())?;
    crate::ext::lookup(operation, path)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| v.split('/').any(|seg| !seg.trim().is_empty()))
        .map(str::to_string)
}

/// Place a command by its `namesFrom` value, `<group>/.../<verb>`: every
/// segment but the last is the group path, the last is the verb. Blank
/// segments (a leading or doubled `/`) are skipped. The grammar does not apply:
/// the value states the placement outright.
fn derive_target_from_name(value: &str, options: &Options) -> DerivedTarget {
    let segs: Vec<String> = value
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(kebab_case)
        .collect();
    let (action, group) = segs
        .split_last()
        .map(|(verb, group)| (verb.clone(), group.to_vec()))
        .unwrap_or_default();
    let mut aliases = Vec::new();
    // The same automatic alias a derived `retrieve` gets.
    if action == "retrieve" && options.action_aliases != Some(false) {
        aliases.push("get".to_string());
    }
    DerivedTarget {
        resource_path: group,
        action,
        aliases,
        path_param_names: Vec::new(),
        names_from: Some(value.to_string()),
    }
}

/// `path` without the `pathPrefixStrip` prefix, compared segment by segment
/// (`/v3` strips `/v3/x`, never `/v30/x`). Unchanged when the option is unset
/// or the path does not start with the prefix.
fn strip_path_prefix(path: &str, prefix: Option<&str>) -> String {
    let Some(prefix) = prefix else {
        return path.to_string();
    };
    let want: Vec<&str> = prefix.split('/').filter(|s| !s.is_empty()).collect();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if want.is_empty() || segs.len() < want.len() || segs[..want.len()] != want[..] {
        return path.to_string();
    }
    format!("/{}", segs[want.len()..].join("/"))
}

/// Route to the grammar in force. Today's rule set is `derive_target_noun_verb`,
/// moved here UNCHANGED — not edited, not refactored — so the default output
/// cannot drift while a second grammar is added beside it.
pub fn derive_target(
    method: &str,
    path: &str,
    operation: &Value,
    options: &Options,
    x: &OperationXCli,
) -> DerivedTarget {
    // The operation's own grammar wins over the converter-wide one. Deciding
    // placement per operation is the whole of grammar mixing: one tree can hold
    // both word orders without a second mechanism for saying which is which.
    let grammar = x.grammar.or(options.grammar).unwrap_or_default();
    let mut target = match names_from_value(operation, options) {
        // A name the document states outright beats any derivation.
        Some(value) => derive_target_from_name(&value, options),
        None => {
            let derive_path = strip_path_prefix(path, options.path_prefix_strip.as_deref());
            match grammar {
                Grammar::NounVerb => {
                    derive_target_noun_verb(method, &derive_path, operation, options)
                }
                Grammar::VerbNoun => {
                    derive_target_verb_noun(method, &derive_path, operation, options)
                }
            }
        }
    };
    // Positional arguments always follow the FULL path: a parameter inside a
    // stripped prefix, or under a stated name, is still one the request needs.
    target.path_param_names = parse_path(path)
        .into_iter()
        .filter(|s| s.is_param)
        .map(|s| s.value)
        .collect();

    // Explicit overrides are applied AFTER derivation, so they are stated in
    // terms of the final tree rather than of whichever rule produced it.
    if let Some(group) = x.group.as_deref() {
        target.resource_path = group
            .split([' ', '/'])
            .filter(|s| !s.is_empty())
            .map(kebab_case)
            .collect();
    }
    if let Some(verb) = x.verb.as_deref().filter(|v| !v.is_empty()) {
        target.action = kebab_case(verb);
    }
    if let Some(aliases) = x.aliases.as_ref() {
        target.aliases = aliases.iter().map(|a| kebab_case(a)).collect();
    }
    target
}

/// The grammar actually used for one operation, for callers that need to know
/// after the fact (the read-pairing pass, which only applies to verb-first
/// commands). A command named by `namesFrom` is resource-first whatever the
/// grammar says, so it never pairs.
pub fn effective_grammar(options: &Options, x: &OperationXCli, target: &DerivedTarget) -> Grammar {
    if target.names_from.is_some() {
        return Grammar::NounVerb;
    }
    x.grammar.or(options.grammar).unwrap_or_default()
}

fn derive_target_noun_verb(
    method: &str,
    path: &str,
    operation: &Value,
    options: &Options,
) -> DerivedTarget {
    let v = verbs(options);
    let custom: Vec<String> = options
        .custom_action_verbs
        .clone()
        .unwrap_or_else(|| {
            DEFAULT_CUSTOM_ACTION_VERBS
                .iter()
                .map(|s| s.to_string())
                .collect()
        })
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    let action_aliases = options.action_aliases != Some(false);

    let segments = parse_path(path);
    let static_segs: Vec<&str> = segments
        .iter()
        .filter(|s| !s.is_param)
        .map(|s| s.value.as_str())
        .collect();
    let path_param_names: Vec<String> = segments
        .iter()
        .filter(|s| s.is_param)
        .map(|s| s.value.clone())
        .collect();
    let has_params = !path_param_names.is_empty();
    let last = segments.last();
    let m = method.to_lowercase();
    let operation_id = operation.get("operationId").and_then(|v| v.as_str());

    let resource_segs: Vec<&str>;
    let action: String;
    let mut aliases: Vec<String> = Vec::new();

    if last.map(|l| l.is_param).unwrap_or(false) {
        resource_segs = static_segs.clone();
        action = if m == "get" {
            let a = v.get_item;
            if action_aliases && a == "retrieve" {
                aliases.push("get".to_string());
            }
            a
        } else if m == "put" || m == "patch" {
            v.update_item
        } else if m == "delete" {
            v.delete_item
        } else {
            leading_verb(operation_id)
                .filter(|s| !s.is_empty())
                .unwrap_or(v.create_collection)
        };
    } else {
        let last_static = static_segs.last().copied();
        let is_custom = has_params
            && last_static
                .map(|s| custom.contains(&s.to_lowercase()))
                .unwrap_or(false);
        if is_custom {
            action = kebab_case(last_static.unwrap());
            resource_segs = static_segs[..static_segs.len() - 1].to_vec();
        } else {
            resource_segs = static_segs.clone();
            action = if m == "get" {
                v.list_collection
            } else if m == "post" {
                v.create_collection
            } else if m == "put" || m == "patch" {
                v.update_item
            } else if m == "delete" {
                v.delete_item
            } else {
                leading_verb(operation_id)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(m.clone())
            };
        }
    }

    DerivedTarget {
        resource_path: resource_segs.iter().map(|s| kebab_case(s)).collect(),
        action: kebab_case(&action),
        aliases,
        path_param_names,
        names_from: None,
    }
}

/// Verb-first placement: the action becomes the top-level node and the resource
/// nouns nest beneath it, with the LAST noun as the leaf.
///
/// Two clauses decide each noun's grammatical number, and between them they
/// cover every shape without a special case:
///
///   N1  a static segment immediately followed by `{param}` is SINGULAR — it
///       names one thing, because the next segment identifies which
///       (`/sdks/{id}/targets` -> `get sdk targets <id>`)
///   N2  everything else is verbatim, EXCEPT the terminal segment when the verb
///       is `create`, which is singular — you create one
///       (`POST /sdks` -> `create sdk`, but `DELETE /sdks` stays `delete sdks`
///       because a bulk delete removes many and saying otherwise would lie)
///
/// `list` and `retrieve` deliberately do NOT appear: both are `get`, and the
/// collection-vs-item distinction is carried by the noun's number, the way
/// kubectl does it. Pairing the two operations into one command happens later,
/// in `lib.rs` — this function only decides placement.
fn derive_target_verb_noun(
    method: &str,
    path: &str,
    operation: &Value,
    options: &Options,
) -> DerivedTarget {
    let v = verbs(options);
    let overrides = options.singular_overrides.clone().unwrap_or_default();
    let custom: Vec<String> = options
        .custom_action_verbs
        .clone()
        .unwrap_or_else(|| {
            DEFAULT_CUSTOM_ACTION_VERBS
                .iter()
                .map(|s| s.to_string())
                .collect()
        })
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();

    let segments = parse_path(path);
    let path_param_names: Vec<String> = segments
        .iter()
        .filter(|s| s.is_param)
        .map(|s| s.value.clone())
        .collect();
    let has_params = !path_param_names.is_empty();
    let m = method.to_lowercase();
    let operation_id = operation.get("operationId").and_then(|v| v.as_str());

    // Indices of the static segments, so "is the NEXT url segment a param?" is
    // answerable per noun. `static_segs` alone loses that.
    let statics: Vec<(usize, &str)> = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_param)
        .map(|(i, s)| (i, s.value.as_str()))
        .collect();

    // Promotion: a trailing static segment that names an action becomes the
    // verb. This is EXACTLY the noun-verb rule (`has_params` + explicit
    // membership), so the two grammars promote identically and there is one
    // thing to learn rather than two.
    //
    // A `!looks_plural` widening was tried and dropped. Measured over the real
    // 64-operation spec it was a wash — it rescued `build sdk <id>` and
    // `sync repo-connection <id>`, and in exchange promoted `release-config`
    // and `role`, neither of which is a verb. The tie breaks on WHICH failure
    // is worse: a wrongly promoted noun lands at the top level, the most
    // visible surface in `--help`, whereas an unpromoted verb merely reads
    // awkwardly inside the right namespace. `customActionVerbs` and
    // `x-cli.verb` are the intended fixes, and both are explicit.
    let last_is_static = segments.last().map(|s| !s.is_param).unwrap_or(false);
    let trailing = statics.last().copied();
    let promoted = match trailing {
        Some((_, seg)) if last_is_static => has_params && custom.contains(&seg.to_lowercase()),
        _ => false,
    };

    let (verb, noun_statics): (String, Vec<(usize, &str)>) = if promoted {
        let (_, seg) = trailing.expect("checked");
        (kebab_case(seg), statics[..statics.len() - 1].to_vec())
    } else {
        let verb = match m.as_str() {
            // list and retrieve collapse into ONE read verb, so the default
            // cannot come from `verbs()` — that resolves `get_item` to
            // "retrieve", which is the noun-verb spelling. Read the raw map:
            // an explicit `getItem` wins, then `listCollection`, else "get".
            "get" => options
                .verb_map
                .as_ref()
                .and_then(|m| m.get_item.clone().or_else(|| m.list_collection.clone()))
                .unwrap_or_else(|| "get".to_string()),
            "post" => v.create_collection.clone(),
            "put" | "patch" => v.update_item.clone(),
            "delete" => v.delete_item.clone(),
            _ => leading_verb(operation_id)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| m.clone()),
        };
        (kebab_case(&verb), statics.clone())
    };

    let is_create = verb == kebab_case(&v.create_collection);
    let last_idx = noun_statics.len().saturating_sub(1);
    let nouns: Vec<String> = noun_statics
        .iter()
        .enumerate()
        .map(|(i, (pos, seg))| {
            let next_is_param = segments.get(pos + 1).map(|s| s.is_param).unwrap_or(false);
            let singular = next_is_param || (is_create && i == last_idx);
            if singular {
                singularize_segment(&kebab_case(seg), &overrides)
            } else {
                kebab_case(seg)
            }
        })
        .collect();

    // A path with no static segments left (only params, or promotion consumed
    // the only one) has no noun to be the leaf, so the verb itself is one.
    let (resource_path, action) = match nouns.split_last() {
        Some((leaf, rest)) => {
            let mut p = vec![verb];
            p.extend_from_slice(rest);
            (p, leaf.clone())
        }
        None => (Vec::new(), verb),
    };

    DerivedTarget {
        resource_path,
        action,
        // No alias: `retrieve`'s automatic `get` alias would collide head-on
        // with `get` as a top-level verb node.
        aliases: Vec::new(),
        path_param_names,
        names_from: None,
    }
}

#[cfg(test)]
mod names_from_tests {
    use super::*;
    use serde_json::json;

    fn opts(v: serde_json::Value) -> Options {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_stated_name_places_the_command_and_keeps_the_full_path_params() {
        let o = opts(json!({ "namesFrom": "x-docs.path" }));
        let op = json!({ "x-docs": { "path": "/billing//invoiceItems/retrieve/" } });
        let t = derive_target("get", "/v1/{org}/items/{id}", &op, &o, &Default::default());
        assert_eq!(t.resource_path, ["billing", "invoice-items"]);
        assert_eq!(t.action, "retrieve");
        assert_eq!(t.aliases, ["get"]);
        assert_eq!(t.path_param_names, ["org", "id"]);
        assert_eq!(
            t.names_from.as_deref(),
            Some("/billing//invoiceItems/retrieve/")
        );
    }

    #[test]
    fn a_blank_or_non_string_value_falls_back_to_the_derived_name() {
        let o = opts(json!({ "namesFrom": "x-docs.path" }));
        for op in [
            json!({}),
            json!({ "x-docs": { "path": " / " } }),
            json!({ "x-docs": { "path": 7 } }),
        ] {
            let t = derive_target("get", "/widgets", &op, &o, &Default::default());
            assert_eq!(
                (t.resource_path.as_slice(), t.action.as_str()),
                (&["widgets".to_string()][..], "list")
            );
            assert!(t.names_from.is_none());
        }
    }

    #[test]
    fn action_aliases_false_drops_the_get_alias_on_a_stated_retrieve() {
        let o = opts(json!({ "namesFrom": "x-docs.path", "actionAliases": false }));
        let op = json!({ "x-docs": { "path": "widgets/retrieve" } });
        assert!(
            derive_target("get", "/w/{id}", &op, &o, &Default::default())
                .aliases
                .is_empty()
        );
    }

    #[test]
    fn the_prefix_strip_is_segment_aligned() {
        assert_eq!(strip_path_prefix("/v3/widgets", Some("/v3")), "/widgets");
        assert_eq!(strip_path_prefix("/v3/widgets", Some("v3/")), "/widgets");
        assert_eq!(
            strip_path_prefix("/v30/widgets", Some("/v3")),
            "/v30/widgets"
        );
        assert_eq!(strip_path_prefix("/v3", Some("/v3")), "/");
        assert_eq!(strip_path_prefix("/widgets", Some("/v3")), "/widgets");
        assert_eq!(strip_path_prefix("/widgets", None), "/widgets");
        assert_eq!(strip_path_prefix("/widgets", Some("/")), "/widgets");
    }

    #[test]
    fn the_prefix_strip_applies_to_both_grammars() {
        let o = opts(json!({ "pathPrefixStrip": "/v3", "grammar": "verb-noun" }));
        let t = derive_target(
            "get",
            "/v3/widgets/{id}",
            &json!({}),
            &o,
            &Default::default(),
        );
        assert_eq!(t.resource_path, ["get"]);
        assert_eq!(t.action, "widget");
        assert_eq!(t.path_param_names, ["id"]);
    }
}
