//! The budgeted tree walk. Runs on a lane thread only.

use std::collections::VecDeque;
use std::time::Instant;

use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use super::element::{
    self, AxElement, AxErr, as_bool, as_elements_capped, as_f64, as_point, as_range, as_size,
    as_string, cf_string, cf_strings,
};
use super::refs::AxRef;
use super::vocab::{self, RawAttrs, RawValue};
use super::{AxTree, TreeBudget};
use crate::permissions::{self, Permission};

/// Same per-node child cap as the Windows UIA walker.
pub(crate) const MAX_CHILDREN_PER_NODE: usize = 250;
/// Upper bound on elements read (kept or pruned) per emitted element, so a
/// tree of uninformative wrappers cannot consume the whole deadline.
const READ_FACTOR: usize = 4;

/// The attributes fetched for every element in one batched round trip.
/// Order matters: `Slot` indexes into it.
///
/// Measured against Chromium (Brave), a batched read costs ~13 us per message
/// plus ~2.3 us per attribute, so attributes only a few roles use are read
/// in a second, role-gated batch (`TEXT_DETAIL`, `RANGE_DETAIL`,
/// `ROW_DETAIL`) instead of on every element.
const ATTRIBUTES: &[&str] = &[
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
    "AXChildren",
];

#[derive(Clone, Copy)]
enum Slot {
    Role,
    Subrole,
    Title,
    Description,
    Value,
    Identifier,
    Position,
    Size,
    Enabled,
    Focused,
    Selected,
    Expanded,
    Children,
}

const TEXT_DETAIL: &[&str] = &[
    "AXNumberOfCharacters",
    "AXSelectedTextRange",
    "AXPlaceholderValue",
];
const RANGE_DETAIL: &[&str] = &["AXMinValue", "AXMaxValue"];
const ROW_DETAIL: &[&str] = &["AXDisclosing"];

const WINDOW_ATTRIBUTES: &[&str] = &["AXTitle", "AXPosition", "AXSize", "AXMinimized", "AXMain"];

/// Interned attribute names, built once per job.
pub(crate) struct Names {
    pub batch: CFRetained<CFArray<CFString>>,
    pub text_detail: CFRetained<CFArray<CFString>>,
    pub range_detail: CFRetained<CFArray<CFString>>,
    pub row_detail: CFRetained<CFArray<CFString>>,
    pub window_batch: CFRetained<CFArray<CFString>>,
    pub windows: CFRetained<CFString>,
    pub value: CFRetained<CFString>,
    pub focused: CFRetained<CFString>,
    pub selected: CFRetained<CFString>,
}

impl Names {
    pub(crate) fn new() -> Self {
        Self {
            batch: cf_strings(ATTRIBUTES),
            text_detail: cf_strings(TEXT_DETAIL),
            range_detail: cf_strings(RANGE_DETAIL),
            row_detail: cf_strings(ROW_DETAIL),
            window_batch: cf_strings(WINDOW_ATTRIBUTES),
            windows: cf_string("AXWindows"),
            value: cf_string("AXValue"),
            focused: cf_string("AXFocused"),
            selected: cf_string("AXSelected"),
        }
    }
}

/// Maps an AX error from a call that must succeed to the contract error.
pub(crate) fn ax_error(error: AxErr, what: &str) -> BackendError {
    if error == AxErr::API_DISABLED && !permissions::status().accessibility {
        return permissions::missing_permission_error(Permission::Accessibility);
    }
    let code = if error.is_timeout() {
        BackendErrorCode::DesktopRequestDeadlineExceeded
    } else if error == AxErr::INVALID_ELEMENT {
        BackendErrorCode::SnapshotStale
    } else {
        BackendErrorCode::AccessibilityUnavailable
    };
    BackendError::new(code, format!("{what}: {error}"))
}

/// One element read during the walk.
struct Visited {
    element: AxElement,
    parent: Option<usize>,
    children: Vec<usize>,
    draft: vocab::NodeDraft,
}

/// Per-walk measurements, for diagnostics and live probes.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WalkStats {
    pub read: usize,
    pub round_trips: usize,
    /// `AXWebArea` elements with no children: the signature of a Chromium
    /// web view whose accessibility tree has not been built.
    pub empty_web_areas: usize,
}

pub(crate) struct WalkOutput {
    pub tree: AxTree,
    pub handles: Vec<AxElement>,
    pub stats: WalkStats,
}

