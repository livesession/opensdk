//! Handler func rendering — port of handler.ts. Assembles the Go handler body
//! (path/query/header/body → runtime.Request) from a leaf's x-openapi binding.

use serde_json::Value;

use crate::golit::Imports;
use crate::model::{build_leaf_model, GoType, LeafModel};
use crate::naming::pascal_case;

const CLI: &str = "github.com/urfave/cli/v3";

fn q(s: &str) -> String {
    serde_json::to_string(s).expect("string serializes")
}

fn read_expr(flag_name: &str, t: GoType) -> String {
    match t {
        GoType::Bool => format!("cmd.Bool({})", q(flag_name)),
        GoType::Int => format!("cmd.Int({})", q(flag_name)),
        GoType::Float => format!("cmd.Float({})", q(flag_name)),
        GoType::Slice => format!("cmd.StringSlice({})", q(flag_name)),
        _ => format!("cmd.String({})", q(flag_name)), // string | json | file
    }
}

/// Split a path into segments, keeping `{param}` groups as separate segments
/// (JS `path.split(/(\{[^}]+\})/).filter(s => s !== '')`).
fn split_path(path: &str) -> Vec<String> {
    let mut segs: Vec<String> = Vec::new();
    let chars: Vec<char> = path.chars().collect();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            // find matching '}' with at least one char inside ([^}]+)
            if let Some(close_rel) = chars[i + 1..].iter().position(|&c| c == '}') {
                if close_rel >= 1 {
                    let close = i + 1 + close_rel;
                    if !lit.is_empty() {
                        segs.push(std::mem::take(&mut lit));
                    }
                    segs.push(chars[i..=close].iter().collect());
                    i = close + 1;
                    continue;
                }
            }
        }
        lit.push(chars[i]);
        i += 1;
    }
    if !lit.is_empty() {
        segs.push(lit);
    }
    segs
}

fn path_expr(model: &LeafModel, imports: &mut Imports) -> String {
    let segs = split_path(&model.path);
    let mut parts: Vec<String> = Vec::new();
    let mut lit = String::new();
    for seg in segs {
        if seg.len() >= 3 && seg.starts_with('{') && seg.ends_with('}') {
            let inner = &seg[1..seg.len() - 1];
            if !lit.is_empty() {
                parts.push(q(&lit));
                lit = String::new();
            }
            let a = model.path_args.iter().find(|p| p.wire_name == inner);
            let var = a.map(|a| a.go_var.as_str()).unwrap_or("\"\"");
            parts.push(format!("url.PathEscape({var})"));
            imports.add(&["net/url"]);
        } else {
            lit.push_str(&seg);
        }
    }
    if !lit.is_empty() {
        parts.push(q(&lit));
    }
    if parts.is_empty() {
        "\"\"".to_string()
    } else {
        parts.join(" + ")
    }
}

pub struct RenderedHandler {
    pub name: String,
    pub code: String,
}

