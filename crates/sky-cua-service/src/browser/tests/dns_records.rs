//! Regression tests for editing DNS-record-style form fields through the
//! browser bridge.
//!
//! These reproduce the workflow that exposed the key-dispatch bug: an agent
//! asked to replace and delete zone-record values found that `Ctrl+A`,
//! `Backspace`, and `Delete` silently no-opped because the CDP key events
//! carried no virtual key code, so Blink never ran the matching editing
//! command. Each test drives the real `click`/`press_key`/`type_text` bridge
//! calls against the fake extension bridge and asserts the wire events carry
//! the DOM `code` and Windows virtual key code that make selection and deletion
//! actually take effect.

use serde_json::{Value, json};
use sky_cua_platform::model::BrowserTargetKind;
use tokio::net::UnixListener;

use super::helpers::*;
use crate::browser::bridge::{click, press_key, type_text};
use crate::browser::protocol::{read_frame, write_frame};
use crate::browser::sockets::SKY_CUA_SOCKET_DIR_ENV;

const TAB_ID: &str = "515";
/// CDP `modifiers` bit for Ctrl (Alt=1, Ctrl=2, Meta=4, Shift=8).
const CTRL: i64 = 2;

/// A DNS record rendered in an editable zone-file table. `field` is the
/// CSS-pixel center of its value `<input>`.
struct DnsRecord {
    name: &'static str,
    record_type: &'static str,
    value: &'static str,
    field: (f64, f64),
}

impl DnsRecord {
    fn label(&self) -> String {
        format!("{} {} record", self.name, self.record_type)
    }
}

/// The starting zone an agent is asked to edit: an A record to repoint and a
/// TXT record to clear, with a CNAME in between to keep the fields distinct.
fn zone_fixture() -> [DnsRecord; 3] {
    [
        DnsRecord {
            name: "@",
            record_type: "A",
            value: "192.0.2.1",
            field: (420.0, 200.0),
        },
        DnsRecord {
            name: "www",
            record_type: "CNAME",
            value: "old.example.com",
            field: (420.0, 260.0),
        },
        DnsRecord {
            name: "@",
            record_type: "TXT",
            value: "v=spf1 include:old.example.net ~all",
            field: (420.0, 320.0),
        },
    ]
}

fn record<'a>(fixture: &'a [DnsRecord], record_type: &str) -> &'a DnsRecord {
    fixture
        .iter()
        .find(|record| record.record_type == record_type)
        .expect("record type present in fixture")
}

/// Serve the fake bridge for `connections` client connections, acknowledging
/// every frame with an empty result and returning the frames in dispatch order.
/// The client opens a fresh connection per bridge action (a click uses two: the
/// agent-cursor move, then the mouse events), so `connections` is the sum across
/// the actions under test.
async fn collect_bridge_frames(listener: UnixListener, connections: usize) -> Vec<Value> {
    let mut frames = Vec::new();
    for _ in 0..connections {
        let mut stream = accept_after_info(&listener).await;
        while let Ok(Some(frame)) = read_frame(&mut stream).await {
            write_frame(
                &mut stream,
                &json!({"jsonrpc": "2.0", "id": frame["id"], "result": {}}),
            )
            .await
            .unwrap();
            frames.push(frame);
        }
    }
    frames
}

fn bridge_method(frame: &Value) -> Option<&str> {
    frame.get("method").and_then(Value::as_str)
}

fn cdp_method(frame: &Value) -> Option<&str> {
    frame["params"].get("method").and_then(Value::as_str)
}

fn command(frame: &Value) -> &Value {
    &frame["params"]["commandParams"]
}

/// The single frame whose top-level bridge method matches (e.g. `moveMouse`).
fn bridge_frame<'a>(frames: &'a [Value], method: &str) -> &'a Value {
    let mut matching = frames
        .iter()
        .filter(|frame| bridge_method(frame) == Some(method));
    let frame = matching
        .next()
        .unwrap_or_else(|| panic!("missing bridge frame {method}"));
    assert!(
        matching.next().is_none(),
        "expected exactly one {method} frame"
    );
    frame
}

/// Frames dispatching a given CDP method, in order (e.g. `Input.dispatchKeyEvent`).
fn cdp_frames<'a>(frames: &'a [Value], method: &str) -> Vec<&'a Value> {
    frames
        .iter()
        .filter(|frame| cdp_method(frame) == Some(method))
        .collect()
}

/// Assert a key-down event carries the fields Blink needs to run its default
/// action: the `rawKeyDown` type (no typed text), the DOM `code`, and the
/// Windows virtual key code. This is the exact contract the bug violated.
fn assert_editing_key_down(
    frame: &Value,
    key: &str,
    code: &str,
    virtual_key_code: i64,
    modifiers: i64,
) {
    let command = command(frame);
    assert_eq!(
        command["type"], "rawKeyDown",
        "{key} down must be rawKeyDown so it acts instead of typing"
    );
    assert_eq!(command["key"], key);
    assert_eq!(command["code"], code, "DOM code for {key}");
    assert_eq!(
        command["windowsVirtualKeyCode"], virtual_key_code,
        "virtual key code for {key}"
    );
    assert_eq!(command["modifiers"], modifiers);
    assert!(
        command.get("text").is_none(),
        "{key} must not carry typed text or Blink types it instead of acting"
    );
}

