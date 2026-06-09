use std::sync::OnceLock;
use std::time::SystemTime;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, MutexGuard};

use super::*;

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

pub(super) async fn reply_to_viewport_scale(
    stream: &mut UnixStream,
    tab_id: i64,
    device_pixel_ratio: f64,
) {
    let scale = read_frame(stream).await.unwrap().unwrap();
    assert_eq!(
        scale.get("method").and_then(Value::as_str),
        Some("executeCdp")
    );
    assert_eq!(scale["params"]["session_id"], "sky-cua-mcp");
    assert_eq!(scale["params"]["target"]["tabId"], tab_id);
    assert_eq!(scale["params"]["method"], "Runtime.evaluate");
    write_frame(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": scale["id"],
            "result": {
                "result": {
                    "value": {"devicePixelRatio": device_pixel_ratio}
                }
            }
        }),
    )
    .await
    .unwrap();
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
    previous_browser: Option<std::ffi::OsString>,
}

impl Drop for BrowserEnvGuard {
    fn drop(&mut self) {
        restore_env(SKY_CUA_BROWSER_ENV, self.previous_browser.take());
    }
}

pub(super) async fn env_lock() -> BrowserEnvGuard {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let previous_browser = std::env::var_os(SKY_CUA_BROWSER_ENV);
    unsafe { std::env::remove_var(SKY_CUA_BROWSER_ENV) };
    BrowserEnvGuard {
        _guard: guard,
        previous_browser,
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
