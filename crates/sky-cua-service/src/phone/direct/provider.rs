//! Service-internal direct Companion provider seam.
//!
//! This adapter intentionally does not manufacture an ADB serial. Callers use
//! the stable device id and link epoch carried by [`DirectRuntimeHandle`], then
//! project the resulting snapshot into public contracts at the integration
//! boundary owned by the phone manager.

use std::time::Duration;

use super::{DirectDeviceEvent, DirectDeviceSnapshot, DirectRuntimeError, DirectRuntimeHandle};

#[derive(Clone)]
pub(crate) struct CompanionDirectProvider {
    runtime: DirectRuntimeHandle,
}

impl CompanionDirectProvider {
    pub(crate) fn new(runtime: DirectRuntimeHandle) -> Self {
        Self { runtime }
    }

    pub(crate) fn runtime(&self) -> DirectRuntimeHandle {
        self.runtime.clone()
    }

    pub(crate) fn list_devices(&self) -> Vec<DirectDeviceSnapshot> {
        self.runtime.snapshots()
    }

    pub(crate) fn device(&self, device_id: &str) -> Option<DirectDeviceSnapshot> {
        self.runtime.snapshot(device_id)
    }

    pub(crate) fn subscribe(&self) -> tokio::sync::broadcast::Receiver<DirectDeviceEvent> {
        self.runtime.subscribe()
    }

    /// Dispatch one typed method over the authenticated direct link. The
    /// handle checks the exact epoch before putting bytes on the wire and never
    /// replays the request after a disconnect.
    pub(crate) async fn dispatch(
        &self,
        device_id: &str,
        link_epoch: u64,
        method: &str,
        params: serde_json::Value,
        idempotent: bool,
        deadline: Duration,
    ) -> Result<serde_json::Value, DirectRuntimeError> {
        self.runtime
            .request(device_id, link_epoch, method, params, idempotent, deadline)
            .await
    }

    pub(crate) async fn send_content(
        &self,
        device_id: &str,
        link_epoch: u64,
        bytes: &[u8],
        mime_type: &str,
        filename: Option<String>,
    ) -> Result<sky_cua_platform::model::ContentRef, DirectRuntimeError> {
        self.runtime
            .send_content(device_id, link_epoch, bytes, mime_type, filename)
            .await
    }
}
