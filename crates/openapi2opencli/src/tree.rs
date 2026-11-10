//! CommandTree — port of tree.ts. Static path segments become nested resource
//! nodes; leaves attach to their node and emit sorted (children by
//! localeCompare, leaves by action rank then localeCompare).
//!
//! Two departures from the original, both about names colliding at one level:
//!
//! - A node can now carry a PAYLOAD, i.e. be runnable in its own right while
//!   still having children. The TS had no way to express that, so a leaf and a
//!   child node sharing a name became two sibling commands with the same name —
//!   which clap rejects in debug builds and silently resolves to the first match
//!   in release ones.
//! - Two leaves colliding used to be renamed `name-2` in silence. That is now an
//!   error. It produced commands like `apis-2`, which is not a thing anyone
//!   meant to ship, and the rename hid the real problem: two operations claiming
//!   one name.

use std::collections::HashSet;

use crate::jsrt::locale_compare;
use crate::model::Command;

fn leaf_rank(name: &str) -> i32 {
    match name {
        "list" => 0,
        "create" => 1,
        "retrieve" | "get" => 2,
        "update" | "modify" | "replace" => 3,
        "delete" => 4,
        _ => 100,
    }
}

struct Node {
    name: String,
    children: Vec<Node>,
    leaves: Vec<Command>,
    used_leaf_names: HashSet<String>,
    /// Set when a command occupies this node itself — it is both invocable and a
    /// parent (`get sdks` lists, `get sdk targets` is its child).
    payload: Option<Command>,
    /// A group some `namesFrom`-named command was placed through. Only these
    /// get a generated description, so derived trees emit what they always did.
    named: bool,
}

impl Node {
    fn new(name: &str) -> Self {
        Node {
            name: name.to_string(),
            children: Vec::new(),
            leaves: Vec::new(),
            used_leaf_names: Default::default(),
            payload: None,
            named: false,
        }
    }
}

/// Two commands claimed the same path. Carries both names so the message can say
/// which, rather than leaving the caller to guess from a `-2` suffix.
#[derive(Debug)]
pub struct Collision {
    pub path: Vec<String>,
    pub name: String,
    /// Set when the clash goes through an alias: the sibling it hits and the
    /// spelling the two share.
    pub alias: Option<AliasClash>,
}

/// Two siblings answering to one spelling, at least one of them through an
/// alias (`retrieve`'s automatic `get` beside a command named `get`).
#[derive(Debug)]
pub struct AliasClash {
    pub other: String,
    pub spelling: String,
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut full = self.path.clone();
        full.push(self.name.clone());
        match &self.alias {
            None => write!(
                f,
                "two operations both map to the command `{}` — rename one with \
                 `x-cli.verb`/`x-cli.group`, or exclude it with `includePaths`",
                full.join(" ")
            ),
            Some(clash) => {
                let mut other = self.path.clone();
                other.push(clash.other.clone());
                write!(
                    f,
                    "the commands `{}` and `{}` both answer to `{}` (a name or an alias) — \
                     rename one with `x-cli.verb`/`x-cli.group`, change its aliases with \
                     `x-cli.aliases`, or exclude it with `includePaths`",
                    full.join(" "),
                    other.join(" "),
                    clash.spelling
                )
            }
        }
    }
}

/// The spellings a command answers to: its name, then its aliases.
fn spellings(c: &Command) -> impl Iterator<Item = &str> {
    std::iter::once(c.name.as_str()).chain(c.aliases.iter().flatten().map(String::as_str))
}

/// Every (spelling, owner) at one level: each leaf's name and aliases, each
/// child node's name and its payload's aliases. `skip` leaves out the child
/// node of that name (the node an arriving leaf is about to occupy).
fn claimed<'a>(node: &'a Node, skip: Option<&str>) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for leaf in &node.leaves {
        out.extend(spellings(leaf).map(|s| (s, leaf.name.as_str())));
    }
    for child in node
        .children
        .iter()
        .filter(|c| Some(c.name.as_str()) != skip)
    {
        out.push((child.name.as_str(), child.name.as_str()));
        if let Some(payload) = &child.payload {
            out.extend(
                payload
                    .aliases
                    .iter()
                    .flatten()
                    .map(|a| (a.as_str(), child.name.as_str())),
            );
        }
    }
    out
}

/// The first spelling of `wanted` some OTHER sibling already answers to.
fn alias_clash(node: &Node, skip: Option<&str>, own: &str, wanted: &[&str]) -> Option<AliasClash> {
    let taken = claimed(node, skip);
    wanted.iter().find_map(|w| {
        taken
            .iter()
            .find(|(spelling, owner)| spelling == w && *owner != own)
            .map(|(spelling, owner)| AliasClash {
                other: owner.to_string(),
                spelling: spelling.to_string(),
            })
    })
}

