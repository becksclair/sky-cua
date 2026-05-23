use std::time::Duration;

use ashpd::desktop::PersistMode;
use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use chrono::Utc;
use enumflags2::BitFlags;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use tracing::{debug, warn};

use crate::portal::preauthorize;
use crate::portal::remote_desktop::{
    ActiveRemoteDesktopSession, PORTAL_SESSION_RESTORE_MISS, PORTAL_SESSION_RESTORED,
    PORTAL_SESSION_STARTED, PORTAL_SESSION_TOKEN_ROTATED, PortalLifecycleEvent, PortalStreamInfo,
};
use crate::portal::session::portal_u32_property;
use crate::portal::token_store::{
    PersistedPortalToken, PortalTokenStore, current_compositor_hint,
    portal_token_compositor_mismatch,
};

const SESSION_START_TIMEOUT: Duration = Duration::from_secs(12);
const PORTAL_PREAUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(5);
const PORTAL_SESSION_INPUT_SETTLE_DELAY: Duration = Duration::from_millis(120);
const CURSOR_MODE_HIDDEN: u32 = 1;
const CURSOR_MODE_EMBEDDED: u32 = 2;
const CURSOR_MODE_METADATA: u32 = 4;

#[derive(Debug)]
pub(crate) struct StartedPortalSession {
    pub session: ActiveRemoteDesktopSession,
    pub lifecycle_events: Vec<PortalLifecycleEvent>,
}

pub(crate) async fn start_session_with_timeout(
    token_store: Option<&PortalTokenStore>,
) -> Result<StartedPortalSession, BackendError> {
    preauthorize_with_timeout(token_store).await;
    start_session(token_store).await
}

async fn start_session(
    token_store: Option<&PortalTokenStore>,
) -> Result<StartedPortalSession, BackendError> {
    debug!("starting combined RemoteDesktop + ScreenCast portal session");
    let mut lifecycle_events = Vec::new();
    let remote_desktop = RemoteDesktop::new().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::PortalUnavailable,
            format!("failed to create RemoteDesktop portal proxy: {error}"),
        )
    })?;
    let screencast = Screencast::new().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::PortalUnavailable,
            format!("failed to create ScreenCast portal proxy: {error}"),
        )
    })?;

    let session = remote_desktop
        .create_session(Default::default())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("failed to create a combined RemoteDesktop session: {error}"),
            )
        })?;

    let stored_token = load_stored_token(token_store, &mut lifecycle_events);

    let setup_result = tokio::time::timeout(SESSION_START_TIMEOUT, async {
        remote_desktop
            .select_devices(
                &session,
                SelectDevicesOptions::default()
                    .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                    .set_restore_token(
                        stored_token
                            .as_ref()
                            .map(|record| record.restore_token.as_str()),
                    )
                    .set_persist_mode(PersistMode::ExplicitlyRevoked),
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::PortalRequestDenied,
                    format!("failed to request remote-control devices from the portal: {error}"),
                )
            })?
            .response()
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::PortalRequestDenied,
                    format!("portal device selection did not complete successfully: {error}"),
                )
            })?;

        let mut source_options = SelectSourcesOptions::default()
            .set_sources(BitFlags::from_flag(SourceType::Monitor))
            .set_multiple(false)
            .set_persist_mode(PersistMode::DoNot);
        if let Some(cursor_mode) = supported_cursor_mode().await {
            source_options = source_options.set_cursor_mode(cursor_mode);
        }

        screencast
            .select_sources(&session, source_options)
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::PortalRequestDenied,
                    format!("failed to request screencast sources from the portal: {error}"),
                )
            })?
            .response()
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::PortalRequestDenied,
                    format!("portal source selection did not complete successfully: {error}"),
                )
            })?;

        remote_desktop
            .start(&session, None, Default::default())
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::PortalRequestDenied,
                    format!("failed to start the RemoteDesktop session: {error}"),
                )
            })?
            .response()
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::PortalRequestDenied,
                    format!("the RemoteDesktop session did not start successfully: {error}"),
                )
            })
    })
    .await;

    let selected = match setup_result {
        Ok(Ok(selected)) => selected,
        Ok(Err(error)) => {
            close_session_after_setup_failure(&session).await;
            return Err(error);
        }
        Err(_) => {
            close_session_after_setup_failure(&session).await;
            return Err(BackendError::new(
                BackendErrorCode::PortalApprovalPending,
                "timed out waiting for the RemoteDesktop portal session to start; the approval prompt may still be waiting for user input",
            ));
        }
    };

    let primary_stream = selected.streams().first().map(stream_to_info);

    if stored_token.is_some() {
        lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_RESTORED,
            message:
                "Reused a persisted RemoteDesktop approval token for the combined portal session."
                    .to_string(),
            details: None,
        });
    } else {
        lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_STARTED,
            message: "Started a new combined RemoteDesktop and ScreenCast portal session."
                .to_string(),
            details: None,
        });
    }

    let remote_desktop_version = remote_desktop.version();
    let screencast_version = screencast.version();
    persist_selected_restore_token(
        token_store,
        stored_token.as_ref(),
        selected.restore_token(),
        &mut lifecycle_events,
        Some(remote_desktop_version),
        Some(screencast_version),
    )
    .await;

    tokio::time::sleep(PORTAL_SESSION_INPUT_SETTLE_DELAY).await;

    Ok(StartedPortalSession {
        session: ActiveRemoteDesktopSession {
            remote_desktop,
            screencast,
            session,
            primary_stream,
            pipewire_remote_fd: None,
            eis_worker: None,
        },
        lifecycle_events,
    })
}

