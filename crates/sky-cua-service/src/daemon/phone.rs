use super::*;

impl ServiceDaemon {
    /// Route a phone request through the service-owned [`PhoneManager`].
    ///
    /// The manager owns session state, the per-session capability-profile cache,
    /// and the `CommandRunner` seam every ADB/companion/scrcpy backend goes
    /// through. Phone control never touches the
    /// desktop session, so this path is intentionally outside the serialized
    /// `desktop_lane`; the manager's own `Mutex` serializes mutable phone state.
    /// Phase 1 backends are stubs: status/devices route through the ADB stub and
    /// every device-bound request returns an honest "not implemented" response
    /// without fabricating a session.
    ///
    /// After the manager handles the request, the daemon mediates the two pieces
    /// of cross-subsystem state the manager cannot reach on its own: it discovers
    /// and maps the managed scrcpy desktop window (the manager owns no desktop
    /// backend), and it draws/hides the host-visible cursor overlay (the manager
    /// owns no overlay). The phone-manager lock and the overlay lock are taken
    /// separately and held only across a read/update, never nested, to avoid
    /// deadlock.
    pub(super) async fn handle_phone_request(&self, request: PhoneRequest) -> ServiceResponse {
        // Derive the phone-scale scrcpy `--max-size` cap for a mirror-bearing
        // connect OUTSIDE the phone lock (the display probe is slow), then apply
        // it under the SAME lock span as handle() so a concurrent connect on
        // another IPC task cannot race the primed value between priming and
        // launch. Applied only when `[phone] max_size` is unset, so the mirror
        // renders phone-sized without overriding an explicit config.
        let scrcpy_size_default = self.scrcpy_host_size_default_for(&request).await;

        // Scan for a pre-existing scrcpy window to adopt (avoid spawning a second
        // mirror) on a scrcpy-bearing connect, OUTSIDE the phone lock (the
        // `list_windows` enumeration is slow), then prime it under the SAME lock
        // span as handle() so the connect path consumes a candidate that cannot be
        // raced by a concurrent connect on another IPC task.
        let adoption_candidate = self.scrcpy_adoption_candidate_for(&request).await;

        let response = {
            let mut phone = self.phone.lock().await;
            if let Some(default) = scrcpy_size_default {
                phone.set_scrcpy_host_size_default(default);
            }
            if adoption_candidate.is_some() {
                phone.set_scrcpy_adoption_candidate(adoption_candidate);
            }
            let response = phone.handle(request).await;
            // Clear the candidate so it never leaks into a later connect for a
            // different serial.
            phone.set_scrcpy_adoption_candidate(None);
            response
        };

        // Discover and map a freshly-launched managed scrcpy window (connect path).
        // The window mapping keeps the mirror sized/located correctly; the agent
        // cursor itself is now drawn on the device by the companion overlay, so no
        // host-desktop cursor is pushed onto the shared OverlayController.
        self.map_scrcpy_window_if_pending().await;

        // Re-read the connected session so the response reflects any mapping the
        // daemon just applied (host_window_mapped flipped true).
        let response = self.refresh_phone_response_after_mapping(response).await;
        ServiceResponse::Phone { response }
    }

