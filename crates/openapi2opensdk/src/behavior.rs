//! SdkBehavior defaults + deep-merge — port of opensdk-core src/behavior.ts.
//! The IR ALWAYS carries the merged behavior block as `spec.sdk`.

use serde_json::{json, Value};

// The merge itself lives in opensdk_core. Note it is the CONVERTER flavour
// (a non-object side yields `source`), deliberately distinct from the emitters'
// in-place one — see that module for the measured divergence.
use opensdk_core::behavior::{deep_merge, normalize_behavior_shorthands};

/// The canonical SDK behavior defaults (single source of truth mirrored from
/// opensdk-core `defaultSdkBehavior()` — keep byte-in-sync).
pub fn default_sdk_behavior() -> Value {
    json!({
        "retry": {
            "maxRetries": 2,
            "retryableStatusCodes": [408, 429, 500, 502, 503, 504],
            "retryConnectionErrors": true,
            "honorRetryAfterHeader": true,
            "backoff": {
                "initialDelayMs": 500,
                "maxDelayMs": 8000,
                "multiplier": 2,
                "jitter": 0.25
            }
        },
        "timeout": {
            "defaultTimeoutMs": 60000
        },
        "errors": {
            "statusCodeMap": {
                "400": "BadRequest",
                "401": "Unauthorized",
                "403": "PermissionDenied",
                "404": "NotFound",
                "409": "Conflict",
                "422": "UnprocessableEntity",
                "429": "RateLimited"
            },
            "clientErrorKind": "API",
            "serverErrorKind": "Internal"
        },
        "userAgent": {
            "sdkIdentifierTemplate": "{package}-{language}/{version}",
            "includeRuntimeVersion": false,
            "aiAgentEnvVars": {
                "CLAUDE_CODE": "claude-code",
                "CURSOR_AGENT": "cursor",
                "CLINE_ACTIVE": "cline",
                "WINDSURF_ACTIVE": "windsurf",
                "COPILOT_AGENT": "copilot"
            }
        },
        "telemetry": {
            "requestIdHeader": "X-Request-ID",
            "headerName": "X-Client-Telemetry",
            "enabledByDefault": false
        },
        "logging": {
            "events": [
                "request.start",
                "request.success",
                "request.retry",
                "request.rate_limited",
                "request.error",
                "request.connection_error"
            ]
        },
        "idempotency": {
            "headerName": "Idempotency-Key",
            "autoGenerateForPost": true
        },
        "pagination": {
            "autoPageDelayMs": 0
        },
        "requestGuard": {
            "optionKeys": [
                "api_key",
                "apiKey",
                "idempotency_key",
                "idempotencyKey",
                "extra_headers",
                "extraHeaders",
                "max_retries",
                "maxRetries",
                "base_url",
                "baseUrl",
                "timeout"
            ]
        }
    })
}

/// Deep-merge overrides into the defaults: arrays and primitives replace,
/// plain objects merge recursively. (JS skips `undefined` source values —
/// absent keys in JSON; an explicit `null` replaces, matching JS.)
///
/// The `pagination: "none"` shorthand is expanded first so it merges INTO the
/// default `pagination` object instead of replacing it.
pub fn merge_sdk_behavior(overrides: Option<&Value>) -> Value {
    let defaults = default_sdk_behavior();
    match overrides {
        Some(over) => {
            let mut over = over.clone();
            normalize_behavior_shorthands(&mut over);
            deep_merge(&defaults, &over)
        }
        None => defaults,
    }
}

/// Fill `sdk.defaultHeaders` from the document, in place.
///
/// Each `sdk.headersFromExtension` entry names a header and the `x-*`
/// extension that carries its value, e.g. `{ "X-Some-Version": "x-release" }`.
/// The extension is read from `info` first, then from the document root; a
/// string, number or boolean value becomes the header's value. A PRESENT
/// extension wins over a static `defaultHeaders` entry of the same name — the
/// static value is the fallback for a document that lacks the extension — and an
/// absent or non-scalar extension changes nothing.
///
/// Public because the CLI re-stamps the IR's `sdk` block per language after the
/// conversion and must re-resolve against the same document.
pub fn resolve_headers_from_extension(sdk: &mut Value, doc: &Value) {
    let Some(from_ext) = sdk
        .get("headersFromExtension")
        .and_then(Value::as_object)
        .cloned()
    else {
        return;
    };
    let mut headers = sdk
        .get("defaultHeaders")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (header, ext) in &from_ext {
        let Some(ext) = ext.as_str().filter(|e| !e.is_empty()) else {
            continue;
        };
        let found = doc
            .get("info")
            .and_then(|i| i.get(ext))
            .or_else(|| doc.get(ext));
        let text = match found {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => continue,
        };
        headers.insert(header.clone(), Value::String(text));
    }
    if headers.is_empty() {
        return;
    }
    if let Some(obj) = sdk.as_object_mut() {
        obj.insert("defaultHeaders".into(), Value::Object(headers));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pagination_shorthand_merges_into_the_default_object() {
        let merged = merge_sdk_behavior(Some(&json!({ "pagination": "none" })));
        assert_eq!(merged["pagination"]["mode"], "none");
        assert_eq!(merged["pagination"]["autoPageDelayMs"], 0);
    }

    #[test]
    fn headers_from_extension_read_info_then_root_and_win_over_the_static_value() {
        let doc = json!({
            "openapi": "3.1.0",
            "x-tenant": "root-level",
            "info": { "title": "t", "version": "1", "x-release": "2024-10-01", "x-num": 7 }
        });
        let mut sdk = json!({
            "defaultHeaders": { "X-Some-Version": "fallback", "X-Static": "s" },
            "headersFromExtension": {
                "X-Some-Version": "x-release",
                "X-Tenant": "x-tenant",
                "X-Num": "x-num",
                "X-Missing": "x-nowhere",
                "X-Blank": ""
            }
        });
        resolve_headers_from_extension(&mut sdk, &doc);
        assert_eq!(
            sdk["defaultHeaders"],
            json!({
                "X-Some-Version": "2024-10-01",
                "X-Static": "s",
                "X-Tenant": "root-level",
                "X-Num": "7"
            })
        );
    }

    #[test]
    fn headers_from_extension_is_a_no_op_without_the_block_or_without_matches() {
        let doc = json!({ "info": { "title": "t" } });
        let mut plain = json!({ "retry": { "maxRetries": 1 } });
        resolve_headers_from_extension(&mut plain, &doc);
        assert_eq!(plain, json!({ "retry": { "maxRetries": 1 } }));
        let mut unmatched = json!({ "headersFromExtension": { "X-A": "x-absent" } });
        resolve_headers_from_extension(&mut unmatched, &doc);
        assert!(unmatched.get("defaultHeaders").is_none());
    }
}
