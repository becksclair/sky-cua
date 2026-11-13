//! Read-only live probes against applications already running on this
//! desktop. Run with
//! `cargo nextest run -p sky-cua-macos --run-ignored only live_ax --no-capture`.
//! `SKY_CUA_AX_PROBE_APPS` overrides the comma-separated owner names probed.

use std::time::Instant;

use super::{AxRuntime, TreeBudget};
use crate::types::MacWindow;
use crate::windows;

const DEFAULT_APPS: &str = "T3 Code (Nightly),Finder,Brave Browser Nightly,ChatGPT";

/// Every normal-layer window, on screen or not (other Spaces), so apps whose
/// windows are not visible right now can still be probed read-only.
fn all_normal_windows() -> Vec<MacWindow> {
    use objc2_core_foundation::{
        CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
    };
    use objc2_core_graphics::{
        CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
        kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowName, kCGWindowNumber,
        kCGWindowOwnerName, kCGWindowOwnerPID,
    };
    let Some(list) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionAll | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    // SAFETY: an array of window-description dictionaries.
    let list: CFRetained<CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(list) };
    let mut windows = Vec::new();
    for index in 0..list.len() {
        // SAFETY: in bounds; the dictionaries are immutable.
        let entry = unsafe { list.get_unchecked(index as isize) };
        // SAFETY: framework constant keys on an immutable dictionary.
        let get = |key: &CFString| unsafe { entry.get_unchecked(key) };
        let number = |key| {
            get(key)
                .and_then(|value| value.downcast_ref::<CFNumber>())
                .and_then(CFNumber::as_i64)
        };
        let string = |key| {
            get(key)
                .and_then(|value| value.downcast_ref::<CFString>())
                .map(ToString::to_string)
        };
        // SAFETY: framework constants.
        let (Some(id), Some(pid), Some(layer)) = (
            unsafe { number(kCGWindowNumber) },
            unsafe { number(kCGWindowOwnerPID) },
            unsafe { number(kCGWindowLayer) },
        ) else {
            continue;
        };
        // SAFETY: framework constants.
        let Some(rect) = unsafe { get(kCGWindowBounds) }
            .and_then(|value| value.downcast_ref::<CFDictionary>())
            .and_then(|dict| {
                let mut rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
                // SAFETY: `rect` is a valid out-pointer.
                unsafe { CGRectMakeWithDictionaryRepresentation(Some(dict), &mut rect) }
                    .then_some(rect)
            })
        else {
            continue;
        };
        if layer != 0 || rect.size.width < 50.0 || rect.size.height < 50.0 {
            continue;
        }
        windows.push(MacWindow {
            window_id: id as u32,
            pid: pid as i32,
            // SAFETY: framework constants.
            owner_name: unsafe { string(kCGWindowOwnerName) }.unwrap_or_default(),
            bundle_id: None,
            executable: executable_path(pid as i32),
            // SAFETY: framework constants.
            title: unsafe { string(kCGWindowName) }.filter(|title| !title.is_empty()),
            bounds: crate::coords::desktop_rect(
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ),
            layer: 0,
            on_screen: true,
            focused: false,
            z_index: windows.len(),
        });
    }
    windows
}

