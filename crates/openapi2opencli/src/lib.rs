//! OpenAPI 3.x → OpenCLI doc converter — Rust port of `@xyd-js/openapi2opencli`
//! (Stage A of the CLI pipeline, S6+ W7). Emits the OpenCLI command tree plus
//! the `x-openapi` request binding on the root and every leaf command.
//!
//! The JS `openapi2opencli(doc)` takes an ALREADY-dereferenced document; the
//! Rust `from_file`/`from_json_str` read + deref via `xyd_openapi`'s DocCtx
//! (lazy `resolve()` — identity when there are no `$ref`s), so the pure
//! conversion sees resolved schemas exactly as the JS does.

mod action;
mod body;
mod command;
mod jsrt;
mod model;
mod options;
mod parameters;
mod response;
mod schema;
mod security;
mod tree;

use serde_json::{Map, Value};

use command::build_leaf_command;
use jsrt::{js_object_keys, kebab_case, slug};
use model::{Command, Info, Spec, XOpenApiRoot};
use oas_doc::DocCtx;
use security::security_schemes_to_x_openapi;
use tree::CommandTree;

pub use options::{Grammar, Options};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("openapi2opencli: {0}")]
    Io(String),
    /// Two operations claimed the same command path. Deliberately fatal: the
    /// old behaviour renamed the second to `name-2`, which shipped commands
    /// nobody meant to publish and hid the fact that two operations were
    /// fighting over one name.
    #[error("openapi2opencli: {0}")]
    Collision(String),
}

const DEFAULT_HTTP_METHODS: [&str; 5] = ["get", "put", "patch", "post", "delete"];

fn truthy_str(v: Option<&Value>) -> Option<String> {
    v.and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn build_info(doc: &Value, cli_name: &str, version: &str) -> Info {
    let src = doc.get("info");
    let mut info = Info {
        title: cli_name.to_string(),
        version: version.to_string(),
        description: None,
        summary: None,
        contact: None,
        license: None,
    };
    info.description = truthy_str(src.and_then(|i| i.get("description")));
    info.summary = truthy_str(src.and_then(|i| i.get("summary")));

    if let Some(contact) = src.and_then(|i| i.get("contact")) {
        let mut out = Map::new();
        for key in ["name", "url", "email"] {
            if let Some(v) = truthy_str(contact.get(key)) {
                out.insert(key.to_string(), Value::String(v));
            }
        }
        if !out.is_empty() {
            info.contact = Some(out);
        }
    }
    if let Some(license) = src.and_then(|i| i.get("license")) {
        let mut out = Map::new();
        for key in ["name", "identifier", "url"] {
            if let Some(v) = truthy_str(license.get(key)) {
                out.insert(key.to_string(), Value::String(v));
            }
        }
        if !out.is_empty() {
            info.license = Some(out);
        }
    }
    info
}

fn build_x_root(doc: &Value, cli_name: &str, options: &Options) -> Option<XOpenApiRoot> {
    let servers: Vec<String> = doc
        .get("servers")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("url").and_then(|u| u.as_str()))
                .filter(|u| !u.is_empty())
                .map(|u| u.to_string())
                .collect()
        })
        .unwrap_or_default();
    let security = security_schemes_to_x_openapi(doc, cli_name, options.auth_env_var.as_deref());

    let root = XOpenApiRoot {
        servers: if servers.is_empty() {
            None
        } else {
            Some(servers)
        },
        security: if security.is_empty() {
            None
        } else {
            Some(security)
        },
    };
    if root.servers.is_none() && root.security.is_none() {
        None
    } else {
        Some(root)
    }
}