#[tokio::test]
async fn agent_replaces_dns_record_value_by_selecting_all_then_typing() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-dns-replace");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    // click (2 connections) + Ctrl+A press_key (1) + type_text (1).
    let server = tokio::spawn(collect_bridge_frames(listener, 4));

    let fixture = zone_fixture();
    let a_record = record(&fixture, "A");
    let new_value = "198.51.100.7";
    assert_ne!(
        new_value,
        a_record.value,
        "replacement must actually change the {}",
        a_record.label()
    );

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };

    // Focus the A-record value field, select its contents, and overtype them.
    let click_response = click(
        Some(BrowserTargetKind::UserChrome),
        TAB_ID.to_string(),
        a_record.field.0,
        a_record.field.1,
    )
    .await;
    let select_all = press_key(
        Some(BrowserTargetKind::UserChrome),
        TAB_ID.to_string(),
        "Ctrl+A".to_string(),
    )
    .await;
    let type_response = type_text(
        Some(BrowserTargetKind::UserChrome),
        TAB_ID.to_string(),
        new_value.to_string(),
    )
    .await;

    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    let frames = server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(click_response.diagnostics.is_empty());
    assert!(select_all.diagnostics.is_empty());
    assert!(type_response.diagnostics.is_empty());

    // The click lands on the A-record value field at its CSS-pixel center.
    let cursor = bridge_frame(&frames, "moveMouse");
    assert_eq!(cursor["params"]["x"], a_record.field.0);
    assert_eq!(cursor["params"]["y"], a_record.field.1);

    // Ctrl+A dispatches exactly one down and one up; the down must reach Blink
    // as a rawKeyDown carrying VK_A (65) with the Ctrl modifier and no text, or
    // select-all silently no-ops (the original bug).
    let keys = cdp_frames(&frames, "Input.dispatchKeyEvent");
    assert_eq!(keys.len(), 2, "Ctrl+A dispatches one down and one up event");
    assert_editing_key_down(keys[0], "A", "KeyA", 65, CTRL);
    assert_eq!(command(keys[1])["type"], "keyUp");

    // The new value is inserted over the now-selected field.
    let inserts = cdp_frames(&frames, "Input.insertText");
    assert_eq!(inserts.len(), 1, "replacement types the new value once");
    assert_eq!(command(inserts[0])["text"], new_value);
}

#[tokio::test]
async fn agent_clears_dns_record_value_with_select_all_and_delete() {
    let _env_guard = env_lock().await;
    let socket_dir = unique_test_dir("sky-cua-browser-dns-delete");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let listener = UnixListener::bind(socket_dir.join("extension-123-test.sock")).unwrap();

    // click (2 connections) + Ctrl+A press_key (1) + Delete press_key (1).
    let server = tokio::spawn(collect_bridge_frames(listener, 4));

    let fixture = zone_fixture();
    let txt_record = record(&fixture, "TXT");
    assert!(
        !txt_record.value.is_empty(),
        "the {} starts with a value to clear",
        txt_record.label()
    );

    let previous = std::env::var_os(SKY_CUA_SOCKET_DIR_ENV);
    unsafe { std::env::set_var(SKY_CUA_SOCKET_DIR_ENV, &socket_dir) };

    // Focus the TXT-record value field, select all, and delete the selection.
    let click_response = click(
        Some(BrowserTargetKind::UserChrome),
        TAB_ID.to_string(),
        txt_record.field.0,
        txt_record.field.1,
    )
    .await;
    let select_all = press_key(
        Some(BrowserTargetKind::UserChrome),
        TAB_ID.to_string(),
        "Ctrl+A".to_string(),
    )
    .await;
    let delete = press_key(
        Some(BrowserTargetKind::UserChrome),
        TAB_ID.to_string(),
        "Delete".to_string(),
    )
    .await;

    restore_env(SKY_CUA_SOCKET_DIR_ENV, previous);
    let frames = server.await.unwrap();
    std::fs::remove_dir_all(socket_dir).unwrap();

    assert!(click_response.diagnostics.is_empty());
    assert!(select_all.diagnostics.is_empty());
    assert!(delete.diagnostics.is_empty());

    // The click lands on the TXT-record value field.
    let cursor = bridge_frame(&frames, "moveMouse");
    assert_eq!(cursor["params"]["x"], txt_record.field.0);
    assert_eq!(cursor["params"]["y"], txt_record.field.1);

    // Ctrl+A then Delete: four key events, and the Delete down must carry
    // VK_DELETE (46) as a rawKeyDown or the field is never cleared.
    let keys = cdp_frames(&frames, "Input.dispatchKeyEvent");
    assert_eq!(
        keys.len(),
        4,
        "Ctrl+A and Delete each dispatch a down and an up"
    );
    assert_editing_key_down(keys[0], "A", "KeyA", 65, CTRL);
    assert_eq!(command(keys[1])["type"], "keyUp");
    assert_editing_key_down(keys[2], "Delete", "Delete", 46, 0);
    assert_eq!(command(keys[3])["type"], "keyUp");

    // Clearing the field types nothing back.
    assert!(
        cdp_frames(&frames, "Input.insertText").is_empty(),
        "deleting a record value must not insert text"
    );
}
