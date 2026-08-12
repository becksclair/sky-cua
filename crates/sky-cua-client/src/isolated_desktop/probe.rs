//! Pure parsers and the dependency probe for the isolated xpra desktop.
//!
//! Everything here is stateless: display-string and `xpra`/`xdpyinfo` output
//! parsers plus the `PATH` dependency probe. None of it spawns a process; the
//! lifecycle/teardown surface that does lives in the parent module. The parent
//! is `#[cfg(unix)]` (declared in `main.rs`), so this submodule inherits that
//! gate and needs none of its own.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};

pub(super) const AT_SPI_BUS_LAUNCHER_PATH: &str = "/usr/lib/at-spi-bus-launcher";
pub(super) const AT_SPI_REGISTRY_PATH: &str = "/usr/lib/at-spi2-registryd";

/// xpra `info` key carrying the sandbox session-bus address under xpra's
/// default dbus launch. Confirmed by the Milestone 1b spike.
pub(super) const XPRA_INFO_DBUS_ADDRESS_KEY: &str = "dbus.env.DBUS_SESSION_BUS_ADDRESS";
/// xpra `info` key carrying the Xauthority file used by its Xorg server.
pub(super) const XPRA_INFO_XAUTHORITY_KEY: &str = "env.XAUTHORITY";

/// Settled virtual-display geometry, read after the server stops reporting the
/// transient startup mode.
///
/// Declared `pub` here only so the parent module can `pub use` it as part of the
/// crate's public API (it is a `IsolatedDesktopHandle::geometry()` return type);
/// this `probe` submodule is itself private, so the type is not exposed twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayGeometry {
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Display for DisplayGeometry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}x{}", self.width, self.height)
    }
}

/// The external binaries the isolated desktop requires, named so a structured
/// diagnostic can report exactly which one is missing rather than surfacing a
/// generic spawn failure. The order is the dependency-check order in
/// [`ensure_dependencies`].
#[allow(dead_code)] // only reachable via the test-only `missing()` order assertion
const REQUIRED_DEPENDENCIES: &[&str] = &[
    "xpra",
    "openbox",
    "xdotool",
    #[cfg(target_os = "linux")]
    AT_SPI_BUS_LAUNCHER_PATH,
    #[cfg(target_os = "linux")]
    AT_SPI_REGISTRY_PATH,
];

/// Presence of the external binaries isolated mode depends on. Reported as
/// structured fields (not prose) so a client learns the truth from the status
/// surface. A `false` for any required binary means isolated mode cannot start.
///
/// Declared `pub` here only so the parent module can `pub use` it as part of the
/// crate's public API (it is a public field of `IsolatedDesktopStatus`); this
/// `probe` submodule is itself private, so the type is not exposed twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsolatedDesktopDependencies {
    /// `xpra` (the virtual X server and the read-only viewer).
    pub xpra: bool,
    /// `openbox` (the in-sandbox window manager). The configured window manager
    /// may differ; this probes the default and most common one.
    pub openbox: bool,
    /// `xdotool` (XTest pointer/keyboard injection and X11 window queries).
    pub xdotool: bool,
    /// Fixed-path AT-SPI accessibility-bus launcher from `at-spi2-core`.
    pub at_spi_bus_launcher: bool,
    /// Fixed-path AT-SPI registry daemon from `at-spi2-core`.
    pub at_spi_registry: bool,
}

impl IsolatedDesktopDependencies {
    /// Probe the host `PATH` for each required binary.
    pub fn probe() -> Self {
        Self::from_lookup(dependency_present)
    }

    /// Pure constructor over a name-presence predicate, so the presence logic is
    /// unit-testable without a real `PATH`.
    fn from_lookup(present: impl Fn(&str) -> bool) -> Self {
        Self {
            xpra: present("xpra"),
            openbox: present("openbox"),
            xdotool: present("xdotool"),
            at_spi_bus_launcher: present(AT_SPI_BUS_LAUNCHER_PATH),
            at_spi_registry: present(AT_SPI_REGISTRY_PATH),
        }
    }

    /// Whether every required binary is present.
    #[allow(dead_code)] // only reachable via the unit tests and the hidden `isolated-desktop` subcommand
    pub fn all_present(&self) -> bool {
        self.xpra && self.openbox && self.xdotool && {
            #[cfg(target_os = "linux")]
            {
                self.at_spi_bus_launcher && self.at_spi_registry
            }
            #[cfg(not(target_os = "linux"))]
            {
                true
            }
        }
    }

    /// The names of the missing required binaries, in [`REQUIRED_DEPENDENCIES`]
    /// order. Empty when all are present.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.xpra {
            missing.push("xpra");
        }
        if !self.openbox {
            missing.push("openbox");
        }
        if !self.xdotool {
            missing.push("xdotool");
        }
        #[cfg(target_os = "linux")]
        {
            if !self.at_spi_bus_launcher {
                missing.push(AT_SPI_BUS_LAUNCHER_PATH);
            }
            if !self.at_spi_registry {
                missing.push(AT_SPI_REGISTRY_PATH);
            }
        }
        missing
    }
}

