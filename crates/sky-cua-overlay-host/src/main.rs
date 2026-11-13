use anyhow::{Context, Result};
use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostBackend, OverlayHostMessage, OverlayHostMessageKind,
    probe_environment_reply,
};
use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

mod serve;

fn main() -> Result<()> {
    serve::record_startup_parent();
    // The overlay host is a long-lived child of the service daemon; drop any
    // descriptors leaked down the launcher chain before serving.
    sky_cua_platform::fd_hygiene::close_inherited_fds();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "serve".to_string());
    match command.as_str() {
        "probe" => print_reply(&probe_environment_reply()),
        "set-cursor" => set_cursor_from_args(std::env::args().skip(2).collect()),
        "serve" => serve::serve_from_args(std::env::args().skip(2).collect()),
        "playground" => sky_cua_overlay_host::run_playground(std::env::args().skip(2).collect()),
        other => anyhow::bail!("unsupported sky-cua-overlay-host mode: {other}"),
    }
}

fn set_cursor_from_args(args: Vec<String>) -> Result<()> {
    let [x, y] = args.as_slice() else {
        anyhow::bail!("usage: sky-cua-overlay-host set-cursor <x> <y>");
    };
    let x = x.parse::<f64>().context("invalid x coordinate")?;
    let y = y.parse::<f64>().context("invalid y coordinate")?;
    let mut backend = OverlayHostBackend::from_env();
    let reply = backend.handle_message(OverlayHostMessage {
        version: OVERLAY_HOST_PROTOCOL_VERSION,
        kind: OverlayHostMessageKind::SetCursor,
        state: Some(AgentCursorState {
            visible: true,
            sequence: 0,
            model_point: Some(AgentCursorPoint {
                x,
                y,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: None,
            }),
            native_point: None,
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 0,
        }),
        gesture: None,
        sequence: None,
        reason: None,
        arrival_wait: None,
    });
    print_reply(&reply)
}

fn print_reply(reply: &sky_cua_overlay_host::OverlayHostReply) -> Result<()> {
    println!("{}", serde_json::to_string(reply)?);
    Ok(())
}
