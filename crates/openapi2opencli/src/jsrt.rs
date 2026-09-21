//! JS-runtime semantics helpers — ports of naming.ts + unique.ts + the
//! Object.keys / String() coercions openapi2opencli relies on.

use serde_json::{Map, Value};
use std::collections::HashSet;

/// naming.ts `splitWords`: camelCase + ACRONYM boundaries, then split on
/// `[\s_\-./]+`, lowercased.
pub fn split_words(input: &str) -> Vec<String> {
    // replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    let chars: Vec<char> = input.chars().collect();
    let mut pass1 = String::with_capacity(input.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        pass1.push(c);
        if let Some(&next) = chars.get(i + 1) {
            if (c.is_ascii_lowercase() || c.is_ascii_digit()) && next.is_ascii_uppercase() {
                pass1.push(' ');
            }
        }
    }
    // replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    let c2: Vec<char> = pass1.chars().collect();
    let mut pass2 = String::with_capacity(pass1.len() + 8);
    for (i, &c) in c2.iter().enumerate() {
        pass2.push(c);
        if c.is_ascii_uppercase() {
            if let (Some(&n1), Some(&n2)) = (c2.get(i + 1), c2.get(i + 2)) {
                if n1.is_ascii_uppercase() && n2.is_ascii_lowercase() {
                    pass2.push(' ');
                }
            }
        }
    }
    pass2
        .split(|c: char| c.is_whitespace() || matches!(c, '_' | '-' | '.' | '/'))
        .map(|w| w.trim().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

pub fn kebab_case(input: &str) -> String {
    split_words(input).join("-")
}

pub fn camel_case(input: &str) -> String {
    split_words(input)
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                w.clone()
            } else {
                let mut cs = w.chars();
                match cs.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect()
}

pub fn screaming_snake_case(input: &str) -> String {
    split_words(input).join("_").to_uppercase()
}

/// naming.ts `slug`: join('-') then strip anything outside [a-z0-9-].
pub fn slug(input: &str) -> String {
    split_words(input)
        .join("-")
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect()
}

/// unique.ts `uniqueName`: suffix `-2`, `-3`, … on collision.
pub fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut name = base.to_string();
    let mut i = 2u32;
    while used.contains(&name) {
        name = format!("{base}-{i}");
        i += 1;
    }
    used.insert(name.clone());
    name
}

/// `Object.keys()` ordering: array-index keys ascending first, then the rest
/// in insertion order.
pub fn js_object_keys(map: &Map<String, Value>) -> Vec<&String> {
    let mut numeric: Vec<(&String, u32)> = Vec::new();
    let mut rest: Vec<&String> = Vec::new();
    for k in map.keys() {
        match as_array_index(k) {
            Some(n) => numeric.push((k, n)),
            None => rest.push(k),
        }
    }
    numeric.sort_by_key(|(_, n)| *n);
    numeric.into_iter().map(|(k, _)| k).chain(rest).collect()
}

fn as_array_index(key: &str) -> Option<u32> {
    if key.is_empty() || key.len() > 10 {
        return None;
    }
    if key != "0" && key.starts_with('0') {
        return None;
    }
    let n: u64 = key.parse().ok()?;
    if n < u32::MAX as u64 {
        Some(n as u32)
    } else {
        None
    }
}

/// JS `String(value)` coercion for enum values (getEnum maps `String(v)`).
pub fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        // Objects/arrays: JS String() → "[object Object]" / comma-joined; enum
        // values are scalars in practice, so this branch is inert.
        Value::Array(a) => a.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// JS truthiness for a JSON value.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Some(_) => true,
    }
}

/// `String.prototype.localeCompare` for ASCII command names — the tree sort
/// key. For the kebab-case ASCII names produced here this is byte ordering.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

// ── inflection ───────────────────────────────────────────────────────────────
//
// Copied from `opensdk_node::jsrt` rather than shared. The three SDK copies
// (node/go/java) are byte-parity ports locked to their own oracles, so moving
// them would risk four emitters' goldens for no functional gain; the convention
// is stated in `opencli2opensdk/src/jsrt.rs`: "Small pure functions; the crates
// stay independent."
//
// Two deliberate differences from those copies, both because this output is
// TYPED BY A HUMAN rather than compiled into a symbol name:
//
//   - `CLI_UNCOUNTABLE` guards words that merely END in `s`. `dns -> dn`,
//     `alias -> alia` and `news -> new` are fine as type names nobody reads,
//     and unusable as commands.
//   - slicing is done with `strip_suffix`, not byte indexing after a char-count
//     guard. The SDK copies can panic on a multi-byte final grapheme; that bug
//     is not worth propagating into a fourth copy.

/// Mass nouns — identical to the SDK copies, kept in lockstep on purpose.
const UNCOUNTABLE: &[&str] = &["data", "media", "series"];

/// Singular nouns that END in `s`. Stripping these produces a word that is not
/// English, which matters when the result is something a person has to type.
/// `apis` is here for a specific reason: every singularizer guards `is` to
/// protect `analysis`/`basis`, so without an override `GET /apis` and
/// `GET /apis/{id}` both claim `apis` and one of them loses.
const CLI_UNCOUNTABLE: &[&str] = &[
    "news", "alias", "canvas", "gas", "lens", "bias", "atlas", "bonus", "campus", "census",
    "corpus", "status", "dns", "sms", "aws", "tls", "cors", "ops", "gps", "ai", "css", "js",
];

