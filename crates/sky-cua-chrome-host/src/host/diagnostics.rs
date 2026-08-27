//! Structured one-line JSON diagnostics for the native host. Emitted to stderr
//! on a best-effort, non-panicking basis. Field values are serializable JSON;
//! event codes are stable strings so operators and the daemon can key on them.

use std::io::{self, Write};

use serde_json::{Map, Value, json};

pub(super) fn emit(event_type: &str, fields: Map<String, Value>) {
    let mut object = Map::new();
    object.insert("type".to_string(), Value::String(event_type.to_string()));
    for (key, value) in fields {
        object.insert(key, value);
    }
    let _ = writeln!(io::stderr(), "{}", Value::Object(object));
}

pub(super) fn settlement_metadata_rejected(reason: &str) {
    emit(
        "sky_cua_host_settlement_metadata_rejected",
        Map::from_iter([("reason".to_string(), Value::String(reason.to_string()))]),
    );
}

pub(super) fn settlement_delivery_converted_to_unknown(
    operation_id: &str,
    chrome_request_id: &str,
    settlement_generation: &str,
    active_generation: Option<&str>,
    queue_len: usize,
    phase_age_ms: u128,
) {
    emit(
        "sky_cua_host_settlement_delivery_converted_to_unknown",
        Map::from_iter([
            (
                "operation_id".to_string(),
                Value::String(operation_id.to_string()),
            ),
            (
                "chrome_request_id".to_string(),
                Value::String(chrome_request_id.to_string()),
            ),
            (
                "settlement_generation".to_string(),
                Value::String(settlement_generation.to_string()),
            ),
            (
                "active_generation".to_string(),
                active_generation.map_or(Value::Null, |generation| {
                    Value::String(generation.to_string())
                }),
            ),
            ("queue_len".to_string(), json!(queue_len)),
            ("phase_age_ms".to_string(), json!(phase_age_ms)),
        ]),
    );
}

pub(super) fn settlement_delivery_evicted(
    operation_id: &str,
    chrome_request_id: &str,
    settlement_generation: &str,
    queue_len_after: usize,
    reason: &str,
) {
    emit(
        "sky_cua_host_settlement_delivery_evicted",
        Map::from_iter([
            (
                "operation_id".to_string(),
                Value::String(operation_id.to_string()),
            ),
            (
                "chrome_request_id".to_string(),
                Value::String(chrome_request_id.to_string()),
            ),
            (
                "settlement_generation".to_string(),
                Value::String(settlement_generation.to_string()),
            ),
            ("queue_len_after".to_string(), json!(queue_len_after)),
            ("reason".to_string(), Value::String(reason.to_string())),
        ]),
    );
}

pub(super) fn control_plane_fenced_unresponsive(
    client_id: usize,
    last_seen_age_ms: u128,
    queue_len: usize,
) {
    emit(
        "sky_cua_host_control_plane_fenced_unresponsive",
        Map::from_iter([
            ("client_id".to_string(), json!(client_id)),
            ("last_seen_age_ms".to_string(), json!(last_seen_age_ms)),
            ("queue_len".to_string(), json!(queue_len)),
        ]),
    );
}

pub(super) fn control_plane_socket_closed_delivery_failure(
    client_id: usize,
    error_kind: &str,
    stage: &str,
) {
    emit(
        "sky_cua_host_control_plane_socket_closed_delivery_failure",
        Map::from_iter([
            ("client_id".to_string(), json!(client_id)),
            (
                "error_kind".to_string(),
                Value::String(error_kind.to_string()),
            ),
            ("stage".to_string(), Value::String(stage.to_string())),
        ]),
    );
}
