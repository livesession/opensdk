//! deriveTarget — method+path → command-tree placement + action verb.
//! Port of action.ts. Simpler custom-action rule than opensdk (only the
//! customVerbs membership test, no after-param heuristic).

use serde_json::Value;

use crate::jsrt::{kebab_case, singularize_segment, split_words};
use crate::options::{Grammar, Options, DEFAULT_CUSTOM_ACTION_VERBS};

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

/// Route to the grammar in force. Today's rule set is `derive_target_noun_verb`,
/// moved here UNCHANGED — not edited, not refactored — so the default output
/// cannot drift while a second grammar is added beside it.
pub fn derive_target(
    method: &str,
    path: &str,
    operation: &Value,
    options: &Options,
) -> DerivedTarget {
    match options.grammar.unwrap_or_default() {
        Grammar::NounVerb => derive_target_noun_verb(method, path, operation, options),
        Grammar::VerbNoun => derive_target_verb_noun(method, path, operation, options),
    }
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
    // `x-opencli.verb` are the intended fixes, and both are explicit.
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
    }
}