pub fn render_handler(
    path_names: &[String],
    command: &Value,
    module: &str,
    imports: &mut Imports,
    enforce_required: bool,
) -> RenderedHandler {
    let model = build_leaf_model(command);
    let name = format!(
        "handle{}",
        path_names
            .iter()
            .map(|n| pascal_case(n))
            .collect::<String>()
    );
    imports.add(&["context", CLI, &format!("{module}/internal/runtime")]);

    let mut lines: Vec<String> = Vec::new();

    // A runnable parent's required flags are declared non-required so they do
    // not block its subcommands (see flags.rs), so the check belongs here —
    // reached only when the parent itself runs. The message matches urfave's
    // own wording; the difference is invisible to users, which is the point.
    if enforce_required {
        let required: Vec<&str> = model
            .flags
            .iter()
            .filter(|f| f.required)
            .map(|f| f.flag_name.as_str())
            .collect();
        if !required.is_empty() {
            imports.add(&["fmt"]);
            let list = required.iter().map(|n| q(n)).collect::<Vec<_>>().join(", ");
            lines.push(format!("for _, required := range []string{{{list}}} {{"));
            lines.push("\tif !cmd.IsSet(required) {".to_string());
            lines.push("\t\treturn fmt.Errorf(\"Required flag %q not set\", required)".to_string());
            lines.push("\t}".to_string());
            lines.push("}".to_string());
        }
    }

    // Path params (positional args) — only those the path actually uses.
    let pe = path_expr(&model, imports);
    for a in &model.path_args {
        if pe.contains(&format!("url.PathEscape({})", a.go_var)) {
            lines.push(format!("{} := cmd.Args().Get({})", a.go_var, a.idx));
        }
    }
    // A command with TWO bindings picks between them on whether its optional
    // positional was supplied: `get sdk` lists, `get sdk <id>` retrieves. Mirrors
    // the same branch in opencli2rust — the request either backend sends for a
    // given argv is identical, which is what the shared `recorded.json` asserts.
    let alt_model = command
        .get("x-openapi")
        .and_then(|x| x.get("whenArgsPresent"))
        .map(|alt| {
            let mut synthetic = command.clone();
            if let Some(obj) = synthetic.as_object_mut() {
                obj.insert("x-openapi".to_string(), alt.clone());
            }
            build_leaf_model(&synthetic)
        });

    let method_expr = match &alt_model {
        None => {
            lines.push(format!("path := {pe}"));
            q(&model.method)
        }
        Some(alt) => {
            let alt_pe = path_expr(alt, imports);
            // The argument the alternate path uses and the primary does not IS the
            // discriminator — the id that turns a collection request into an item one.
            let mut discriminators: Vec<String> = Vec::new();
            for a in &alt.path_args {
                let esc = format!("url.PathEscape({})", a.go_var);
                if alt_pe.contains(&esc) && !pe.contains(&esc) {
                    lines.push(format!("{} := cmd.Args().Get({})", a.go_var, a.idx));
                    discriminators.push(format!("{} != \"\"", a.go_var));
                }
            }
            lines.push(format!("method, path := {}, {pe}", q(&model.method)));
            if !discriminators.is_empty() {
                lines.push(format!("if {} {{", discriminators.join(" && ")));
                lines.push(format!("\tmethod, path = {}, {alt_pe}", q(&alt.method)));
                lines.push("}".to_string());
            }
            "method".to_string()
        }
    };

    // Query params.
    let query_flags: Vec<&_> = model
        .flags
        .iter()
        .filter(|f| f.location == "query")
        .collect();
    if !query_flags.is_empty() {
        imports.add(&["net/url"]);
        lines.push("query := url.Values{}".to_string());
        for f in &query_flags {
            let read = read_expr(&f.flag_name, f.go_type);
            let val = if f.go_type != GoType::String {
                imports.add(&["fmt"]);
                format!("fmt.Sprint({read})")
            } else {
                read
            };
            lines.push(format!("if cmd.IsSet({}) {{", q(&f.flag_name)));
            lines.push(format!("\tquery.Set({}, {})", q(&f.wire_name), val));
            lines.push("}".to_string());
        }
    }

    // Header / cookie params.
    let header_flags: Vec<&_> = model
        .flags
        .iter()
        .filter(|f| f.location == "header" || f.location == "cookie")
        .collect();
    if !header_flags.is_empty() {
        imports.add(&["net/http"]);
        lines.push("headers := http.Header{}".to_string());
        for f in &header_flags {
            let read = read_expr(&f.flag_name, f.go_type);
            let val = if f.go_type != GoType::String {
                imports.add(&["fmt"]);
                format!("fmt.Sprint({read})")
            } else {
                read
            };
            lines.push(format!("if cmd.IsSet({}) {{", q(&f.flag_name)));
            lines.push(format!("\theaders.Set({}, {})", q(&f.wire_name), val));
            lines.push("}".to_string());
        }
    }

    // Body.
    if model.has_body {
        imports.add(&["encoding/json"]);
        if let (Some("json"), Some(bjo)) =
            (model.body_style.as_deref(), model.body_json_option.as_ref())
        {
            lines.push("var bodyBytes []byte".to_string());
            lines.push(format!("if cmd.IsSet({}) {{", q(bjo)));
            lines.push(format!("\tbodyBytes = []byte(cmd.String({}))", q(bjo)));
            lines.push("}".to_string());
        } else {
            lines.push("body := map[string]any{}".to_string());
            for f in model.flags.iter().filter(|x| x.location == "body") {
                lines.push(format!("if cmd.IsSet({}) {{", q(&f.flag_name)));
                if f.go_type == GoType::Json {
                    lines.push(format!("\traw := cmd.String({})", q(&f.flag_name)));
                    lines.push("\tvar v any".to_string());
                    lines.push(
                        "\tif err := json.Unmarshal([]byte(raw), &v); err != nil {".to_string(),
                    );
                    lines.push("\t\tv = raw".to_string());
                    lines.push("\t}".to_string());
                    lines.push(format!("\tbody[{}] = v", q(&f.wire_name)));
                } else {
                    lines.push(format!(
                        "\tbody[{}] = {}",
                        q(&f.wire_name),
                        read_expr(&f.flag_name, f.go_type)
                    ));
                }
                lines.push("}".to_string());
            }
            lines.push("bodyBytes, err := json.Marshal(body)".to_string());
            lines.push("if err != nil {".to_string());
            lines.push("\treturn err".to_string());
            lines.push("}".to_string());
        }
    }

    // Assemble the request.
    lines.push("req := runtime.Request{".to_string());
    lines.push(format!("\tMethod: {method_expr},"));
    lines.push("\tPath: path,".to_string());
    if !query_flags.is_empty() {
        lines.push("\tQuery: query,".to_string());
    }
    if !header_flags.is_empty() {
        lines.push("\tHeaders: headers,".to_string());
    }
    if model.has_body {
        lines.push("\tBody: bodyBytes,".to_string());
    }
    lines.push("}".to_string());
    lines.push("return runtime.Do(ctx, req)".to_string());

    let body = lines
        .iter()
        .map(|l| format!("\t{l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let code = format!("func {name}(ctx context.Context, cmd *cli.Command) error {{\n{body}\n}}");
    RenderedHandler { name, code }
}
