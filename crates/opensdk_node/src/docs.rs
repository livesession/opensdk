//! The two DOCS capabilities — `generateUsage` and `generateTypeReference`.
//!
//! These render the per-operation SDK view on an API-reference page, NOT part of
//! the generated SDK tree: a self-contained runnable snippet (client init + ONE
//! call) and the method signature + request/response types as field rows.
//!
//! Ports `tests-node.ts` `generateNodeUsage` and `resource.ts`
//! `generateNodeTypeReference`. Both reuse the generated-code machinery — the
//! client attribute chain (`camel_case` / `node_method_name`), the shared
//! example planner, the call-arg renderer, the params naming and the return-type
//! plan — so a docs snippet can never describe a call the SDK does not emit.

use opensdk_core::emitter::{
    opt_str, RenderedTypeField, RenderedTypeGroup, RenderedTypeReference, RenderedTypeResponse,
};
use opensdk_core::nested::{
    finish_type_reference, response_root, NestedRender, TypeReferenceRoots,
};
use serde_json::Value;

use crate::example_plan::{plan_method_example, ExampleOpts};
use crate::ir::{Field, Method, NamedType, Spec, TypeRef};
use crate::jsrt::{camel_case, json_string, node_method_name, prop_key};
use crate::model::{node_type, ModelRefs};
use crate::plan::plan_operation;
use crate::resource::{
    node_return_display, params_arg_name, params_required, params_type_name, NodeCtx,
};
use crate::tests_gen::{has_response, render_call_args};
use crate::type_plan::{field_to_neutral, plan_type_reference, ref_schema_name, NeutralTypeField};

fn parse_spec(spec_json: &Value) -> Spec {
    serde_json::from_value(spec_json.clone()).expect("OpenSDK IR did not match the expected shape")
}

fn parse_method(method_json: &Value) -> Method {
    serde_json::from_value(method_json.clone())
        .expect("OpenSDK IR method did not match the expected shape")
}

fn parse_field(field_json: &Value) -> Field {
    serde_json::from_value(field_json.clone())
        .expect("OpenSDK IR field did not match the expected shape")
}

fn parse_type_ref(type_json: &Value) -> TypeRef {
    serde_json::from_value(type_json.clone())
        .expect("OpenSDK IR type did not match the expected shape")
}

/// `client.<attr>.<attr>.<action>` for a resource-name chain + a method action.
fn call_chain(chain: &[String], method: &Method) -> String {
    let mut segments: Vec<String> = chain.iter().map(|s| camel_case(s)).collect();
    segments.push(node_method_name(&method.action));
    format!("client.{}", segments.join("."))
}

/// A single per-operation USAGE SNIPPET (docs): a self-contained, runnable-looking
/// TypeScript example that constructs the client and makes ONE call.
///
/// `options` is the `emitterOptions` bag (`Value::Null` for none). Besides the
/// usual package/export/env options it honors `baseUrlEnv`, a DOCS-ONLY option:
/// when set, the client reads its base URL from that env var so the snippet can
/// hit a recording server. Unset (the default) leaves the block — and every
/// committed usage golden — byte-identical.
pub fn generate_node_usage(
    spec_json: &Value,
    chain: &[String],
    method_json: &Value,
    options: &Value,
) -> String {
    let spec = parse_spec(spec_json);
    let method = parse_method(method_json);
    let opts = crate::project::resolve_node_options(&spec, options);
    let types: Vec<&NamedType> = spec.types.iter().collect();

    let op = plan_operation(&method, &types);
    // A doc usage snippet: ALL fields with realistic (spec example/default) values.
    let example = plan_method_example(
        &method,
        &types,
        ExampleOpts {
            with_optional: true,
            realistic: true,
        },
    );
    let required_arg = params_required(
        op.has_body,
        op.body_required,
        &method.query_params,
        &method.header_params,
    );
    let call = format!(
        "{}({})",
        call_chain(chain, &method),
        render_call_args(&example, required_arg)
    );

    let import_line = if opts.default_export {
        format!(
            "import {} from {};",
            opts.client_name,
            json_string(&opts.pkg)
        )
    } else {
        format!(
            "import {{ {} }} from {};",
            opts.client_name,
            json_string(&opts.pkg)
        )
    };

    let mut lines = vec![
        import_line,
        String::new(),
        format!("const client = new {}({{", opts.client_name),
        format!("  apiKey: process.env[{}],", json_string(&opts.env_var)),
    ];
    // JS truthiness: an empty `baseUrlEnv` is as good as unset.
    if let Some(env) = opt_str(options, "baseUrlEnv").filter(|s| !s.is_empty()) {
        lines.push(format!("  baseURL: process.env[{}],", json_string(env)));
    }
    lines.push("});".to_string());
    lines.push(String::new());

    // Capture + log the result only when the method returns one (binary download,
    // paginated list, or a primary response).
    if has_response(&op) {
        lines.push(format!("const result = await {call};"));
        lines.push("console.log(result);".to_string());
    } else {
        lines.push(format!("await {call};"));
    }
    format!("{}\n", lines.join("\n"))
}

