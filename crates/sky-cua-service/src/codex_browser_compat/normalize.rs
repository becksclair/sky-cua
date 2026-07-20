use std::time::Duration;

use serde_json::{Map, Value, json};
use sky_cua_platform::model::{
    BrowserCallerKind, BrowserCallerProvenance, BrowserMcpClientInfo, BrowserProvenanceSource,
};

use super::{
    CodexConnectionContext, CodexLogicalIdentity, CodexNormalizedRequest, CodexOperationClass,
    CodexOperationScope, DEFAULT_MUTATION_DEADLINE_MS, DEFAULT_READ_DEADLINE_MS, MAX_DEADLINE_MS,
    NEXT_OPERATION_ID, fresh_id,
};

pub(super) fn normalize_request(
    raw_request: Value,
    upstream_id: u64,
    connection: &CodexConnectionContext,
) -> Result<CodexNormalizedRequest, ()> {
    let method = raw_request
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.is_empty())
        .ok_or(())?
        .to_owned();
    let params = raw_request.get("params").cloned().unwrap_or(Value::Null);
    let class = classify_method(&method, &params);
    let scope = classify_scope(&method, &params);
    // The upstream Browser-client protocol has no control-plane deadline
    // field. Method payloads may legitimately contain names such as
    // `timeoutMs`, so treating an arbitrary nested value as our scheduler
    // deadline would let operation semantics redefine daemon policy. Derive
    // the deadline solely from the owner-side classification.
    let deadline_ms = match class {
        CodexOperationClass::ReadOnly => DEFAULT_READ_DEADLINE_MS,
        CodexOperationClass::AbsoluteSet | CodexOperationClass::Mutation => {
            DEFAULT_MUTATION_DEADLINE_MS
        }
    }
    .min(MAX_DEADLINE_MS);
    let logical_identity = extract_logical_identity(&raw_request);
    let caller_provenance = extract_caller_provenance(&raw_request, connection);
    let identity_synthetic = metadata_value(&raw_request, "identity_synthetic")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(CodexNormalizedRequest {
        operation_id: fresh_id("codex-operation", &NEXT_OPERATION_ID),
        upstream_id,
        method: method.clone(),
        params: params.clone(),
        raw_request,
        connection: connection.clone(),
        logical_identity,
        caller_provenance,
        identity_synthetic,
        class,
        scope,
        canonical_fingerprint: canonical_fingerprint(&method, &params),
        deadline: Duration::from_millis(deadline_ms),
    })
}

fn extract_caller_provenance(
    value: &Value,
    connection: &CodexConnectionContext,
) -> BrowserCallerProvenance {
    let declared_caller = metadata_value(value, "caller_provenance")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|declared| !declared.is_empty())
        .map(|declared| declared.chars().take(128).collect::<String>());
    let client_info = metadata_value(value, "client_info").and_then(parse_client_info);
    let normalized = declared_caller
        .as_deref()
        .and_then(BrowserCallerKind::from_provenance_label);
    let inferred = client_info
        .as_ref()
        .and_then(|info| BrowserCallerKind::from_provenance_label(&info.name));
    let (caller, source) = match normalized {
        Some(caller) => (caller, BrowserProvenanceSource::RequestMetadataDeclaration),
        None if declared_caller.is_some() => (
            BrowserCallerKind::LegacyUnknown,
            BrowserProvenanceSource::LegacyFallback,
        ),
        None if inferred.is_some() => (
            inferred.expect("inferred caller was checked"),
            BrowserProvenanceSource::ClientInfoInference,
        ),
        None => (
            BrowserCallerKind::CodexDesktop,
            BrowserProvenanceSource::HostProvidedIab,
        ),
    };
    BrowserCallerProvenance {
        caller,
        source,
        connection_id: connection.connection_id.clone(),
        declared_caller,
        client_info,
    }
}

fn metadata_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let params = value.get("params")?;
    params
        .get(key)
        .or_else(|| params.get("request_meta").and_then(|meta| meta.get(key)))
        .or_else(|| params.get("_meta").and_then(|meta| meta.get(key)))
}

