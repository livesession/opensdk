//! The transitive TYPE view behind a per-operation type reference.
//!
//! A [`RenderedTypeReference`] renders ONE level: the request and response
//! fields, each naming the IR type behind it (`refTypeName`) without that
//! type's own fields. This module adds the rest — every IR type reachable from
//! the request and the response, rendered in the language as a lookup table
//! (`nested`, keyed by IR type name), and the OpenAPI schema each IR type came
//! from (`schemaRef`) on every rendered type and field. A reference page can
//! then expand `list` → `has_more`, `next_cursor`, `total` and `data[]` → the
//! item type → its fields, and link a type to its schema page only when one
//! exists: a type the converter hoisted from an inline schema carries no
//! `schemaRef`, whatever name it was given.
//!
//! The walk is language-neutral (it reads the raw IR); the language supplies
//! two renderers through [`NestedRender`] — a MODEL field row and a type
//! string — so the rows match the SDK's own models byte for byte.
//!
//! # Depth and cycles
//!
//! Depth counts named-type hops from the operation: the request body type and
//! the response type are depth 1, the types their fields name are depth 2, and
//! so on. The walk is breadth-first with a visited set, so a type is rendered
//! once, at the shallowest depth it is reached, and a self- or
//! mutually-referencing type terminates. A type first reached deeper than the
//! cap is left out of `nested` (the `refTypeName` that points at it stays). The
//! cap is [`DEFAULT_NESTED_DEPTH`], overridable per docs call with the
//! `nestedDepth` key of the emitter options bag; `0` renders no nested types.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde_json::{json, Value};

use crate::emitter::{
    RenderedNestedType, RenderedTypeExpr, RenderedTypeField, RenderedTypeReference,
};

/// How many named-type hops from the operation `nested` renders by default.
pub const DEFAULT_NESTED_DEPTH: usize = 8;

/// The depth cap for one docs call: the options bag's `nestedDepth` (a
/// non-negative integer) or [`DEFAULT_NESTED_DEPTH`].
pub fn nested_depth(options: &Value) -> usize {
    options
        .get("nestedDepth")
        .and_then(Value::as_u64)
        .map(|d| d as usize)
        .unwrap_or(DEFAULT_NESTED_DEPTH)
}

/// The IR type name behind a TypeRef: a direct `ref`, or the element type of an
/// `array` / the value type of a `map`, however deeply wrapped. `None` for a
/// scalar, a const, `any`, and an unnamed ref.
pub fn ref_type_name(type_ref: Option<&Value>) -> Option<String> {
    let r = type_ref?;
    match r.get("kind").and_then(Value::as_str) {
        Some("ref") => r
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_string),
        Some("array") => ref_type_name(r.get("items")),
        Some("map") => ref_type_name(r.get("values")),
        _ => None,
    }
}

/// The two language renderers the walk needs.
pub struct NestedRender<'a> {
    /// A MODEL field (a raw IR `Field`) as a rendered row — the language's
    /// response-side rendering, which is how its generated models declare it.
    pub field: &'a dyn Fn(&Value) -> RenderedTypeField,
    /// A raw IR TypeRef as the language type string.
    pub lang_type: &'a dyn Fn(Option<&Value>) -> String,
}

/// The TypeRefs a type reference starts from.
#[derive(Default)]
pub struct TypeReferenceRoots<'a> {
    /// `requestBody.type`.
    pub request_body: Option<&'a Value>,
    /// Each query/header param's `type`.
    pub request_params: Vec<&'a Value>,
    /// The response type the language's planner settled on: the item type of a
    /// paginated list, `None` for a binary or empty response.
    pub response: Option<&'a Value>,
}

impl<'a> TypeReferenceRoots<'a> {
    /// The request roots from the raw IR method plus the given response root.
    pub fn from_method(method: &'a Value, response: Option<&'a Value>) -> Self {
        let request_params = ["queryParams", "headerParams"]
            .into_iter()
            .filter_map(|key| method.get(key).and_then(Value::as_array))
            .flatten()
            .filter_map(|p| p.get("type"))
            .collect();
        Self {
            request_body: method.get("requestBody").and_then(|b| b.get("type")),
            request_params,
            response,
        }
    }

    fn all(&self) -> impl Iterator<Item = &'a Value> + '_ {
        self.request_body
            .into_iter()
            .chain(self.request_params.iter().copied())
            .chain(self.response)
    }
}