/// One request/response field row — the params-interface key casing (`prop_key`
/// of the LOGICAL name, matching how the params interface keys body/query/header)
/// and the plain TS type (the `?` lives on the key, not the type).
fn node_render_field(f: &NeutralTypeField) -> RenderedTypeField {
    RenderedTypeField {
        name: prop_key(&f.logical_name),
        lang_type: node_type(Some(&f.type_ref), &mut ModelRefs::new()),
        required: f.required,
        description: f.description.clone(),
        deprecated: f.deprecated,
        ref_type_name: ref_schema_name(&f.type_ref),
        schema_ref: None,
    }
}

/// The per-operation TYPE REFERENCE for Node/TypeScript: the idiomatic call
/// signature, the request params type's field rows, and the response type — the
/// SDK-native view Atlas renders in place of the REST param definitions. Node
/// carries a synthesized params interface, so `request.typeName` is set.
///
/// `options` is the `emitterOptions` bag; the only key this path reads is the
/// `nestedDepth` cap of `opensdk_core::nested`.
pub fn generate_node_type_reference(
    spec_json: &Value,
    chain: &[String],
    method_json: &Value,
    options: &Value,
) -> RenderedTypeReference {
    let spec = parse_spec(spec_json);
    let method = parse_method(method_json);
    let types: Vec<&NamedType> = spec.types.iter().collect();
    let auto_generate_for_post = spec
        .sdk
        .as_ref()
        .and_then(|s| s.idempotency.as_ref())
        .and_then(|i| i.auto_generate_for_post)
        .unwrap_or(true);
    let ctx = NodeCtx {
        types: types.clone(),
        auto_generate_for_post,
    };

    let op = plan_operation(&method, &types);
    let neutral = plan_type_reference(&method, &types);

    let request_fields: Vec<RenderedTypeField> = neutral
        .request_fields
        .iter()
        .map(node_render_field)
        .collect();
    let has_params = !neutral.request_fields.is_empty();

    // The call signature: `client.<attr>.<action>(<path args>, <params arg>):
    // Promise<<return>>`. Positional path params by camel name, then the single
    // params argument (`body` / `query` / `params`, `?`-marked when optional).
    let mut args: Vec<String> = method
        .path_params
        .iter()
        .map(|p| camel_case(&p.name))
        .collect();
    let arg_name = has_params.then(|| {
        params_arg_name(
            op.has_body,
            method.query_params.len(),
            method.header_params.len(),
        )
    });
    if let Some(arg) = arg_name {
        let required = params_required(
            op.has_body,
            op.body_required,
            &method.query_params,
            &method.header_params,
        );
        args.push(if required {
            arg.to_string()
        } else {
            format!("{arg}?")
        });
    }
    let signature = format!(
        "{}({}): Promise<{}>",
        call_chain(chain, &method),
        args.join(", "),
        node_return_display(&op, &method, &ctx)
    );

    let mut tr = RenderedTypeReference {
        signature,
        request: RenderedTypeGroup {
            type_name: has_params.then(|| params_type_name(chain, &method.action)),
            ref_type_name: None,
            schema_ref: None,
            arg_name: arg_name.map(str::to_string),
            fields: request_fields,
        },
        response: node_response(&op, &neutral),
        nested: Default::default(),
    };
    // The transitive view behind the two groups (`opensdk_core::nested`): a
    // nested struct's rows render as the models file declares them — the same
    // `node_render_field` over a MODEL field, plain `node_type` for a type.
    let response = tr
        .response
        .type_name
        .is_some()
        .then(|| response_root(method_json, op.page_name.is_some()))
        .flatten();
    let render_field = |raw: &Value| node_render_field(&field_to_neutral(&parse_field(raw)));
    let render_type =
        |raw: Option<&Value>| node_type(raw.map(parse_type_ref).as_ref(), &mut ModelRefs::new());
    finish_type_reference(
        &mut tr,
        spec_json,
        &TypeReferenceRoots::from_method(method_json, response),
        options,
        &NestedRender {
            field: &render_field,
            lang_type: &render_type,
        },
    );
    tr
}

fn node_response(
    op: &crate::plan::OperationPlan,
    neutral: &crate::type_plan::NeutralTypeReference,
) -> RenderedTypeResponse {
    if let Some(ct) = op.binary_content_type.as_deref() {
        return RenderedTypeResponse {
            type_name: None,
            ref_type_name: None,
            schema_ref: None,
            fields: None,
            lang_type: Some("Response".to_string()),
            note: Some(format!("binary download ({ct})")),
        };
    }
    let Some(ref_) = neutral
        .response_type
        .as_ref()
        .filter(|_| op.primary_response != crate::plan::PrimaryResponseKind::None)
    else {
        return RenderedTypeResponse {
            type_name: None,
            ref_type_name: None,
            schema_ref: None,
            fields: None,
            lang_type: Some("void".to_string()),
            note: Some("no response body".to_string()),
        };
    };
    let type_name = node_type(Some(ref_), &mut ModelRefs::new());
    let note = op.page_name.map(|p| format!("paginated ({p})"));
    match neutral.response_fields.as_ref() {
        Some(fields) => RenderedTypeResponse {
            type_name: Some(type_name),
            ref_type_name: None,
            schema_ref: None,
            fields: Some(fields.iter().map(node_render_field).collect()),
            lang_type: None,
            note,
        },
        None => RenderedTypeResponse {
            type_name: Some(type_name.clone()),
            ref_type_name: None,
            schema_ref: None,
            fields: None,
            lang_type: Some(type_name),
            note,
        },
    }
}
