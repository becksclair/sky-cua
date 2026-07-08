use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Shared prefix of every request id this service sends. A response whose id
/// carries this prefix but does not match the in-flight request is a belated
/// reply to one of our own earlier requests on a reused stream, not a foreign
/// or malformed frame.
pub(super) const BRIDGE_REQUEST_ID_PREFIX: &str = "sky-cua-browser-";

pub(super) const LIST_TABS_REQUEST_ID: &str = "sky-cua-browser-list-tabs";
pub(super) const BRIDGE_INFO_REQUEST_ID: &str = "sky-cua-browser-info";
pub(super) const OPEN_TAB_REQUEST_ID: &str = "sky-cua-browser-open-tab";
pub(super) const ATTACH_TAB_REQUEST_ID: &str = "sky-cua-browser-attach-tab";
pub(super) const DETACH_TAB_FOR_RETRY_REQUEST_ID: &str = "sky-cua-browser-detach-tab-for-retry";
pub(super) const ATTACH_TAB_RETRY_REQUEST_ID: &str = "sky-cua-browser-attach-tab-retry";
pub(super) const ENABLE_PAGE_REQUEST_ID: &str = "sky-cua-browser-enable-page";
pub(super) const ENABLE_PAGE_RETRY_REQUEST_ID: &str = "sky-cua-browser-enable-page-retry";
pub(super) const RECOVER_CLAIM_TAB_REQUEST_ID: &str = "sky-cua-browser-recover-claim-tab";
pub(super) const RECOVER_CLAIM_TAB_RETRY_REQUEST_ID: &str =
    "sky-cua-browser-recover-claim-tab-retry";
pub(super) const RECOVER_ENABLE_PAGE_REQUEST_ID: &str = "sky-cua-browser-recover-enable-page";
pub(super) const WAKE_TAB_REQUEST_ID: &str = "sky-cua-browser-wake-tab";
pub(super) const RECOVER_WAKE_TAB_REQUEST_ID: &str = "sky-cua-browser-recover-wake-tab";
#[allow(dead_code)]
pub(super) const RESOLVE_ELEMENT_REQUEST_ID: &str = "sky-cua-browser-resolve-element";
#[allow(dead_code)]
pub(super) const ELEMENT_FOCUS_REQUEST_ID: &str = "sky-cua-browser-element-focus";
pub(super) const NAVIGATE_REQUEST_ID: &str = "sky-cua-browser-navigate";
pub(super) const CLAIM_TAB_REQUEST_ID: &str = "sky-cua-browser-claim-tab";
pub(super) const CLAIM_TAB_RETRY_REQUEST_ID: &str = "sky-cua-browser-claim-tab-retry";
pub(super) const RECLAIM_SESSION_TABS_REQUEST_ID: &str = "sky-cua-browser-reclaim-session-tabs";
pub(super) const MOVE_MOUSE_REQUEST_ID: &str = "sky-cua-browser-move-mouse";
pub(super) const VIEWPORT_SCALE_REQUEST_ID: &str = "sky-cua-browser-viewport-scale";
pub(super) const SNAPSHOT_REQUEST_ID: &str = "sky-cua-browser-snapshot";
pub(super) const SCREENSHOT_REQUEST_ID: &str = "sky-cua-browser-screenshot";
pub(super) const FOCUS_EMULATION_REQUEST_ID: &str = "sky-cua-browser-focus-emulation";
pub(super) const CLICK_MOVE_REQUEST_ID: &str = "sky-cua-browser-click-move";
pub(super) const CLICK_DOWN_REQUEST_ID: &str = "sky-cua-browser-click-down";
pub(super) const CLICK_UP_REQUEST_ID: &str = "sky-cua-browser-click-up";
pub(super) const TYPE_TEXT_REQUEST_ID: &str = "sky-cua-browser-type-text";
pub(super) const KEY_DOWN_REQUEST_ID: &str = "sky-cua-browser-key-down";
pub(super) const KEY_UP_REQUEST_ID: &str = "sky-cua-browser-key-up";
pub(super) const SCROLL_REQUEST_ID: &str = "sky-cua-browser-scroll";
pub(super) const EVAL_REQUEST_ID: &str = "sky-cua-browser-eval";
// A single response frame carries the whole base64 PNG of a `Page.captureScreenshot`,
// which on a 4K/high-DPI viewport routinely exceeds a few MiB once base64-inflated
// and wrapped in the JSON-RPC envelope. The native host forwards frames up to 100
// MiB (`sky-cua-chrome-host` `frame::MAX_FRAME_SIZE`), so a low service-side cap was
// the sole choke point turning a valid large capture into a terminal, non-recoverable
// bridge failure. Keep this at or below the host cap so the host stays the bound.
pub(super) const MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

pub(super) async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    message: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    if body.len() > u32::MAX as usize {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "message too large for 4-byte length prefix",
        ));
    }

    writer.write_all(&(body.len() as u32).to_ne_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

pub(super) async fn read_frame(
    reader: &mut (impl AsyncRead + Unpin),
) -> std::io::Result<Option<Value>> {
    let mut header = [0_u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }

    let length = u32::from_ne_bytes(header) as usize;
    if length > MAX_FRAME_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "frame exceeds maximum size",
        ));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await?;

    serde_json::from_slice(&body).map(Some).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid browser bridge JSON frame: {error}"),
        )
    })
}