/// Breadth-first walk from `root` within `budget`, emitted in pre-order.
///
/// Breadth-first reading means a budget stop keeps the shallow structure of
/// the whole window (toolbars, sidebars, content panes) rather than one deep
/// branch; the in-memory pre-order pass afterwards yields the contract's
/// flat parent-before-child order.
pub(crate) fn walk(
    names: &Names,
    root: &AxElement,
    pid: i32,
    generation: u64,
    deadline: Instant,
    budget: TreeBudget,
) -> Result<WalkOutput, BackendError> {
    let mut visited: Vec<Visited> = Vec::new();
    let mut queue: VecDeque<(AxElement, Option<usize>, usize)> = VecDeque::new();
    queue.push_back((root.clone(), None, 0));
    let read_cap = budget.max_elements.saturating_mul(READ_FACTOR);
    let mut kept = 0usize;
    let mut truncated = false;
    let mut timed_out = false;
    let mut stats = WalkStats::default();
    let mut window_frame = None;

    while let Some((element, parent, depth)) = queue.pop_front() {
        if kept >= budget.max_elements || visited.len() >= read_cap {
            truncated = true;
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        let read = read_element(names, &element, deadline, &mut stats);
        let (attrs, children, children_capped) = match read {
            Ok(read) => read,
            Err(error) if error.is_timeout() => {
                timed_out = true;
                break;
            }
            Err(error) if error == AxErr::API_DISABLED => {
                return Err(ax_error(
                    error,
                    "Accessibility was disabled during the walk",
                ));
            }
            Err(error) if parent.is_none() => {
                return Err(ax_error(error, "failed to read the AX window element"));
            }
            // The element vanished between listing and reading.
            Err(_) => continue,
        };
        if children_capped {
            truncated = true;
        }
        if parent.is_none() {
            window_frame = vocab::desktop_frame(&attrs);
        }
        if children.is_empty() && attrs.role.as_deref() == Some("AXWebArea") {
            stats.empty_web_areas += 1;
        }
        let draft = vocab::draft(&attrs, window_frame);
        if parent.is_none() || draft.is_informative() {
            kept += 1;
        }
        let index = visited.len();
        if let Some(parent) = parent {
            visited[parent].children.push(index);
        }
        visited.push(Visited {
            element,
            parent,
            children: Vec::new(),
            draft,
        });
        if !children.is_empty() {
            if depth + 1 > budget.max_depth {
                truncated = true;
            } else {
                for child in children {
                    queue.push_back((child, Some(index), depth + 1));
                }
            }
        }
    }
    if visited.is_empty() {
        return Err(BackendError::new(
            BackendErrorCode::DesktopRequestDeadlineExceeded,
            format!(
                "the {} ms Accessibility budget expired before pid {pid} returned its window element",
                budget.deadline.as_millis()
            ),
        ));
    }
    if !queue.is_empty() && !timed_out {
        truncated = true;
    }

    let (elements, handles) = emit_pre_order(visited, pid, generation, budget.max_elements);
    Ok(WalkOutput {
        tree: AxTree {
            elements,
            truncated,
            timed_out,
        },
        handles,
        stats,
    })
}

fn emit_pre_order(
    mut visited: Vec<Visited>,
    pid: i32,
    generation: u64,
    max_elements: usize,
) -> (Vec<sky_cua_platform::model::ElementNode>, Vec<AxElement>) {
    let mut elements = Vec::new();
    let mut handles = Vec::new();
    // (visited index, nearest kept ancestor's output index)
    let mut stack: Vec<(usize, Option<usize>)> = vec![(0, None)];
    while let Some((index, parent_out)) = stack.pop() {
        let node = &mut visited[index];
        let keep = node.parent.is_none() || node.draft.is_informative();
        let child_parent = if keep && elements.len() < max_elements {
            let out = elements.len();
            let draft = std::mem::replace(&mut node.draft, empty_draft());
            let reference = AxRef {
                pid,
                generation,
                index: out,
            };
            elements.push(draft.into_node(out, parent_out, reference.format()));
            handles.push(node.element.clone());
            Some(out)
        } else if keep {
            // Over the emission cap: drop this subtree.
            continue;
        } else {
            parent_out
        };
        for &child in node.children.iter().rev() {
            stack.push((child, child_parent));
        }
    }
    (elements, handles)
}

fn empty_draft() -> vocab::NodeDraft {
    vocab::NodeDraft {
        role: String::new(),
        name: None,
        description: None,
        value: None,
        text: None,
        numeric_value: None,
        supports_editable_text: false,
        state_flags: Vec::new(),
        semantic_actions: Vec::new(),
        frame: None,
    }
}

/// Reads one element: one batched attribute round trip, one for its action
/// names, and settability probes only where they decide an advertised
/// action. Returns its attributes, its first [`MAX_CHILDREN_PER_NODE`]
/// children, and whether children past that cap were dropped.
///
/// One element costs up to six round trips, so `deadline` is checked before
/// each follow-up one and reported as a timeout. The walk then overruns its
/// deadline by at most one message, which is what the lane's
/// `DEADLINE_GRACE` allows for; without the check a slow application could
/// hold the lane past the caller's wait and lose the partial tree.
fn read_element(
    names: &Names,
    element: &AxElement,
    deadline: Instant,
    stats: &mut WalkStats,
) -> Result<(RawAttrs, Vec<AxElement>, bool), AxErr> {
    stats.read += 1;
    stats.round_trips += 1;
    let values = element.copy_multiple(&names.batch)?;
    let slot = |slot: Slot| -> Option<&CFRetained<CFType>> {
        values
            .get(slot as usize)
            .and_then(|value| value.as_ref().ok())
    };
    // A dead or hung element reports the error in every slot.
    if let Some(Err(error)) = values.get(Slot::Role as usize)
        && (*error == AxErr::INVALID_ELEMENT || error.is_timeout())
    {
        return Err(*error);
    }
    let text = |slot_id: Slot| slot(slot_id).and_then(|value| as_string(value));
    let flag = |slot_id: Slot| slot(slot_id).and_then(|value| as_bool(value));
    let value = slot(Slot::Value).and_then(|value| {
        as_string(value)
            .map(RawValue::Text)
            .or_else(|| element::is_number(value).then(|| as_f64(value).map(RawValue::Number))?)
    });
    let mut attrs = RawAttrs {
        role: text(Slot::Role),
        subrole: text(Slot::Subrole),
        title: text(Slot::Title),
        description: text(Slot::Description),
        value,
        identifier: text(Slot::Identifier),
        position: slot(Slot::Position).and_then(|value| as_point(value)),
        size: slot(Slot::Size).and_then(|value| as_size(value)),
        enabled: flag(Slot::Enabled),
        focused: flag(Slot::Focused),
        selected: flag(Slot::Selected),
        expanded: flag(Slot::Expanded),
        ..RawAttrs::default()
    };
    let (children, children_capped) = slot(Slot::Children)
        .map(|value| as_elements_capped(value, MAX_CHILDREN_PER_NODE))
        .unwrap_or_default();

    let role = attrs.role.clone().unwrap_or_default();
    match vocab::value_kind(&role, attrs.subrole.as_deref()) {
        vocab::ValueKind::Text => {
            let detail = read_detail(element, &names.text_detail, deadline, stats)?;
            attrs.number_of_characters = detail(0)
                .and_then(|value| as_f64(&value))
                .map(|count| count as i64);
            attrs.selected_text_range = detail(1).and_then(|value| as_range(&value));
            attrs.placeholder = detail(2).and_then(|value| as_string(&value));
        }
        vocab::ValueKind::Range => {
            let detail = read_detail(element, &names.range_detail, deadline, stats)?;
            attrs.min_value = detail(0).and_then(|value| as_f64(&value));
            attrs.max_value = detail(1).and_then(|value| as_f64(&value));
        }
        _ if role == "AXRow" && attrs.subrole.as_deref() == Some("AXOutlineRow") => {
            let detail = read_detail(element, &names.row_detail, deadline, stats)?;
            attrs.disclosing = detail(0).and_then(|value| as_bool(&value));
        }
        _ => {}
    }
    let probe_actions = vocab::wants_action_probes(&role);
    if probe_actions {
        within(deadline)?;
        stats.round_trips += 1;
        attrs.actions = match element.action_names() {
            Ok(actions) => actions,
            Err(error) if error.is_timeout() => return Err(error),
            Err(_) => Vec::new(),
        };
    }
    if vocab::wants_value_settable(&role, attrs.subrole.as_deref()) {
        stats.round_trips += 1;
        attrs.value_settable = settable(element, &names.value, deadline)?;
    }
    if probe_actions && attrs.focused.is_some() {
        // An element without an AXFocused value cannot take focus.
        stats.round_trips += 1;
        attrs.focus_settable = settable(element, &names.focused, deadline)?;
    }
    if vocab::wants_selected_settable(&attrs) {
        stats.round_trips += 1;
        attrs.selected_settable = settable(element, &names.selected, deadline)?;
    }
    Ok((attrs, children, children_capped))
}

/// `Ok` while `deadline` has not passed; afterwards the timeout error the
/// walk already treats as "stop and keep what was read".
fn within(deadline: Instant) -> Result<(), AxErr> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(AxErr::CANNOT_COMPLETE)
    }
}

