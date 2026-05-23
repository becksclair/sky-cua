use ashpd::desktop::remote_desktop::{
    Axis, KeyState, NotifyKeyboardKeysymOptions, NotifyPointerAxisDiscreteOptions,
    NotifyPointerAxisOptions, NotifyPointerButtonOptions, NotifyPointerMotionAbsoluteOptions,
    RemoteDesktop,
};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use crate::portal::remote_desktop::{MouseButton, PortalStreamInfo, evdev_button};

pub(crate) async fn pointer_move_absolute(
    remote_desktop: &RemoteDesktop,
    session: &ashpd::desktop::Session<RemoteDesktop>,
    stream: &PortalStreamInfo,
    x: f64,
    y: f64,
) -> Result<(), BackendError> {
    remote_desktop
        .notify_pointer_motion_absolute(
            session,
            stream.node_id,
            x,
            y,
            NotifyPointerMotionAbsoluteOptions::default(),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to inject absolute pointer motion through the portal: {error}"),
            )
        })
}

pub(crate) async fn pointer_button(
    remote_desktop: &RemoteDesktop,
    session: &ashpd::desktop::Session<RemoteDesktop>,
    button: MouseButton,
    pressed: bool,
) -> Result<(), BackendError> {
    remote_desktop
        .notify_pointer_button(
            session,
            evdev_button(button),
            if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            },
            NotifyPointerButtonOptions::default(),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to inject pointer button through the portal: {error}"),
            )
        })
}

pub(crate) async fn scroll_vertical_discrete(
    remote_desktop: &RemoteDesktop,
    session: &ashpd::desktop::Session<RemoteDesktop>,
    steps: i32,
) -> Result<(), BackendError> {
    remote_desktop
        .notify_pointer_axis_discrete(
            session,
            Axis::Vertical,
            steps,
            NotifyPointerAxisDiscreteOptions::default(),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to inject vertical scroll through the portal: {error}"),
            )
        })
}

pub(crate) async fn scroll_vertical_smooth(
    remote_desktop: &RemoteDesktop,
    session: &ashpd::desktop::Session<RemoteDesktop>,
    delta_y: f64,
) -> Result<(), BackendError> {
    remote_desktop
        .notify_pointer_axis(
            session,
            0.0,
            delta_y,
            NotifyPointerAxisOptions::default().set_finish(true),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to inject smooth vertical scroll through the portal: {error}"),
            )
        })
}

pub(crate) async fn send_keysym_raw(
    remote_desktop: &RemoteDesktop,
    session: &ashpd::desktop::Session<RemoteDesktop>,
    keysym: i32,
) -> Result<(), BackendError> {
    // Fire press and release concurrently to halve the per-character D-Bus
    // round-trip latency on the legacy fallback path.
    let (press, release) = tokio::join!(
        send_keysym_state(remote_desktop, session, keysym, KeyState::Pressed),
        send_keysym_state(remote_desktop, session, keysym, KeyState::Released),
    );
    press?;
    release?;
    Ok(())
}

pub(crate) async fn send_keysym_state(
    remote_desktop: &RemoteDesktop,
    session: &ashpd::desktop::Session<RemoteDesktop>,
    keysym: i32,
    state: KeyState,
) -> Result<(), BackendError> {
    remote_desktop
        .notify_keyboard_keysym(
            session,
            keysym,
            state,
            NotifyKeyboardKeysymOptions::default(),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to inject keyboard keysym through the portal: {error}"),
            )
        })
}