/// The response root as every language's planner picks it: the paginated item
/// type when the operation is paginated, else `primaryResponse`.
pub fn response_root(method: &Value, paginated: bool) -> Option<&Value> {
    paginated
        .then(|| method.pointer("/pagination/itemType"))
        .flatten()
        .or_else(|| method.get("primaryResponse"))
}

/// Complete a one-level type reference: name the IR type behind the request
/// and the response (`refTypeName`), stamp `schemaRef` on every rendered type
/// and field, and render the reachable types into `nested`.
pub fn finish_type_reference(
    tr: &mut RenderedTypeReference,
    spec: &Value,
    roots: &TypeReferenceRoots<'_>,
    options: &Value,
    render: &NestedRender<'_>,
) {
    let types = symbol_table(spec);
    tr.request.ref_type_name = ref_type_name(roots.request_body);
    tr.request.schema_ref = schema_ref_of(&types, tr.request.ref_type_name.as_deref());
    tr.response.ref_type_name = ref_type_name(roots.response);
    tr.response.schema_ref = schema_ref_of(&types, tr.response.ref_type_name.as_deref());
    stamp_fields(&mut tr.request.fields, &types);
    if let Some(fields) = tr.response.fields.as_mut() {
        stamp_fields(fields, &types);
    }
    tr.nested = render_nested(&types, roots, nested_depth(options), render);
}

type Types<'a> = HashMap<&'a str, &'a Value>;

fn symbol_table(spec: &Value) -> Types<'_> {
    spec.get("types")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("name").and_then(Value::as_str).map(|n| (n, t)))
                .collect()
        })
        .unwrap_or_default()
}

