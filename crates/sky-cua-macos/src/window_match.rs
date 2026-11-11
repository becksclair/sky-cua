//! Correlates an Accessibility window element with a CoreGraphics window.
//!
//! macOS has no public call that returns the `CGWindowID` behind an AX window
//! element. This module does the correlation with public data only: both the
//! CoreGraphics window list and AX report a window's frame in the same global
//! point space, so an exact frame match within one process is the identity
//! key. Ties are broken by title, then by the application's main-window flag.
//! The frame is only a key when no other CoreGraphics window of the process
//! shares it; a frame twin demands the same title or main-window proof.
//! Anything still ambiguous fails closed; a title alone is never identity.
//!
//! The CoreGraphics list and the AX read are taken at different instants, so a
//! window that is moving or animating can miss the frame match. Callers treat
//! `NotFound` as retryable once against a fresh window list.

use crate::types::MacWindow;

/// Frame tolerance in points. Both sources report the window-server frame, so
/// values agree exactly in practice; the tolerance only absorbs rounding.
const FRAME_TOLERANCE: f64 = 1.0;

/// Geometry and identity hints read from one element of an application's
/// `AXWindows` attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct AxWindowCandidate {
    pub title: Option<String>,
    /// `AXPosition`, global points.
    pub position: Option<(f64, f64)>,
    /// `AXSize`, points.
    pub size: Option<(f64, f64)>,
    pub minimized: bool,
    /// True when this element is the application's `AXMainWindow`.
    pub main: bool,
}

/// Result of correlating one CoreGraphics window against AX candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowMatch {
    /// Index into the candidate slice.
    Exact(usize),
    /// More than one candidate is indistinguishable from the target.
    Ambiguous(Vec<usize>),
    NotFound,
}

/// Finds the AX candidate that is `target`.
///
/// `app_windows_front_to_back` is every CoreGraphics window of the target's
/// process in front-to-back order (it includes `target`); it is used only to
/// decide whether `target` is the application's front-most window.
#[must_use]
pub fn match_ax_window(
    target: &MacWindow,
    app_windows_front_to_back: &[MacWindow],
    candidates: &[AxWindowCandidate],
) -> WindowMatch {
    let mut matches: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| !candidate.minimized && frame_matches(target, candidate))
        .map(|(index, _)| index)
        .collect();

    if matches.len() > 1
        && let Some(title) = target.title.as_deref().filter(|title| !title.is_empty())
    {
        let titled: Vec<usize> = matches
            .iter()
            .copied()
            .filter(|&index| candidates[index].title.as_deref() == Some(title))
            .collect();
        if !titled.is_empty() {
            matches = titled;
        }
    }

    if matches.len() > 1 && is_front_most_of_app(target, app_windows_front_to_back) {
        let main: Vec<usize> = matches
            .iter()
            .copied()
            .filter(|&index| candidates[index].main)
            .collect();
        if main.len() == 1 {
            matches = main;
        }
    }

    match matches.as_slice() {
        [] => WindowMatch::NotFound,
        // Correlation runs from the CoreGraphics window to AX candidates, so a
        // lone frame match proves identity only when no other CoreGraphics
        // window of the process shares the frame. Otherwise the candidate may
        // belong to that twin (for example a borderless overlay the
        // application does not list in `AXWindows`).
        [index]
            if has_frame_twin(target, app_windows_front_to_back)
                && !distinguishes_from_twins(
                    target,
                    app_windows_front_to_back,
                    &candidates[*index],
                ) =>
        {
            WindowMatch::Ambiguous(matches)
        }
        [index] => WindowMatch::Exact(*index),
        _ => WindowMatch::Ambiguous(matches),
    }
}

fn frame_twins<'a>(
    target: &'a MacWindow,
    app_windows_front_to_back: &'a [MacWindow],
) -> impl Iterator<Item = &'a MacWindow> {
    app_windows_front_to_back.iter().filter(move |window| {
        window.pid == target.pid
            && window.window_id != target.window_id
            && (window.bounds.x - target.bounds.x).abs() <= FRAME_TOLERANCE
            && (window.bounds.y - target.bounds.y).abs() <= FRAME_TOLERANCE
            && (window.bounds.width - target.bounds.width).abs() <= FRAME_TOLERANCE
            && (window.bounds.height - target.bounds.height).abs() <= FRAME_TOLERANCE
    })
}

fn has_frame_twin(target: &MacWindow, app_windows_front_to_back: &[MacWindow]) -> bool {
    frame_twins(target, app_windows_front_to_back)
        .next()
        .is_some()
}

/// True when `candidate` is tied to `target` by more than the shared frame:
/// a title no frame twin carries, or the main-window flag on the
/// application's front-most window.
fn distinguishes_from_twins(
    target: &MacWindow,
    app_windows_front_to_back: &[MacWindow],
    candidate: &AxWindowCandidate,
) -> bool {
    let by_title = target
        .title
        .as_deref()
        .filter(|title| !title.is_empty())
        .is_some_and(|title| {
            candidate.title.as_deref() == Some(title)
                && frame_twins(target, app_windows_front_to_back)
                    .all(|twin| twin.title.as_deref() != Some(title))
        });
    by_title || (candidate.main && is_front_most_of_app(target, app_windows_front_to_back))
}

