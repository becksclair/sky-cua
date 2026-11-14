//! The AppShot window fence for desktop actions, under each way a backend
//! can target them.

use sky_cua_platform::backend::ActionTargeting;
use sky_cua_platform::model::AppShotRejectionReason;

use super::*;

/// A backend whose action targeting, focused window, and window presence are
/// set per test, recording every action that reaches it.
#[derive(Debug, Clone)]
struct FenceBackend {
    inner: FakeBackend,
    targeting: ActionTargeting,
    window_focused: bool,
    window_present: bool,
    dispatched: Arc<std::sync::Mutex<Vec<ActionRequest>>>,
}

#[async_trait::async_trait]
impl DesktopBackend for FenceBackend {
    fn action_targeting(&self) -> ActionTargeting {
        self.targeting
    }

    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError> {
        self.inner.probe_environment().await
    }

    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError> {
        self.inner.list_apps().await
    }

    async fn list_windows(&self) -> Result<Vec<WindowInfo>, BackendError> {
        let mut window = fake_window();
        window.focused = self.window_focused;
        Ok(if self.window_present {
            vec![window]
        } else {
            Vec::new()
        })
    }

    async fn resolve_window_target(
        &self,
        target: &WindowTarget,
    ) -> Result<WindowInfo, BackendError> {
        if self.window_present {
            self.inner.resolve_window_target(target).await
        } else {
            Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "the fake window is gone",
            ))
        }
    }

    async fn get_app_state(
        &self,
        selector: Option<AppSelector>,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        self.inner.get_app_state(selector, capture_screen).await
    }

    async fn get_app_state_for_window(
        &self,
        window: &WindowInfo,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        self.inner
            .get_app_state_for_window(window, capture_screen)
            .await
    }

    async fn screenshot(
        &self,
        target: Option<WindowTarget>,
        display_target: Option<DisplayTarget>,
    ) -> Result<AppStateSnapshot, BackendError> {
        self.inner.screenshot(target, display_target).await
    }

    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError> {
        self.dispatched
            .lock()
            .expect("dispatched lock")
            .push(request);
        Ok(self.inner.outcome.clone())
    }
}

struct Fence {
    daemon: ServiceDaemon,
    dispatched: Arc<std::sync::Mutex<Vec<ActionRequest>>>,
    dir: PathBuf,
}

impl Fence {
    /// A daemon over a backend with the given targeting whose one window is
    /// present but not focused.
    fn new(name: &str, targeting: ActionTargeting, window_present: bool) -> Self {
        let dir = unique_temp_dir(name);
        let source = dir.join("window.png");
        std::fs::write(&source, b"deterministic-window-image").expect("write source image");
        let mut capture = capture_with_path(&source);
        capture.capture_scope = CaptureScope::Window;
        let dispatched = Arc::new(std::sync::Mutex::new(Vec::new()));
        let daemon = daemon_with_backend(Box::new(FenceBackend {
            inner: FakeBackend {
                snapshot: snapshot(Some(capture), Vec::new()),
                outcome: success_outcome(),
                presence: None,
                recorded_action: None,
            },
            targeting,
            window_focused: false,
            window_present,
            dispatched: dispatched.clone(),
        }));
        Self {
            daemon,
            dispatched,
            dir,
        }
    }

    /// Types into the window of a freshly authorized AppShot.
    async fn type_into_appshot_window(&self) -> ServiceResponse {
        let request =
            authorized_action_request(&self.daemon, ActionName::TypeText, json!({"text": "hi"}))
                .await;
        self.act(request).await
    }

    async fn act(&self, request: ActionRequest) -> ServiceResponse {
        self.daemon
            .handle(ServiceRequest::ExecuteAction {
                request: Box::new(request),
            })
            .await
    }

    fn dispatched(&self) -> Vec<ActionRequest> {
        self.dispatched.lock().expect("dispatched lock").clone()
    }
}

impl Drop for Fence {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn a_focus_targeting_backend_rejects_an_action_for_an_unfocused_window() {
    let fence = Fence::new("fence-focused-window", ActionTargeting::FocusedWindow, true);

    let response = fence.type_into_appshot_window().await;

    match response {
        ServiceResponse::AppShotRequired { rejection } => {
            assert_eq!(rejection.reason, AppShotRejectionReason::WrongTarget);
            assert_eq!(rejection.fresh_appshot.trigger, AppShotTrigger::Recovery);
        }
        other => panic!("expected AppShotRequired, got {other:?}"),
    }
    assert!(fence.dispatched().is_empty());
}

#[tokio::test]
async fn a_window_targeting_backend_acts_on_an_unfocused_window() {
    let fence = Fence::new("fence-request-window", ActionTargeting::RequestWindow, true);

    let response = fence.type_into_appshot_window().await;

    match response {
        ServiceResponse::ExecuteAction { outcome } => assert!(outcome.success),
        other => panic!("expected the action to be dispatched, got {other:?}"),
    }
    let dispatched = fence.dispatched();
    assert_eq!(dispatched.len(), 1);
    assert_eq!(
        dispatched[0]
            .resolved_focused_app
            .as_ref()
            .and_then(|app| app.window_handle.as_deref()),
        Some("fake-window")
    );
}

#[tokio::test]
async fn a_window_targeting_backend_rejects_an_action_for_a_vanished_window() {
    let fence = Fence::new(
        "fence-vanished-window",
        ActionTargeting::RequestWindow,
        false,
    );

    let response = fence.type_into_appshot_window().await;

    match response {
        ServiceResponse::Error { code, .. } => assert_eq!(code, "AppShotRequired"),
        other => panic!("expected an AppShotRequired error, got {other:?}"),
    }
    assert!(fence.dispatched().is_empty());
}

#[tokio::test]
async fn a_window_targeting_backend_still_requires_the_latest_snapshot() {
    let fence = Fence::new("fence-stale-snapshot", ActionTargeting::RequestWindow, true);
    let request =
        authorized_action_request(&fence.daemon, ActionName::TypeText, json!({"text": "hi"})).await;
    let mut newer = snapshot(None, Vec::new());
    newer.snapshot_id = "newer".to_string();
    fence.daemon.snapshots.lock().await.store(newer);

    let response = fence.act(request).await;

    match response {
        ServiceResponse::AppShotRequired { rejection } => {
            assert_eq!(rejection.reason, AppShotRejectionReason::Stale);
        }
        other => panic!("expected AppShotRequired, got {other:?}"),
    }
    assert!(fence.dispatched().is_empty());
}