fn parse_client_info(value: &Value) -> Option<BrowserMcpClientInfo> {
    let object = value.as_object()?;
    let non_empty = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    Some(BrowserMcpClientInfo {
        name: non_empty("name")?.chars().take(128).collect(),
        version: non_empty("version")?.chars().take(128).collect(),
        title: non_empty("title").map(|title| title.chars().take(128).collect()),
    })
}

fn classify_method(method: &str, params: &Value) -> CodexOperationClass {
    match method {
        "ping" | "getInfo" | "getTabs" | "getUserTabs" | "getUserHistory" => {
            CodexOperationClass::ReadOnly
        }
        "moveMouse" => CodexOperationClass::AbsoluteSet,
        "executeCdp" => params
            .get("method")
            .and_then(Value::as_str)
            .map(classify_cdp)
            .unwrap_or(CodexOperationClass::Mutation),
        _ => CodexOperationClass::Mutation,
    }
}

pub(super) fn classify_cdp(method: &str) -> CodexOperationClass {
    if method == "Page.captureScreenshot"
        || method.starts_with("Accessibility.get")
        || method.starts_with("Browser.get")
        || method.starts_with("CSS.get")
        || method.starts_with("DOM.get")
        || method.starts_with("DOMSnapshot.")
        || method.starts_with("Network.get")
        || method.starts_with("Page.get")
        || method.starts_with("Performance.get")
        || method.starts_with("Runtime.get")
        || method.starts_with("SystemInfo.get")
        || method.starts_with("Target.get")
    {
        CodexOperationClass::ReadOnly
    } else if method.starts_with("Emulation.set")
        || method.starts_with("Network.set")
        || method.starts_with("Page.set")
    {
        CodexOperationClass::AbsoluteSet
    } else {
        CodexOperationClass::Mutation
    }
}

fn classify_scope(method: &str, params: &Value) -> CodexOperationScope {
    if matches!(method, "finalizeTabs" | "turnEnded") {
        return CodexOperationScope::Daemon;
    }
    if method == "executeCdp"
        && params
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| {
                method.starts_with("Browser.")
                    || method.starts_with("SystemInfo.")
                    || method.starts_with("Target.")
            })
    {
        return CodexOperationScope::Bridge;
    }
    find_string(params, &["tabId", "tab_id"])
        .or_else(|| find_number(params, &["tabId", "tab_id"]))
        .map(CodexOperationScope::Tab)
        .unwrap_or(CodexOperationScope::Bridge)
}

fn extract_logical_identity(value: &Value) -> CodexLogicalIdentity {
    let params = value.get("params");
    let turn_metadata = params
        .and_then(|params| params.get("_meta"))
        .and_then(|metadata| metadata.get("x-codex-turn-metadata"));
    CodexLogicalIdentity {
        session_id: direct_string(params, &["session_id", "sessionId"])
            .or_else(|| direct_string(Some(value), &["session_id", "sessionId"]))
            .or_else(|| direct_string(turn_metadata, &["session_id", "sessionId"])),
        thread_id: direct_string(params, &["thread_id", "threadId"])
            .or_else(|| direct_string(Some(value), &["thread_id", "threadId"]))
            .or_else(|| direct_string(turn_metadata, &["thread_id", "threadId"])),
        turn_id: direct_string(params, &["turn_id", "turnId"])
            .or_else(|| direct_string(Some(value), &["turn_id", "turnId"]))
            .or_else(|| direct_string(turn_metadata, &["turn_id", "turnId"])),
    }
}

fn direct_string(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    let object = value?.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str).map(str::to_owned))
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    find_value(value, keys).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        _ => None,
    })
}

fn find_number(value: &Value, keys: &[&str]) -> Option<String> {
    find_value(value, keys).and_then(|value| value.as_u64().map(|value| value.to_string()))
}

fn find_value<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(value) = object.get(*key) {
                    return Some(value);
                }
            }
            object.values().find_map(|value| find_value(value, keys))
        }
        Value::Array(values) => values.iter().find_map(|value| find_value(value, keys)),
        _ => None,
    }
}

pub(super) fn canonical_fingerprint(method: &str, params: &Value) -> String {
    serde_json::to_string(&json!({
        "method": method,
        "params": canonicalize(params),
    }))
    .expect("JSON values always serialize")
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut canonical = Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonicalize(&object[key]));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}
