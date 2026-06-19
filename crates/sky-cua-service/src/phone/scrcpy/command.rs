//! scrcpy launch-command construction and the codec retry policy.
//!
//! Deterministic argv with a sanitized window title (`sky-cua-phone-<safe-serial>`)
//! and the documented flags. H.265 launch failure retries with H.264, then with
//! no explicit codec. scrcpy's default SDK mouse mode is kept (no `--mouse=uhid`).
//! The actual process spawn is delegated through a closure so the launch policy
//! is unit-testable without spawning scrcpy.

use sky_cua_platform::config::{PhoneConfig, ResolvedPhoneSelection};
use sky_cua_platform::model::{DisplayInfo, primary_flagged_display};

/// Window-title prefix for every sky-cua-managed scrcpy window. The ownership
/// model uses this prefix to recognize windows it is allowed to stop.
pub(in crate::phone) const WINDOW_TITLE_PREFIX: &str = "sky-cua-phone-";

/// Codec attempts in the order they are tried. H.265 first for bandwidth, then
/// H.264, then no explicit codec (scrcpy default) as the last resort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::phone) enum ScrcpyCodec {
    /// `--video-codec=h265`.
    H265,
    /// `--video-codec=h264`.
    H264,
    /// No `--video-codec` flag; scrcpy chooses.
    Default,
}

impl ScrcpyCodec {
    /// The CLI codec token, if any. `Default` emits no flag.
    fn flag_value(self) -> Option<&'static str> {
        match self {
            ScrcpyCodec::H265 => Some("h265"),
            ScrcpyCodec::H264 => Some("h264"),
            ScrcpyCodec::Default => None,
        }
    }

    /// The value recorded in [`sky_cua_platform::model::PhoneScrcpyCapabilities::video_codec`]
    /// for the codec that actually launched.
    pub(in crate::phone) fn capability_value(self) -> Option<String> {
        self.flag_value().map(str::to_string)
    }
}

/// The fixed retry order after a codec-attributable launch failure: H.265, then
/// H.264, then scrcpy's default codec selection.
pub(in crate::phone) const CODEC_RETRY_ORDER: [ScrcpyCodec; 3] =
    [ScrcpyCodec::H265, ScrcpyCodec::H264, ScrcpyCodec::Default];

/// The launch knobs pulled from `[phone]` config plus the target serial.
///
/// Built from [`PhoneConfig`] rather than the resolved selection because the
/// window/codec geometry fields are config-only (the resolved selection carries
/// connection/companion policy, not scrcpy frame geometry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) struct ScrcpyLaunchSpec {
    /// The exact serial passed to `--serial` (USB serial, emulator serial, or
    /// `host:port`).
    pub(in crate::phone) serial: String,
    /// `--window-width`, when configured.
    pub(in crate::phone) window_width: Option<u32>,
    /// `--window-height`, when configured.
    pub(in crate::phone) window_height: Option<u32>,
    /// `--max-size`, when configured.
    pub(in crate::phone) max_size: Option<u32>,
    /// `--max-fps`, when configured.
    pub(in crate::phone) max_fps: Option<u32>,
    /// `--v4l2-sink`, when configured (Linux V4L2 loopback target).
    pub(in crate::phone) v4l2_sink: Option<String>,
}

