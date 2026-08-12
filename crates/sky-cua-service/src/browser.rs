mod activity;
mod affinity;
mod bridge;
mod cdp;
mod control_plane;
mod coordinates;
mod diagnostics;
mod executor;
mod keepalive;
mod model_image;
mod probe;
mod protocol;
mod readiness;
mod resolve;
mod session;
mod snapshot;
mod sockets;
mod status;
mod tabs;
mod transport;

#[cfg(test)]
mod tests;

pub(crate) use activity::{browser_session_lingering, mark_bridge_activity};
pub(crate) use bridge::{
    browser_bridge_diagnostics, browser_env_values_present, claim_tab_with_identity,
    click_element_with_identity, click_with_identity, eval_with_policy_and_identity,
    list_tabs_with_identity, move_mouse_with_identity, navigate_with_identity,
    observe_appshot_with_identity, open_tab_with_identity, press_key_with_identity,
    screenshot_with_identity, scroll_with_identity, snapshot_with_identity,
    type_text_element_with_identity, type_text_with_identity,
};
pub(crate) use control_plane::{BrowserControlRuntime, execute_high_level};
pub(crate) use status::{
    browser_status_deadline_exceeded, browser_status_from_deferred_doctor,
    browser_status_from_doctor,
};
pub(crate) use transport::{BrowserControlMode, browser_control_mode};