fn frame_matches(target: &MacWindow, candidate: &AxWindowCandidate) -> bool {
    let (Some((x, y)), Some((width, height))) = (candidate.position, candidate.size) else {
        return false;
    };
    let bounds = &target.bounds;
    (bounds.x - x).abs() <= FRAME_TOLERANCE
        && (bounds.y - y).abs() <= FRAME_TOLERANCE
        && (bounds.width - width).abs() <= FRAME_TOLERANCE
        && (bounds.height - height).abs() <= FRAME_TOLERANCE
}

fn is_front_most_of_app(target: &MacWindow, app_windows_front_to_back: &[MacWindow]) -> bool {
    app_windows_front_to_back
        .iter()
        .filter(|window| window.pid == target.pid)
        .min_by_key(|window| window.z_index)
        .is_some_and(|front| front.window_id == target.window_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::desktop_rect;

    fn window(window_id: u32, z_index: usize, title: Option<&str>, frame: [f64; 4]) -> MacWindow {
        MacWindow {
            window_id,
            pid: 42,
            owner_name: "TextEdit".to_string(),
            bundle_id: Some("com.apple.TextEdit".to_string()),
            executable: None,
            title: title.map(str::to_string),
            bounds: desktop_rect(frame[0], frame[1], frame[2], frame[3]),
            layer: 0,
            on_screen: true,
            focused: z_index == 0,
            z_index,
        }
    }

    fn candidate(title: Option<&str>, frame: [f64; 4], main: bool) -> AxWindowCandidate {
        AxWindowCandidate {
            title: title.map(str::to_string),
            position: Some((frame[0], frame[1])),
            size: Some((frame[2], frame[3])),
            minimized: false,
            main,
        }
    }

    const A: [f64; 4] = [100.0, 80.0, 640.0, 480.0];
    const B: [f64; 4] = [140.0, 120.0, 640.0, 480.0];

    #[test]
    fn unique_frame_is_exact_even_when_titles_are_withheld() {
        let target = window(7, 0, None, A);
        let candidates = [
            candidate(Some("b.txt"), B, false),
            candidate(Some("a.txt"), A, true),
        ];
        assert_eq!(
            match_ax_window(&target, std::slice::from_ref(&target), &candidates),
            WindowMatch::Exact(1)
        );
    }

    #[test]
    fn identical_frames_are_separated_by_title() {
        let target = window(7, 1, Some("a.txt"), A);
        let front = window(8, 0, Some("b.txt"), A);
        let candidates = [
            candidate(Some("b.txt"), A, true),
            candidate(Some("a.txt"), A, false),
        ];
        assert_eq!(
            match_ax_window(&target, &[front, target.clone()], &candidates),
            WindowMatch::Exact(1)
        );
    }

    #[test]
    fn identical_frames_and_titles_use_main_only_for_the_front_window() {
        let front = window(8, 0, Some("Untitled"), A);
        let back = window(7, 1, Some("Untitled"), A);
        let app = [front.clone(), back.clone()];
        let candidates = [
            candidate(Some("Untitled"), A, false),
            candidate(Some("Untitled"), A, true),
        ];
        assert_eq!(
            match_ax_window(&front, &app, &candidates),
            WindowMatch::Exact(1)
        );
        assert_eq!(
            match_ax_window(&back, &app, &candidates),
            WindowMatch::Ambiguous(vec![0, 1])
        );
    }

    #[test]
    fn lone_frame_match_is_ambiguous_when_a_twin_shares_the_frame() {
        // A borderless overlay the application does not list in AXWindows
        // sits exactly over the document window; only the document has an AX
        // counterpart, and titles are withheld.
        let overlay = window(9, 0, None, A);
        let document = window(7, 1, None, A);
        let app = [overlay.clone(), document.clone()];
        let candidates = [candidate(Some("a.txt"), A, false)];
        assert_eq!(
            match_ax_window(&overlay, &app, &candidates),
            WindowMatch::Ambiguous(vec![0])
        );
        assert_eq!(
            match_ax_window(&document, &app, &candidates),
            WindowMatch::Ambiguous(vec![0])
        );
        // With titles available the document is identified and the overlay is not.
        let titled_document = window(7, 1, Some("a.txt"), A);
        let app = [overlay.clone(), titled_document.clone()];
        assert_eq!(
            match_ax_window(&titled_document, &app, &candidates),
            WindowMatch::Exact(0)
        );
        assert_eq!(
            match_ax_window(&overlay, &app, &candidates),
            WindowMatch::Ambiguous(vec![0])
        );
    }

    #[test]
    fn title_alone_is_never_identity() {
        let target = window(7, 0, Some("a.txt"), A);
        let candidates = [candidate(Some("a.txt"), B, true)];
        assert_eq!(
            match_ax_window(&target, std::slice::from_ref(&target), &candidates),
            WindowMatch::NotFound
        );
    }

    #[test]
    fn minimized_and_frameless_candidates_are_ignored() {
        let target = window(7, 0, Some("a.txt"), A);
        let mut minimized = candidate(Some("a.txt"), A, false);
        minimized.minimized = true;
        let mut frameless = candidate(Some("a.txt"), A, false);
        frameless.size = None;
        assert_eq!(
            match_ax_window(
                &target,
                std::slice::from_ref(&target),
                &[minimized, frameless]
            ),
            WindowMatch::NotFound
        );
    }

    #[test]
    fn sub_point_rounding_still_matches() {
        let target = window(7, 0, None, A);
        let candidates = [candidate(None, [100.4, 79.6, 640.0, 480.5], false)];
        assert_eq!(
            match_ax_window(&target, std::slice::from_ref(&target), &candidates),
            WindowMatch::Exact(0)
        );
    }
}
