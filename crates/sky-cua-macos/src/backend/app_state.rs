//! `get_app_state` assembly: window selection, capture, and the AX tree.
//!
//! The capture and the tree read run concurrently. The tree is read in
//! desktop points and converted into the capture's pixel space afterwards, so
//! neither waits for the other.

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
use sky_cua_platform::model::{
    AppSelector, AppStateSnapshot, CaptureInfo, CaptureScope, CaptureScreenMode, CoordinateSpace,
    ElementNode, SemanticBackendKind, WindowInfo,
};
use sky_cua_platform::new_snapshot_id;

use super::focus::app_windows;
use super::{MacosDesktopBackend, capabilities};
use crate::ax::{AxRuntime, AxTree, TreeBudget};
use crate::capture::{self, CaptureSource};
use crate::coords;
use crate::displays;
use crate::permissions::{self, Permission};
use crate::types::MacWindow;
use crate::windows;

/// Diagnostic code for a tree cut off by the element or depth budget.
const ACCESSIBILITY_TREE_TRUNCATED: &str = "AccessibilityTreeTruncated";

/// A tree read plus the error of a first attempt that was retried.
struct TreeRead {
    result: Result<AxTree, BackendError>,
    retried_after: Option<BackendError>,
}

impl MacosDesktopBackend {
    pub(super) async fn app_state(
        &self,
        selector: Option<AppSelector>,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        let windows_front_to_back = windows::enumerate_windows();
        let selected = select_app_window(&windows_front_to_back, selector.as_ref())?;
        self.snapshot_for(selected, &windows_front_to_back, capture_screen)
            .await
    }

    /// The exact-window snapshot the service's AppShot path pairs with a
    /// separate `screenshot`. It never falls back to another window: a window
    /// that has disappeared is an error.
    pub(super) async fn app_state_for_window(
        &self,
        window: &WindowInfo,
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        let window_id: u32 = window.window_id.trim().parse().map_err(|_| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!(
                    "macOS window_id must be a decimal CGWindowID, got {:?}",
                    window.window_id
                ),
            )
        })?;
        let windows_front_to_back = windows::enumerate_windows();
        let selected = windows_front_to_back
            .iter()
            .find(|candidate| candidate.window_id == window_id)
            .cloned()
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("macOS window {window_id} is no longer on screen"),
                )
            })?;
        self.snapshot_for(Some(selected), &windows_front_to_back, capture_screen)
            .await
    }

    /// Captures `selected` (or the primary display when there is no window)
    /// and reads its AX tree concurrently.
    async fn snapshot_for(
        &self,
        selected: Option<MacWindow>,
        windows_front_to_back: &[MacWindow],
        capture_screen: CaptureScreenMode,
    ) -> Result<AppStateSnapshot, BackendError> {
        let snapshot_id = new_snapshot_id();
        let (environment, status) = Self::probe();
        let capabilities = capabilities(&environment);
        let mut diagnostics = DiagnosticBuilder::new();
        if !status.post_event {
            let error = permissions::missing_permission_error(Permission::PostEvent);
            diagnostics.push_code(
                error.code,
                "macOS event posting is not permitted; physical input actions are unavailable",
                Some(error.message),
            );
        }

        let capture_source = match selected.as_ref() {
            Some(window) => Some((CaptureSource::Window(window.clone()), CaptureScope::Window)),
            None => displays::primary_display(&environment.displays).map(|display| {
                (
                    CaptureSource::Display(display),
                    CaptureScope::PrimaryDisplay,
                )
            }),
        };
        let capture_future = async {
            if capture_screen == CaptureScreenMode::Never {
                return None;
            }
            if !status.screen_capture {
                return Some(Err(permissions::missing_permission_error(
                    Permission::ScreenCapture,
                )));
            }
            let Some((source, scope)) = capture_source else {
                return Some(Err(BackendError::new(
                    BackendErrorCode::CaptureSourceGeometryMissing,
                    "no macOS window or display is available to capture",
                )));
            };
            Some(capture::capture(&snapshot_id, source, scope).await)
        };
        let tree_future = async {
            let window = selected.as_ref()?;
            if environment.semantic_backend != SemanticBackendKind::MacosAccessibility {
                return Some(TreeRead {
                    result: Err(permissions::missing_permission_error(
                        Permission::Accessibility,
                    )),
                    retried_after: None,
                });
            }
            Some(read_tree_with_retry(&self.ax, window, windows_front_to_back).await)
        };
        let (capture_result, tree_read) = tokio::join!(capture_future, tree_future);

        let capture = capture_result.map(|result| match result {
            Ok(capture) => capture,
            Err(error) => {
                diagnostics.push_code(
                    error.code,
                    "macOS ScreenCaptureKit screenshot capture failed",
                    Some(error.message),
                );
                capture::empty_capture()
            }
        });

        let (focused_app, elements) = match selected.as_ref() {
            Some(window) => {
                let focused_app = windows::window_to_focused_app(window, &environment.displays);
                let elements = tree_elements(window, tree_read, capture.as_ref(), &mut diagnostics);
                (Some(focused_app), elements)
            }
            None => {
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    "No on-screen macOS application window was found",
                    None,
                );
                (None, Vec::new())
            }
        };

        Ok(AppStateSnapshot {
            snapshot_id,
            created_at: chrono::Utc::now(),
            environment,
            capabilities,
            focused_app,
            capture,
            elements,
            diagnostics: diagnostics.finish(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        })
    }
}

