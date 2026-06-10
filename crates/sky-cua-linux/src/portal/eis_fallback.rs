use std::sync::Arc;
use std::time::Duration;

use ashpd::desktop::remote_desktop::{ConnectToEISOptions, KeyState};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use tracing::{debug, warn};

use crate::portal::eis_input::{
    EisAction, EisOperationError, EisWorkerHandle, eis_fallback_details, should_fallback_to_legacy,
    should_reset_session_before_legacy_fallback, spawn_eis_worker,
};
use crate::portal::eis_keymap::{keysym_for_char, keysym_for_key_name};
use crate::portal::remote_desktop::{
    MouseButton, PORTAL_EIS_INPUT_FALLBACK, PORTAL_EIS_INPUT_USED, PORTAL_EIS_POINTER_DISABLED,
    PORTAL_SESSION_REBUILT, PortalLifecycleEvent, RemoteDesktopSessionManager,
};
use crate::virtual_input::{LinuxVirtualInput, virtual_input_keyboard_available};
use crate::x11::input_xtest;

const PORTAL_EIS_ENV: &str = "SKY_CUA_PORTAL_EIS";

/// Whether RemoteDesktop EIS should drive pointer-positioned actions.
///
/// This is a diagnostic knob for isolating compositor input-lane behavior;
/// both the EIS and legacy NotifyPointer* lanes dispatch clicks at the
/// requested coordinates on the proven compositors. Keyboard EIS is never
/// gated here: key events carry no coordinates. Note that on KWin with
/// fractional scaling and panels, the *visible* hardware cursor can render
/// offset by the work-area origin during RemoteDesktop input on both lanes
/// while input dispatch stays accurate; the agent cursor overlay marks the
/// true input position.
fn eis_pointer_input_enabled() -> bool {
    eis_pointer_input_enabled_from(std::env::var(PORTAL_EIS_ENV).ok().as_deref())
}

fn eis_pointer_input_enabled_from(mode: Option<&str>) -> bool {
    !matches!(
        mode.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("never" | "off" | "0" | "false")
    )
}

impl RemoteDesktopSessionManager {
    async fn push_eis_pointer_disabled_event(&self, action: &str) {
        self.push_lifecycle_event(PortalLifecycleEvent {
            code: PORTAL_EIS_POINTER_DISABLED,
            message: format!(
                "EIS pointer {action} skipped by session policy; using legacy RemoteDesktop pointer calls."
            ),
            details: None,
        })
        .await;
    }

