mod bridge;
mod cdp;
mod coordinates;
mod diagnostics;
mod executor;
mod probe;
mod protocol;
mod session;
mod snapshot;
mod sockets;
mod status;
mod tabs;
mod transport;

pub(crate) use bridge::{
    browser_bridge_diagnostics, browser_env_values_present, claim_tab, click, list_tabs,
    move_mouse, navigate, open_tab, press_key, screenshot, scroll, snapshot, type_text,
};
pub(crate) use status::{browser_status_from_deferred_doctor, browser_status_from_doctor};