impl ScrcpyLaunchSpec {
    /// Build a launch spec from the `[phone]` config table for `serial`.
    ///
    /// Retained alongside the wired [`ScrcpyLaunchSpec::from_selection`] for the
    /// command builder's own tests, which exercise argv construction directly from
    /// a [`PhoneConfig`]; the live driver builds specs from the resolved selection.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(in crate::phone) fn from_config(config: &PhoneConfig, serial: &str) -> Self {
        Self {
            serial: serial.to_string(),
            window_width: config.window_width,
            window_height: config.window_height,
            max_size: config.max_size,
            max_fps: config.max_fps,
            v4l2_sink: normalize_optional(config.v4l2_sink.clone()),
        }
    }

    /// Build a launch spec from the resolved phone selection for `serial`.
    ///
    /// Mirrors [`ScrcpyLaunchSpec::from_config`] but reads the geometry/codec knobs
    /// from the already-layered [`ResolvedPhoneSelection`] the driver holds, so the
    /// launch path does not re-parse the config table. The selection's geometry
    /// fields are config-only (no env override layer), matching the config table.
    pub(in crate::phone) fn from_selection(
        selection: &ResolvedPhoneSelection,
        serial: &str,
    ) -> Self {
        Self {
            serial: serial.to_string(),
            window_width: selection.window_width,
            window_height: selection.window_height,
            max_size: selection.max_size,
            max_fps: selection.max_fps,
            v4l2_sink: normalize_optional(selection.v4l2_sink.clone()),
        }
    }

    /// Fill `max_size` from a host-derived default when config left it unset.
    ///
    /// An explicit `[phone] max_size` always wins; this only supplies the
    /// phone-scale cap the daemon computes from the host display topology so the
    /// mirror renders phone-sized instead of at the device's full (often very
    /// high) resolution. A `None` default is a no-op.
    pub(in crate::phone) fn apply_default_max_size(&mut self, default: Option<u32>) {
        if self.max_size.is_none() {
            self.max_size = default;
        }
    }

    /// The deterministic, sanitized window title for this serial.
    pub(in crate::phone) fn window_title(&self) -> String {
        scrcpy_window_title(&self.serial)
    }

    /// Build the full argv (excluding the scrcpy binary itself) for `codec`.
    ///
    /// Flag order is fixed so tests can assert exact argv. The documented flags
    /// are emitted unconditionally: `--no-audio`, `--keyboard=uhid`,
    /// `--always-on-top`, and `--window-borderless`. scrcpy's default SDK mouse
    /// mode is intentionally kept
    /// (no `--mouse=uhid`), because UHID mouse mode captures the host pointer and
    /// is worse for agent coordinate injection.
    pub(in crate::phone) fn argv(&self, codec: ScrcpyCodec) -> Vec<String> {
        let mut argv = Vec::new();
        argv.push(format!("--serial={}", self.serial));
        argv.push(format!("--window-title={}", self.window_title()));
        if let Some(width) = self.window_width {
            argv.push(format!("--window-width={width}"));
        }
        if let Some(height) = self.window_height {
            argv.push(format!("--window-height={height}"));
        }
        if let Some(max_size) = self.max_size {
            argv.push(format!("--max-size={max_size}"));
        }
        if let Some(max_fps) = self.max_fps {
            argv.push(format!("--max-fps={max_fps}"));
        }
        if let Some(codec_value) = codec.flag_value() {
            argv.push(format!("--video-codec={codec_value}"));
        }
        argv.push("--no-audio".to_string());
        argv.push("--keyboard=uhid".to_string());
        argv.push("--always-on-top".to_string());
        // Borderless so the compositor adds no title-bar/decoration: the window
        // bounds then equal the video content bounds, which keeps the
        // device->host content-rect mapping (and the host-visible cursor overlay)
        // aligned instead of shifted down by the decoration height.
        argv.push("--window-borderless".to_string());
        if let Some(sink) = self.v4l2_sink.as_deref() {
            argv.push(format!("--v4l2-sink={sink}"));
        }
        argv
    }
}

/// Sanitize a serial into a window-title-safe slug and prefix it.
///
/// `host:port` wireless serials and any odd characters are reduced to
/// `[A-Za-z0-9._-]`; every other byte collapses to `-`. The result is stable for
/// a given serial so the ownership model can match windows by title.
pub(in crate::phone) fn scrcpy_window_title(serial: &str) -> String {
    let mut slug = String::with_capacity(serial.len());
    for ch in serial.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            slug.push(ch);
        } else {
            slug.push('-');
        }
    }
    let trimmed = slug.trim_matches('-');
    let safe = if trimmed.is_empty() {
        "device"
    } else {
        trimmed
    };
    format!("{WINDOW_TITLE_PREFIX}{safe}")
}

/// Normalize an optional config string: trim and drop empties so a blank config
/// value is treated as "unset" rather than an empty flag argument.
fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Mirror long edge as a fraction of the host display's logical height.
///
/// A portrait phone mirror is constrained by vertical desktop space, so the
/// device's longer edge (what scrcpy's `--max-size` caps) is sized against the
/// host display's logical height. ~0.55 reads as a phone sitting on the desk:
/// roughly half the screen height, never filling it.
const HOST_HEIGHT_FRACTION: f64 = 0.55;
/// Lower/upper guards so an unusual display never yields an absurdly small or
/// large mirror; the fraction drives the value within this band.
const MIN_MIRROR_LONG_EDGE: f64 = 480.0;
const MAX_MIRROR_LONG_EDGE: f64 = 1600.0;

