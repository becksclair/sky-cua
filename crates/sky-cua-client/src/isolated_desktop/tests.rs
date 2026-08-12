use super::owned_bus::{pid_is_dbus_daemon, read_owned_bus, remove_owned_bus_state};
use super::*;

#[test]
fn isolated_desktop_builds_spawn_env() {
    let env = build_spawn_env(
        ":100",
        "unix:path=/tmp/dbus-abc,guid=deadbeef",
        "/run/user/1000/xpra/Xauthority",
        Path::new("/run/user/1000/sky-cua/service-isolated-100.sock"),
    );
    let lookup = |key: &str| {
        env.iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(lookup("DISPLAY"), Some(":100"));
    assert_eq!(lookup("XDG_SESSION_TYPE"), Some("x11"));
    assert_eq!(lookup("QT_QPA_PLATFORM"), Some("xcb"));
    assert_eq!(lookup("GDK_BACKEND"), Some("x11"));
    assert_eq!(lookup("NO_AT_BRIDGE"), Some("0"));
    assert_eq!(lookup("ACCESSIBILITY_ENABLED"), Some("1"));
    assert_eq!(
        lookup("DBUS_SESSION_BUS_ADDRESS"),
        Some("unix:path=/tmp/dbus-abc,guid=deadbeef")
    );
    assert_eq!(lookup("XAUTHORITY"), Some("/run/user/1000/xpra/Xauthority"));
    assert_eq!(
        lookup("SKY_CUA_SERVICE_SOCKET_PATH"),
        Some("/run/user/1000/sky-cua/service-isolated-100.sock")
    );
    assert_eq!(
        lookup(CODEX_BROWSER_SOCKET_PATH_ENV),
        Some("/run/user/1000/sky-cua/service-isolated-100.codex-browser.sock")
    );
}

#[test]
fn isolated_desktop_removes_wayland_and_stale_atspi_bus_env() {
    assert_eq!(removed_env(), vec!["WAYLAND_DISPLAY", "AT_SPI_BUS_ADDRESS"]);
}

#[test]
fn isolated_desktop_socket_path_follows_runtime_dir_convention() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let old = std::env::var_os("XDG_RUNTIME_DIR");
    // SAFETY: serialized by ENV_LOCK; restored below.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000") };
    let path = isolated_socket_path(100).expect("socket path resolves");
    match old {
        Some(value) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", value) },
        None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
    }
    assert_eq!(
        path,
        PathBuf::from("/run/user/1000/sky-cua/service-isolated-100.sock")
    );
}

#[test]
fn isolated_desktop_socket_path_requires_runtime_dir() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let old = std::env::var_os("XDG_RUNTIME_DIR");
    // SAFETY: serialized by ENV_LOCK; restored below.
    unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };
    let result = isolated_socket_path(100);
    if let Some(value) = old {
        // SAFETY: serialized by ENV_LOCK; restores the pre-test value.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", value) }
    }
    assert!(result.is_err());
}

#[test]
fn owned_bus_state_round_trips_and_rejects_malformed() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let old = std::env::var_os("XDG_RUNTIME_DIR");
    let temp = std::env::temp_dir().join(format!("sky-cua-owned-bus-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    // SAFETY: serialized by ENV_LOCK; restored below.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &temp) };

    assert_eq!(read_owned_bus(123), None);
    persist_owned_bus(123, "unix:path=/run/sandbox-bus,guid=abc", 4242);
    assert_eq!(
        read_owned_bus(123),
        Some(("unix:path=/run/sandbox-bus,guid=abc".to_string(), 4242))
    );
    // An init/invalid owner pid is rejected as malformed.
    persist_owned_bus(124, "unix:path=/run/x", 1);
    assert_eq!(read_owned_bus(124), None);
    remove_owned_bus_state(123);
    assert_eq!(read_owned_bus(123), None);

    let _ = std::fs::remove_dir_all(&temp);
    match old {
        Some(value) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", value) },
        None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
    }
}

#[test]
fn recover_owned_bus_drops_a_stale_record() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let old = std::env::var_os("XDG_RUNTIME_DIR");
    let temp = std::env::temp_dir().join(format!("sky-cua-stale-bus-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    // SAFETY: serialized by ENV_LOCK; restored below.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &temp) };

    // Persist a record whose pid is THIS process — not a `dbus-daemon`, so the
    // recorded bus is treated as dead: `recover_owned_bus` returns `None` and
    // removes the stale record so the caller restarts rather than reusing it.
    persist_owned_bus(200, "unix:path=/run/dead-bus", std::process::id());
    assert_eq!(recover_owned_bus(200), None);
    assert_eq!(read_owned_bus(200), None);

    let _ = std::fs::remove_dir_all(&temp);
    match old {
        Some(value) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", value) },
        None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
    }
}

#[test]
fn pid_is_dbus_daemon_rejects_non_dbus_processes() {
    // This test binary is not a dbus-daemon; pid 1 (init/systemd) is not one;
    // a very high pid almost certainly does not exist. None should match.
    assert!(!pid_is_dbus_daemon(std::process::id()));
    assert!(!pid_is_dbus_daemon(1));
    assert!(!pid_is_dbus_daemon(u32::MAX - 1));
}

#[test]
fn parse_largest_connected_mode_picks_biggest_and_skips_disconnected() {
    let sample = "\
Screen 0: minimum 320 x 200, current 4480 x 1440, maximum 16384 x 16384
DP-3 connected primary 2560x1440+0+0 (normal left inverted right x axis y axis) 597mm x 336mm
   2560x1440    179.94*+
HDMI-1 connected 1920x1080+2560+0 (normal left inverted right x axis y axis) 510mm x 290mm
   1920x1080     60.00*+
DP-1 disconnected (normal left inverted right x axis y axis)
";
    assert_eq!(parse_largest_connected_mode(sample), Some((2560, 1440)));
}

#[test]
fn parse_largest_connected_mode_none_without_a_connected_output() {
    let sample = "Screen 0: current 0 x 0\nDP-1 disconnected (normal)\n";
    assert_eq!(parse_largest_connected_mode(sample), None);
}

#[test]
fn three_quarter_even_scales_and_floors_to_even() {
    assert_eq!(three_quarter_even(2560, 1440), (1920, 1080));
    assert_eq!(three_quarter_even(3840, 2160), (2880, 1620));
    // 1366*3/4 = 1024 (even); 766*3/4 floors 574.5 -> 574 (even).
    assert_eq!(three_quarter_even(1366, 766), (1024, 574));
    // 1362*3/4 floors to 1021 (odd) -> 1020; 768*3/4 = 576 (even).
    assert_eq!(three_quarter_even(1362, 768), (1020, 576));
}

#[test]
fn resolve_resolution_passes_explicit_values_through() {
    assert_eq!(resolve_resolution("1280x800"), "1280x800");
    assert_eq!(resolve_resolution("2560x1440"), "2560x1440");
}

/// Serializes the env-mutating socket-path tests so they cannot race on
/// `XDG_RUNTIME_DIR` under the parallel test runner.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