fn dependency_present(name: &str) -> bool {
    if name.contains('/') {
        std::fs::metadata(name)
            .map(|metadata| {
                use std::os::unix::fs::PermissionsExt as _;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false)
    } else {
        command_on_path(name)
    }
}

/// Whether `name` is an executable on the host `PATH`. Mirrors the
/// `command_exists` idiom used by the Linux backend's helper-spawn sites.
fn command_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| directory.join(name).exists())
}

/// Fail with a structured, dependency-naming error when a required binary is
/// absent, so the fail-closed error in `connect_or_spawn` is actionable rather
/// than a generic spawn failure deep inside `start_xpra_desktop`.
pub(super) fn ensure_dependencies() -> Result<()> {
    let deps = IsolatedDesktopDependencies::probe();
    let missing = deps.missing();
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "isolated desktop dependencies are missing: {}. Install the packages \
         providing them; command dependencies must be on PATH",
        missing.join(", "),
    );
}

/// Parse the numeric display from a `":N"` string (or a bare `"N"`). Returns
/// `None` for anything that is not a non-negative integer display.
pub(super) fn parse_display_number(display: &str) -> Option<u32> {
    display.trim().trim_start_matches(':').parse::<u32>().ok()
}

/// Read the `/tmp/.X*-lock` file names present on the host. Missing `/tmp` or
/// an empty directory yields an empty set rather than an error.
pub(super) fn read_x_lock_names() -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let entries = match std::fs::read_dir("/tmp") {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(error) => return Err(error).context("failed to scan /tmp for X11 display locks"),
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            names.insert(name.to_string());
        }
    }
    Ok(names)
}

/// Pick the first free display number not represented by a `.X<N>-lock` entry,
/// starting from 100 (the conventional headless range that avoids the user's
/// real `:0`/`:1` session). Pure over a directory listing so it is testable.
pub(super) fn first_free_display_number(lock_names: &BTreeSet<String>) -> u32 {
    let taken: BTreeSet<u32> = lock_names
        .iter()
        .filter_map(|name| x_lock_display_number(name))
        .collect();
    (100u32..)
        .find(|candidate| !taken.contains(candidate))
        .expect("the 32-bit display space cannot be exhausted")
}

/// Extract the display number from an X lock file name `".X<N>-lock"`.
fn x_lock_display_number(name: &str) -> Option<u32> {
    name.strip_prefix(".X")
        .and_then(|rest| rest.strip_suffix("-lock"))
        .and_then(|number| number.parse::<u32>().ok())
}

/// Parse a `"<width>x<height>"` resolution string into a [`DisplayGeometry`].
/// Returns `None` for unparseable strings (the settle loop then falls back to
/// stability detection).
pub(super) fn parse_resolution(resolution: &str) -> Option<DisplayGeometry> {
    let (width, height) = resolution.trim().split_once('x')?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some(DisplayGeometry { width, height })
}

/// Parse `"  dimensions:    1920x1080 pixels (508x286 millimeters)"` from
/// `xdpyinfo` output into a [`DisplayGeometry`].
pub(super) fn parse_xdpyinfo_dimensions(output: &str) -> Option<DisplayGeometry> {
    for line in output.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("dimensions:") else {
            continue;
        };
        let token = rest.split_whitespace().next()?;
        let (width, height) = token.split_once('x')?;
        let width = width.trim().parse::<u32>().ok()?;
        let height = height.trim().parse::<u32>().ok()?;
        return Some(DisplayGeometry { width, height });
    }
    None
}

/// Whether `xpra list` output reports `display` as a live session. Pure over
/// the captured text so it is testable without a real xpra.
pub(super) fn xpra_list_has_live_display(output: &str, display: &str) -> bool {
    let number = match parse_display_number(display) {
        Some(number) => number,
        None => return false,
    };
    let normalized = format!(":{number}");
    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        // A dead/unknown session line names the display but is not "LIVE";
        // require the live marker and the exact display token.
        if line_names_display(line, &normalized) && lower.contains("live") {
            return true;
        }
    }
    false
}

/// Whether a `xpra list` line names exactly `:N` as a standalone token (so
/// `:10` does not match a query for `:1`).
fn line_names_display(line: &str, normalized_display: &str) -> bool {
    line.split(|ch: char| ch.is_whitespace() || ch == ',')
        .any(|token| token == normalized_display)
}