async fn close_session_after_setup_failure(session: &ashpd::desktop::Session<RemoteDesktop>) {
    if let Err(error) = session.close().await {
        warn!(
            error = %error,
            "failed to close partially configured RemoteDesktop portal session after setup failure"
        );
    }
}

async fn supported_cursor_mode() -> Option<CursorMode> {
    match portal_u32_property("org.freedesktop.portal.ScreenCast", "AvailableCursorModes").await {
        Ok(mask) => cursor_mode_for_available_modes(mask).or_else(|| {
            warn!(
                available_cursor_modes = mask,
                "portal reported no supported cursor modes; omitting cursor mode from ScreenCast SelectSources"
            );
            None
        }),
        Err(error) => {
            warn!(
                error = %error,
                "could not read portal cursor modes; falling back to metadata cursor mode"
            );
            Some(CursorMode::Metadata)
        }
    }
}

fn cursor_mode_for_available_modes(mask: u32) -> Option<CursorMode> {
    if mask & CURSOR_MODE_METADATA != 0 {
        Some(CursorMode::Metadata)
    } else if mask & CURSOR_MODE_EMBEDDED != 0 {
        Some(CursorMode::Embedded)
    } else if mask & CURSOR_MODE_HIDDEN != 0 {
        Some(CursorMode::Hidden)
    } else {
        None
    }
}

fn load_stored_token(
    token_store: Option<&PortalTokenStore>,
    lifecycle_events: &mut Vec<PortalLifecycleEvent>,
) -> Option<PersistedPortalToken> {
    let token_store = token_store?;
    match token_store.load() {
        Ok(Some(record)) => {
            if let Some(details) = portal_token_compositor_mismatch(&record) {
                lifecycle_events.push(PortalLifecycleEvent {
                    code: PORTAL_SESSION_RESTORE_MISS,
                    message: "Persisted portal restore token belongs to another compositor; falling back to a fresh portal session."
                        .to_string(),
                    details: Some(details),
                });
                None
            } else {
                Some(record)
            }
        }
        Ok(None) => None,
        Err(error) => {
            lifecycle_events.push(PortalLifecycleEvent {
                code: PORTAL_SESSION_RESTORE_MISS,
                message: "Could not load the persisted portal restore token; falling back to a fresh portal session."
                    .to_string(),
                details: Some(error.to_string()),
            });
            None
        }
    }
}

