use super::phone::phone_request_is_write;
use super::*;

impl ServiceDaemon {
    pub(super) async fn ensure_session_presence_for_request(&self, request: &ServiceRequest) {
        if !self.session_presence_config.enabled || !request_should_hold_presence(request) {
            return;
        }

        let mut held = self.session_presence_held.lock().await;
        if *held {
            return;
        }

        let intent = self.session_presence_config.intent();
        match self.backend.ensure_session_presence(intent).await {
            Ok(status) => {
                debug!(
                    backend = status.backend,
                    supported = status.supported,
                    detail = status.detail,
                    "session presence ensured"
                );
                *held = true;
            }
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "session presence ensure failed"
                );
            }
        }
    }

    pub(super) async fn release_idle_session_presence_if_needed(&self) {
        if !self.session_presence_config.enabled {
            return;
        }
        if self.sessions.idle_for().await < self.session_presence_config.idle_release {
            return;
        }

        let mut held = self.session_presence_held.lock().await;
        if !*held {
            return;
        }

        match self
            .backend
            .release_session_presence(self.session_presence_config.relock)
            .await
        {
            Ok(status) => {
                debug!(
                    backend = status.backend,
                    supported = status.supported,
                    detail = status.detail,
                    "session presence released after idle timeout"
                );
            }
            Err(error) => {
                debug!(
                    code = error.code,
                    message = error.message,
                    "session presence idle release failed"
                );
            }
        }
        *held = false;
    }
}

impl SessionPresenceConfig {
    const DEFAULT_IDLE_RELEASE_SECS: u64 = 90;

    pub(super) fn from_env() -> Self {
        Self {
            enabled: env_bool("SKY_CUA_PRESENCE_ENABLED", false),
            idle_release: Duration::from_secs(env_u64(
                "SKY_CUA_PRESENCE_IDLE_RELEASE_SECS",
                Self::DEFAULT_IDLE_RELEASE_SECS,
            )),
            unlock: env_bool("SKY_CUA_PRESENCE_UNLOCK", true),
            relock: env_bool("SKY_CUA_PRESENCE_RELOCK", true),
            inhibit_lock: env_bool("SKY_CUA_PRESENCE_INHIBIT_LOCK", true),
            inhibit_suspend: env_bool("SKY_CUA_PRESENCE_INHIBIT_SUSPEND", true),
        }
    }

    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self {
            enabled: false,
            idle_release: Duration::from_secs(Self::DEFAULT_IDLE_RELEASE_SECS),
            unlock: true,
            relock: true,
            inhibit_lock: true,
            inhibit_suspend: true,
        }
    }

    fn intent(&self) -> SessionPresenceIntent {
        SessionPresenceIntent {
            unlock: self.unlock,
            inhibit_lock: self.inhibit_lock,
            inhibit_suspend: self.inhibit_suspend,
        }
    }
}

pub(super) fn session_presence_disabled_response() -> ServiceResponse {
    error_response(
        sky_cua_platform::BackendErrorCode::ActionUnsupportedForEnvironment.as_str(),
        "session presence is disabled; set SKY_CUA_PRESENCE_ENABLED=1 on the daemon to allow \
         unlock and inhibitor requests",
    )
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

pub(super) fn request_should_hold_presence(request: &ServiceRequest) -> bool {
    match request {
        ServiceRequest::Health
        | ServiceRequest::Doctor
        | ServiceRequest::AgentCursorStatus
        | ServiceRequest::PhoneDirectCreateEnrollment
        | ServiceRequest::SessionPresence { .. }
        | ServiceRequest::CancelTurn { .. } => false,
        ServiceRequest::Browser { request, .. } => !matches!(
            request,
            BrowserRequest::Status | BrowserRequest::ListTabs { .. }
        ),
        // Phone control drives an Android device, but a write action (tap/swipe/
        // type/press, an app/notification mutation, connect/pair/install) is an
        // active operation the agent is mid-flow on, so it holds presence the same
        // way a desktop write does. Read-only phone perception (status, listing,
        // observe, screenshot, capability/companion queries) does not.
        ServiceRequest::Phone { request, .. } => phone_request_is_write(request),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cua_screenshot_acquires_automatic_session_presence() {
        assert!(request_should_hold_presence(
            &ServiceRequest::GetScreenshot {
                context: None,
                mouse_size_px: None,
            }
        ));
    }
}