    pub async fn click_at(&self, x: f64, y: f64, button: MouseButton) -> Result<(), BackendError> {
        if !eis_pointer_input_enabled() {
            self.push_eis_pointer_disabled_event("click").await;
            self.pointer_move_absolute(x, y).await?;
            return self.click(button).await;
        }
        let action = EisAction::Click { x, y, button };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the pointer click through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                Ok(())
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                debug!(
                    message = %failure.error.message,
                    "EIS pointer click failed; falling back to legacy RemoteDesktop NotifyPointer calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS pointer click failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;
                self.pointer_move_absolute(x, y).await?;
                self.click(button).await
            }
        }
    }

    pub async fn drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
        if !eis_pointer_input_enabled() {
            self.push_eis_pointer_disabled_event("drag").await;
            return self.legacy_drag(from, to).await;
        }
        let action = EisAction::Drag { from, to };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the pointer drag through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                Ok(())
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                debug!(
                    message = %failure.error.message,
                    "EIS pointer drag failed; falling back to legacy RemoteDesktop NotifyPointer calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS pointer drag failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;
                self.legacy_drag(from, to).await
            }
        }
    }

    async fn legacy_drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
        self.pointer_move_absolute(from.0, from.1).await?;
        self.pointer_button(MouseButton::Left, true).await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let result = async {
            self.pointer_move_absolute(to.0, to.1).await?;
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.pointer_button(MouseButton::Left, false).await
        }
        .await;
        if result.is_err() {
            let _ = self.pointer_button(MouseButton::Left, false).await;
        }
        result
    }

    pub async fn scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
    ) -> Result<(), BackendError> {
        if !eis_pointer_input_enabled() {
            self.push_eis_pointer_disabled_event("scroll").await;
            self.pointer_move_absolute(x, y).await?;
            return if let Some(delta_y) = delta_y {
                self.scroll_vertical_smooth(delta_y).await
            } else {
                self.scroll_vertical_discrete(steps).await
            };
        }
        let action = EisAction::ScrollVertical {
            x,
            y,
            delta_y,
            steps,
        };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the pointer scroll through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                Ok(())
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                debug!(
                    message = %failure.error.message,
                    "EIS pointer scroll failed; falling back to legacy RemoteDesktop NotifyPointer calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS pointer scroll failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;
                self.pointer_move_absolute(x, y).await?;
                if let Some(delta_y) = delta_y {
                    self.scroll_vertical_smooth(delta_y).await
                } else {
                    self.scroll_vertical_discrete(steps).await
                }
            }
        }
    }

    pub async fn send_text(&self, text: &str) -> Result<(), BackendError> {
        let action = EisAction::SendText {
            text: Arc::from(text),
        };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected keyboard text through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                return Ok(());
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                warn!(
                    message = %failure.error.message,
                    "EIS keyboard text failed; falling back to legacy RemoteDesktop NotifyKeyboard calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS keyboard text failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;

                // Prefer X11/XTest or LinuxVirtualInput over per-character D-Bus round-trips.
                // These paths spawn subprocesses and must not block the async executor.
                if input_xtest::xtest_is_available() {
                    let text = text.to_string();
                    match tokio::task::spawn_blocking(move || input_xtest::send_text(&text)).await {
                        Ok(Ok(())) => {
                            self.push_lifecycle_event(PortalLifecycleEvent {
                                code: PORTAL_EIS_INPUT_FALLBACK,
                                message:
                                    "RemoteDesktop EIS text failed; fell back to X11/XTest input."
                                        .to_string(),
                                details: None,
                            })
                            .await;
                            return Ok(());
                        }
                        Ok(Err(error)) => {
                            warn!(message = %error.message, "XTest text fallback failed");
                        }
                        Err(error) => {
                            warn!(task_error = %error, "XTest text fallback task panicked");
                        }
                    }
                }
                if virtual_input_keyboard_available() {
                    let text = text.to_string();
                    match tokio::task::spawn_blocking(move || {
                        LinuxVirtualInput::new().and_then(|vi| vi.type_text(&text))
                    })
                    .await
                    {
                        Ok(Ok(())) => {
                            self.push_lifecycle_event(PortalLifecycleEvent {
                                code: PORTAL_EIS_INPUT_FALLBACK,
                                message: "RemoteDesktop EIS text failed; fell back to Linux virtual input.".to_string(),
                                details: None,
                            })
                            .await;
                            return Ok(());
                        }
                        Ok(Err(error)) => {
                            warn!(message = %error.message, "LinuxVirtualInput text fallback failed");
                        }
                        Err(error) => {
                            warn!(task_error = %error, "LinuxVirtualInput text fallback task panicked");
                        }
                    }
                }
            }
        }

        // Final resort: legacy per-character keysym injection via D-Bus.
        for character in text.chars() {
            let keysym = keysym_for_char(character).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "cannot type unsupported character {character:?} through keysym injection"
                    ),
                )
            })?;
            self.send_keysym_raw(keysym).await?;
        }
        Ok(())
    }

    pub async fn press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        self.press_key_sequence_with_fallbacks(keys, true).await
    }

    pub async fn press_key_sequence_portal_only(
        &self,
        keys: &[String],
    ) -> Result<(), BackendError> {
        self.press_key_sequence_with_fallbacks(keys, false).await
    }

    async fn press_key_sequence_with_fallbacks(
        &self,
        keys: &[String],
        allow_native_fallbacks: bool,
    ) -> Result<(), BackendError> {
        if keys.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires at least one key",
            ));
        }

        let action = EisAction::PressKeySequence {
            keys: Arc::from(keys.to_vec().into_boxed_slice()),
        };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the key sequence through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                return Ok(());
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                warn!(
                    message = %failure.error.message,
                    "EIS key sequence failed; falling back to legacy RemoteDesktop NotifyKeyboard calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS key sequence failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;

                // Prefer native fallbacks for ordinary key presses, but KDE clipboard paste must
                // stay inside the portal path so Wayland apps receive the paste chord.
                if allow_native_fallbacks && input_xtest::xtest_is_available() {
                    let keys = keys.to_vec();
                    match tokio::task::spawn_blocking(move || {
                        input_xtest::press_key_sequence(&keys)
                    })
                    .await
                    {
                        Ok(Ok(())) => {
                            self.push_lifecycle_event(PortalLifecycleEvent {
                                code: PORTAL_EIS_INPUT_FALLBACK,
                                message: "RemoteDesktop EIS key sequence failed; fell back to X11/XTest input.".to_string(),
                                details: None,
                            })
                            .await;
                            return Ok(());
                        }
                        Ok(Err(error)) => {
                            warn!(message = %error.message, "XTest key sequence fallback failed");
                        }
                        Err(error) => {
                            warn!(task_error = %error, "XTest key sequence fallback task panicked");
                        }
                    }
                }
                if allow_native_fallbacks && virtual_input_keyboard_available() {
                    let keys = keys.to_vec();
                    match tokio::task::spawn_blocking(move || {
                        LinuxVirtualInput::new().and_then(|vi| vi.press_key_sequence(&keys))
                    })
                    .await
                    {
                        Ok(Ok(())) => {
                            self.push_lifecycle_event(PortalLifecycleEvent {
                                code: PORTAL_EIS_INPUT_FALLBACK,
                                message: "RemoteDesktop EIS key sequence failed; fell back to Linux virtual input.".to_string(),
                                details: None,
                            })
                            .await;
                            return Ok(());
                        }
                        Ok(Err(error)) => {
                            warn!(message = %error.message, "LinuxVirtualInput key sequence fallback failed");
                        }
                        Err(error) => {
                            warn!(task_error = %error, "LinuxVirtualInput key sequence fallback task panicked");
                        }
                    }
                }
            }
        }

        // Final resort: legacy per-keysym injection via D-Bus.
        let mut resolved = Vec::new();
        for key in keys {
            let keysym = keysym_for_key_name(key).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("unsupported key name {key:?}"),
                )
            })?;
            resolved.push(keysym);
        }

        if resolved.len() == 1 {
            return self.send_keysym_raw(resolved[0]).await;
        }

        let mut pressed_modifiers = Vec::with_capacity(resolved.len() - 1);
        for keysym in &resolved[..resolved.len() - 1] {
            if let Err(error) = self.send_keysym_state(*keysym, KeyState::Pressed).await {
                for pressed in pressed_modifiers.iter().rev() {
                    let _ = self.send_keysym_state(*pressed, KeyState::Released).await;
                }
                return Err(error);
            }
            pressed_modifiers.push(*keysym);
        }

        let mut result = self
            .send_keysym_raw(*resolved.last().expect("chord has a last key"))
            .await;
        for keysym in pressed_modifiers.iter().rev() {
            if let Err(error) = self.send_keysym_state(*keysym, KeyState::Released).await
                && result.is_ok()
            {
                result = Err(error);
            }
        }
        result
    }

    async fn run_eis_action_with_retry(
        &self,
        action: EisAction,
    ) -> Result<String, EisOperationError> {
        match self.run_eis_action_once(action.clone()).await {
            Ok(details) => Ok(details),
            Err(failure)
                if failure.established
                    && failure.error.code != BackendErrorCode::InvalidRequest.as_str() =>
            {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_SESSION_REBUILT,
                    message: "Rebuilding the cached RemoteDesktop session after EIS input failed."
                        .to_string(),
                    details: Some(failure.error.message.clone()),
                })
                .await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                self.reset_session().await;
                self.run_eis_action_once(action).await
            }
            Err(failure) if !failure.established => {
                // The EIS worker thread likely died (SendError or oneshot drop).
                // Clear the stale cached handle so the retry can rebuild it.
                {
                    let mut state = self.inner.write().await;
                    if let Some(session) = state.session.as_mut() {
                        session.eis_worker = None;
                    }
                }
                self.run_eis_action_once(action).await
            }
            Err(failure) => Err(failure),
        }
    }

    async fn run_eis_action_once(&self, action: EisAction) -> Result<String, EisOperationError> {
        let worker = self.eis_worker().await?;
        worker.execute(action).await
    }

    async fn eis_worker(&self) -> Result<EisWorkerHandle, EisOperationError> {
        // Ensure the portal session exists without holding the state lock.
        // The full startup handshake can take 12+ seconds; holding the lock
        // would block every other portal-backed operation.
        self.ensure_session_started()
            .await
            .map_err(|error| EisOperationError {
                error,
                established: false,
            })?;

        let (fd, session_generation) = {
            let mut state = self.inner.write().await;
            let session_generation = state.session_generation;
            let Some(session) = state.session.as_mut() else {
                return Err(EisOperationError {
                    error: BackendError::new(
                        BackendErrorCode::Internal,
                        "portal session was reset while acquiring the EIS worker",
                    ),
                    established: false,
                });
            };
            if let Some(worker) = session.eis_worker.as_ref() {
                return Ok(worker.clone());
            }

            let fd = session
                .remote_desktop
                .connect_to_eis(&session.session, ConnectToEISOptions::default())
                .await
                .map_err(|error| EisOperationError {
                    error: BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        format!("failed to connect RemoteDesktop session to EIS: {error}"),
                    ),
                    established: false,
                })?;
            (fd, session_generation)
        };

        let worker = spawn_eis_worker(fd)
            .await
            .map_err(|error| EisOperationError {
                error,
                established: false,
            })?;

        let mut state = self.inner.write().await;
        if state.session_generation != session_generation {
            return Err(EisOperationError {
                error: BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    "RemoteDesktop session changed while starting the EIS worker",
                ),
                established: false,
            });
        }
        let Some(session) = state.session.as_mut() else {
            return Err(EisOperationError {
                error: BackendError::new(
                    BackendErrorCode::Internal,
                    "portal session was reset while starting the EIS worker",
                ),
                established: false,
            });
        };
        if let Some(worker) = session.eis_worker.as_ref() {
            return Ok(worker.clone());
        }
        session.eis_worker = Some(worker.clone());
        Ok(worker)
    }
}

#[cfg(test)]
mod tests {
    use super::eis_pointer_input_enabled_from;

    #[test]
    fn eis_pointer_policy_defaults_to_enabled() {
        assert!(eis_pointer_input_enabled_from(None));
        assert!(eis_pointer_input_enabled_from(Some("auto")));
        assert!(eis_pointer_input_enabled_from(Some("always")));
        assert!(eis_pointer_input_enabled_from(Some("unknown-value")));
    }

    #[test]
    fn eis_pointer_policy_disables_on_explicit_opt_out() {
        assert!(!eis_pointer_input_enabled_from(Some("never")));
        assert!(!eis_pointer_input_enabled_from(Some(" off ")));
        assert!(!eis_pointer_input_enabled_from(Some("0")));
        assert!(!eis_pointer_input_enabled_from(Some("FALSE")));
    }
}