async fn persist_selected_restore_token(
    token_store: Option<&PortalTokenStore>,
    previous_record: Option<&PersistedPortalToken>,
    restore_token: Option<&str>,
    lifecycle_events: &mut Vec<PortalLifecycleEvent>,
    remote_desktop_version: Option<u32>,
    screencast_version: Option<u32>,
) {
    let Some(token_store) = token_store else {
        return;
    };
    let Some(restore_token) = restore_token else {
        if previous_record.is_some() {
            lifecycle_events.push(PortalLifecycleEvent {
                code: PORTAL_SESSION_RESTORE_MISS,
                message: "Portal startup succeeded but did not return a replacement restore token."
                    .to_string(),
                details: None,
            });
        }
        return;
    };

    let record = PersistedPortalToken {
        restore_token: restore_token.to_string(),
        updated_at: Utc::now(),
        xdg_session_type: std::env::var("XDG_SESSION_TYPE").ok(),
        compositor: current_compositor_hint(),
        remote_desktop_version,
        screencast_version,
    };

    match token_store.save(&record) {
        Ok(()) => lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_TOKEN_ROTATED,
            message: match previous_record {
                Some(previous) if previous.restore_token != record.restore_token => {
                    "Rotated the persisted RemoteDesktop restore token for future sessions."
                        .to_string()
                }
                Some(_) => {
                    "Refreshed the persisted RemoteDesktop restore token for future sessions."
                        .to_string()
                }
                None => "Stored a persisted RemoteDesktop restore token for future sessions."
                    .to_string(),
            },
            details: Some(format!("token_path={}", token_store.path().display())),
        }),
        Err(error) => lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_RESTORE_MISS,
            message:
                "Portal startup succeeded but the replacement restore token could not be persisted."
                    .to_string(),
            details: Some(error.to_string()),
        }),
    }
}

pub(crate) async fn preauthorize_with_timeout(token_store: Option<&PortalTokenStore>) {
    match tokio::time::timeout(
        PORTAL_PREAUTHORIZATION_TIMEOUT,
        preauthorize::preauthorize_remote_desktop(token_store),
    )
    .await
    {
        Ok(()) => {}
        Err(_) => warn!(
            "timed out preauthorizing RemoteDesktop portal state; falling back to the normal portal startup path"
        ),
    }
}

fn stream_to_info(stream: &ashpd::desktop::screencast::Stream) -> PortalStreamInfo {
    use sky_cua_platform::model::{CoordinateSpace, RectF};
    let logical_rect = match (stream.position(), stream.size()) {
        (Some((x, y)), Some((width, height))) => Some(RectF {
            x: f64::from(x),
            y: f64::from(y),
            width: f64::from(width),
            height: f64::from(height),
            space: CoordinateSpace::DesktopLogical,
        }),
        _ => None,
    };

    PortalStreamInfo {
        node_id: stream.pipe_wire_node_id(),
        stream_id: stream.id().map(ToOwned::to_owned),
        mapping_id: stream.mapping_id().map(ToOwned::to_owned),
        source_type: stream.source_type().map(source_type_code),
        logical_rect,
    }
}

fn source_type_code(source_type: SourceType) -> u32 {
    match source_type {
        SourceType::Monitor => 1,
        SourceType::Window => 2,
        SourceType::Virtual => 4,
    }
}

#[cfg(test)]
mod tests {
    use ashpd::desktop::screencast::CursorMode;

    use super::cursor_mode_for_available_modes;

    #[test]
    fn chooses_supported_portal_cursor_mode() {
        assert!(matches!(
            cursor_mode_for_available_modes(4),
            Some(CursorMode::Metadata)
        ));
        assert!(matches!(
            cursor_mode_for_available_modes(2),
            Some(CursorMode::Embedded)
        ));
        assert!(matches!(
            cursor_mode_for_available_modes(1),
            Some(CursorMode::Hidden)
        ));
        // Embedded is preferred over Hidden when both are available.
        assert!(matches!(
            cursor_mode_for_available_modes(3),
            Some(CursorMode::Embedded)
        ));
        assert!(cursor_mode_for_available_modes(0).is_none());
    }
}