/// Parse the sandbox session-bus address from `xpra info` output. Pure over the
/// captured text. Reads ONLY the authoritative `dbus.env.DBUS_SESSION_BUS_ADDRESS`
/// key, which xpra emits for the private bus its own `--dbus-launch` created.
/// The plain `env.DBUS_SESSION_BUS_ADDRESS` key is deliberately NOT used: it
/// reflects the xpra server's process environment, which on the user's live
/// session is the HOST session bus — adopting it would pin the isolated daemon
/// (and every app it launches) to the user's real D-Bus and re-open the KDE/GNOME
/// single-instance escape this feature closes. When `dbus.env.` is absent (xpra
/// did not launch a private bus), this returns `None` and `ensure` falls through
/// to a client-owned `dbus-daemon`.
pub(super) fn parse_xpra_info_dbus_address(output: &str) -> Option<String> {
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != XPRA_INFO_DBUS_ADDRESS_KEY {
            continue;
        }
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Parse the Xauthority file used by the xpra server from `xpra info` output.
/// The isolated daemon must inherit this exact value or X clients cannot
/// authenticate to the private display.
pub(super) fn parse_xpra_info_xauthority(output: &str) -> Option<String> {
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != XPRA_INFO_XAUTHORITY_KEY {
            continue;
        }
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_desktop_parses_display_numbers() {
        assert_eq!(parse_display_number(":100"), Some(100));
        assert_eq!(parse_display_number("100"), Some(100));
        assert_eq!(parse_display_number(" :7 "), Some(7));
        assert_eq!(parse_display_number(":auto"), None);
        assert_eq!(parse_display_number("auto"), None);
        assert_eq!(parse_display_number(""), None);
    }

    #[test]
    fn isolated_desktop_extracts_x_lock_display_number() {
        assert_eq!(x_lock_display_number(".X0-lock"), Some(0));
        assert_eq!(x_lock_display_number(".X100-lock"), Some(100));
        assert_eq!(x_lock_display_number(".X100"), None);
        assert_eq!(x_lock_display_number("X100-lock"), None);
        assert_eq!(x_lock_display_number(".Xabc-lock"), None);
    }

    #[test]
    fn isolated_desktop_scans_for_first_free_display() {
        // Empty /tmp: the conventional headless base (100) is free.
        let empty = BTreeSet::new();
        assert_eq!(first_free_display_number(&empty), 100);

        // The user's real :0 lock does not push the headless choice up.
        let with_real_session = BTreeSet::from([".X0-lock".to_string()]);
        assert_eq!(first_free_display_number(&with_real_session), 100);

        // 100 and 101 taken, plus unrelated entries: 102 is chosen.
        let crowded = BTreeSet::from([
            ".X0-lock".to_string(),
            ".X100-lock".to_string(),
            ".X101-lock".to_string(),
            "not-a-lock".to_string(),
            ".X103-lock".to_string(),
        ]);
        assert_eq!(first_free_display_number(&crowded), 102);
    }

    #[test]
    fn isolated_desktop_parses_xdpyinfo_dimensions() {
        let output = "\
name of display:    :100.0
version number:    11.0
  dimensions:    1920x1080 pixels (508x286 millimeters)
  resolution:    96x96 dots per inch
";
        assert_eq!(
            parse_xdpyinfo_dimensions(output),
            Some(DisplayGeometry {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(parse_xdpyinfo_dimensions("no dimensions here"), None);
    }

    #[test]
    fn isolated_desktop_parses_xpra_info_dbus_address() {
        // The authoritative `dbus.env.` key wins, matching the spike output.
        let output = "\
dbus.env.DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/dbus-XvTZ9lxXuj,guid=212fe6c7
dbus.env.DBUS_SESSION_BUS_PID=4133936
env.DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/dbus-XvTZ9lxXuj,guid=212fe6c7
env.DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket
";
        assert_eq!(
            parse_xpra_info_dbus_address(output),
            Some("unix:path=/tmp/dbus-XvTZ9lxXuj,guid=212fe6c7".to_string())
        );

        // When only the plain `env.` key is present (no authoritative
        // `dbus.env.`), return None so `ensure` falls through to a client-owned
        // dbus-daemon. `env.` reflects the xpra server's OWN process environment,
        // which on the user's session is the HOST bus; adopting it would pin the
        // sandbox to the user's real D-Bus and re-open the single-instance escape.
        let env_only = "env.DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/dbus-host\n";
        assert_eq!(parse_xpra_info_dbus_address(env_only), None);

        // No dbus field at all yields None (drives the owned-bus fallback).
        assert_eq!(parse_xpra_info_dbus_address("subsystems='glib'\n"), None);
        // An empty value is ignored rather than returned.
        assert_eq!(
            parse_xpra_info_dbus_address("dbus.env.DBUS_SESSION_BUS_ADDRESS=\n"),
            None
        );
    }

    #[test]
    fn isolated_desktop_parses_xpra_info_xauthority() {
        assert_eq!(
            parse_xpra_info_xauthority(
                "env.DISPLAY=:100\nenv.XAUTHORITY=/run/user/1000/xpra/Xauthority\n"
            ),
            Some("/run/user/1000/xpra/Xauthority".to_string())
        );
        assert_eq!(parse_xpra_info_xauthority("env.XAUTHORITY=\n"), None);
        assert_eq!(parse_xpra_info_xauthority("env.DISPLAY=:100\n"), None);
    }

    #[test]
    fn isolated_desktop_detects_live_display_in_xpra_list() {
        let output = "\
Found the following xpra sessions:
/run/user/1000/xpra:
\t:100\tLIVE\tsession_name
\t:101\tDEAD\tstale_session
";
        assert!(xpra_list_has_live_display(output, ":100"));
        // DEAD sessions are not reused.
        assert!(!xpra_list_has_live_display(output, ":101"));
        // A display not present is not live.
        assert!(!xpra_list_has_live_display(output, ":102"));
        // No false positive for the empty-session message.
        assert!(!xpra_list_has_live_display(
            "No xpra sessions found",
            ":100"
        ));
    }

    #[test]
    fn isolated_desktop_detects_live_display_in_real_xpra_list_format() {
        // The xpra 6.4 `list` line phrasing observed on the development host;
        // the display number is the trailing token, not a tab-separated column.
        let output = "\tLIVE session at :100\n";
        assert!(xpra_list_has_live_display(output, ":100"));
        assert!(!xpra_list_has_live_display(output, ":10"));
        assert!(!xpra_list_has_live_display(output, ":101"));
    }

    #[test]
    fn isolated_desktop_parses_resolution() {
        assert_eq!(
            parse_resolution("1920x1080"),
            Some(DisplayGeometry {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(
            parse_resolution(" 2560x1440 "),
            Some(DisplayGeometry {
                width: 2560,
                height: 1440
            })
        );
        assert_eq!(parse_resolution("1920"), None);
        assert_eq!(parse_resolution("0x1080"), None);
        assert_eq!(parse_resolution("widexhigh"), None);
    }

    #[test]
    fn isolated_desktop_live_display_token_match_is_exact() {
        // `:10` LIVE must not satisfy a query for `:1`.
        let output = "\t:10\tLIVE\tsession\n";
        assert!(xpra_list_has_live_display(output, ":10"));
        assert!(!xpra_list_has_live_display(output, ":1"));
    }

    #[test]
    fn isolated_desktop_dependencies_report_presence() {
        // Everything present: all_present is true and nothing is missing.
        let all = IsolatedDesktopDependencies::from_lookup(|_| true);
        assert_eq!(
            all,
            IsolatedDesktopDependencies {
                xpra: true,
                openbox: true,
                xdotool: true,
                at_spi_bus_launcher: true,
                at_spi_registry: true,
            }
        );
        assert!(all.all_present());
        assert!(all.missing().is_empty());

        // Nothing present: every required binary is reported missing in order.
        let none = IsolatedDesktopDependencies::from_lookup(|_| false);
        assert!(!none.all_present());
        assert_eq!(none.missing(), REQUIRED_DEPENDENCIES);

        // A single absent binary is named precisely.
        let no_xdotool = IsolatedDesktopDependencies::from_lookup(|name| name != "xdotool");
        assert!(!no_xdotool.all_present());
        assert_eq!(no_xdotool.missing(), vec!["xdotool"]);
        assert!(no_xdotool.xpra);
        assert!(no_xdotool.openbox);
        assert!(!no_xdotool.xdotool);
        assert!(no_xdotool.at_spi_bus_launcher);
        assert!(no_xdotool.at_spi_registry);
    }

    #[test]
    fn isolated_desktop_at_spi_dependency_gating_is_target_conditioned() {
        let no_at_spi = IsolatedDesktopDependencies::from_lookup(|name| {
            name != AT_SPI_BUS_LAUNCHER_PATH && name != AT_SPI_REGISTRY_PATH
        });

        #[cfg(target_os = "linux")]
        {
            assert!(!no_at_spi.all_present());
            assert_eq!(
                no_at_spi.missing(),
                vec![AT_SPI_BUS_LAUNCHER_PATH, AT_SPI_REGISTRY_PATH]
            );
        }

        #[cfg(not(target_os = "linux"))]
        {
            assert!(no_at_spi.all_present());
            assert!(no_at_spi.missing().is_empty());
        }
    }

    #[test]
    fn isolated_desktop_required_dependencies_match_probe_fields() {
        // The named-constant list and the struct's `missing` order must agree so
        // diagnostics and the probe never drift apart.
        let none = IsolatedDesktopDependencies::from_lookup(|_| false);
        assert_eq!(none.missing(), REQUIRED_DEPENDENCIES);
    }
}
