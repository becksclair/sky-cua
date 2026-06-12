//! Shared fake-server fixtures and environment guards for browser tests.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

use serde_json::{Value, json};
use sky_cua_platform::model::BROWSER_EVAL_ENV;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, MutexGuard};

use crate::browser::protocol::{
    BRIDGE_INFO_REQUEST_ID, LIST_TABS_REQUEST_ID, read_frame, write_frame,
};
use crate::browser::sockets::{CODEX_SOCKET_DIR_ENV, SKY_CUA_BROWSER_ENV, SKY_CUA_SOCKET_DIR_ENV};

pub(super) fn unique_test_dir(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

pub(super) async fn reply_with_tabs(
    listener: impl std::borrow::Borrow<UnixListener>,
    tab_id: i64,
    title: &'static str,
) {
    let (mut stream, _) = listener.borrow().accept().await.unwrap();
    let request = read_frame(&mut stream).await.unwrap().unwrap();
    assert_eq!(
        request.get("method").and_then(Value::as_str),
        Some("getUserTabs")
    );
    write_frame(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": LIST_TABS_REQUEST_ID,
            "result": [
                {
                    "id": tab_id,
                    "title": title,
                    "url": "https://example.test/bridge"
                }
            ]
        }),
    )
    .await
    .unwrap();
}

pub(super) async fn reply_with_info(listener: impl std::borrow::Borrow<UnixListener>) {
    let mut stream = accept_after_info(listener.borrow()).await;
    let _ = read_frame(&mut stream).await;
}

pub(super) async fn accept_after_info(
    listener: impl std::borrow::Borrow<UnixListener>,
) -> UnixStream {
    let (mut stream, _) = listener.borrow().accept().await.unwrap();
    reply_to_info_request(&mut stream).await;
    stream
}

pub(super) async fn reply_to_info_request(stream: &mut UnixStream) {
    let request = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        request.get("method").and_then(Value::as_str),
        Some("getInfo")
    );
    assert_eq!(
        request.get("id").and_then(Value::as_str),
        Some(BRIDGE_INFO_REQUEST_ID)
    );
    write_frame(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": BRIDGE_INFO_REQUEST_ID,
            "result": {"name": "sky-cua-test-bridge"}
        }),
    )
    .await
    .unwrap();
}

pub(super) async fn accept_until_non_info_request(listener: &UnixListener) -> (UnixStream, Value) {
    let (mut stream, _) = listener.accept().await.unwrap();
    loop {
        let request = read_frame(&mut stream).await.unwrap().unwrap();
        if request.get("method").and_then(Value::as_str) != Some("getInfo") {
            return (stream, request);
        }
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {"name": "sky-cua-test-bridge"}
            }),
        )
        .await
        .unwrap();
    }
}

pub(super) async fn reply_with_opened_tab(
    listener: impl std::borrow::Borrow<UnixListener>,
    tab_id: i64,
) {
    let mut stream = accept_after_info(listener.borrow()).await;
    let create = read_frame(&mut stream).await.unwrap().unwrap();
    assert_eq!(
        create.get("method").and_then(Value::as_str),
        Some("createTab")
    );
    write_frame(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": create["id"],
            "result": {
                "id": tab_id,
                "title": "First Live Tab",
                "url": "about:blank",
                "active": true
            }
        }),
    )
    .await
    .unwrap();

    let attach = read_frame(&mut stream).await.unwrap().unwrap();
    assert_eq!(attach.get("method").and_then(Value::as_str), Some("attach"));
    write_frame(
        &mut stream,
        &json!({"jsonrpc": "2.0", "id": attach["id"], "result": {}}),
    )
    .await
    .unwrap();

    let enable = read_frame(&mut stream).await.unwrap().unwrap();
    assert_eq!(
        enable.get("method").and_then(Value::as_str),
        Some("executeCdp")
    );
    assert_eq!(enable["params"]["method"], "Page.enable");
    write_frame(
        &mut stream,
        &json!({"jsonrpc": "2.0", "id": enable["id"], "result": {}}),
    )
    .await
    .unwrap();
}

