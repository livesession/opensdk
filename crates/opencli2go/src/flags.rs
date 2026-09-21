//! Flag rendering — port of flags.ts.

use crate::golit::{go_bool, go_slice, go_str, go_struct, GoVal};
use crate::model::FlagModel;

pub fn render_flag(f: &FlagModel) -> GoVal {
    render_flag_with(f, false)
}

/// `relax_required` drops `Required: true`.
///
/// urfave enforces a parent's required flags even when a SUBCOMMAND is what
/// runs — `api create sdk target s1` fails demanding the `--api-id` that
/// belongs to `api create sdk`, making every subcommand of a runnable parent
/// unreachable. There is no `subcommand_negates_reqs` equivalent, so the
/// requirement moves into the parent's own Action, which is the only place it
/// was ever meant to apply.
pub fn render_flag_with(f: &FlagModel, relax_required: bool) -> GoVal {
    let type_name = format!("cli.{}", f.go_type.flag_type());
    let mut fields: Vec<(String, GoVal)> = vec![("Name".to_string(), go_str(&f.flag_name))];
    if !f.aliases.is_empty() {
        let aliases = f.aliases.iter().map(|a| go_str(a)).collect();
        fields.push(("Aliases".to_string(), go_slice("string", aliases)));
    }
    if let Some(desc) = &f.description {
        fields.push(("Usage".to_string(), go_str(desc)));
    }
    if f.required && !relax_required {
        fields.push(("Required".to_string(), go_bool(true)));
    }
    if f.hidden {
        fields.push(("Hidden".to_string(), go_bool(true)));
    }
    go_struct(&type_name, fields, true)
}

pub fn render_flags(flags: &[FlagModel]) -> Vec<GoVal> {
    flags.iter().map(render_flag).collect()
}

pub fn render_flags_relaxed(flags: &[FlagModel]) -> Vec<GoVal> {
    flags.iter().map(|f| render_flag_with(f, true)).collect()
}
