//! Request-scoped reuse of per-backend window listings.
//!
//! One desktop observe resolves its target, captures it and correlates AT-SPI
//! through separate backend calls; each used to relist every window backend
//! (~300 ms per KWin listing). Inside a
//! [`request_scope`](sky_cua_platform::request_scope) each backend is listed at
//! most once and later passes reuse it. Outside a scope nothing is cached.

use sky_cua_platform::request_scope;

use super::registry::{BACKEND_ORDER, BackendKind, unique_cross_backend_focus_target};
use super::types::LinuxWindowInfo;

fn key(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::GnomeExtension => "sky-cua-linux.windows.gnome-extension",
        BackendKind::GnomeIntrospect => "sky-cua-linux.windows.gnome-introspect",
        BackendKind::Cosmic => "sky-cua-linux.windows.cosmic",
        BackendKind::Kwin => "sky-cua-linux.windows.kwin",
        BackendKind::Hyprland => "sky-cua-linux.windows.hyprland",
        BackendKind::I3 => "sky-cua-linux.windows.i3",
        BackendKind::X11 => "sky-cua-linux.windows.x11",
    }
}

/// The listing this request already made for `backend`, if any.
pub(super) fn cached(backend: BackendKind) -> Option<Vec<LinuxWindowInfo>> {
    request_scope::get(key(backend))
}

/// Remember a successful listing for the rest of the request.
pub(super) fn store(backend: BackendKind, windows: &[LinuxWindowInfo]) {
    if request_scope::is_active() {
        request_scope::put(key(backend), windows.to_vec());
    }
}

/// Record a verified activation so later passes in the same request see the
/// post-activation focus without relisting: the verified window replaces its
/// memoized entry, its unique cross-backend alias (the X11 twin of an XWayland
/// window) is marked focused, and every other window is marked unfocused.
pub fn note_request_window_focused(focused: &LinuxWindowInfo) {
    if !request_scope::is_active() {
        return;
    }
    let mut lists: Vec<(BackendKind, Vec<LinuxWindowInfo>)> = BACKEND_ORDER
        .iter()
        .filter_map(|backend| cached(*backend).map(|windows| (*backend, windows)))
        .collect();
    apply_verified_focus(&mut lists, focused);
    for (backend, windows) in lists {
        request_scope::put(key(backend), windows);
    }
}

fn apply_verified_focus(
    lists: &mut [(BackendKind, Vec<LinuxWindowInfo>)],
    focused: &LinuxWindowInfo,
) {
    let flattened: Vec<LinuxWindowInfo> = lists
        .iter()
        .flat_map(|(_, windows)| windows.iter().cloned())
        .collect();
    let alias = unique_cross_backend_focus_target(&flattened, focused)
        .map(|alias| (alias.backend.clone(), alias.window_id.clone()));
    for (_, windows) in lists.iter_mut() {
        for window in windows.iter_mut() {
            if window.backend == focused.backend && window.window_id == focused.window_id {
                *window = focused.clone();
                window.focused = true;
            } else {
                window.focused = alias.as_ref().is_some_and(|(backend, window_id)| {
                    window.backend == *backend && window.window_id == *window_id
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{CoordinateSpace, RectF};

    use super::*;

    fn window(backend: &str, window_id: &str, title: &str, focused: bool) -> LinuxWindowInfo {
        LinuxWindowInfo {
            window_id: window_id.to_string(),
            title: Some(title.to_string()),
            app_id: Some("brave-browser.desktop".to_string()),
            wm_class: Some("brave-browser".to_string()),
            pid: Some(42),
            bounds: Some(RectF {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
                space: CoordinateSpace::DesktopLogical,
            }),
            display: None,
            display_intersections: Vec::new(),
            workspace: None,
            focused,
            hidden: false,
            client_type: None,
            backend: backend.to_string(),
            terminal: None,
            terminal_target_sessions: Vec::new(),
        }
    }

    #[test]
    fn verified_focus_updates_memoized_lists_and_alias() {
        let target = window("kwin", "kwin:{target}", "Leo AI - Brave", false);
        let mut lists = vec![
            (
                BackendKind::Kwin,
                vec![
                    target.clone(),
                    window("kwin", "kwin:{other}", "Editor", true),
                ],
            ),
            (
                BackendKind::X11,
                vec![
                    window("x11", "0x42", "Leo AI - Brave", false),
                    window("x11", "0x99", "Editor", true),
                ],
            ),
        ];

        let mut verified = target;
        verified.focused = true;
        verified.bounds.as_mut().expect("bounds").x = 50.0;
        apply_verified_focus(&mut lists, &verified);

        let flags: Vec<(&str, bool)> = lists
            .iter()
            .flat_map(|(_, windows)| windows.iter())
            .map(|window| (window.window_id.as_str(), window.focused))
            .collect();
        assert_eq!(
            flags,
            vec![
                ("kwin:{target}", true),
                ("kwin:{other}", false),
                ("0x42", true),
                ("0x99", false),
            ]
        );
        assert_eq!(
            lists[0].1[0].bounds.as_ref().map(|bounds| bounds.x),
            Some(50.0),
            "the verified snapshot replaces the memoized entry"
        );
    }

    #[tokio::test]
    async fn listings_are_scoped_to_one_request() {
        let listed = vec![window("kwin", "kwin:{a}", "Leo AI - Brave", false)];
        store(BackendKind::Kwin, &listed);
        assert!(
            cached(BackendKind::Kwin).is_none(),
            "no scope: nothing cached"
        );

        request_scope::scope(async {
            store(BackendKind::Kwin, &listed);
            assert!(cached(BackendKind::X11).is_none());
            note_request_window_focused(&listed[0]);
            let memoized = cached(BackendKind::Kwin).expect("memoized kwin listing");
            assert!(memoized[0].focused);
        })
        .await;
        assert!(cached(BackendKind::Kwin).is_none());
    }
}