pub(super) async fn reply_to_attach_and_enable(stream: &mut UnixStream, tab_id: i64) {
    let attach = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(attach.get("method").and_then(Value::as_str), Some("attach"));
    assert_eq!(attach["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(attach["params"]["tabId"], tab_id);
    write_frame(
        stream,
        &json!({"jsonrpc": "2.0", "id": attach["id"], "result": {}}),
    )
    .await
    .unwrap();

    let enable = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        enable.get("method").and_then(Value::as_str),
        Some("executeCdp")
    );
    assert_eq!(enable["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(enable["params"]["target"]["tabId"], tab_id);
    assert_eq!(enable["params"]["method"], "Page.enable");
    write_frame(
        stream,
        &json!({"jsonrpc": "2.0", "id": enable["id"], "result": {}}),
    )
    .await
    .unwrap();
}

pub(super) async fn reply_to_detach(stream: &mut UnixStream, tab_id: i64) {
    let detach = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(detach.get("method").and_then(Value::as_str), Some("detach"));
    assert_eq!(detach["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(detach["params"]["tabId"], tab_id);
    write_frame(
        stream,
        &json!({"jsonrpc": "2.0", "id": detach["id"], "result": {}}),
    )
    .await
    .unwrap();
}

/// Serve one snapshot `Runtime.evaluate` request with a minimal page payload.
/// The reply shape encodes the bridge snapshot contract; tests share this so
/// the contract lives in one place.
pub(super) async fn reply_to_snapshot_request(
    stream: &mut UnixStream,
    tab_id: i64,
    title: &str,
    url: &str,
) {
    let request = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        request.get("method").and_then(Value::as_str),
        Some("executeCdp")
    );
    assert_eq!(request["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(request["params"]["target"]["tabId"], tab_id);
    assert_eq!(request["params"]["method"], "Runtime.evaluate");
    write_frame(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "result": {
                    "type": "object",
                    "value": {
                        "title": title,
                        "url": url,
                        "viewport": {"width": 800, "height": 600, "devicePixelRatio": 1},
                        "text": "ready",
                        "elements": []
                    }
                }
            }
        }),
    )
    .await
    .unwrap();
}

pub(super) async fn reply_to_viewport_metrics(
    stream: &mut UnixStream,
    tab_id: i64,
    css_width: f64,
    css_height: f64,
    device_pixel_ratio: f64,
) {
    let metrics = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        metrics.get("method").and_then(Value::as_str),
        Some("executeCdp")
    );
    assert_eq!(metrics["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(metrics["params"]["target"]["tabId"], tab_id);
    assert_eq!(metrics["params"]["method"], "Runtime.evaluate");
    write_frame(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": metrics["id"],
            "result": {
                "result": {
                    "value": {
                        "width": css_width,
                        "height": css_height,
                        "devicePixelRatio": device_pixel_ratio
                    }
                }
            }
        }),
    )
    .await
    .unwrap();
}

/// Encode a solid-color PNG for fake CDP screenshot replies.
pub(super) fn test_png_base64(width: u32, height: u32) -> String {
    use base64::Engine as _;

    let mut bytes = Vec::new();
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([40, 90, 160]));
    image::DynamicImage::ImageRgb8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode test png");
    base64::engine::general_purpose::STANDARD.encode(&bytes)
}

pub(super) async fn reply_with_info_then_hang_on_create(listener: UnixListener) {
    let mut stream = accept_after_info(&listener).await;
    let create = read_frame(&mut stream).await.unwrap().unwrap();
    assert_eq!(
        create.get("method").and_then(Value::as_str),
        Some("createTab")
    );
    std::future::pending::<()>().await;
}

pub(super) async fn hold_connection(listener: UnixListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let _request = read_frame(&mut stream).await.unwrap().unwrap();
    std::future::pending::<()>().await;
}

pub(super) struct BrowserEnvGuard {
    _guard: MutexGuard<'static, ()>,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Drop for BrowserEnvGuard {
    fn drop(&mut self) {
        for (name, value) in &mut self.previous {
            restore_env(name, value.take());
        }
    }
}

/// Env vars browser tests mutate. The guard snapshots all of them at lock
/// acquisition and Drop-restores them, so a panicking test cannot leak values
/// (e.g. `SKY_CUA_BROWSER_EVAL=on` or a temp socket dir) into later tests.
const GUARDED_ENV_VARS: &[&str] = &[
    SKY_CUA_BROWSER_ENV,
    BROWSER_EVAL_ENV,
    SKY_CUA_SOCKET_DIR_ENV,
    CODEX_SOCKET_DIR_ENV,
    sky_cua_platform::config::MACHINE_CONFIG_PATH_ENV,
];

/// Serialize env-mutating browser tests. Browser selection and eval opt-in are
/// also reset to a deterministic absent state, and the machine config path is
/// pinned to a nonexistent file so tests never read the developer's real
/// ~/.config/sky-cua/sky-cua.toml (a seeded `browser` pin there filters the
/// fixture sockets out of discovery and hangs the open-tab tests); socket dir
/// overrides are left as-is because each test sets its own before use.
pub(super) async fn env_lock() -> BrowserEnvGuard {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let previous = GUARDED_ENV_VARS
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    unsafe { std::env::remove_var(SKY_CUA_BROWSER_ENV) };
    unsafe { std::env::remove_var(BROWSER_EVAL_ENV) };
    unsafe {
        std::env::set_var(
            sky_cua_platform::config::MACHINE_CONFIG_PATH_ENV,
            "/nonexistent/sky-cua-test-machine-config.toml",
        )
    };
    BrowserEnvGuard {
        _guard: guard,
        previous,
    }
}

pub(super) fn restore_env(name: &str, previous: Option<std::ffi::OsString>) {
    unsafe {
        if let Some(value) = previous {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}