    /// Compute the host-derived phone-scale scrcpy `--max-size` cap for a connect
    /// that will launch a managed mirror, or `None` for any other request.
    ///
    /// The outer `Option` says whether to set the manager field at all: `None`
    /// for non-scrcpy requests, which never read it. The inner `Option<u32>` is
    /// the cap itself — `None` when the host topology is unknown, leaving any
    /// configured `[phone] max_size` to stand. The display probe runs here,
    /// outside the phone lock, so the slow topology probe never widens lock
    /// contention; the caller applies the result under the same lock span as
    /// `handle`, closing the race a separate prime/handle lock pair would open.
    /// Only a scrcpy-bearing `Connect` probes, so the common no-mirror path pays
    /// nothing. A failed probe degrades to an uncapped (full-resolution) mirror
    /// and is logged rather than silently swallowed.
    async fn scrcpy_host_size_default_for(&self, request: &PhoneRequest) -> Option<Option<u32>> {
        let PhoneRequest::Connect(connect) = request else {
            return None;
        };
        if !(connect.start_scrcpy || connect.backend == Some(PhoneBackendKind::Scrcpy)) {
            return None;
        }
        let displays = match self.backend.list_displays().await {
            Ok(displays) => displays,
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "phone scrcpy sizing: list_displays failed; mirror will use full resolution"
                );
                Vec::new()
            }
        };
        Some(host_scrcpy_default_max_size(&displays))
    }

    /// Find a pre-existing scrcpy desktop window the connect path should adopt
    /// instead of spawning a duplicate mirror, or `None` for any request that does
    /// not launch a mirror, an unnamed serial, or when no adoptable window exists.
    ///
    /// The manager owns no desktop backend, so the daemon enumerates `list_windows`
    /// here (outside the phone lock; the scan is slow) and lets the manager match
    /// the deterministic `sky-cua-phone-<safe-serial>` title. Adoption is scoped to
    /// a connect that names an explicit serial: an unspecified serial is resolved
    /// inside the manager (default/single-device), so the deterministic title is
    /// not known pre-handle, and the safe fallback is to launch fresh rather than
    /// guess. A failed enumeration degrades to a fresh launch and is logged.
    async fn scrcpy_adoption_candidate_for(
        &self,
        request: &PhoneRequest,
    ) -> Option<crate::phone::ScrcpyAdoptionCandidate> {
        let PhoneRequest::Connect(connect) = request else {
            return None;
        };
        if !(connect.start_scrcpy || connect.backend == Some(PhoneBackendKind::Scrcpy)) {
            return None;
        }
        let serial = connect
            .serial
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let windows = match self.backend.list_windows().await {
            Ok(windows) => windows,
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "phone scrcpy adoption: list_windows failed; connect will launch a fresh mirror"
                );
                return None;
            }
        };
        self.phone
            .lock()
            .await
            .find_adoptable_scrcpy_window(serial, &windows)
    }

    /// If any session has a live managed scrcpy mirror with no host-window mapping
    /// yet, locate its desktop window and store the content-rect mapping. Bounded
    /// retry: the window can take ~1-2s to register after launch, so this polls a
    /// few times, but gives up rather than blocking forever, leaving
    /// `host_window_mapped=false` honestly.
    ///
    /// When nothing is awaiting an initial map, a single re-query of an
    /// already-mapped session's window catches a stale mapping (the operator
    /// resized the scrcpy window, or the host display scale changed); a drifted
    /// content rect is recomputed and an unchanged one is a no-op. The re-map runs
    /// only on this per-request window-work path the daemon already takes — no new
    /// polling loop.
    async fn map_scrcpy_window_if_pending(&self) {
        const MAX_ATTEMPTS: usize = 10;
        const POLL_INTERVAL: Duration = Duration::from_millis(200);

        let Some(target) = self.phone.lock().await.scrcpy_window_to_map() else {
            // No initial map pending: check whether an already-mapped window
            // drifted (resize / host-scale change) and recompute if so.
            self.remap_scrcpy_window_if_drifted().await;
            return;
        };

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            let windows = match self.backend.list_windows().await {
                Ok(windows) => windows,
                Err(error) => {
                    debug!(
                        code = error.code,
                        message = error.message,
                        "scrcpy window mapping: list_windows failed"
                    );
                    // The backend cannot enumerate windows at all; mapping is not
                    // achievable for this session, so mark the round exhausted to
                    // avoid re-polling on every subsequent phone request.
                    self.phone
                        .lock()
                        .await
                        .mark_scrcpy_mapping_exhausted(&target.session_id);
                    return;
                }
            };
            let Some(window) = select_scrcpy_window(&windows, target.pid, &target.window_title)
            else {
                continue;
            };
            let Some(bounds) = window.bounds.clone() else {
                // Matched a window with no bounds; cannot compute a content rect.
                continue;
            };
            // The manager owns the content-rect math (letterboxing, rotation,
            // fractional scale); the daemon supplies the discovered host rect and
            // the device size/rotation it read from the same target.
            let mapped = self.phone.lock().await.set_scrcpy_window_mapping(
                &target.session_id,
                &bounds,
                target.device_size.clone(),
                target.rotation_degrees,
            );
            if mapped {
                return;
            }
        }

        // The bounded retry round ended without a mapping: the window never
        // registered (or never produced bounds). Mark the round exhausted so the
        // daemon stops re-running this ~2s poll on every future phone request for
        // this session; `host_window_mapped` stays honestly false. A fresh runtime
        // on reconnect/refresh re-arms the attempt.
        self.phone
            .lock()
            .await
            .mark_scrcpy_mapping_exhausted(&target.session_id);
    }

    /// Re-query an already host-mapped scrcpy window once and recompute its
    /// content-rect mapping if the live window rect drifted from the stored one
    /// (the operator resized the scrcpy window, or the host display scale changed).
    ///
    /// Bounded by construction: a single `list_windows` (no retry loop — the window
    /// already registered, since it is mapped) on the per-request path the daemon
    /// already runs. The live bounds are fed back through the manager's idempotent
    /// `set_scrcpy_window_mapping`, so an unchanged rect recomputes the same content
    /// rect and returns without profile churn, while a changed rect rebuilds the
    /// mapping so the host cursor overlay tracks the resized window. A failed
    /// enumeration keeps the existing mapping because the host backend is
    /// temporarily unavailable. A vanished or bounds-less window clears the stale
    /// host mapping so the overlay plane is disabled instead of drawing against an
    /// old rectangle.
    async fn remap_scrcpy_window_if_drifted(&self) {
        let Some(target) = self.phone.lock().await.scrcpy_window_to_remap() else {
            return;
        };
        let windows = match self.backend.list_windows().await {
            Ok(windows) => windows,
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "scrcpy window re-map: list_windows failed; keeping the existing mapping"
                );
                return;
            }
        };
        let Some(window) = select_scrcpy_window(&windows, target.pid, &target.window_title) else {
            let _ = self
                .phone
                .lock()
                .await
                .clear_scrcpy_window_mapping(&target.session_id);
            return;
        };
        let Some(bounds) = window.bounds.clone() else {
            let _ = self
                .phone
                .lock()
                .await
                .clear_scrcpy_window_mapping(&target.session_id);
            return;
        };
        // Idempotent on the manager side: an unchanged content rect is a no-op,
        // a drifted one is recomputed.
        let _ = self.phone.lock().await.set_scrcpy_window_mapping(
            &target.session_id,
            &bounds,
            target.device_size.clone(),
            target.rotation_degrees,
        );
    }

    /// Re-fetch the connected session after a mapping so a `phone_connect`
    /// response reflects `host_window_mapped=true`. Only `Connected` responses
    /// carry a session view to refresh; everything else passes through unchanged.
    async fn refresh_phone_response_after_mapping(
        &self,
        response: sky_cua_platform::model::PhoneResponse,
    ) -> sky_cua_platform::model::PhoneResponse {
        use sky_cua_platform::model::PhoneResponse;
        let PhoneResponse::Connected(session) = &response else {
            return response;
        };
        let refreshed = self.phone.lock().await.session_view(&session.session_id);
        match refreshed {
            Some(session) => PhoneResponse::Connected(session),
            None => response,
        }
    }
}