pub struct CommandTree {
    root: Node,
    /// Verb-first placement: rank the top level as verbs when emitting.
    verb_first: bool,
    /// Help text for a named group, by group name (from the document's tags).
    group_descriptions: std::collections::HashMap<String, String>,
}

impl Default for CommandTree {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandTree {
    pub fn new() -> Self {
        CommandTree {
            root: Node::new(""),
            verb_first: false,
            group_descriptions: Default::default(),
        }
    }

    /// Descriptions for the groups `namesFrom` creates, keyed by group name. A
    /// named group without an entry is described as `<name> commands`.
    pub fn with_group_descriptions(
        mut self,
        descriptions: std::collections::HashMap<String, String>,
    ) -> Self {
        self.group_descriptions = descriptions;
        self
    }

    #[cfg(test)]
    pub fn insert(&mut self, resource_path: &[String], leaf: Command) -> Result<(), Collision> {
        self.insert_placed(resource_path, leaf, false)
    }

    /// [`insert`](Self::insert), recording whether the placement came from a
    /// `namesFrom` value — the groups it passes through then get help text.
    pub fn insert_placed(
        &mut self,
        resource_path: &[String],
        leaf: Command,
        named: bool,
    ) -> Result<(), Collision> {
        let mut node = &mut self.root;
        for (depth, seg) in resource_path.iter().enumerate() {
            // A leaf already sitting here under this name is the SAME command
            // the path is descending through — promote it to a node and keep it
            // as that node's payload rather than shadowing it.
            if let Some(i) = node.leaves.iter().position(|l| &l.name == seg) {
                let promoted = node.leaves.remove(i);
                node.used_leaf_names.remove(seg);
                let mut n = Node::new(seg);
                n.payload = Some(promoted);
                node.children.push(n);
                let idx = node.children.len() - 1;
                node = &mut node.children[idx];
                node.named |= named;
                continue;
            }
            let pos = node.children.iter().position(|c| &c.name == seg);
            let idx = match pos {
                Some(i) => i,
                None => {
                    // A new group must not take a spelling a sibling answers to
                    // through an alias: clap rejects the duplicate outright.
                    if let Some(clash) = alias_clash(node, None, seg, &[seg.as_str()]) {
                        return Err(Collision {
                            path: resource_path[..depth].to_vec(),
                            name: seg.clone(),
                            alias: Some(clash),
                        });
                    }
                    node.children.push(Node::new(seg));
                    node.children.len() - 1
                }
            };
            node = &mut node.children[idx];
            node.named |= named;
        }

        let wanted: Vec<&str> = spellings(&leaf).collect();

        // Mirror case: a child node already owns this name, so the arriving leaf
        // belongs ON it rather than beside it.
        if node.children.iter().any(|c| c.name == leaf.name) {
            if let Some(clash) = alias_clash(node, Some(&leaf.name), &leaf.name, &wanted) {
                return Err(Collision {
                    path: resource_path.to_vec(),
                    name: leaf.name.clone(),
                    alias: Some(clash),
                });
            }
            let child = node
                .children
                .iter_mut()
                .find(|c| c.name == leaf.name)
                .expect("checked above");
            if child.payload.is_some() {
                return Err(Collision {
                    path: resource_path.to_vec(),
                    name: leaf.name,
                    alias: None,
                });
            }
            child.payload = Some(leaf);
            return Ok(());
        }

        if node.used_leaf_names.contains(&leaf.name) {
            return Err(Collision {
                path: resource_path.to_vec(),
                name: leaf.name,
                alias: None,
            });
        }
        // Names AND aliases share one namespace per level: a stated `get` beside
        // `retrieve` (whose alias is `get`) is two commands answering to `get`.
        if let Some(clash) = alias_clash(node, None, &leaf.name, &wanted) {
            return Err(Collision {
                path: resource_path.to_vec(),
                name: leaf.name.clone(),
                alias: Some(clash),
            });
        }
        node.used_leaf_names.insert(leaf.name.clone());
        node.leaves.push(leaf);
        Ok(())
    }

    /// Rank the top level as verbs rather than sorting it alphabetically.
    pub fn verb_first(mut self) -> Self {
        self.verb_first = true;
        self
    }

