use sky_cua_platform::model::{CoordinateSpace, RectF, WindowInfo};

use super::discovery::{AccessibleTopLevel, DiscoveredApp};
use crate::backend::{dimensions_approximately_match, near_zero};

const MIN_BOUNDS_IOU: f64 = 0.80;
const MIN_BOUNDS_IOU_MARGIN: f64 = 0.50;

#[derive(Debug)]
pub(crate) enum WindowAccessibilityMatch<'a> {
    Matched {
        top_level: &'a AccessibleTopLevel,
        provenance: MatchProvenance,
    },
    Unavailable {
        reason: &'static str,
    },
    Ambiguous {
        reason: &'static str,
        candidate_count: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MatchProvenance {
    /// Client PID the match was scoped to; `None` on the PID-less path
    /// (compositors such as COSMIC that expose no client PID).
    pub pid: Option<u32>,
    pub normalized_title: bool,
    pub active_bounds_tiebreak: bool,
    pub bounds_iou: Option<f64>,
}

pub(crate) fn match_window_accessibility<'a>(
    window: &WindowInfo,
    apps: &'a [DiscoveredApp],
) -> WindowAccessibilityMatch<'a> {
    let Some(pid) = window.pid.filter(|pid| *pid != 0) else {
        return match_window_accessibility_without_pid(window, apps);
    };
    let app_candidates = apps
        .iter()
        .filter(|app| app.info.pid == Some(pid))
        .collect::<Vec<_>>();
    let app = match app_candidates.as_slice() {
        [app] => *app,
        [] => {
            return WindowAccessibilityMatch::Unavailable {
                reason: "no AT-SPI application root had the compositor client PID",
            };
        }
        candidates => {
            return WindowAccessibilityMatch::Ambiguous {
                reason: "multiple AT-SPI application roots shared the compositor client PID",
                candidate_count: candidates.len(),
            };
        }
    };

    let wanted_title = normalize_title(window.title.as_deref().unwrap_or_default());
    if wanted_title.is_empty() {
        return WindowAccessibilityMatch::Unavailable {
            reason: "the compositor window did not expose a usable title",
        };
    }
    let Some(top_levels) = app.top_levels.as_ref() else {
        return WindowAccessibilityMatch::Unavailable {
            reason: "top-level AT-SPI window discovery was incomplete",
        };
    };
    let title_candidates = top_levels
        .iter()
        .filter(|candidate| normalize_title(&candidate.title) == wanted_title)
        .collect::<Vec<_>>();
    match_title_candidates(
        &title_candidates,
        window.bounds.as_ref(),
        Some(pid),
        false, // within a PID-scoped app a unique title is sufficient identity
        "no top-level AT-SPI window had the normalized compositor title",
        "duplicate-title AT-SPI windows had no unique active high-IoU winner",
    )
}

fn duplicate_title_winner<'a>(
    window_bounds: Option<&RectF>,
    candidates: &[&'a AccessibleTopLevel],
) -> Option<(&'a AccessibleTopLevel, f64)> {
    let window_bounds = window_bounds?;
    if window_bounds.space != CoordinateSpace::DesktopLogical {
        return None;
    }
    let active = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.active || candidate.focused)
        .collect::<Vec<_>>();
    let [winner] = active.as_slice() else {
        return None;
    };
    if candidates.iter().any(|candidate| {
        candidate
            .bounds
            .as_ref()
            .is_none_or(|bounds| bounds.space != CoordinateSpace::DesktopLogical)
    }) {
        return None;
    }
    let winner_iou = winner
        .bounds
        .as_ref()
        .map(|bounds| rect_iou(window_bounds, bounds))?;
    let runner_up_iou = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.object_ref != winner.object_ref)
        .filter_map(|candidate| candidate.bounds.as_ref())
        .map(|bounds| rect_iou(window_bounds, bounds))
        .fold(0.0, f64::max);
    (winner_iou >= MIN_BOUNDS_IOU && winner_iou - runner_up_iou >= MIN_BOUNDS_IOU_MARGIN)
        .then_some((*winner, winner_iou))
}