/// The window a `get_app_state` selector names. A supplied selector that
/// matches nothing is an error, never another application; with no selector
/// the focused window, then the front-most, is used.
fn select_app_window(
    windows_front_to_back: &[MacWindow],
    selector: Option<&AppSelector>,
) -> Result<Option<MacWindow>, BackendError> {
    if let Some(selector) = selector.filter(|selector| !selector_fields(selector).is_empty()) {
        return windows::select_window(windows_front_to_back, selector)
            .map(Some)
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "no on-screen macOS window matched selector {}",
                        selector_fields(selector).join(", ")
                    ),
                )
            });
    }
    Ok(windows_front_to_back
        .iter()
        .find(|window| window.focused)
        .or_else(|| windows_front_to_back.first())
        .cloned())
}

/// The supplied (non-blank) selector fields as `field=value`.
fn selector_fields(selector: &AppSelector) -> Vec<String> {
    [
        ("app_id", selector.app_id.as_deref()),
        ("desktop_file_id", selector.desktop_file_id.as_deref()),
        ("window_title", selector.window_title.as_deref()),
        ("name", selector.name.as_deref()),
    ]
    .into_iter()
    .filter_map(|(field, value)| {
        value
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("{field}={value}"))
    })
    .collect()
}

/// Reads `window`'s tree, retrying once against a freshly enumerated window
/// list when the AX window could not be correlated (a window that moved
/// between the window-list read and the AX read misses the frame match).
async fn read_tree_with_retry(
    ax: &AxRuntime,
    window: &MacWindow,
    windows_front_to_back: &[MacWindow],
) -> TreeRead {
    let budget = TreeBudget::default();
    let first = ax
        .read_window_tree(
            window,
            &app_windows(windows_front_to_back, window.pid),
            budget,
        )
        .await;
    let error = match first {
        Err(error) if is_correlation_failure(&error) => error,
        result => {
            return TreeRead {
                result,
                retried_after: None,
            };
        }
    };
    let fresh = windows::enumerate_windows();
    let Some(fresh_window) = fresh
        .iter()
        .find(|candidate| candidate.window_id == window.window_id)
    else {
        return TreeRead {
            result: Err(error),
            retried_after: None,
        };
    };
    let result = ax
        .read_window_tree(fresh_window, &app_windows(&fresh, fresh_window.pid), budget)
        .await;
    TreeRead {
        result,
        retried_after: Some(error),
    }
}

/// `read_window_tree` reports a failed exact correlation as
/// `AccessibilityCoverageLimited`.
fn is_correlation_failure(error: &BackendError) -> bool {
    error.code == BackendErrorCode::AccessibilityCoverageLimited.as_str()
}

