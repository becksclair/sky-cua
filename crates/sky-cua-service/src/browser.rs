mod activity;
mod affinity;
mod bridge;
mod cdp;
mod coordinates;
mod diagnostics;
mod executor;
mod keepalive;
mod model_image;
mod probe;
mod protocol;
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
    browser_bridge_diagnostics, browser_env_values_present, claim_tab, click, eval_with_policy,
    list_tabs, move_mouse, navigate, open_tab, press_key, screenshot, scroll, snapshot, type_text,
};
pub(crate) use status::{browser_status_from_deferred_doctor, browser_status_from_doctor};