fn rect_iou(left: &RectF, right: &RectF) -> f64 {
    if left.width <= 0.0 || left.height <= 0.0 || right.width <= 0.0 || right.height <= 0.0 {
        return 0.0;
    }
    let intersection_width = (left.x + left.width).min(right.x + right.width) - left.x.max(right.x);
    let intersection_height =
        (left.y + left.height).min(right.y + right.height) - left.y.max(right.y);
    if intersection_width <= 0.0 || intersection_height <= 0.0 {
        return 0.0;
    }
    let intersection = intersection_width * intersection_height;
    intersection / (left.width * left.height + right.width * right.height - intersection)
}

/// Shared title-candidate dispatch for the PID-scoped and PID-less correlation
/// paths: a normalized-title filter, then unique / absent / duplicate-title
/// (bounds-IoU tiebreak) resolution with provenance construction.
///
/// `corroborate_single` requires a single candidate to overlap the compositor
/// window's desktop-logical bounds when both are known; the PID-less path spans
/// every app in the session, so a title collision with clearly non-matching
/// geometry must not silently win.
fn match_title_candidates<'a>(
    candidates: &[&'a AccessibleTopLevel],
    window_bounds: Option<&RectF>,
    pid: Option<u32>,
    corroborate_single: bool,
    no_match_reason: &'static str,
    ambiguous_reason: &'static str,
) -> WindowAccessibilityMatch<'a> {
    match candidates {
        [] => WindowAccessibilityMatch::Unavailable {
            reason: no_match_reason,
        },
        [single] => {
            let bounds_corroborate = match (window_bounds, single.bounds.as_ref()) {
                (Some(window_bounds), Some(candidate_bounds))
                    if window_bounds.space == CoordinateSpace::DesktopLogical
                        && candidate_bounds.space == CoordinateSpace::DesktopLogical =>
                {
                    // The compositor and AT-SPI bounds come from heterogeneous
                    // sources that legitimately diverge on scaled outputs and
                    // spanning windows, so a strict IoU threshold would
                    // false-reject the correct single window. Corroborate only
                    // far enough to disprove a title collision: reject when
                    // the two rects are clearly disjoint, accept any overlap.
                    let overlaps = rect_iou(window_bounds, candidate_bounds) > 0.0;
                    // Some compositors (COSMIC) report every AT-SPI top-level
                    // with a zero origin, so a correct window whose compositor
                    // origin is non-zero looks disjoint even though its size is
                    // right. When the origin is unusable, corroborate on
                    // dimensions alone instead of false-rejecting the match.
                    let zero_origin_size_match = near_zero(candidate_bounds.x)
                        && near_zero(candidate_bounds.y)
                        && dimensions_approximately_match(window_bounds, candidate_bounds);
                    overlaps || zero_origin_size_match
                }
                _ => true,
            };
            if corroborate_single && !bounds_corroborate {
                WindowAccessibilityMatch::Ambiguous {
                    reason: "title correlation matched a single AT-SPI window but its bounds did not corroborate the compositor window",
                    candidate_count: 1,
                }
            } else {
                WindowAccessibilityMatch::Matched {
                    top_level: single,
                    provenance: MatchProvenance {
                        pid,
                        normalized_title: true,
                        active_bounds_tiebreak: false,
                        bounds_iou: None,
                    },
                }
            }
        }
        multiple => match duplicate_title_winner(window_bounds, multiple) {
            Some((top_level, iou)) => WindowAccessibilityMatch::Matched {
                top_level,
                provenance: MatchProvenance {
                    pid,
                    normalized_title: true,
                    active_bounds_tiebreak: true,
                    bounds_iou: Some(iou),
                },
            },
            None => WindowAccessibilityMatch::Ambiguous {
                reason: ambiguous_reason,
                candidate_count: multiple.len(),
            },
        },
    }
}

/// PID-less correlation path for compositors (e.g. COSMIC) that do not expose
/// the client PID through their toplevel protocol.  Matches the compositor
/// window to an AT-SPI top-level by normalised title across every app whose
/// top-levels were enumerated, with bounds-IoU tiebreak for duplicate titles.
fn match_window_accessibility_without_pid<'a>(
    window: &WindowInfo,
    apps: &'a [DiscoveredApp],
) -> WindowAccessibilityMatch<'a> {
    let wanted_title = normalize_title(window.title.as_deref().unwrap_or_default());
    if wanted_title.is_empty() {
        return WindowAccessibilityMatch::Unavailable {
            reason: "the compositor window did not expose a usable title for PID-less correlation",
        };
    }

    let candidates: Vec<&'a AccessibleTopLevel> = apps
        .iter()
        .filter_map(|app| app.top_levels.as_ref())
        .flatten()
        .filter(|tl| normalize_title(&tl.title) == wanted_title)
        .collect();

    match_title_candidates(
        &candidates,
        window.bounds.as_ref(),
        None,
        true, // PID-less spans all apps; require bounds corroboration when available
        "no top-level AT-SPI window matched the compositor window title",
        "title correlation matched multiple AT-SPI windows without a PID tiebreak",
    )
}