/// Derive a phone-scale scrcpy `--max-size` cap from the host display topology.
///
/// Without a cap scrcpy mirrors at the device's full (often very high)
/// resolution, so a modern phone fills the desktop. This sizes the mirror's long
/// edge to a fraction of the primary display's logical height (clamped) so the
/// window reads as a phone regardless of how hi-res the device is. The daemon
/// primes the result onto the manager before a scrcpy-bearing connect (see
/// `PhoneManager::set_scrcpy_host_size_default`), which applies it through
/// [`ScrcpyLaunchSpec::apply_default_max_size`].
///
/// Returns `None` when no usable display is known (empty topology, or a primary
/// with a degenerate logical height), leaving the configured `max_size`
/// (possibly unset) untouched rather than guessing.
pub(crate) fn host_scrcpy_default_max_size(displays: &[DisplayInfo]) -> Option<u32> {
    let display = primary_or_tallest_display(displays)?;
    let height = display.logical_rect.height;
    if !height.is_finite() || height <= 0.0 {
        return None;
    }
    let long_edge =
        (height * HOST_HEIGHT_FRACTION).clamp(MIN_MIRROR_LONG_EDGE, MAX_MIRROR_LONG_EDGE);
    Some(long_edge.round() as u32)
}

/// The display to size the mirror against: the primary if one is flagged, else
/// the tallest by logical height (the binding axis for a portrait mirror). The
/// primary-flag rule is the shared [`primary_flagged_display`]; only the
/// tallest fallback is local to mirror sizing.
fn primary_or_tallest_display(displays: &[DisplayInfo]) -> Option<&DisplayInfo> {
    primary_flagged_display(displays).or_else(|| {
        displays.iter().max_by(|a, b| {
            a.logical_rect
                .height
                .partial_cmp(&b.logical_rect.height)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    })
}

/// Result of a single scrcpy launch attempt at one codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) enum LaunchAttempt {
    /// scrcpy started and stayed up long enough to be considered launched.
    Launched {
        codec: ScrcpyCodec,
        pid: Option<u32>,
    },
    /// This attempt failed; `retryable` marks codec-attributable failures that
    /// justify trying the next codec.
    Failed { codec: ScrcpyCodec, retryable: bool },
}

/// Attempt to launch scrcpy across the codec retry order, returning the first
/// success or the final failure.
///
/// The actual process spawn is delegated to `attempt` so the launch policy
/// (codec order, retry-on-codec-failure) is unit-testable without spawning
/// scrcpy: tests pass a closure returning canned [`LaunchAttempt`]s; the real
/// driver passes one backed by the [`crate::phone::command::CommandRunner`] seam.
/// Non-retryable failures (e.g. device offline) stop the sequence immediately
/// rather than burning every codec.
///
/// The live scrcpy driver in `manager::scrcpy_lane` mirrors this exact codec
/// order and stop-on-success policy inline (its spawn + liveness check is async,
/// and this helper takes a synchronous closure), so the helper itself is exercised
/// by the command-builder tests that validate the policy in isolation.
#[cfg_attr(not(test), expect(dead_code))]
pub(in crate::phone) fn launch_with_retry<F>(mut attempt: F) -> LaunchAttempt
where
    F: FnMut(ScrcpyCodec) -> LaunchAttempt,
{
    let mut last = LaunchAttempt::Failed {
        codec: ScrcpyCodec::Default,
        retryable: false,
    };
    for codec in CODEC_RETRY_ORDER {
        let result = attempt(codec);
        match result {
            LaunchAttempt::Launched { .. } => return result,
            LaunchAttempt::Failed { retryable, .. } => {
                last = result;
                if !retryable {
                    break;
                }
            }
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::{CoordinateSpace, RectF};

    fn config_with_geometry() -> PhoneConfig {
        PhoneConfig {
            window_width: Some(540),
            window_height: Some(1200),
            max_size: Some(1440),
            max_fps: Some(60),
            video_codec: Some("h265".to_string()),
            v4l2_sink: None,
            ..PhoneConfig::default()
        }
    }

    fn display(name: &str, primary: bool, logical_height: f64) -> DisplayInfo {
        DisplayInfo {
            display_id: format!("kwin:{name}"),
            name: Some(name.to_string()),
            index: 0,
            primary,
            logical_rect: RectF {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: logical_height,
                space: CoordinateSpace::DesktopLogical,
            },
            pixel_size: None,
            scale_factor: None,
            backend: "kwin".to_string(),
        }
    }

    #[test]
    fn default_max_size_is_fraction_of_primary_logical_height() {
        // 1440 logical height * 0.55 = 792, within the clamp band.
        let displays = vec![
            display("HDMI-A-1", false, 2160.0),
            display("DP-1", true, 1440.0),
        ];
        assert_eq!(host_scrcpy_default_max_size(&displays), Some(792));
    }

    #[test]
    fn default_max_size_clamps_extremes() {
        // A very short display floors at the minimum long edge (480).
        let short = vec![display("tiny", true, 600.0)];
        assert_eq!(host_scrcpy_default_max_size(&short), Some(480));
        // A very tall display caps at the maximum long edge (1600).
        let tall = vec![display("huge", true, 4320.0)];
        assert_eq!(host_scrcpy_default_max_size(&tall), Some(1600));
    }

    #[test]
    fn default_max_size_falls_back_to_tallest_without_primary() {
        // No display flagged primary: the tallest drives the size (2160*0.55=1188).
        let displays = vec![display("a", false, 1080.0), display("b", false, 2160.0)];
        assert_eq!(host_scrcpy_default_max_size(&displays), Some(1188));
    }

    #[test]
    fn default_max_size_none_for_empty_or_degenerate_topology() {
        assert_eq!(host_scrcpy_default_max_size(&[]), None);
        let degenerate = vec![display("zero", true, 0.0)];
        assert_eq!(host_scrcpy_default_max_size(&degenerate), None);
        // A non-finite logical height (a backend that skips normalization) must
        // not slip past the `is_finite` guard into a bogus `as u32` cast.
        let nan = vec![display("nan", true, f64::NAN)];
        assert_eq!(host_scrcpy_default_max_size(&nan), None);
        let inf = vec![display("inf", true, f64::INFINITY)];
        assert_eq!(host_scrcpy_default_max_size(&inf), None);
    }

    #[test]
    fn window_title_sanitizes_wireless_serial() {
        assert_eq!(
            scrcpy_window_title("172.16.255.58:38781"),
            "sky-cua-phone-172.16.255.58-38781"
        );
        assert_eq!(
            scrcpy_window_title("emulator-5554"),
            "sky-cua-phone-emulator-5554"
        );
        // Odd characters collapse to '-' and edges are trimmed.
        assert_eq!(
            scrcpy_window_title("  weird/serial  "),
            "sky-cua-phone-weird-serial"
        );
        // Degenerate serial still yields a stable, non-empty title.
        assert_eq!(scrcpy_window_title("///"), "sky-cua-phone-device");
    }

    #[test]
    fn argv_h265_emits_documented_flags_without_uhid_mouse() {
        let spec = ScrcpyLaunchSpec::from_config(&config_with_geometry(), "emulator-5554");
        let argv = spec.argv(ScrcpyCodec::H265);
        assert_eq!(
            argv,
            vec![
                "--serial=emulator-5554".to_string(),
                "--window-title=sky-cua-phone-emulator-5554".to_string(),
                "--window-width=540".to_string(),
                "--window-height=1200".to_string(),
                "--max-size=1440".to_string(),
                "--max-fps=60".to_string(),
                "--video-codec=h265".to_string(),
                "--no-audio".to_string(),
                "--keyboard=uhid".to_string(),
                "--always-on-top".to_string(),
                "--window-borderless".to_string(),
            ]
        );
        // SDK mouse mode is the default: no --mouse=uhid must ever appear.
        assert!(!argv.iter().any(|a| a == "--mouse=uhid"));
    }

    #[test]
    fn argv_retry_drops_codec_flag_progressively() {
        let spec = ScrcpyLaunchSpec::from_config(&config_with_geometry(), "dev1");
        let h264 = spec.argv(ScrcpyCodec::H264);
        assert!(h264.iter().any(|a| a == "--video-codec=h264"));
        assert!(!h264.iter().any(|a| a == "--video-codec=h265"));

        let default = spec.argv(ScrcpyCodec::Default);
        assert!(!default.iter().any(|a| a.starts_with("--video-codec=")));
        // Still carries the non-codec documented flags.
        assert!(default.iter().any(|a| a == "--no-audio"));
        assert!(default.iter().any(|a| a == "--keyboard=uhid"));
        assert!(default.iter().any(|a| a == "--always-on-top"));
    }

    #[test]
    fn from_selection_mirrors_from_config_geometry() {
        // The resolved selection carries the same geometry/codec knobs as the
        // config table, so a spec built from either must produce identical argv.
        let config = config_with_geometry();
        let mut selection =
            sky_cua_platform::config::resolve_phone_selection(&PhoneConfig::default());
        selection.window_width = config.window_width;
        selection.window_height = config.window_height;
        selection.max_size = config.max_size;
        selection.max_fps = config.max_fps;
        selection.video_codec = config.video_codec.clone();
        selection.v4l2_sink = Some("/dev/video10".to_string());

        let from_config = ScrcpyLaunchSpec::from_config(
            &PhoneConfig {
                v4l2_sink: Some("/dev/video10".to_string()),
                ..config
            },
            "emulator-5554",
        );
        let from_selection = ScrcpyLaunchSpec::from_selection(&selection, "emulator-5554");
        assert_eq!(from_selection, from_config);
        assert_eq!(
            from_selection.argv(ScrcpyCodec::H265),
            from_config.argv(ScrcpyCodec::H265)
        );
    }

    #[test]
    fn argv_appends_v4l2_sink_when_configured() {
        let mut config = config_with_geometry();
        config.v4l2_sink = Some("/dev/video10".to_string());
        let spec = ScrcpyLaunchSpec::from_config(&config, "dev1");
        let argv = spec.argv(ScrcpyCodec::H265);
        assert!(argv.iter().any(|a| a == "--v4l2-sink=/dev/video10"));
    }

    #[test]
    fn apply_default_max_size_only_fills_when_unset() {
        // Config left max_size unset: the host-derived default fills it.
        let mut spec = ScrcpyLaunchSpec::from_config(&PhoneConfig::default(), "dev1");
        assert_eq!(spec.max_size, None);
        spec.apply_default_max_size(Some(792));
        assert_eq!(spec.max_size, Some(792));
        assert!(
            spec.argv(ScrcpyCodec::H265)
                .iter()
                .any(|a| a == "--max-size=792")
        );

        // An explicit config max_size always wins over the host default.
        let mut configured = ScrcpyLaunchSpec::from_config(&config_with_geometry(), "dev1");
        assert_eq!(configured.max_size, Some(1440));
        configured.apply_default_max_size(Some(792));
        assert_eq!(configured.max_size, Some(1440));

        // A `None` default is a no-op even when max_size is unset.
        let mut unset = ScrcpyLaunchSpec::from_config(&PhoneConfig::default(), "dev1");
        unset.apply_default_max_size(None);
        assert_eq!(unset.max_size, None);
    }

    #[test]
    fn argv_omits_unset_geometry_flags() {
        let spec = ScrcpyLaunchSpec::from_config(&PhoneConfig::default(), "dev1");
        let argv = spec.argv(ScrcpyCodec::H265);
        assert!(!argv.iter().any(|a| a.starts_with("--window-width=")));
        assert!(!argv.iter().any(|a| a.starts_with("--max-fps=")));
        // Mandatory flags survive even with no geometry config.
        assert!(argv.iter().any(|a| a == "--no-audio"));
        assert!(argv[0] == "--serial=dev1");
    }

    #[test]
    fn launch_with_retry_succeeds_on_h265_first() {
        let mut tried = Vec::new();
        let result = launch_with_retry(|codec| {
            tried.push(codec);
            LaunchAttempt::Launched {
                codec,
                pid: Some(42),
            }
        });
        assert_eq!(tried, vec![ScrcpyCodec::H265]);
        assert_eq!(
            result,
            LaunchAttempt::Launched {
                codec: ScrcpyCodec::H265,
                pid: Some(42)
            }
        );
    }

    #[test]
    fn launch_with_retry_falls_back_to_h264_then_default() {
        let mut tried = Vec::new();
        let result = launch_with_retry(|codec| {
            tried.push(codec);
            match codec {
                ScrcpyCodec::Default => LaunchAttempt::Launched { codec, pid: None },
                other => LaunchAttempt::Failed {
                    codec: other,
                    retryable: true,
                },
            }
        });
        assert_eq!(
            tried,
            vec![ScrcpyCodec::H265, ScrcpyCodec::H264, ScrcpyCodec::Default]
        );
        assert_eq!(
            result,
            LaunchAttempt::Launched {
                codec: ScrcpyCodec::Default,
                pid: None
            }
        );
    }

    #[test]
    fn launch_with_retry_stops_on_non_retryable_failure() {
        let mut tried = Vec::new();
        let result = launch_with_retry(|codec| {
            tried.push(codec);
            LaunchAttempt::Failed {
                codec,
                retryable: false,
            }
        });
        // Device-offline style failure must not burn every codec.
        assert_eq!(tried, vec![ScrcpyCodec::H265]);
        assert_eq!(
            result,
            LaunchAttempt::Failed {
                codec: ScrcpyCodec::H265,
                retryable: false
            }
        );
    }
}