/// A second, role-gated batch. Returns a slot accessor.
fn read_detail(
    element: &AxElement,
    attributes: &CFArray<CFString>,
    deadline: Instant,
    stats: &mut WalkStats,
) -> Result<impl Fn(usize) -> Option<CFRetained<CFType>>, AxErr> {
    within(deadline)?;
    stats.round_trips += 1;
    let values = match element.copy_multiple(attributes) {
        Ok(values) => values,
        Err(error) if error.is_timeout() => return Err(error),
        Err(_) => Vec::new(),
    };
    Ok(move |index: usize| {
        values
            .get(index)
            .and_then(|value| value.as_ref().ok())
            .cloned()
    })
}

fn settable(
    element: &AxElement,
    attribute: &CFString,
    deadline: Instant,
) -> Result<Option<bool>, AxErr> {
    within(deadline)?;
    match element.is_settable(attribute) {
        Ok(settable) => Ok(Some(settable)),
        Err(error) if error.is_timeout() => Err(error),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: Option<&str>) -> vocab::NodeDraft {
        vocab::NodeDraft {
            role: "group".to_string(),
            name: name.map(str::to_string),
            ..empty_draft()
        }
    }

    /// Builds a visited list from `(parent, name)` pairs in read order;
    /// AX handles are local application references (no IPC).
    fn visited(nodes: &[(Option<usize>, Option<&str>)]) -> Vec<Visited> {
        let mut visited: Vec<Visited> = Vec::new();
        for (index, (parent, name)) in nodes.iter().enumerate() {
            if let Some(parent) = parent {
                visited[*parent].children.push(index);
            }
            visited.push(Visited {
                element: AxElement::application(-1),
                parent: *parent,
                children: Vec::new(),
                draft: named(*name),
            });
        }
        visited
    }

    #[test]
    fn breadth_first_reads_are_emitted_in_pre_order_with_pruned_wrappers_skipped() {
        // Read order (breadth-first):
        // 0 window -> [1 toolbar, 2 unnamed wrapper]
        // 1 toolbar -> [3 Back]
        // 2 wrapper -> [4 Content]
        let nodes = visited(&[
            (None, Some("Window")),
            (Some(0), Some("Toolbar")),
            (Some(0), None),
            (Some(1), Some("Back")),
            (Some(2), Some("Content")),
        ]);
        let (elements, handles) = emit_pre_order(nodes, 42, 9, 100);
        let summary: Vec<(usize, Option<usize>, Option<&str>)> = elements
            .iter()
            .map(|node| (node.element_index, node.parent_index, node.name.as_deref()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (0, None, Some("Window")),
                (1, Some(0), Some("Toolbar")),
                (2, Some(1), Some("Back")),
                // The unnamed wrapper is pruned; its child re-parents to 0.
                (3, Some(0), Some("Content")),
            ]
        );
        assert_eq!(handles.len(), elements.len());
        assert_eq!(
            elements[3].backend_ref.as_deref(),
            Some("ax:pid=42;gen=9;idx=3")
        );
    }

    #[test]
    fn the_root_is_kept_even_when_uninformative_and_the_cap_drops_whole_subtrees() {
        let nodes = visited(&[
            (None, None),
            (Some(0), Some("A")),
            (Some(0), Some("B")),
            (Some(1), Some("A1")),
        ]);
        let (elements, _) = emit_pre_order(nodes, 1, 1, 2);
        let names: Vec<Option<&str>> = elements.iter().map(|node| node.name.as_deref()).collect();
        assert_eq!(names, vec![None, Some("A")]);
    }

    #[test]
    fn an_expired_deadline_stops_an_element_read_as_a_timeout() {
        let later = Instant::now() + std::time::Duration::from_secs(3600);
        assert!(within(later).is_ok());
        // The walk keeps the elements already read when a read times out.
        assert!(within(Instant::now()).unwrap_err().is_timeout());
    }

    #[test]
    fn children_past_the_per_node_cap_are_reported_as_dropped() {
        let (rows, dropped) = element::take_at_most(0..1000, MAX_CHILDREN_PER_NODE);
        assert_eq!(rows.len(), MAX_CHILDREN_PER_NODE);
        assert_eq!(rows.last(), Some(&(MAX_CHILDREN_PER_NODE - 1)));
        assert!(
            dropped,
            "a 1000-row table capped at the per-node limit is truncated"
        );

        let (rows, dropped) =
            element::take_at_most(0..MAX_CHILDREN_PER_NODE, MAX_CHILDREN_PER_NODE);
        assert_eq!(rows.len(), MAX_CHILDREN_PER_NODE);
        assert!(!dropped, "exactly the cap drops nothing");

        let (rows, dropped) = element::take_at_most(0..3, MAX_CHILDREN_PER_NODE);
        assert_eq!(rows, vec![0, 1, 2]);
        assert!(!dropped);
    }
}