/// Convert an (already-dereferenced or ref-carrying) OpenAPI doc. `ctx`
/// resolves `$ref`s lazily.
fn convert(ctx: &DocCtx, doc: &Value, options: &Options) -> Result<Spec, Error> {
    let title = doc
        .get("info")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .unwrap_or("cli");
    let cli_name = options.cli_name.clone().unwrap_or_else(|| {
        let s = slug(title);
        if s.is_empty() {
            "cli".to_string()
        } else {
            s
        }
    });
    let version = options
        .version
        .clone()
        .or_else(|| {
            doc.get("info")
                .and_then(|i| i.get("version"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .unwrap_or_else(|| "0.0.0".to_string());
    let methods: Vec<String> = options
        .include_methods
        .clone()
        .unwrap_or_else(|| DEFAULT_HTTP_METHODS.iter().map(|m| m.to_string()).collect())
        .into_iter()
        .map(|m| m.to_lowercase())
        .collect();

    let x_openapi = build_x_root(doc, &cli_name, options);

    let mut tree = CommandTree::new();
    if options.grammar.unwrap_or_default() == Grammar::VerbNoun {
        tree = tree.verb_first();
    }
    // Collected first, then paired, then inserted — pairing needs to see both
    // halves of a read pair, which streaming inserts cannot.
    let mut built_leaves: Vec<(String, String, command::BuiltLeaf)> = Vec::new();
    let empty = Map::new();
    let paths = doc
        .get("paths")
        .and_then(|p| p.as_object())
        .unwrap_or(&empty);

    for path in js_object_keys(paths) {
        let path_item = &paths[path];
        if path_item.is_null() {
            continue;
        }
        if let Some(prefixes) = options.include_paths.as_ref() {
            if !prefixes.iter().any(|p| path.starts_with(p.as_str())) {
                continue;
            }
        }
        let path_item_params: Vec<Value> = match path_item.get("parameters") {
            Some(Value::Array(arr)) => arr.clone(),
            _ => Vec::new(),
        };

        for method in &methods {
            let Some(operation) = path_item.get(method.as_str()) else {
                continue;
            };
            if !operation.is_object() {
                continue;
            }
            let built =
                build_leaf_command(ctx, method, path, operation, &path_item_params, options);
            built_leaves.push((method.clone(), path.to_string(), built));
        }
    }

    // Under verb-noun, a collection GET and its item GET are ONE command: the
    // plural name lists, the singular is an alias, and supplying the positional
    // retrieves. That is how kubectl reads (`get pods`, `get pod my-pod`, and
    // `get pods my-pod` all work), and it is what keeps `get sdks` and
    // `get sdk` from being two commands one character apart.
    //
    // It also removes a whole class of collisions rather than patching them: a
    // resource whose plural and singular are the SAME WORD — `apis`, and
    // anything else the inflector leaves alone — no longer produces two
    // commands fighting for one name.
    if options.grammar.unwrap_or_default() == Grammar::VerbNoun {
        pair_reads(&mut built_leaves);
    }

    for (_, _, built) in built_leaves {
        tree.insert(&built.resource_path, built.command)
            .map_err(|c| Error::Collision(c.to_string()))?;
    }

    let mut commands: Vec<Command> = tree.emit();

    // `rootCommand` wraps the finished tree rather than prefixing every insert.
    // Post-processing is the better seam for three reasons: the wrapper can
    // carry a description (nodes built during insertion cannot — `emit_node`
    // fills them from `Default`), the child sort and collision handling have
    // already run so wrapping provably cannot perturb them, and an empty tree
    // stays empty instead of emitting a wrapper around nothing.
    if let Some(root) = options
        .root_command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !commands.is_empty() {
            commands = vec![Command {
                name: kebab_case(root),
                commands: Some(commands),
                ..Default::default()
            }];
        }
    }

    Ok(Spec {
        opencli: "1.0.0".to_string(),
        info: build_info(doc, &cli_name, &version),
        x_openapi,
        commands: if commands.is_empty() {
            None
        } else {
            Some(commands)
        },
    })
}

/// Convert a dereferenced OpenAPI document (as a JSON Value) to an OpenCLI doc.
pub fn openapi2opencli(doc: &Value, options: Option<Options>) -> Result<Spec, Error> {
    let options = options.unwrap_or_default();
    // preprocess materializes $ref-with-siblings merges; DocCtx resolves refs.
    let (processed, stamps) = DocCtx::preprocess(doc);
    let ctx = DocCtx::with_merged_stamps(&processed, &stamps);
    convert(&ctx, &processed, &options)
}

/// Read + deref an OpenAPI spec file, then convert (tier-1 fixtures + napi).
pub fn openapi2opencli_from_file(path: &str, options: Option<Options>) -> Result<Spec, Error> {
    let raw = oas_doc::read_spec(path).map_err(|e| Error::Io(e.to_string()))?;
    openapi2opencli(&raw, options)
}

/// napi transport: a dereferenced-or-raw doc as a JSON string → OpenCLI JSON.
pub fn openapi2opencli_from_json_str(
    doc_json: &str,
    options_json: Option<&str>,
) -> Result<String, Error> {
    let doc: Value = serde_json::from_str(doc_json).map_err(|e| Error::Io(e.to_string()))?;
    let options: Option<Options> = match options_json {
        Some(s) => {
            Some(serde_json::from_str(s).map_err(|e| Error::Io(format!("bad options: {e}")))?)
        }
        None => None,
    };
    let spec = openapi2opencli(&doc, options)?;
    serde_json::to_string(&spec).map_err(|e| Error::Io(e.to_string()))
}

/// Collapse each collection-GET / item-GET pair into ONE command.
///
/// The pair is identified by the STATIC path segments, which are identical for
/// `/sdks` and `/sdks/{id}` — the surviving handle after singularization has
/// already made the two commands' names differ. The collection wins the name
/// (plural, so `get sdks` reads as a list), the item's name becomes an alias
/// (so `get sdk <id>` works), and the item's binding rides along under
/// `whenArgsPresent` together with its positional, made optional.
///
/// Anything that is not exactly one collection + one item is left alone: two
/// item GETs on differently-named params, or a lone collection, have no pair to
/// form and merging them would be inventing behaviour.
fn pair_reads(leaves: &mut Vec<(String, String, command::BuiltLeaf)>) {
    use std::collections::HashMap;

    // Group GET operations by their static segments, remembering whether the
    // path ends in a parameter (the item) or not (the collection).
    let mut groups: HashMap<String, (Option<usize>, Option<usize>)> = HashMap::new();
    for (i, (method, path, _)) in leaves.iter().enumerate() {
        if method.to_lowercase() != "get" {
            continue;
        }
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let statics: Vec<&str> = segs
            .iter()
            .copied()
            .filter(|s| !s.starts_with('{'))
            .collect();
        let ends_in_param = segs.last().map(|s| s.starts_with('{')).unwrap_or(false);
        let key = statics.join("/");
        let slot = groups.entry(key).or_insert((None, None));
        if ends_in_param {
            // Only the FIRST item GET pairs; a second one means the shape is
            // not a plain collection/item pair.
            if slot.1.is_none() {
                slot.1 = Some(i);
            } else {
                slot.1 = None;
            }
        } else if slot.0.is_none() {
            slot.0 = Some(i);
        } else {
            slot.0 = None;
        }
    }

    let mut drop_idx: Vec<usize> = Vec::new();
    for (collection, item) in groups.into_values() {
        let (Some(ci), Some(ii)) = (collection, item) else {
            continue;
        };
        // Both halves must sit at the same place in the tree, or merging them
        // would move one of them.
        if leaves[ci].2.resource_path != leaves[ii].2.resource_path {
            continue;
        }

        let item_leaf = leaves[ii].2.command.clone();
        let collection_cmd = &mut leaves[ci].2.command;

        // The SINGULAR spelling is canonical and the plural becomes the alias —
        // not the other way round, which is the version that does not work.
        //
        // Sub-resources reach the same node through N1, which singularizes a
        // segment followed by `{param}`: `/sdks/{id}/targets` nests under a node
        // named `sdk`. Naming the merged command `sdks` would leave that node a
        // SIBLING of a command aliased `sdk` — a name/alias duplication clap
        // rejects outright. Naming it `sdk` makes it the same node, so the
        // leaf↔node merge folds them into one runnable parent and every
        // spelling resolves: `get sdk`, `get sdks`, `get sdk <id>`,
        // `get sdks <id>`, `get sdk targets <id>`.
        let plural = collection_cmd.name.clone();
        if item_leaf.name != plural {
            collection_cmd.name = item_leaf.name.clone();
            let mut aliases = collection_cmd.aliases.clone().unwrap_or_default();
            if !aliases.contains(&plural) {
                aliases.push(plural);
            }
            collection_cmd.aliases = Some(aliases);
        }

        // The item's positionals, made OPTIONAL — their absence is what selects
        // the list binding at runtime.
        if let Some(args) = item_leaf.arguments.clone() {
            let optional: Vec<model::Argument> = args
                .into_iter()
                .map(|mut a| {
                    a.required = None;
                    a
                })
                .collect();
            let mut merged = collection_cmd.arguments.clone().unwrap_or_default();
            merged.extend(optional);
            collection_cmd.arguments = Some(merged);
        }

        if let Some(item_binding) = item_leaf.x_openapi {
            if let Some(binding) = collection_cmd.x_openapi.as_mut() {
                binding.when_args_present = Some(Box::new(item_binding));
            }
        }

        drop_idx.push(ii);
    }

    drop_idx.sort_unstable();
    for i in drop_idx.into_iter().rev() {
        leaves.remove(i);
    }
}