/// Singularize one lowercase English word.
///
/// `overrides` is consulted first so a spec can name its own irregulars; the
/// built-in ladder handles the regular cases and no-ops on anything it does not
/// recognise, which is why an already-singular noun passes through untouched.
pub fn singularize_with(
    word: &str,
    overrides: &std::collections::BTreeMap<String, String>,
) -> String {
    if let Some(explicit) = overrides.get(word) {
        return explicit.clone();
    }
    if word.chars().count() < 3 || UNCOUNTABLE.contains(&word) || CLI_UNCOUNTABLE.contains(&word) {
        return word.to_string();
    }
    if word.chars().count() > 4 {
        if let Some(stem) = word.strip_suffix("ies") {
            return format!("{stem}y");
        }
        for suffix in ["sses", "xes", "ches", "shes", "uses"] {
            if word.ends_with(suffix) {
                // Drop only the "es": sses->ss, xes->x, ches->ch, uses->us.
                return word[..word.len() - 2].to_string();
            }
        }
    }
    if word.ends_with("ss") || word.ends_with("us") || word.ends_with("is") {
        return word.to_string();
    }
    word.strip_suffix('s').unwrap_or(word).to_string()
}

/// Singularize a kebab-case path SEGMENT by its last word only, so
/// `api-keys -> api-key` and `package-registries -> package-registry` while the
/// qualifier in front is left alone (`docs-projects -> docs-project`).
pub fn singularize_segment(
    segment: &str,
    overrides: &std::collections::BTreeMap<String, String>,
) -> String {
    // A whole-segment override wins over per-word handling, so a spec can fix
    // `sdk-targets` outright without knowing how the split works.
    if let Some(explicit) = overrides.get(segment) {
        return explicit.clone();
    }
    let words = split_words(segment);
    match words.split_last() {
        None => segment.to_string(),
        Some((last, rest)) => {
            let mut out: Vec<String> = rest.to_vec();
            out.push(singularize_with(last, overrides));
            out.join("-")
        }
    }
}

#[cfg(test)]
mod inflect_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn s(w: &str) -> String {
        singularize_with(w, &BTreeMap::new())
    }
    fn seg(w: &str) -> String {
        singularize_segment(w, &BTreeMap::new())
    }

    #[test]
    fn regular_plurals() {
        assert_eq!(s("sdks"), "sdk");
        assert_eq!(s("projects"), "project");
        assert_eq!(s("registries"), "registry");
        assert_eq!(s("boxes"), "box");
        assert_eq!(s("batches"), "batch");
        assert_eq!(s("statuses"), "status");
    }

    #[test]
    fn already_singular_words_are_untouched() {
        // The common case in a real spec: most segments are not plural at all.
        for w in ["usage", "overview", "context", "auth", "me", "login"] {
            assert_eq!(s(w), w, "{w} must pass through");
        }
        // NOT `stats`: it is a real plural of `stat`, so singularizing it is
        // correct here. `GET /overview/stats` still reads `get overview stats`,
        // because the terminal segment is only singularized for `create` — the
        // grammar rule keeps it plural, not the inflector.
        assert_eq!(s("stats"), "stat");
    }

    #[test]
    fn singular_nouns_ending_in_s_are_not_mangled() {
        // Each of these would otherwise become a word that is not English, in a
        // string the user has to type.
        for w in [
            "news", "alias", "dns", "sms", "status", "analysis", "address",
        ] {
            assert_eq!(s(w), w, "{w} must not be stripped");
        }
    }

    #[test]
    fn apis_is_guarded_because_it_would_otherwise_collide() {
        // `apis` ends in `is`, so the generic ladder leaves it alone and
        // `GET /apis` + `GET /apis/{id}` both claim `apis`. On a product whose
        // binary is `api`, the loser shipped as `apis-2`.
        assert_eq!(s("apis"), "apis");
        let mut ov = BTreeMap::new();
        ov.insert("apis".to_string(), "api".to_string());
        assert_eq!(singularize_with("apis", &ov), "api");
    }

    #[test]
    fn segments_singularize_only_their_last_word() {
        assert_eq!(seg("api-keys"), "api-key");
        assert_eq!(seg("package-registries"), "package-registry");
        assert_eq!(seg("repo-connections"), "repo-connection");
        assert_eq!(seg("sdk-targets"), "sdk-target");
        // The qualifier is left alone even though it is itself plural.
        assert_eq!(seg("docs-projects"), "docs-project");
    }

    #[test]
    fn a_whole_segment_override_wins() {
        let mut ov = BTreeMap::new();
        ov.insert("sdk-targets".to_string(), "target".to_string());
        assert_eq!(singularize_segment("sdk-targets", &ov), "target");
    }

    #[test]
    fn multibyte_input_does_not_panic() {
        // The SDK copies index by byte after a char-count guard; this one uses
        // strip_suffix, so a final multi-byte grapheme is safe.
        for w in ["café", "naïve", "日本語", "señor"] {
            let _ = s(w);
        }
    }
}