/// Whether a phone request mutates device or session state (and therefore holds
/// session presence). Read-only perception/inspection tools return `false`.
pub(super) fn phone_request_is_write(request: &PhoneRequest) -> bool {
    matches!(
        request,
        PhoneRequest::Connect(_)
            | PhoneRequest::Disconnect(_)
            | PhoneRequest::PairWireless(_)
            | PhoneRequest::Tap(_)
            | PhoneRequest::Swipe(_)
            | PhoneRequest::TypeText(_)
            | PhoneRequest::PressKey(_)
            | PhoneRequest::InstallCompanion(_)
            | PhoneRequest::NotificationOpen(_)
            | PhoneRequest::NotificationDismiss(_)
            | PhoneRequest::NotificationAction(_)
            | PhoneRequest::NotificationReply(_)
            | PhoneRequest::AppLaunch(_)
            | PhoneRequest::AppOpenIntent(_)
            | PhoneRequest::AppForceStop(_)
            | PhoneRequest::AppInstall(_)
            | PhoneRequest::OpenSettings(_)
    )
}

/// Pick the scrcpy desktop window out of a window list for a managed mirror.
///
/// Matching is by `pid` first (robust: the mirror's process id is the strongest
/// signal and survives title collisions), then falls back to an exact
/// `window_title` match (the `sky-cua-phone-<safe-serial>` slug) when no pid is
/// known or no pid matched. Pure and testable: callers pass a constructed window
/// list and the target's pid/title.
pub(super) fn select_scrcpy_window<'a>(
    windows: &'a [WindowInfo],
    pid: Option<u32>,
    window_title: &str,
) -> Option<&'a WindowInfo> {
    if let Some(pid) = pid
        && let Some(window) = windows.iter().find(|window| window.pid == Some(pid))
    {
        return Some(window);
    }
    windows
        .iter()
        .find(|window| window.title.as_deref() == Some(window_title))
}