/// The snapshot elements for the selected window: the AX tree when it is
/// useful, otherwise the single window fallback element, with diagnostics for
/// every degradation.
fn tree_elements(
    window: &MacWindow,
    tree_read: Option<TreeRead>,
    capture: Option<&CaptureInfo>,
    diagnostics: &mut DiagnosticBuilder,
) -> Vec<ElementNode> {
    let Some(TreeRead {
        result,
        retried_after,
    }) = tree_read
    else {
        return vec![window_element(window, capture)];
    };
    if let Some(error) = retried_after {
        diagnostics.push_code(
            "AccessibilityWindowRetry",
            "macOS Accessibility window correlation failed; retried once with a fresh window list",
            Some(error.message),
        );
    }
    match result {
        Ok(mut tree) if is_useful_tree(&tree.elements) => {
            if tree.timed_out {
                diagnostics.push(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    "macOS Accessibility tree read hit its deadline; the tree is partial",
                    Some(format!("elements={}", tree.elements.len())),
                );
            }
            if tree.truncated {
                // A budget cap, not a coverage failure: every element read is
                // real, so the tree stays usable (AT-SPI truncates the same
                // way on Linux).
                diagnostics.push_code(
                    ACCESSIBILITY_TREE_TRUNCATED,
                    "macOS Accessibility tree reached its element or depth cap; later elements were omitted",
                    Some(format!("elements={}", tree.elements.len())),
                );
            }
            if let Err(error) = convert_element_bounds(&mut tree.elements, capture) {
                diagnostics.push_code(
                    error.code,
                    "Accessibility element bounds could not be mapped into screenshot pixels; they remain in desktop points",
                    Some(error.message),
                );
            }
            tree.elements
        }
        Ok(tree) => {
            let mut details = format!("window_id={}", window.window_id);
            if tree.timed_out {
                details.push_str(" timed_out=true");
            }
            diagnostics.push(
                BackendErrorCode::AccessibilityCoverageLimited,
                "macOS Accessibility returned no useful child elements; using CoreGraphics window fallback tree",
                Some(details),
            );
            vec![window_element(window, capture)]
        }
        Err(error) => {
            diagnostics.push_code(
                error.code,
                "macOS Accessibility tree collection failed; using CoreGraphics window fallback tree",
                Some(error.message),
            );
            vec![window_element(window, capture)]
        }
    }
}

fn is_useful_tree(elements: &[ElementNode]) -> bool {
    elements.len() > 1
        || elements
            .iter()
            .any(|element| !element.semantic_actions.is_empty())
}

/// Maps desktop-point element bounds into the capture's pixel space. Bounds
/// already in another space are left untouched.
fn convert_element_bounds(
    elements: &mut [ElementNode],
    capture: Option<&CaptureInfo>,
) -> Result<(), BackendError> {
    for element in elements.iter_mut() {
        if let Some(bounds) = element.bounds.as_mut()
            && bounds.space == CoordinateSpace::DesktopLogical
        {
            *bounds = coords::element_bounds(bounds, capture)?;
        }
    }
    Ok(())
}