fn schema_ref_of(types: &Types<'_>, name: Option<&str>) -> Option<String> {
    types
        .get(name?)?
        .get("schemaRef")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn stamp_fields(fields: &mut [RenderedTypeField], types: &Types<'_>) {
    for f in fields {
        f.schema_ref = schema_ref_of(types, f.ref_type_name.as_deref());
    }
}

/// Every TypeRef a named type's body points at (struct fields, union variants,
/// an alias target).
fn child_type_refs(named: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    if let Some(fields) = named.get("fields").and_then(Value::as_array) {
        out.extend(fields.iter().filter_map(|f| f.get("type")));
    }
    if let Some(variants) = named.get("variants").and_then(Value::as_array) {
        out.extend(variants.iter());
    }
    out.extend(named.get("of"));
    out
}

fn render_nested(
    types: &Types<'_>,
    roots: &TypeReferenceRoots<'_>,
    max_depth: usize,
    render: &NestedRender<'_>,
) -> BTreeMap<String, RenderedNestedType> {
    let mut out = BTreeMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = roots
        .all()
        .filter_map(|r| ref_type_name(Some(r)))
        .map(|name| (name, 1))
        .collect();
    while let Some((name, depth)) = queue.pop_front() {
        if depth > max_depth || seen.contains(&name) {
            continue;
        }
        // A dangling ref (no such type in the IR) is skipped, not rendered.
        let Some(named) = types.get(name.as_str()) else {
            continue;
        };
        seen.insert(name.clone());
        queue.extend(
            child_type_refs(named)
                .into_iter()
                .filter_map(|r| ref_type_name(Some(r)))
                .map(|child| (child, depth + 1)),
        );
        out.insert(name.clone(), render_named(&name, named, types, render));
    }
    out
}

fn type_expr(
    type_ref: Option<&Value>,
    types: &Types<'_>,
    render: &NestedRender<'_>,
) -> RenderedTypeExpr {
    let ref_type_name = ref_type_name(type_ref);
    RenderedTypeExpr {
        lang_type: (render.lang_type)(type_ref),
        schema_ref: schema_ref_of(types, ref_type_name.as_deref()),
        ref_type_name,
    }
}

fn render_named(
    name: &str,
    named: &Value,
    types: &Types<'_>,
    render: &NestedRender<'_>,
) -> RenderedNestedType {
    let kind = named
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("struct")
        .to_string();
    let mut entry = RenderedNestedType {
        type_name: (render.lang_type)(Some(&json!({ "kind": "ref", "name": name }))),
        kind: kind.clone(),
        schema_ref: schema_ref_of(types, Some(name)),
        description: named
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        fields: None,
        values: None,
        variants: None,
        of: None,
    };
    match kind.as_str() {
        "struct" => {
            let mut fields: Vec<RenderedTypeField> = named
                .get("fields")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().map(|f| (render.field)(f)).collect())
                .unwrap_or_default();
            stamp_fields(&mut fields, types);
            entry.fields = Some(fields);
        }
        "enum" => {
            entry.values = Some(
                named
                    .get("values")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .map(|v| v.get("value").cloned().unwrap_or(Value::Null))
                            .collect()
                    })
                    .unwrap_or_default(),
            );
        }
        "union" => {
            entry.variants = Some(
                named
                    .get("variants")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .map(|v| type_expr(Some(v), types, render))
                            .collect()
                    })
                    .unwrap_or_default(),
            );
        }
        "alias" => entry.of = Some(type_expr(named.get("of"), types, render)),
        _ => {}
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emitter::{RenderedTypeGroup, RenderedTypeResponse};

    /// A list envelope whose paging block is inline (no `schemaRef`), an item
    /// type that names a component which names another, an enum, a union, an
    /// alias, and a self-reference.
    fn spec() -> Value {
        json!({
            "types": [
                { "name": "WidgetList", "kind": "struct", "schemaRef": "#/components/schemas/WidgetList",
                  "fields": [
                    { "name": "data", "type": { "kind": "array", "items": { "kind": "ref", "name": "Widget" } }, "required": true },
                    { "name": "list", "type": { "kind": "ref", "name": "WidgetListList" }, "required": true }
                  ] },
                { "name": "WidgetListList", "kind": "struct",
                  "fields": [
                    { "name": "has_more", "type": { "kind": "scalar", "scalar": "boolean" }, "required": true },
                    { "name": "total", "type": { "kind": "scalar", "scalar": "integer" }, "required": false }
                  ] },
                { "name": "Widget", "kind": "struct", "schemaRef": "#/components/schemas/Widget", "description": "A widget.",
                  "fields": [
                    { "name": "id", "type": { "kind": "scalar", "scalar": "string" }, "required": true },
                    { "name": "part", "type": { "kind": "ref", "name": "Part" }, "required": false },
                    { "name": "status", "type": { "kind": "ref", "name": "WidgetStatus" }, "required": false },
                    { "name": "parent", "type": { "kind": "ref", "name": "Widget" }, "required": false },
                    { "name": "tags", "type": { "kind": "ref", "name": "Tags" }, "required": false },
                    { "name": "shape", "type": { "kind": "ref", "name": "Shape" }, "required": false },
                    { "name": "labels", "type": { "kind": "map", "values": { "kind": "ref", "name": "Label" } }, "required": false }
                  ] },
                { "name": "Part", "kind": "struct", "schemaRef": "#/components/schemas/Part",
                  "fields": [ { "name": "sku", "type": { "kind": "scalar", "scalar": "string" }, "required": true } ] },
                { "name": "WidgetStatus", "kind": "enum", "schemaRef": "#/components/schemas/WidgetStatus", "base": "string",
                  "values": [ { "value": "draft" }, { "value": "active" } ] },
                { "name": "Tags", "kind": "alias", "schemaRef": "#/components/schemas/Tags",
                  "of": { "kind": "array", "items": { "kind": "scalar", "scalar": "string" } } },
                { "name": "Shape", "kind": "union", "schemaRef": "#/components/schemas/Shape",
                  "variants": [ { "kind": "ref", "name": "Part" }, { "kind": "scalar", "scalar": "string" } ] },
                { "name": "Label", "kind": "struct", "schemaRef": "#/components/schemas/Label",
                  "fields": [ { "name": "text", "type": { "kind": "scalar", "scalar": "string" }, "required": true } ] }
            ]
        })
    }

    fn method() -> Value {
        json!({
            "queryParams": [ { "name": "status", "type": { "kind": "ref", "name": "WidgetStatus" }, "required": false } ],
            "primaryResponse": { "kind": "ref", "name": "WidgetList" }
        })
    }

    /// A stand-in language: `name: <kind>` rows and `T<Name>` / `[T]` types.
    fn lang_type(t: Option<&Value>) -> String {
        match t.and_then(|r| r.get("kind")).and_then(Value::as_str) {
            Some("ref") => format!("T{}", t.unwrap()["name"].as_str().unwrap()),
            Some("array") => format!("[{}]", lang_type(t.unwrap().get("items"))),
            Some("map") => format!("{{{}}}", lang_type(t.unwrap().get("values"))),
            Some("scalar") => t.unwrap()["scalar"].as_str().unwrap().to_string(),
            _ => "any".to_string(),
        }
    }

    fn field(f: &Value) -> RenderedTypeField {
        RenderedTypeField {
            name: f["name"].as_str().unwrap().to_string(),
            lang_type: lang_type(f.get("type")),
            required: f["required"].as_bool().unwrap_or(false),
            description: None,
            deprecated: None,
            ref_type_name: ref_type_name(f.get("type")),
            schema_ref: None,
        }
    }

    fn one_level() -> RenderedTypeReference {
        RenderedTypeReference {
            signature: "client.widgets.list(status?)".to_string(),
            request: RenderedTypeGroup {
                type_name: None,
                arg_name: None,
                ref_type_name: None,
                schema_ref: None,
                fields: vec![field(&method()["queryParams"][0])],
            },
            response: RenderedTypeResponse {
                type_name: Some("TWidgetList".to_string()),
                ref_type_name: None,
                schema_ref: None,
                fields: Some(
                    spec()["types"][0]["fields"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(field)
                        .collect(),
                ),
                lang_type: None,
                note: None,
            },
            nested: BTreeMap::new(),
        }
    }

    fn finish(options: Value) -> RenderedTypeReference {
        let spec = spec();
        let method = method();
        let mut tr = one_level();
        let roots = TypeReferenceRoots::from_method(&method, method.get("primaryResponse"));
        finish_type_reference(
            &mut tr,
            &spec,
            &roots,
            &options,
            &NestedRender {
                field: &field,
                lang_type: &lang_type,
            },
        );
        tr
    }

    #[test]
    fn ref_type_name_looks_through_arrays_and_maps() {
        assert_eq!(
            ref_type_name(Some(&json!({ "kind": "ref", "name": "Widget" }))).as_deref(),
            Some("Widget")
        );
        assert_eq!(
            ref_type_name(Some(&json!({ "kind": "array", "items": { "kind": "array", "items": { "kind": "ref", "name": "Widget" } } }))).as_deref(),
            Some("Widget")
        );
        assert_eq!(
            ref_type_name(Some(
                &json!({ "kind": "map", "values": { "kind": "ref", "name": "Label" } })
            ))
            .as_deref(),
            Some("Label")
        );
        assert_eq!(
            ref_type_name(Some(&json!({ "kind": "scalar", "scalar": "string" }))),
            None
        );
        assert_eq!(
            ref_type_name(Some(&json!({ "kind": "ref", "name": "" }))),
            None
        );
        assert_eq!(ref_type_name(None), None);
    }

    #[test]
    fn the_groups_name_their_ir_type_and_schema() {
        let tr = finish(Value::Null);
        assert_eq!(tr.request.ref_type_name, None, "no request body");
        assert_eq!(tr.response.ref_type_name.as_deref(), Some("WidgetList"));
        assert_eq!(
            tr.response.schema_ref.as_deref(),
            Some("#/components/schemas/WidgetList")
        );
        // The query param's enum: refTypeName from the language, schemaRef stamped here.
        assert_eq!(
            tr.request.fields[0].ref_type_name.as_deref(),
            Some("WidgetStatus")
        );
        assert_eq!(
            tr.request.fields[0].schema_ref.as_deref(),
            Some("#/components/schemas/WidgetStatus")
        );
        let response_fields = tr.response.fields.as_ref().unwrap();
        // `data: [TWidget]` names the item type THROUGH the array.
        assert_eq!(response_fields[0].ref_type_name.as_deref(), Some("Widget"));
        assert_eq!(
            response_fields[0].schema_ref.as_deref(),
            Some("#/components/schemas/Widget")
        );
        // The inline paging block has a name but no schema.
        assert_eq!(
            response_fields[1].ref_type_name.as_deref(),
            Some("WidgetListList")
        );
        assert_eq!(response_fields[1].schema_ref, None);
    }

    #[test]
    fn nested_holds_every_reachable_type_once_and_terminates_on_cycles() {
        let tr = finish(Value::Null);
        let names: Vec<&String> = tr.nested.keys().collect();
        assert_eq!(
            names,
            [
                "Label",
                "Part",
                "Shape",
                "Tags",
                "Widget",
                "WidgetList",
                "WidgetListList",
                "WidgetStatus"
            ]
        );
        let widget = &tr.nested["Widget"];
        assert_eq!(widget.type_name, "TWidget");
        assert_eq!(widget.kind, "struct");
        assert_eq!(widget.description.as_deref(), Some("A widget."));
        assert_eq!(
            widget.schema_ref.as_deref(),
            Some("#/components/schemas/Widget")
        );
        let fields = widget.fields.as_ref().unwrap();
        // The self-reference is a plain row pointing back at the same key.
        let parent = fields.iter().find(|f| f.name == "parent").unwrap();
        assert_eq!(parent.ref_type_name.as_deref(), Some("Widget"));
        assert_eq!(
            parent.schema_ref.as_deref(),
            Some("#/components/schemas/Widget")
        );
        // Through a map.
        let labels = fields.iter().find(|f| f.name == "labels").unwrap();
        assert_eq!(labels.lang_type, "{TLabel}");
        assert_eq!(labels.ref_type_name.as_deref(), Some("Label"));

        let block = &tr.nested["WidgetListList"];
        assert_eq!(block.schema_ref, None, "hoisted inline type: no schema");
        let block_fields: Vec<&str> = block
            .fields
            .as_ref()
            .unwrap()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(block_fields, ["has_more", "total"]);

        let status = &tr.nested["WidgetStatus"];
        assert_eq!(status.kind, "enum");
        assert_eq!(status.values, Some(vec![json!("draft"), json!("active")]));
        assert!(status.fields.is_none());

        let tags = &tr.nested["Tags"];
        assert_eq!(tags.kind, "alias");
        let of = tags.of.as_ref().unwrap();
        assert_eq!(of.lang_type, "[string]");
        assert_eq!(of.ref_type_name, None);

        let shape = &tr.nested["Shape"];
        assert_eq!(shape.kind, "union");
        let variants = shape.variants.as_ref().unwrap();
        assert_eq!(variants[0].lang_type, "TPart");
        assert_eq!(variants[0].ref_type_name.as_deref(), Some("Part"));
        assert_eq!(
            variants[0].schema_ref.as_deref(),
            Some("#/components/schemas/Part")
        );
        assert_eq!(variants[1].lang_type, "string");
        assert_eq!(variants[1].ref_type_name, None);
    }

    #[test]
    fn the_depth_cap_counts_hops_from_the_operation() {
        // Depth 1: only the roots (the response envelope, the query enum).
        let tr = finish(json!({ "nestedDepth": 1 }));
        assert_eq!(
            tr.nested.keys().collect::<Vec<_>>(),
            ["WidgetList", "WidgetStatus"]
        );
        // Depth 2 adds what the envelope names; Widget's own refs wait for 3.
        let tr = finish(json!({ "nestedDepth": 2 }));
        assert_eq!(
            tr.nested.keys().collect::<Vec<_>>(),
            ["Widget", "WidgetList", "WidgetListList", "WidgetStatus"]
        );
        // Zero: the one-level reference only.
        let tr = finish(json!({ "nestedDepth": 0 }));
        assert!(tr.nested.is_empty());
        // The default is generous enough for the whole fixture.
        assert_eq!(nested_depth(&Value::Null), DEFAULT_NESTED_DEPTH);
        assert_eq!(nested_depth(&json!({ "nestedDepth": 3 })), 3);
    }

    #[test]
    fn a_dangling_ref_is_skipped() {
        let spec = json!({ "types": [] });
        let method = json!({ "primaryResponse": { "kind": "ref", "name": "Ghost" } });
        let mut tr = one_level();
        tr.response.fields = None;
        finish_type_reference(
            &mut tr,
            &spec,
            &TypeReferenceRoots::from_method(&method, method.get("primaryResponse")),
            &Value::Null,
            &NestedRender {
                field: &field,
                lang_type: &lang_type,
            },
        );
        assert_eq!(tr.response.ref_type_name.as_deref(), Some("Ghost"));
        assert_eq!(tr.response.schema_ref, None);
        assert!(tr.nested.is_empty());
    }

    #[test]
    fn response_root_prefers_the_paginated_item_type() {
        let method = json!({
            "primaryResponse": { "kind": "ref", "name": "WidgetList" },
            "pagination": { "itemType": { "kind": "ref", "name": "Widget" } }
        });
        assert_eq!(response_root(&method, true).unwrap()["name"], "Widget");
        assert_eq!(response_root(&method, false).unwrap()["name"], "WidgetList");
        let unpaged = json!({ "primaryResponse": { "kind": "ref", "name": "Widget" } });
        assert_eq!(response_root(&unpaged, true).unwrap()["name"], "Widget");
        assert_eq!(response_root(&json!({}), false), None);
    }
}
