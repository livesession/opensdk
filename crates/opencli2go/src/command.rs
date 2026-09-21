//! Command tree + resource-file rendering — port of command.ts.

use serde_json::Value;

use crate::flags::{render_flags, render_flags_relaxed};
use crate::golit::{go_bool, go_file, go_slice, go_str, go_struct, lit, GoVal, Imports};
use crate::handler::render_handler;
use crate::model::build_leaf_model;
use crate::naming::{pascal_case, split_words};

const CLI: &str = "github.com/urfave/cli/v3";

fn render_command(
    command: &Value,
    path_names: &[String],
    module: &str,
    imports: &mut Imports,
    handlers: &mut Vec<String>,
) -> GoVal {
    imports.add(&[CLI]);
    let name = command.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let mut fields: Vec<(String, GoVal)> = vec![("Name".to_string(), go_str(name))];

    if let Some(aliases) = command.get("aliases").and_then(|a| a.as_array()) {
        if !aliases.is_empty() {
            let rendered = aliases
                .iter()
                .filter_map(|a| a.as_str())
                .map(go_str)
                .collect();
            fields.push(("Aliases".to_string(), go_slice("string", rendered)));
        }
    }
    if let Some(desc) = command
        .get("description")
        .and_then(|d| d.as_str())
        .filter(|s| !s.is_empty())
    {
        fields.push(("Usage".to_string(), go_str(desc)));
    }
    if command.get("hidden") == Some(&Value::Bool(true)) {
        fields.push(("Hidden".to_string(), go_bool(true)));
    }

    // "Has children" and "is itself runnable" are INDEPENDENT — see the same
    // restructure in opencli2rust. A node carrying both `commands` and
    // `x-openapi` used to take the children branch and silently lose its
    // `Action`, so the command existed but could never be invoked. urfave/cli v3
    // supports `Action` alongside `Commands`; verified by running a generated
    // tree, not by reading the docs.
    //
    // Field order is unchanged for every single-concern node, which is what
    // keeps the goldens byte-identical: a pure branch still emits Commands in
    // this position, a pure leaf still emits Flags then Action.
    let sub_commands = command.get("commands").and_then(|c| c.as_array());
    let subs = sub_commands.filter(|s| !s.is_empty());

    if command.get("x-openapi").is_some() {
        let model = build_leaf_model(command);
        // A runnable parent (binding AND children) declares its required flags
        // as optional and enforces them in its own Action instead — urfave
        // would otherwise apply them to every subcommand too.
        let runnable_parent = subs.is_some();
        let flags = if runnable_parent {
            render_flags_relaxed(&model.flags)
        } else {
            render_flags(&model.flags)
        };
        if !flags.is_empty() {
            fields.push(("Flags".to_string(), go_slice("cli.Flag", flags)));
        }
        let handler = render_handler(path_names, command, module, imports, runnable_parent);
        fields.push(("Action".to_string(), lit(handler.name)));
        handlers.push(handler.code);
    }

    if let Some(subs) = subs {
        let rendered: Vec<GoVal> = subs
            .iter()
            .map(|sub| {
                let sub_name = sub
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut child_path = path_names.to_vec();
                child_path.push(sub_name);
                render_command(sub, &child_path, module, imports, handlers)
            })
            .collect();
        fields.push(("Commands".to_string(), go_slice("*cli.Command", rendered)));
    }

    go_struct("cli.Command", fields, true)
}

pub struct ResourceFile {
    pub path: String,
    pub content: String,
    pub constructor: String,
}

/// Render one `pkg/cmd/<resource>.go` file for a top-level command + subtree.
pub fn render_resource_file(top_command: &Value, module: &str) -> ResourceFile {
    let mut imports = Imports::new();
    let mut handlers: Vec<String> = Vec::new();
    let top_name = top_command
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let go_struct_val = render_command(
        top_command,
        std::slice::from_ref(&top_name),
        module,
        &mut imports,
        &mut handlers,
    );

    let ctor_name = format!("New{}Command", pascal_case(&top_name));
    let ctor = format!(
        "func {ctor_name}() *cli.Command {{\n\treturn {}\n}}",
        go_struct_val(1)
    );
    let mut decls = vec![ctor];
    decls.extend(handlers);
    let content = go_file("cmd", &imports, &decls);

    let file_base = {
        let joined = split_words(&top_name).join("");
        if joined.is_empty() {
            "command".to_string()
        } else {
            joined
        }
    };
    ResourceFile {
        path: format!("pkg/cmd/{file_base}.go"),
        content,
        constructor: ctor_name,
    }
}