/// The single honest element for a window whose AX tree is unavailable: its
/// real bounds and title, no invented children or actions.
pub(super) fn window_element(window: &MacWindow, capture: Option<&CaptureInfo>) -> ElementNode {
    let bounds =
        coords::element_bounds(&window.bounds, capture).unwrap_or_else(|_| window.bounds.clone());
    let mut state_flags = Vec::with_capacity(2);
    if window.focused {
        state_flags.push("focused".to_string());
    }
    state_flags.push("coregraphics_fallback".to_string());
    ElementNode {
        element_index: 0,
        parent_index: None,
        role: "window".to_string(),
        name: window.title.clone(),
        description: Some(
            "Top-level CoreGraphics window fallback; no Accessibility tree was read".to_string(),
        ),
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags,
        semantic_actions: Vec::new(),
        bounds: Some(bounds),
        backend_ref: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::desktop_rect;
    use sky_cua_platform::model::{CaptureBackendKind, PixelSize, RectF};

    #[test]
    fn window_fallback_element_uses_screenshot_local_pixels() {
        let window = window();
        let capture = capture(420.0, 184.0, 866.0, 535.0, 2.0);

        let element = window_element(&window, Some(&capture));

        assert_eq!(
            element.bounds,
            Some(RectF {
                x: 0.0,
                y: 0.0,
                width: 1732.0,
                height: 1070.0,
                space: CoordinateSpace::StreamPixels,
            })
        );
        assert_eq!(element.role, "window");
        assert!(element.semantic_actions.is_empty());
        assert_eq!(element.backend_ref, None);
        assert_eq!(
            element.state_flags,
            vec!["focused".to_string(), "coregraphics_fallback".to_string()]
        );
    }

    #[test]
    fn window_fallback_without_capture_keeps_desktop_points() {
        let window = window();

        let element = window_element(&window, None);

        assert_eq!(element.bounds, Some(window.bounds));
    }

    #[test]
    fn tree_bounds_convert_once() {
        let capture = capture(100.0, 50.0, 800.0, 600.0, 2.0);
        let mut elements = vec![
            element(Some(desktop_rect(150.0, 70.0, 40.0, 20.0)), Vec::new()),
            element(
                Some(RectF {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                    space: CoordinateSpace::StreamPixels,
                }),
                Vec::new(),
            ),
            element(None, Vec::new()),
        ];

        convert_element_bounds(&mut elements, Some(&capture)).unwrap();

        assert_eq!(
            elements[0].bounds,
            Some(RectF {
                x: 100.0,
                y: 40.0,
                width: 80.0,
                height: 40.0,
                space: CoordinateSpace::StreamPixels,
            })
        );
        assert_eq!(elements[1].bounds.as_ref().unwrap().x, 1.0);
        assert_eq!(elements[2].bounds, None);
    }

    #[test]
    fn single_window_element_without_actions_is_not_useful() {
        assert!(!is_useful_tree(&[element(None, Vec::new())]));
        assert!(is_useful_tree(&[element(
            None,
            vec!["AXRaise".to_string()]
        )]));
        assert!(is_useful_tree(&[
            element(None, Vec::new()),
            element(None, Vec::new())
        ]));
    }

    #[test]
    fn failed_tree_reads_fall_back_with_their_own_code() {
        let mut diagnostics = DiagnosticBuilder::new();
        let elements = tree_elements(
            &window(),
            Some(TreeRead {
                result: Err(BackendError::new(
                    BackendErrorCode::PermissionRequired,
                    "grant Accessibility",
                )),
                retried_after: None,
            }),
            None,
            &mut diagnostics,
        );

        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].role, "window");
        let diagnostics = diagnostics.finish();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "PermissionRequired");
    }

    #[test]
    fn partial_trees_are_reported() {
        let mut diagnostics = DiagnosticBuilder::new();
        let elements = tree_elements(
            &window(),
            Some(TreeRead {
                result: Ok(AxTree {
                    elements: vec![element(None, Vec::new()), element(None, Vec::new())],
                    truncated: true,
                    timed_out: true,
                }),
                retried_after: Some(BackendError::new(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    "no AX window matched",
                )),
            }),
            None,
            &mut diagnostics,
        );

        assert_eq!(elements.len(), 2);
        let codes: Vec<String> = diagnostics
            .finish()
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(
            codes,
            vec![
                "AccessibilityWindowRetry",
                "AccessibilityCoverageLimited",
                "AccessibilityTreeTruncated"
            ]
        );
    }

    #[test]
    fn unmatched_selector_is_an_error_not_another_app() {
        let windows = vec![window()];
        let selector = AppSelector {
            app_id: Some("com.apple.Safari".to_string()),
            name: Some("Safari".to_string()),
            ..AppSelector::default()
        };

        let error = select_app_window(&windows, Some(&selector)).unwrap_err();

        assert_eq!(error.code, BackendErrorCode::InvalidRequest.as_str());
        assert_eq!(
            error.message,
            "no on-screen macOS window matched selector app_id=com.apple.Safari, name=Safari"
        );
    }

    #[test]
    fn selector_picks_its_match_and_no_selector_falls_back_to_focus() {
        let mut background = window();
        background.window_id = 78;
        background.focused = false;
        background.z_index = 0;
        background.owner_name = "Notes".to_string();
        background.bundle_id = Some("com.apple.Notes".to_string());
        let mut focused = window();
        focused.z_index = 1;
        let windows = vec![background, focused];

        let by_name = AppSelector {
            name: Some("notes".to_string()),
            ..AppSelector::default()
        };
        let selected = select_app_window(&windows, Some(&by_name)).unwrap();
        assert_eq!(selected.map(|window| window.window_id), Some(78));

        for selector in [
            None,
            Some(AppSelector::default()),
            Some(AppSelector {
                name: Some("  ".to_string()),
                ..AppSelector::default()
            }),
        ] {
            let selected = select_app_window(&windows, selector.as_ref()).unwrap();
            assert_eq!(selected.map(|window| window.window_id), Some(77));
        }

        let selected = select_app_window(&[], None).unwrap();
        assert!(selected.is_none());
    }

    fn window() -> MacWindow {
        MacWindow {
            window_id: 77,
            pid: 42,
            owner_name: "Browser".to_string(),
            bundle_id: Some("com.example.browser".to_string()),
            executable: None,
            title: Some("Sumwall Browser".to_string()),
            bounds: desktop_rect(420.0, 184.0, 866.0, 535.0),
            layer: 0,
            on_screen: true,
            focused: true,
            z_index: 0,
        }
    }

    fn element(bounds: Option<RectF>, semantic_actions: Vec<String>) -> ElementNode {
        ElementNode {
            element_index: 0,
            parent_index: None,
            role: "button".to_string(),
            name: None,
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions,
            bounds,
            backend_ref: None,
        }
    }

    fn capture(x: f64, y: f64, width: f64, height: f64, scale: f64) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::ScreenCaptureKit,
            image_backend: Some(CaptureBackendKind::ScreenCaptureKit),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: Some(desktop_rect(x, y, width, height)),
            source_logical_rect: None,
            pixel_size: Some(PixelSize {
                width: (width * scale) as u32,
                height: (height * scale) as u32,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: Some(scale),
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: None,
            model_image_quality: None,
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }
}