fn executable_path(pid: i32) -> Option<String> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buffer` is writable for the length passed.
    let len = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    buffer.truncate(usize::try_from(len).ok()?);
    String::from_utf8(buffer).ok()
}

/// Each target app's windows, largest first, with the app's full list.
fn probe_targets() -> Vec<(Vec<MacWindow>, Vec<MacWindow>)> {
    let names = std::env::var("SKY_CUA_AX_PROBE_APPS").unwrap_or_else(|_| DEFAULT_APPS.to_string());
    let on_screen = windows::enumerate_windows();
    let all = all_normal_windows();
    eprintln!(
        "on-screen owners: {:?}; all normal-layer owners: {:?}",
        on_screen
            .iter()
            .map(|window| window.owner_name.as_str())
            .collect::<Vec<_>>(),
        all.iter()
            .map(|window| window.owner_name.as_str())
            .collect::<std::collections::BTreeSet<_>>()
    );
    names
        .split(',')
        .filter_map(|name| {
            let window = all.iter().find(|window| window.owner_name == name.trim())?;
            let app_windows: Vec<MacWindow> = all
                .iter()
                .filter(|candidate| candidate.pid == window.pid)
                .cloned()
                .collect();
            let mut by_size = app_windows.clone();
            by_size.sort_by(|a, b| {
                (b.bounds.width * b.bounds.height).total_cmp(&(a.bounds.width * a.bounds.height))
            });
            Some((by_size, app_windows))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live desktop probe"]
async fn live_ax_read_trees() {
    let started = Instant::now();
    for _ in 0..3 {
        let _ = crate::permissions::status();
    }
    eprintln!("permissions::status average {:?}", started.elapsed() / 3);

    let runtime = AxRuntime::new();
    let targets = probe_targets();
    eprintln!(
        "targets: {:?}",
        targets
            .iter()
            .map(|(windows, _)| (&windows[0].owner_name, windows.len(), windows[0].pid))
            .collect::<Vec<_>>()
    );
    for (candidates, app_windows) in targets {
        // The first window AX can correlate; hidden helper windows have no
        // AX counterpart.
        let mut chosen = None;
        for window in &candidates {
            match runtime
                .read_window_tree(window, &app_windows, TreeBudget::default())
                .await
            {
                Ok(_) => {
                    chosen = Some(window.clone());
                    break;
                }
                Err(error) => eprintln!(
                    "{} window {} {:?}: {}",
                    window.owner_name,
                    window.window_id,
                    (
                        window.bounds.x,
                        window.bounds.y,
                        window.bounds.width,
                        window.bounds.height
                    ),
                    error.code
                ),
            }
        }
        let Some(window) = chosen else {
            continue;
        };
        for pass in ["first", "repeat"] {
            let started = Instant::now();
            let result = runtime
                .read_window_tree(&window, &app_windows, TreeBudget::default())
                .await;
            let elapsed = started.elapsed();
            match result {
                Ok(tree) => {
                    let count = tree.elements.len().max(1);
                    eprintln!(
                        "{} [{pass}] window {} correlation=Exact elements={} truncated={} timed_out={} wall={:?} per_emitted={:?}",
                        window.owner_name,
                        window.window_id,
                        tree.elements.len(),
                        tree.truncated,
                        tree.timed_out,
                        elapsed,
                        elapsed / u32::try_from(count).unwrap_or(u32::MAX),
                    );
                    if pass == "first" {
                        for node in tree.elements.iter().take(12) {
                            eprintln!(
                                "    #{} parent={:?} {} {:?} value={:?} flags={:?} actions={:?} bounds={:?} ref={:?}",
                                node.element_index,
                                node.parent_index,
                                node.role,
                                node.name,
                                node.value
                                    .as_deref()
                                    .map(|value| value.chars().take(40).collect::<String>()),
                                node.state_flags,
                                node.semantic_actions,
                                node.bounds.as_ref().map(|b| (b.x, b.y, b.width, b.height)),
                                node.backend_ref,
                            );
                        }
                        let mut roles = std::collections::BTreeMap::<&str, usize>::new();
                        for node in &tree.elements {
                            *roles.entry(node.role.as_str()).or_default() += 1;
                        }
                        eprintln!("    roles {roles:?}");
                        assert!(tree.elements.iter().enumerate().all(|(index, node)| {
                            node.element_index == index
                                && node.parent_index.is_none_or(|parent| parent < index)
                        }));
                        assert_eq!(tree.elements[0].role, "window");
                    }
                }
                Err(error) => eprintln!(
                    "{} [{pass}] window {} error {}: {} (wall {:?})",
                    window.owner_name, window.window_id, error.code, error.message, elapsed
                ),
            }
        }
    }
    eprintln!("runtime {runtime:?}");
}

#[test]
#[ignore = "live desktop probe"]
fn live_ax_dump_window_candidates() {
    use super::element::{
        AxElement, as_bool, as_elements, as_point, as_size, as_string, cf_string, cf_strings,
    };
    let pid: i32 = std::env::var("SKY_CUA_AX_PROBE_PID")
        .unwrap()
        .parse()
        .unwrap();
    let app = AxElement::application(pid);
    let windows = app.copy_attribute(&cf_string("AXWindows")).unwrap();
    let names = cf_strings(&[
        "AXTitle",
        "AXPosition",
        "AXSize",
        "AXMinimized",
        "AXMain",
        "AXRole",
        "AXSubrole",
    ]);
    for window in as_elements(&windows, 64) {
        let values = window.copy_multiple(&names).unwrap();
        let get = |index: usize| values[index].as_ref().ok();
        eprintln!(
            "title={:?} pos={:?} size={:?} min={:?} main={:?} role={:?}/{:?}",
            get(0).and_then(|v| as_string(v)),
            get(1).and_then(|v| as_point(v)),
            get(2).and_then(|v| as_size(v)),
            get(3).and_then(|v| as_bool(v)),
            get(4).and_then(|v| as_bool(v)),
            get(5).and_then(|v| as_string(v)),
            get(6).and_then(|v| as_string(v)),
        );
    }
}

/// Walks the first AX window of `SKY_CUA_AX_PROBE_PID` directly (minimized
/// windows included, no window correlation, no activation write) and
/// reports per-element cost.
#[test]
#[ignore = "live desktop probe"]
fn live_ax_walk_pid() {
    use super::element::{AxElement, as_elements, cf_string};
    use super::tree::{self, Names};
    let pid: i32 = std::env::var("SKY_CUA_AX_PROBE_PID")
        .unwrap()
        .parse()
        .unwrap();
    let app = AxElement::application(pid);
    app.set_messaging_timeout(1.0).unwrap();
    // `SKY_CUA_AX_PROBE_ROOT=menubar` walks the application's native menu
    // bar instead (AppKit, present even when no window is on this Space).
    let root = if std::env::var("SKY_CUA_AX_PROBE_ROOT").as_deref() == Ok("menubar") {
        app.copy_attribute(&cf_string("AXMenuBar"))
            .ok()
            .and_then(|value| {
                value
                    .downcast::<objc2_application_services::AXUIElement>()
                    .ok()
            })
            .map(AxElement::from_retained)
    } else {
        let windows = app.copy_attribute(&cf_string("AXWindows")).unwrap();
        as_elements(&windows, 1).into_iter().next()
    };
    let Some(root) = root else {
        eprintln!("pid {pid} has no AX root to walk");
        return;
    };
    let names = Names::new();
    for pass in ["first", "repeat"] {
        let started = Instant::now();
        let output = tree::walk(
            &names,
            &root,
            pid,
            1,
            started + TreeBudget::default().deadline,
            TreeBudget::default(),
        )
        .unwrap();
        let elapsed = started.elapsed();
        eprintln!(
            "pid {pid} [{pass}] emitted={} read={} round_trips={} empty_web_areas={} truncated={} timed_out={} wall={elapsed:?} per_read={:?} per_emitted={:?}",
            output.tree.elements.len(),
            output.stats.read,
            output.stats.round_trips,
            output.stats.empty_web_areas,
            output.tree.truncated,
            output.tree.timed_out,
            elapsed / u32::try_from(output.stats.read.max(1)).unwrap(),
            elapsed / u32::try_from(output.tree.elements.len().max(1)).unwrap(),
        );
        if pass == "first" {
            let mut roles = std::collections::BTreeMap::<&str, usize>::new();
            for node in &output.tree.elements {
                *roles.entry(node.role.as_str()).or_default() += 1;
            }
            eprintln!("    roles {roles:?}");
            for node in output
                .tree
                .elements
                .iter()
                .filter(|node| {
                    node.text.is_some()
                        || node.numeric_value.is_some()
                        || node.semantic_actions.len() > 2
                })
                .take(8)
            {
                eprintln!(
                    "    #{} parent={:?} {} {:?} flags={:?} actions={:?} text={:?} numeric={:?}",
                    node.element_index,
                    node.parent_index,
                    node.role,
                    node.name
                        .as_deref()
                        .map(|name| name.chars().take(40).collect::<String>()),
                    node.state_flags,
                    node.semantic_actions,
                    node.text.as_ref().map(|text| (
                        text.character_count,
                        text.caret_offset,
                        text.truncated,
                        text.content
                            .as_deref()
                            .map(|c| c.chars().take(30).collect::<String>())
                    )),
                    node.numeric_value,
                );
            }
        }
    }
}

/// Compares batched-read cost by attribute count on the first AX window of
/// `SKY_CUA_AX_PROBE_PID`.
#[test]
#[ignore = "live desktop probe"]
fn live_ax_batch_cost() {
    use super::element::{AxElement, as_elements, cf_string, cf_strings};
    let pid: i32 = std::env::var("SKY_CUA_AX_PROBE_PID")
        .unwrap()
        .parse()
        .unwrap();
    let app = AxElement::application(pid);
    let windows = app.copy_attribute(&cf_string("AXWindows")).unwrap();
    let root = as_elements(&windows, 1).into_iter().next().unwrap();
    let children = cf_string("AXChildren");
    let mut elements = vec![root];
    let mut index = 0;
    while elements.len() < 800 && index < elements.len() {
        if let Ok(value) = elements[index].copy_attribute(&children) {
            elements.extend(as_elements(&value, 250));
        }
        index += 1;
    }
    elements.truncate(800);
    let full = cf_strings(&[
        "AXRole",
        "AXSubrole",
        "AXTitle",
        "AXDescription",
        "AXValue",
        "AXIdentifier",
        "AXPosition",
        "AXSize",
        "AXEnabled",
        "AXFocused",
        "AXSelected",
        "AXExpanded",
        "AXDisclosing",
        "AXChildren",
        "AXNumberOfCharacters",
        "AXSelectedTextRange",
        "AXMinValue",
        "AXMaxValue",
        "AXPlaceholderValue",
    ]);
    let core = cf_strings(&[
        "AXRole",
        "AXSubrole",
        "AXTitle",
        "AXDescription",
        "AXValue",
        "AXPosition",
        "AXSize",
        "AXEnabled",
        "AXFocused",
        "AXSelected",
        "AXExpanded",
        "AXChildren",
    ]);
    let role_only = cf_strings(&["AXRole"]);
    for (label, names) in [
        ("full19", &full),
        ("core12", &core),
        ("role1", &role_only),
        ("full19-again", &full),
    ] {
        let started = Instant::now();
        for element in &elements {
            let _ = element.copy_multiple(names);
        }
        eprintln!(
            "{label}: {:?} per element over {}",
            started.elapsed() / elements.len() as u32,
            elements.len()
        );
    }
    let started = Instant::now();
    for element in &elements {
        let _ = element.action_names();
    }
    eprintln!(
        "action_names: {:?} per element",
        started.elapsed() / elements.len() as u32
    );
}
