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
}

impl Node {
    fn new(name: &str) -> Self {
        Node {
            name: name.to_string(),
            children: Vec::new(),
            leaves: Vec::new(),
            used_leaf_names: Default::default(),
            payload: None,
        }
    }
}

/// Two commands claimed the same path. Carries both names so the message can say
/// which, rather than leaving the caller to guess from a `-2` suffix.
#[derive(Debug)]
pub struct Collision {
    pub path: Vec<String>,
    pub name: String,
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut full = self.path.clone();
        full.push(self.name.clone());
        write!(
            f,
            "two operations both map to the command `{}` — rename one with \
             `x-opencli.verb`/`x-opencli.group`, or exclude it with `includePaths`",
            full.join(" ")
        )
    }
}

pub struct CommandTree {
    root: Node,
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
        }
    }

    pub fn insert(&mut self, resource_path: &[String], leaf: Command) -> Result<(), Collision> {
        let mut node = &mut self.root;
        for seg in resource_path {
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
                continue;
            }
            let pos = node.children.iter().position(|c| &c.name == seg);
            let idx = match pos {
                Some(i) => i,
                None => {
                    node.children.push(Node::new(seg));
                    node.children.len() - 1
                }
            };
            node = &mut node.children[idx];
        }

        // Mirror case: a child node already owns this name, so the arriving leaf
        // belongs ON it rather than beside it.
        if let Some(child) = node.children.iter_mut().find(|c| c.name == leaf.name) {
            if child.payload.is_some() {
                return Err(Collision {
                    path: resource_path.to_vec(),
                    name: leaf.name,
                });
            }
            child.payload = Some(leaf);
            return Ok(());
        }

        if node.used_leaf_names.contains(&leaf.name) {
            return Err(Collision {
                path: resource_path.to_vec(),
                name: leaf.name,
            });
        }
        node.used_leaf_names.insert(leaf.name.clone());
        node.leaves.push(leaf);
        Ok(())
    }

    pub fn emit(&self) -> Vec<Command> {
        emit_children(&self.root)
    }
}

fn emit_children(node: &Node) -> Vec<Command> {
    let mut children: Vec<&Node> = node.children.iter().collect();
    children.sort_by(|a, b| locale_compare(&a.name, &b.name));
    let mut out: Vec<Command> = children.iter().map(|c| emit_node(c)).collect();

    let mut leaves = node.leaves.clone();
    leaves.sort_by(|a, b| {
        leaf_rank(&a.name)
            .cmp(&leaf_rank(&b.name))
            .then_with(|| locale_compare(&a.name, &b.name))
    });
    out.extend(leaves);
    out
}

fn emit_node(node: &Node) -> Command {
    let subs = emit_children(node);
    let commands = if subs.is_empty() { None } else { Some(subs) };
    match node.payload.clone() {
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
    }
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
