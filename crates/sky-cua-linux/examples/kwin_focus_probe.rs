//! Live probe for KWin scripted active-window readback and verified
//! activation. Run on a KDE Plasma Wayland session:
//! `cargo run -p sky-cua-linux --example kwin_focus_probe`

use sky_cua_linux::{env_probe, kwin, kwin_script};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let environment = env_probe::probe_environment().await?;
    println!("compositor: {:?}", environment.compositor);

    let active = kwin_script::active_window().await?;
    println!("active window (script): {active:?}");

    let windows = kwin::discover_windows(&environment).await?;
    println!("discovered {} windows:", windows.len());
    for window in &windows {
        println!(
            "  focused={} {} {:?}",
            window.app.is_focused_candidate, window.window_id, window.app.window_title
        );
    }

    // Round-trip: activate the first non-focused window, verify, then
    // activate the originally focused window again.
    let originally_focused = windows
        .iter()
        .find(|window| window.app.is_focused_candidate)
        .cloned();
    if let Some(target) = windows
        .iter()
        .find(|window| !window.app.is_focused_candidate)
    {
        println!("activating {} ...", target.window_id);
        kwin::activate_window(&target.window_id).await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let now_active = kwin_script::active_window().await?;
        println!("active after activation: {now_active:?}");
        if let Some(back) = originally_focused {
            println!("restoring {} ...", back.window_id);
            kwin::activate_window(&back.window_id).await?;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            println!(
                "active after restore: {:?}",
                kwin_script::active_window().await?
            );
        }
    } else {
        println!("no second window available for the activation round-trip");
    }
    Ok(())
}