pub(crate) fn normalize_title(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            character if character.is_whitespace() => ' ',
            character => character,
        })
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" - ", "-")
        .replace("- ", "-")
        .replace(" -", "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::AppInfo;
    use zbus::names::UniqueName;
    use zbus::zvariant::ObjectPath;

    fn object_ref(path: &str) -> atspi::ObjectRefOwned {
        atspi::ObjectRef::new_owned(
            UniqueName::try_from(":1.77".to_string()).expect("valid unique name"),
            ObjectPath::try_from(path.to_string()).expect("valid object path"),
        )
    }

    fn bounds(x: f64, y: f64, width: f64, height: f64) -> RectF {
        RectF {
            x,
            y,
            width,
            height,
            space: CoordinateSpace::DesktopLogical,
        }
    }

    fn top_level(
        path: &str,
        title: &str,
        active: bool,
        focused: bool,
        bounds: RectF,
    ) -> AccessibleTopLevel {
        AccessibleTopLevel {
            object_ref: object_ref(path),
            title: title.to_string(),
            active,
            focused,
            bounds: Some(bounds),
        }
    }

    fn app(pid: u32, name: &str, top_levels: Vec<AccessibleTopLevel>) -> DiscoveredApp {
        DiscoveredApp {
            info: AppInfo {
                app_id: format!(":1.77:/org/a11y/{name}"),
                name: name.to_string(),
                pid: Some(pid),
                executable: None,
                desktop_file_id: None,
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: None,
                window_title: None,
                is_focused_candidate: false,
            },
            object_ref: object_ref(&format!("/org/a11y/{name}")),
            top_levels: Some(top_levels),
        }
    }

    fn window(pid: u32, title: &str, bounds: RectF) -> WindowInfo {
        WindowInfo {
            window_id: "kwin:42".to_string(),
            title: Some(title.to_string()),
            app_id: None,
            wm_class: None,
            pid: Some(pid),
            bounds: Some(bounds),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: None,
            backend: "kwin".to_string(),
            terminal: None,
        }
    }

    fn pidless_window(title: &str, bounds: RectF) -> WindowInfo {
        WindowInfo {
            window_id: "cosmic:99".to_string(),
            title: Some(title.to_string()),
            app_id: None,
            wm_class: None,
            pid: None,
            bounds: Some(bounds),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused: true,
            hidden: false,
            client_type: None,
            backend: "cosmic".to_string(),
            terminal: None,
        }
    }

    fn matched_path<'a>(
        window: &WindowInfo,
        apps: &'a [DiscoveredApp],
    ) -> &'a atspi::ObjectRefOwned {
        match match_window_accessibility(window, apps) {
            WindowAccessibilityMatch::Matched { top_level, .. } => &top_level.object_ref,
            result => panic!("expected match, got {result:?}"),
        }
    }

    #[test]
    fn kate_normalizes_whitespace_and_dash_variants() {
        let apps = [app(
            208_663,
            "kate",
            vec![top_level(
                "/org/a11y/kate/frame/1",
                "Welcome  — Kate",
                true,
                true,
                bounds(0.0, 0.0, 900.0, 700.0),
            )],
        )];
        let target = window(208_663, "Welcome — Kate", bounds(0.0, 0.0, 900.0, 700.0));
        assert_eq!(
            matched_path(&target, &apps),
            &apps[0].top_levels.as_ref().unwrap()[0].object_ref
        );
    }

    #[test]
    fn dolphin_matches_the_named_window_within_the_pid_matched_app() {
        let apps = [app(
            37_544,
            "dolphin",
            vec![
                top_level(
                    "/org/a11y/dolphin/frame/1",
                    ".openclaw",
                    false,
                    false,
                    bounds(0.0, 0.0, 500.0, 500.0),
                ),
                top_level(
                    "/org/a11y/dolphin/frame/2",
                    "Downloads",
                    true,
                    true,
                    bounds(500.0, 0.0, 800.0, 700.0),
                ),
            ],
        )];
        let target = window(37_544, "Downloads", bounds(500.0, 0.0, 800.0, 700.0));
        assert_eq!(
            matched_path(&target, &apps),
            &apps[0].top_levels.as_ref().unwrap()[1].object_ref
        );
    }

    #[test]
    fn ghostty_matches_exact_pid_and_title() {
        let apps = [app(
            51_001,
            "ghostty",
            vec![top_level(
                "/org/a11y/ghostty/frame/1",
                "sky-cua",
                true,
                true,
                bounds(20.0, 30.0, 1000.0, 700.0),
            )],
        )];
        let target = window(51_001, "sky-cua", bounds(20.0, 30.0, 1000.0, 700.0));
        assert_eq!(
            matched_path(&target, &apps),
            &apps[0].top_levels.as_ref().unwrap()[0].object_ref
        );
    }

    #[test]
    fn refreshed_window_identity_selects_current_same_pid_top_level() {
        let apps = [app(
            51_001,
            "editor",
            vec![
                top_level(
                    "/org/a11y/editor/frame/old",
                    "Old document",
                    false,
                    false,
                    bounds(0.0, 0.0, 500.0, 500.0),
                ),
                top_level(
                    "/org/a11y/editor/frame/current",
                    "Current document",
                    true,
                    true,
                    bounds(500.0, 0.0, 900.0, 700.0),
                ),
            ],
        )];
        let refreshed = window(51_001, "Current document", bounds(500.0, 0.0, 900.0, 700.0));
        assert_eq!(
            matched_path(&refreshed, &apps),
            &apps[0].top_levels.as_ref().unwrap()[1].object_ref
        );
    }

    #[test]
    fn electron_duplicate_titles_use_unique_active_high_iou_frame() {
        let apps = [app(
            7_537,
            "ChatGPT",
            vec![
                top_level(
                    "/org/a11y/chatgpt/frame/1",
                    "ChatGPT",
                    true,
                    true,
                    bounds(72.0, 0.0, 1635.0, 1067.0),
                ),
                top_level(
                    "/org/a11y/chatgpt/frame/2",
                    "ChatGPT",
                    false,
                    false,
                    bounds(100.0, 100.0, 400.0, 300.0),
                ),
            ],
        )];
        let target = window(7_537, "ChatGPT", bounds(72.0, 0.0, 1635.0, 1067.0));
        assert_eq!(
            matched_path(&target, &apps),
            &apps[0].top_levels.as_ref().unwrap()[0].object_ref
        );
    }

    #[test]
    fn pid_mismatch_fails_closed() {
        let apps = [app(99, "kate", Vec::new())];
        let target = window(100, "Kate", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Unavailable { .. }
        ));
    }

    #[test]
    fn duplicate_titles_without_unique_active_frame_are_ambiguous() {
        let shared = bounds(0.0, 0.0, 800.0, 600.0);
        let apps = [app(
            100,
            "dolphin",
            vec![
                top_level(
                    "/org/a11y/dolphin/frame/1",
                    "Files",
                    true,
                    true,
                    shared.clone(),
                ),
                top_level("/org/a11y/dolphin/frame/2", "Files", true, true, shared),
            ],
        )];
        let target = window(100, "Files", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Ambiguous { .. }
        ));
    }

    #[test]
    fn duplicate_titles_without_high_iou_margin_have_no_winner() {
        let apps = [app(
            100,
            "electron",
            vec![
                top_level(
                    "/org/a11y/electron/frame/1",
                    "Editor",
                    true,
                    true,
                    bounds(0.0, 0.0, 800.0, 600.0),
                ),
                top_level(
                    "/org/a11y/electron/frame/2",
                    "Editor",
                    false,
                    false,
                    bounds(10.0, 10.0, 800.0, 600.0),
                ),
            ],
        )];
        let target = window(100, "Editor", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Ambiguous { .. }
        ));
    }

    #[test]
    fn duplicate_title_with_unknown_sibling_bounds_fails_closed() {
        let mut sibling = top_level(
            "/org/a11y/electron/frame/2",
            "Editor",
            false,
            false,
            bounds(20.0, 20.0, 400.0, 300.0),
        );
        sibling.bounds = None;
        let apps = [app(
            100,
            "electron",
            vec![
                top_level(
                    "/org/a11y/electron/frame/1",
                    "Editor",
                    true,
                    true,
                    bounds(0.0, 0.0, 800.0, 600.0),
                ),
                sibling,
            ],
        )];
        let target = window(100, "Editor", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Ambiguous { .. }
        ));
    }

    #[test]
    fn incomplete_top_level_discovery_fails_closed() {
        let mut discovered = app(100, "electron", Vec::new());
        discovered.top_levels = None;
        let target = window(100, "Editor", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &[discovered]),
            WindowAccessibilityMatch::Unavailable { .. }
        ));
    }

    #[test]
    fn selected_top_level_root_isolated_from_sibling_tree() {
        let apps = [app(
            100,
            "dolphin",
            vec![
                top_level(
                    "/org/a11y/dolphin/frame/sibling",
                    "Other",
                    false,
                    false,
                    bounds(0.0, 0.0, 400.0, 400.0),
                ),
                top_level(
                    "/org/a11y/dolphin/frame/selected",
                    "Files",
                    true,
                    true,
                    bounds(500.0, 0.0, 800.0, 600.0),
                ),
            ],
        )];
        let target = window(100, "Files", bounds(500.0, 0.0, 800.0, 600.0));
        assert_eq!(
            matched_path(&target, &apps),
            &object_ref("/org/a11y/dolphin/frame/selected")
        );
    }

    // --- PID-less correlation (COSMIC fallback) ---

    #[test]
    fn cosmic_no_pid_unique_title_matches() {
        let apps = [
            app(
                100,
                "cosmic_term",
                vec![top_level(
                    "/org/a11y/term/frame/1",
                    "Cosmic Terminal",
                    true,
                    true,
                    bounds(0.0, 0.0, 800.0, 600.0),
                )],
            ),
            app(
                200,
                "cosmic_files",
                vec![top_level(
                    "/org/a11y/files/frame/1",
                    "Cosmic Files",
                    false,
                    false,
                    bounds(0.0, 0.0, 1200.0, 800.0),
                )],
            ),
        ];
        let target = pidless_window("Cosmic Terminal", bounds(0.0, 0.0, 800.0, 600.0));
        let result = match_window_accessibility(&target, &apps);
        assert!(matches!(result, WindowAccessibilityMatch::Matched { .. }));
        assert_eq!(
            matched_path(&target, &apps),
            &object_ref("/org/a11y/term/frame/1")
        );
    }

    #[test]
    fn cosmic_no_pid_empty_title_fails_closed() {
        let apps = [app(
            100,
            "cosmic_term",
            vec![top_level(
                "/org/a11y/term/frame/1",
                "Cosmic Terminal",
                true,
                true,
                bounds(0.0, 0.0, 800.0, 600.0),
            )],
        )];
        let mut target = pidless_window("", bounds(0.0, 0.0, 800.0, 600.0));
        target.title = None;
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Unavailable { .. }
        ));
    }

    #[test]
    fn cosmic_no_pid_no_matching_title_fails_closed() {
        let apps = [app(
            100,
            "cosmic_term",
            vec![top_level(
                "/org/a11y/term/frame/1",
                "Cosmic Terminal",
                true,
                true,
                bounds(0.0, 0.0, 800.0, 600.0),
            )],
        )];
        let target = pidless_window("Firefox", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Unavailable { .. }
        ));
    }

    #[test]
    fn cosmic_no_pid_single_title_with_disjoint_bounds_is_ambiguous() {
        // A unique title match whose desktop-logical bounds are clearly
        // disjoint from the compositor window must not silently win: the
        // PID-less pool spans every app, so a title collision with
        // non-overlapping geometry is rejected. Overlapping-but-different
        // rects are accepted because the two bounds sources (compositor vs
        // AT-SPI) legitimately diverge on scaled/spanning windows.
        let apps = [app(
            100,
            "cosmic_editor",
            vec![top_level(
                "/org/a11y/editor/frame/1",
                "Editor",
                true,
                true,
                bounds(1000.0, 1000.0, 300.0, 200.0),
            )],
        )];
        let target = pidless_window("Editor", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Ambiguous { .. }
        ));
    }

    #[test]
    fn cosmic_no_pid_zero_origin_matching_size_matches_nonzero_compositor_bounds() {
        // COSMIC reports the AT-SPI top-level with a zero origin while the
        // compositor window is placed at a non-zero origin; both carry the same
        // size. The zero origin is unusable, so the match must be corroborated
        // on dimensions instead of being rejected as a disjoint title collision.
        let apps = [app(
            100,
            "zenity",
            vec![top_level(
                "/org/a11y/zenity/dialog/1",
                "sky-cua zenity smoke",
                true,
                true,
                bounds(0.0, 0.0, 300.0, 248.0),
            )],
        )];
        let target = pidless_window("sky-cua zenity smoke", bounds(703.0, 161.0, 300.0, 248.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Matched { .. }
        ));
    }

    #[test]
    fn cosmic_no_pid_zero_origin_with_wrong_size_stays_ambiguous() {
        // A zero origin does not disable the title-collision guard entirely: a
        // same-title candidate whose size clearly differs from the compositor
        // window is still rejected.
        let apps = [app(
            100,
            "zenity",
            vec![top_level(
                "/org/a11y/zenity/dialog/1",
                "sky-cua zenity smoke",
                true,
                true,
                bounds(0.0, 0.0, 500.0, 400.0),
            )],
        )];
        let target = pidless_window("sky-cua zenity smoke", bounds(703.0, 161.0, 300.0, 248.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Ambiguous { .. }
        ));
    }

    #[test]
    fn cosmic_no_pid_single_title_with_overlapping_but_smaller_bounds_matches() {
        // The same window reported by two different bounds sources (e.g.
        // compositor geometry at (0,0,800,600) vs AT-SPI extents at
        // (2,2,700,500)) still overlaps, so it must not be rejected as a title
        // collision even though its IoU is below the duplicate-title tiebreak
        // threshold.
        let apps = [app(
            100,
            "cosmic_editor",
            vec![top_level(
                "/org/a11y/editor/frame/1",
                "Editor",
                true,
                true,
                bounds(2.0, 2.0, 700.0, 500.0),
            )],
        )];
        let target = pidless_window("Editor", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Matched { .. }
        ));
    }

    #[test]
    fn cosmic_no_pid_single_title_without_bounds_still_matches() {
        // Without bounds on either side, PID-less correlation cannot
        // corroborate and falls back to the title match (best effort).
        let apps = [app(
            100,
            "cosmic_term",
            vec![top_level(
                "/org/a11y/term/frame/1",
                "Cosmic Terminal",
                true,
                true,
                bounds(0.0, 0.0, 800.0, 600.0),
            )],
        )];
        let mut target = pidless_window("Cosmic Terminal", bounds(0.0, 0.0, 800.0, 600.0));
        target.bounds = None;
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Matched { .. }
        ));
    }

    #[test]
    fn cosmic_no_pid_duplicate_title_resolved_by_bounds_and_active() {
        let apps = [
            app(
                100,
                "cosmic_term",
                vec![top_level(
                    "/org/a11y/term/frame/1",
                    "Editor",
                    true,
                    true,
                    bounds(0.0, 0.0, 800.0, 600.0),
                )],
            ),
            app(
                200,
                "cosmic_files",
                vec![top_level(
                    "/org/a11y/files/frame/1",
                    "Editor",
                    false,
                    false,
                    bounds(200.0, 150.0, 500.0, 400.0),
                )],
            ),
        ];
        let target = pidless_window("Editor", bounds(0.0, 0.0, 800.0, 600.0));
        let result = match_window_accessibility(&target, &apps);
        assert!(matches!(result, WindowAccessibilityMatch::Matched { .. }));
        assert_eq!(
            matched_path(&target, &apps),
            &object_ref("/org/a11y/term/frame/1")
        );
    }

    #[test]
    fn cosmic_no_pid_duplicate_title_no_clear_winner_ambiguous() {
        let shared = bounds(0.0, 0.0, 800.0, 600.0);
        let apps = [
            app(
                100,
                "cosmic_term",
                vec![top_level(
                    "/org/a11y/term/frame/1",
                    "Editor",
                    false,
                    false,
                    shared.clone(),
                )],
            ),
            app(
                200,
                "cosmic_files",
                vec![top_level(
                    "/org/a11y/files/frame/1",
                    "Editor",
                    false,
                    false,
                    shared,
                )],
            ),
        ];
        let target = pidless_window("Editor", bounds(0.0, 0.0, 800.0, 600.0));
        assert!(matches!(
            match_window_accessibility(&target, &apps),
            WindowAccessibilityMatch::Ambiguous { .. }
        ));
    }
}