    pub fn emit(&self) -> Vec<Command> {
        emit_children(&self.root, self.verb_first, &self.group_descriptions)
    }
}

/// Under verb-noun the tree's top level is verbs, and sorting them
/// alphabetically puts `delete` second in `--help`. Rank them the way the action
/// leaves already are, so the listing reads read-then-write in both grammars.
///
/// Only consulted when the tree was built verb-first. Under noun-verb the top
/// level is resource nouns, and a resource legitimately named `get` or `delete`
/// would otherwise be silently reordered — so the no-op is structural, not a bet
/// on nobody having such a resource.
fn root_rank(name: &str) -> i32 {
    match name {
        "get" | "list" | "retrieve" => 0,
        "create" => 1,
        "update" | "modify" | "replace" => 2,
        "delete" => 3,
        _ => 100,
    }
}

type Descriptions = std::collections::HashMap<String, String>;

fn emit_children(node: &Node, rank_this_level: bool, descriptions: &Descriptions) -> Vec<Command> {
    let mut children: Vec<&Node> = node.children.iter().collect();
    if rank_this_level {
        children.sort_by(|a, b| {
            root_rank(&a.name)
                .cmp(&root_rank(&b.name))
                .then_with(|| locale_compare(&a.name, &b.name))
        });
    } else {
        children.sort_by(|a, b| locale_compare(&a.name, &b.name));
    }
    let mut out: Vec<Command> = children
        .iter()
        .map(|c| emit_node(c, descriptions))
        .collect();

    let mut leaves = node.leaves.clone();
    leaves.sort_by(|a, b| {
        leaf_rank(&a.name)
            .cmp(&leaf_rank(&b.name))
            .then_with(|| locale_compare(&a.name, &b.name))
    });
    out.extend(leaves);
    out
}

fn emit_node(node: &Node, descriptions: &Descriptions) -> Command {
    // Below the root, ordering is alphabetical in both grammars.
    let subs = emit_children(node, false, descriptions);
    let commands = if subs.is_empty() { None } else { Some(subs) };
    let mut command = match node.payload.clone() {
        // A runnable parent: everything the command carried, plus its children.
        // The node's own name wins — the payload was placed here BY that name.
        Some(payload) => Command {
            name: node.name.clone(),
            commands,
            ..payload
        },
        None => Command {
            name: node.name.clone(),
            commands,
            ..Default::default()
        },
    };
    // A group `namesFrom` created has no operation to take help text from, so
    // `--help` would list it bare: use the matching tag's description, else a
    // generated one.
    if node.named && command.description.is_none() {
        command.description = Some(
            descriptions
                .get(&node.name)
                .cloned()
                .unwrap_or_else(|| format!("{} commands", node.name)),
        );
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(name: &str) -> Command {
        Command {
            name: name.to_string(),
            ..Default::default()
        }
    }

    fn seg(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_leaf_becomes_a_runnable_node_when_a_child_arrives_under_it() {
        let mut t = CommandTree::new();
        t.insert(&seg(&["get"]), cmd("sdks")).unwrap();
        t.insert(&seg(&["get", "sdks"]), cmd("targets")).unwrap();

        let out = t.emit();
        let get = &out[0];
        let sdks = &get.commands.as_ref().unwrap()[0];
        assert_eq!(sdks.name, "sdks");
        assert_eq!(sdks.commands.as_ref().unwrap()[0].name, "targets");
    }

    #[test]
    fn the_same_holds_when_the_child_arrives_first() {
        // Insertion order is document order, so both orders must converge or the
        // emitted tree depends on how the spec happened to be written.
        let mut t = CommandTree::new();
        t.insert(&seg(&["get", "sdks"]), cmd("targets")).unwrap();
        t.insert(&seg(&["get"]), cmd("sdks")).unwrap();

        let out = t.emit();
        let sdks = &out[0].commands.as_ref().unwrap()[0];
        assert_eq!(sdks.name, "sdks");
        assert_eq!(sdks.commands.as_ref().unwrap()[0].name, "targets");
    }

    #[test]
    fn a_promoted_leaf_keeps_its_own_fields() {
        // The payload is the command, not just its name: dropping its aliases or
        // description here is how a runnable parent would lose its binding.
        let mut t = CommandTree::new();
        let mut leaf = cmd("sdks");
        leaf.aliases = Some(vec!["sdk".into()]);
        leaf.description = Some("List SDKs".into());
        t.insert(&seg(&["get"]), leaf).unwrap();
        t.insert(&seg(&["get", "sdks"]), cmd("targets")).unwrap();

        let out = t.emit();
        let sdks = &out[0].commands.as_ref().unwrap()[0];
        assert_eq!(sdks.aliases.as_deref(), Some(&["sdk".to_string()][..]));
        assert_eq!(sdks.description.as_deref(), Some("List SDKs"));
        assert!(sdks.commands.is_some());
    }

    #[test]
    fn two_leaves_claiming_one_name_is_an_error_not_a_silent_rename() {
        let mut t = CommandTree::new();
        t.insert(&seg(&["get"]), cmd("apis")).unwrap();
        let err = t.insert(&seg(&["get"]), cmd("apis")).unwrap_err();
        assert_eq!(err.name, "apis");
        assert!(err.to_string().contains("get apis"), "{err}");
    }

    #[test]
    fn a_second_command_on_an_occupied_node_is_also_an_error() {
        let mut t = CommandTree::new();
        t.insert(&seg(&["get", "sdks"]), cmd("targets")).unwrap();
        t.insert(&seg(&["get"]), cmd("sdks")).unwrap();
        // `sdks` is now the node's payload; a third claim has nowhere to go.
        assert!(t.insert(&seg(&["get"]), cmd("sdks")).is_err());
    }

    fn aliased(name: &str, aliases: &[&str]) -> Command {
        Command {
            aliases: Some(aliases.iter().map(|a| a.to_string()).collect()),
            ..cmd(name)
        }
    }

    #[test]
    fn a_name_equal_to_a_siblings_alias_is_an_error_in_either_order() {
        let mut t = CommandTree::new();
        t.insert(&seg(&["widgets"]), aliased("retrieve", &["get"]))
            .unwrap();
        let err = t.insert(&seg(&["widgets"]), cmd("get")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the commands `widgets get` and `widgets retrieve` both answer to `get` (a name or \
             an alias) — rename one with `x-cli.verb`/`x-cli.group`, change its aliases with \
             `x-cli.aliases`, or exclude it with `includePaths`"
        );

        let mut t = CommandTree::new();
        t.insert(&seg(&["widgets"]), cmd("get")).unwrap();
        let err = t
            .insert(&seg(&["widgets"]), aliased("retrieve", &["get"]))
            .unwrap_err();
        assert_eq!(err.alias.as_ref().unwrap().other, "get");
    }

    #[test]
    fn two_siblings_sharing_an_alias_or_a_group_named_like_an_alias_are_errors() {
        let mut t = CommandTree::new();
        t.insert(&seg(&["w"]), aliased("a", &["x"])).unwrap();
        assert!(t.insert(&seg(&["w"]), aliased("b", &["x"])).is_err());
        // A group created under a spelling a sibling's alias already holds.
        assert!(t.insert(&seg(&["w", "x"]), cmd("leaf")).is_err());
        // A runnable parent's aliases count too.
        let mut t = CommandTree::new();
        t.insert(&seg(&["get"]), aliased("sdk", &["sdks"])).unwrap();
        t.insert(&seg(&["get", "sdk"]), cmd("targets")).unwrap();
        assert!(t.insert(&seg(&["get"]), cmd("sdks")).is_err());
    }

    #[test]
    fn a_commands_own_alias_and_an_unrelated_alias_are_fine() {
        let mut t = CommandTree::new();
        t.insert(&seg(&["w"]), aliased("retrieve", &["get", "retrieve"]))
            .unwrap();
        t.insert(&seg(&["w"]), aliased("list", &["ls"])).unwrap();
        // The mirror case keeps working: a leaf landing on its own group.
        t.insert(&seg(&["w", "lifecycle"]), cmd("archive")).unwrap();
        t.insert(&seg(&["w"]), aliased("lifecycle", &["lc"]))
            .unwrap();
    }

    #[test]
    fn ordinary_trees_are_unaffected() {
        // The noun-verb shape: resource nodes with action leaves, ranked.
        let mut t = CommandTree::new();
        t.insert(&seg(&["sdks"]), cmd("delete")).unwrap();
        t.insert(&seg(&["sdks"]), cmd("list")).unwrap();
        t.insert(&seg(&["sdks"]), cmd("create")).unwrap();

        let out = t.emit();
        let names: Vec<&str> = out[0]
            .commands
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["list", "create", "delete"], "action rank preserved");
    }
}

#[cfg(test)]
mod root_rank_tests {
    use super::*;

    fn cmd(name: &str) -> Command {
        Command {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn verb_first_ranks_the_top_level_read_then_write() {
        let mut t = CommandTree::new().verb_first();
        for verb in ["delete", "update", "get", "create", "publish"] {
            t.insert(&[verb.to_string()], cmd("sdk")).unwrap();
        }
        let emitted = t.emit();
        let names: Vec<&str> = emitted.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["get", "create", "update", "delete", "publish"]);
    }

    #[test]
    fn noun_verb_leaves_the_top_level_alphabetical() {
        // A resource legitimately NAMED `get` must not be hoisted; this is why
        // the rank is gated on the grammar rather than applied at depth 0.
        let mut t = CommandTree::new();
        for res in ["sdks", "get", "apis"] {
            t.insert(&[res.to_string()], cmd("list")).unwrap();
        }
        let emitted = t.emit();
        let names: Vec<&str> = emitted.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["apis", "get", "sdks"]);
    }
}
