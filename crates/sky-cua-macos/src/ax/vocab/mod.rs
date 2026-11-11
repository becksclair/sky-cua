//! Pure mapping from raw AX attribute values to the shared element contract.
//!
//! Roles, state flags, and semantic action names use the vocabulary the
//! Windows UI Automation and Linux AT-SPI backends already emit, so the
//! service's selector matching and the client's rendering treat macOS
//! elements the same way.

use sky_cua_platform::model::{
    ElementNode, ElementNumericValueReadback, ElementTextReadback, ElementTextSelection,
};

use crate::coords;

/// Same cap as the Windows (`MAX_UIA_TEXT_READBACK_CHARS`) and Linux
/// (`MAX_TEXT_READBACK_CHARS`) backends.
pub(crate) const MAX_TEXT_READBACK_CHARS: usize = 4096;

/// Marker flag, like the Windows backend's `uia`.
pub(crate) const BACKEND_FLAG: &str = "ax";

/// The raw facts read for one element, already decoded from CF types.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RawAttrs {
    pub role: Option<String>,
    pub subrole: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub value: Option<RawValue>,
    pub identifier: Option<String>,
    pub position: Option<(f64, f64)>,
    pub size: Option<(f64, f64)>,
    pub enabled: Option<bool>,
    pub focused: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub disclosing: Option<bool>,
    pub number_of_characters: Option<i64>,
    pub selected_text_range: Option<(isize, isize)>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub placeholder: Option<String>,
    /// Raw AX action names (`AXPress`, `AXShowMenu`, ...).
    pub actions: Vec<String>,
    /// `AXUIElementIsAttributeSettable(AXValue)`, read only for editable and
    /// range roles.
    pub value_settable: Option<bool>,
    /// `AXUIElementIsAttributeSettable(AXFocused)`.
    pub focus_settable: Option<bool>,
    /// `AXUIElementIsAttributeSettable(AXSelected)`, read only for rows and
    /// cells.
    pub selected_settable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawValue {
    Text(String),
    Number(f64),
}

/// How an element's value is typed, which decides readback and `set_value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    /// Free text: text fields, text areas, combo boxes.
    Text,
    /// A bounded number: sliders, steppers, progress and level indicators,
    /// and scroll bars (whose value is the scroll position).
    Range,
    /// Checkbox-like 0/1/2 state.
    Toggle,
    Other,
}

pub(crate) fn value_kind(role: &str, subrole: Option<&str>) -> ValueKind {
    match role {
        "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField" => ValueKind::Text,
        "AXSlider"
        | "AXIncrementor"
        | "AXStepper"
        | "AXProgressIndicator"
        | "AXLevelIndicator"
        | "AXValueIndicator"
        | "AXScrollBar" => ValueKind::Range,
        "AXCheckBox" | "AXSwitch" | "AXToggle" => ValueKind::Toggle,
        "AXRadioButton" if subrole != Some("AXTabButton") => ValueKind::Toggle,
        _ => ValueKind::Other,
    }
}

/// Roles whose `AXValue` settability is worth one extra round trip.
pub(crate) fn wants_value_settable(role: &str, subrole: Option<&str>) -> bool {
    matches!(
        value_kind(role, subrole),
        ValueKind::Text | ValueKind::Range
    ) && !matches!(role, "AXProgressIndicator" | "AXLevelIndicator")
}

pub(crate) fn is_secure(subrole: Option<&str>) -> bool {
    subrole == Some("AXSecureTextField")
}

/// Maps an AX role (and subrole) to the shared role vocabulary.
pub(crate) fn role_name(role: &str, subrole: Option<&str>) -> String {
    let mapped = match (role, subrole) {
        ("AXRadioButton", Some("AXTabButton")) => "tab",
        ("AXRow", Some("AXOutlineRow")) => "tree_item",
        ("AXGroup", Some("AXTab")) => "tab",
        ("AXWindow", _) => "window",
        ("AXButton" | "AXMenuButton" | "AXDisclosureTriangle" | "AXColorWell", _) => "button",
        ("AXCheckBox" | "AXSwitch" | "AXToggle", _) => "checkbox",
        ("AXRadioButton", _) => "radio_button",
        ("AXPopUpButton" | "AXComboBox", _) => "combo_box",
        ("AXTextField" | "AXTextArea" | "AXStaticText" | "AXSearchField", _) => "text",
        ("AXLink", _) => "link",
        ("AXImage", _) => "image",
        ("AXList", _) => "list",
        ("AXOutline", _) => "tree",
        ("AXTable", _) => "table",
        ("AXRow", _) => "list_item",
        ("AXCell", _) => "cell",
        ("AXColumn", _) => "column",
        ("AXMenu", _) => "menu",
        ("AXMenuBar", _) => "menu_bar",
        ("AXMenuItem" | "AXMenuBarItem", _) => "menu_item",
        ("AXGroup" | "AXSplitGroup" | "AXRadioGroup" | "AXLayoutArea", _) => "group",
        ("AXScrollArea", _) => "pane",
        ("AXScrollBar", _) => "scroll_bar",
        ("AXSlider", _) => "slider",
        ("AXIncrementor" | "AXStepper", _) => "spinner",
        ("AXProgressIndicator" | "AXBusyIndicator", _) => "progress_bar",
        ("AXTabGroup", _) => "tab_list",
        ("AXToolbar", _) => "tool_bar",
        ("AXSplitter", _) => "separator",
        ("AXHeading", _) => "heading",
        ("AXWebArea" | "AXDocument", _) => "document",
        ("AXHelpTag", _) => "tool_tip",
        ("AXSheet" | "AXDrawer", _) => "dialog",
        ("AXApplication", _) => "application",
        ("AXUnknown", _) => "custom",
        _ => return slug_from_ax(role),
    };
    mapped.to_string()
}

/// `AXGrowArea` -> `grow_area`; empty or unknown -> `custom`.
fn slug_from_ax(role: &str) -> String {
    let stem = role.strip_prefix("AX").unwrap_or(role);
    let mut slug = String::with_capacity(stem.len() + 4);
    for (index, ch) in stem.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 && !slug.ends_with('_') {
                slug.push('_');
            }
            slug.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if !slug.ends_with('_') && !slug.is_empty() {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "custom".to_string()
    } else {
        slug
    }
}

/// Roles whose `AXExpanded == false` means "collapsed". Chromium and WebKit
/// report `AXExpanded = false` on every element, so a false value elsewhere
/// carries no information; a true value is always meaningful.
const COLLAPSIBLE_ROLES: &[&str] = &["AXComboBox", "AXPopUpButton", "AXMenuButton"];

/// Expanded state from `AXExpanded`, an outline row's `AXDisclosing`, or a
/// disclosure triangle's 0/1 value.
pub(crate) fn expanded_state(attrs: &RawAttrs) -> Option<bool> {
    let role = attrs.role.as_deref().unwrap_or_default();
    if role == "AXDisclosureTriangle"
        && let Some(RawValue::Number(value)) = attrs.value
    {
        return Some(value != 0.0);
    }
    if role == "AXRow"
        && attrs.subrole.as_deref() == Some("AXOutlineRow")
        && let Some(disclosing) = attrs.disclosing
    {
        return Some(disclosing);
    }
    match attrs.expanded {
        Some(true) => Some(true),
        Some(false) if COLLAPSIBLE_ROLES.contains(&role) => Some(false),
        _ => None,
    }
}

/// Roles whose `AXSelected` settability is worth one extra round trip.
pub(crate) fn wants_selected_settable(attrs: &RawAttrs) -> bool {
    attrs.selected.is_some() && matches!(attrs.role.as_deref(), Some("AXRow" | "AXCell"))
}

/// Static text never takes focus or carries its own actions in AppKit, and
/// in Chromium only exposes the universal `AXShowMenu`/`AXScrollToVisible`;
/// skipping its action and focus probes saves two round trips on the most
/// common web element.
pub(crate) fn wants_action_probes(role: &str) -> bool {
    role != "AXStaticText"
}

fn toggle_state(attrs: &RawAttrs) -> Option<&'static str> {
    let role = attrs.role.as_deref()?;
    if value_kind(role, attrs.subrole.as_deref()) != ValueKind::Toggle {
        return None;
    }
    match attrs.value {
        Some(RawValue::Number(0.0)) => Some("toggle_off"),
        Some(RawValue::Number(1.0)) => Some("toggle_on"),
        Some(RawValue::Number(2.0)) => Some("toggle_indeterminate"),
        _ => None,
    }
}

fn has_action(attrs: &RawAttrs, name: &str) -> bool {
    attrs.actions.iter().any(|action| action == name)
}

/// Whether `SelectElement` has an AX path on this element.
pub(crate) fn selectable(attrs: &RawAttrs) -> bool {
    if has_action(attrs, "AXPick") || attrs.selected_settable == Some(true) {
        return true;
    }
    let role = role_name(
        attrs.role.as_deref().unwrap_or_default(),
        attrs.subrole.as_deref(),
    );
    role == "tab" && has_action(attrs, "AXPress")
}

pub(crate) fn toggleable(attrs: &RawAttrs) -> bool {
    has_action(attrs, "AXPress")
        && attrs
            .role
            .as_deref()
            .is_some_and(|role| value_kind(role, attrs.subrole.as_deref()) == ValueKind::Toggle)
}

/// Whether `SetValue` has an AX path: a settable text or range value on a
/// non-secure element.
pub(crate) fn value_editable(attrs: &RawAttrs) -> bool {
    attrs.value_settable == Some(true) && !is_secure(attrs.subrole.as_deref())
}

/// Semantic action names in the shared vocabulary.
pub(crate) fn semantic_actions(attrs: &RawAttrs) -> Vec<String> {
    let mut actions: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        if !actions.iter().any(|existing| existing == name) {
            actions.push(name.to_string());
        }
    };
    let press = has_action(attrs, "AXPress");
    if press {
        push("click");
    }
    if attrs.focus_settable == Some(true) {
        push("focus");
    }
    if press {
        push("activate");
    }
    if selectable(attrs) {
        push("select");
    }
    match expanded_state(attrs) {
        Some(false) => push("expand"),
        Some(true) => push("collapse"),
        None => {}
    }
    if toggleable(attrs) {
        push("toggle");
    }
    if value_editable(attrs) {
        push("set_value");
    }
    if has_action(attrs, "AXShowMenu") {
        push("showmenu");
    }
    for action in &attrs.actions {
        if let Some(extra) = passthrough_action(action) {
            push(extra);
        }
    }
    actions
}

/// Standard AX actions with no shared-vocabulary equivalent that are still
/// worth advertising for `perform_action`.
fn passthrough_action(action: &str) -> Option<&'static str> {
    match action {
        "AXIncrement" => Some("increment"),
        "AXDecrement" => Some("decrement"),
        "AXConfirm" => Some("confirm"),
        "AXCancel" => Some("cancel"),
        "AXRaise" => Some("raise"),
        _ => None,
    }
}

pub(crate) fn state_flags(attrs: &RawAttrs, offscreen: bool) -> Vec<String> {
    let mut flags = vec![BACKEND_FLAG.to_string()];
    match attrs.enabled {
        Some(true) => flags.push("enabled".to_string()),
        Some(false) => flags.push("disabled".to_string()),
        None => {}
    }
    if attrs.focus_settable == Some(true) {
        flags.push("focusable".to_string());
    }
    if attrs.focused == Some(true) {
        flags.push("focused".to_string());
    }
    if offscreen {
        flags.push("offscreen".to_string());
    }
    if attrs.selected == Some(true) {
        flags.push("selected".to_string());
    }
    if is_secure(attrs.subrole.as_deref()) {
        flags.push("password".to_string());
    }
    if attrs.value_settable == Some(false)
        && attrs
            .role
            .as_deref()
            .is_some_and(|role| value_kind(role, attrs.subrole.as_deref()) == ValueKind::Text)
    {
        flags.push("readonly".to_string());
    }
    match expanded_state(attrs) {
        Some(true) => flags.push("expanded".to_string()),
        Some(false) => flags.push("collapsed".to_string()),
        None => {}
    }
    if let Some(state) = toggle_state(attrs) {
        flags.push(state.to_string());
    }
    flags
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty()).cloned()
}

fn cap_chars(value: &str) -> (String, bool) {
    match value.char_indices().nth(MAX_TEXT_READBACK_CHARS) {
        Some((byte, _)) => (value[..byte].to_string(), true),
        None => (value.to_string(), false),
    }
}

/// Text readback for editable text roles, shaped like the other backends'.
pub(crate) fn text_readback(attrs: &RawAttrs) -> Option<ElementTextReadback> {
    let role = attrs.role.as_deref()?;
    if value_kind(role, attrs.subrole.as_deref()) != ValueKind::Text {
        return None;
    }
    let sensitive = is_secure(attrs.subrole.as_deref());
    let text = match &attrs.value {
        Some(RawValue::Text(text)) => Some(text.as_str()),
        _ => None,
    };
    let full_len = attrs
        .number_of_characters
        .or_else(|| text.map(|text| text.encode_utf16().count() as i64))
        .unwrap_or(0)
        .max(0);
    let (content, truncated) = match text {
        Some(text) if !sensitive => {
            let (content, capped) = cap_chars(text);
            (Some(content), capped)
        }
        _ => (None, false),
    };
    let truncated = truncated || full_len > MAX_TEXT_READBACK_CHARS as i64;
    let (caret_offset, selections) = match attrs.selected_text_range {
        Some((location, length)) if location >= 0 && length >= 0 => {
            let start = i32::try_from(location).unwrap_or(i32::MAX);
            let end = i32::try_from(location + length).unwrap_or(i32::MAX);
            if length == 0 {
                (Some(start), Vec::new())
            } else {
                (
                    Some(end),
                    vec![ElementTextSelection {
                        start_offset: start,
                        end_offset: end,
                    }],
                )
            }
        }
        _ => (None, Vec::new()),
    };
    Some(ElementTextReadback {
        character_count: i32::try_from(full_len).unwrap_or(i32::MAX),
        caret_offset,
        content,
        content_suppressed: sensitive,
        truncated,
        selections,
    })
}

pub(crate) fn numeric_readback(attrs: &RawAttrs) -> Option<ElementNumericValueReadback> {
    let role = attrs.role.as_deref()?;
    if value_kind(role, attrs.subrole.as_deref()) != ValueKind::Range {
        return None;
    }
    let Some(RawValue::Number(current)) = attrs.value else {
        return None;
    };
    // A scroll bar reports its position as a fraction and carries no
    // AXMinValue/AXMaxValue, so its range is the unit interval by definition.
    let (default_minimum, default_maximum) = if role == "AXScrollBar" {
        (0.0, 1.0)
    } else {
        (current, current)
    };
    Some(ElementNumericValueReadback {
        current,
        minimum: attrs.min_value.unwrap_or(default_minimum),
        maximum: attrs.max_value.unwrap_or(default_maximum),
        // AX has no standard increment attribute; 0.0 is the shared
        // "no defined increment" sentinel the other backends use.
        minimum_increment: 0.0,
        text: None,
    })
}

/// The accessible name: `AXTitle`, else `AXDescription`, else the text of a
/// static-text element (macOS keeps label text in `AXValue`).
fn name_for(attrs: &RawAttrs) -> (Option<String>, NameSource) {
    if let Some(title) = non_empty(attrs.title.as_ref()) {
        return (Some(title), NameSource::Title);
    }
    if let Some(description) = non_empty(attrs.description.as_ref()) {
        return (Some(description), NameSource::Description);
    }
    if attrs.role.as_deref() == Some("AXStaticText")
        && let Some(RawValue::Text(text)) = &attrs.value
        && !text.trim().is_empty()
    {
        return (Some(cap_chars(text).0), NameSource::Value);
    }
    (None, NameSource::None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameSource {
    Title,
    Description,
    Value,
    None,
}

fn value_for(
    attrs: &RawAttrs,
    name_source: NameSource,
    text: Option<&ElementTextReadback>,
    numeric: Option<&ElementNumericValueReadback>,
) -> Option<String> {
    if is_secure(attrs.subrole.as_deref()) || name_source == NameSource::Value {
        return None;
    }
    if let Some(text) = text {
        return text.content.clone();
    }
    if let Some(numeric) = numeric {
        return Some(numeric.current.to_string());
    }
    match &attrs.value {
        Some(RawValue::Text(value)) if !value.trim().is_empty() => Some(cap_chars(value).0),
        _ => None,
    }
}

fn description_for(attrs: &RawAttrs, name_source: NameSource) -> Option<String> {
    let mut parts = Vec::new();
    if name_source == NameSource::Title
        && let Some(description) = non_empty(attrs.description.as_ref())
        && attrs.title.as_ref() != Some(&description)
    {
        parts.push(description);
    }
    if let Some(placeholder) = non_empty(attrs.placeholder.as_ref()) {
        parts.push(format!("placeholder={placeholder}"));
    }
    if let Some(role) = attrs.role.as_deref() {
        parts.push(format!("ax_role={role}"));
    }
    if let Some(subrole) = non_empty(attrs.subrole.as_ref()) {
        parts.push(format!("subrole={subrole}"));
    }
    if let Some(identifier) = non_empty(attrs.identifier.as_ref()) {
        parts.push(format!("identifier={identifier}"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Desktop-point frame, `None` for empty frames (as the Windows backend
/// drops zero-area UIA rectangles).
pub(crate) fn desktop_frame(attrs: &RawAttrs) -> Option<(f64, f64, f64, f64)> {
    let ((x, y), (width, height)) = (attrs.position?, attrs.size?);
    (width > 0.0 && height > 0.0 && x.is_finite() && y.is_finite()).then_some((x, y, width, height))
}

fn intersects(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// A node ready for emission, before its final index is known.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeDraft {
    pub role: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub text: Option<ElementTextReadback>,
    pub numeric_value: Option<ElementNumericValueReadback>,
    pub supports_editable_text: bool,
    pub state_flags: Vec<String>,
    pub semantic_actions: Vec<String>,
    pub frame: Option<(f64, f64, f64, f64)>,
}

impl NodeDraft {
    /// Same pruning rule as the Linux AT-SPI walker: an element with no name,
    /// value, readback, action, or focus carries no information and its
    /// children are re-parented to the nearest kept ancestor.
    pub(crate) fn is_informative(&self) -> bool {
        self.name.is_some()
            || self.value.is_some()
            || self.text.is_some()
            || self.numeric_value.is_some()
            || !self.semantic_actions.is_empty()
            || self.state_flags.iter().any(|flag| flag == "focused")
    }

    pub(crate) fn into_node(
        self,
        element_index: usize,
        parent_index: Option<usize>,
        backend_ref: String,
    ) -> ElementNode {
        let bounds = self
            .frame
            .map(|(x, y, width, height)| coords::desktop_rect(x, y, width, height));
        ElementNode {
            element_index,
            parent_index,
            role: self.role,
            name: self.name,
            description: self.description,
            value: self.value,
            text: self.text,
            numeric_value: self.numeric_value,
            supports_editable_text: self.supports_editable_text,
            state_flags: self.state_flags,
            semantic_actions: self.semantic_actions,
            bounds,
            backend_ref: Some(backend_ref),
        }
    }
}

/// Builds the contract-facing draft for one element. `window_frame` is the
/// root window's frame, used for the `offscreen` flag.
pub(crate) fn draft(attrs: &RawAttrs, window_frame: Option<(f64, f64, f64, f64)>) -> NodeDraft {
    let role = role_name(
        attrs.role.as_deref().unwrap_or("AXUnknown"),
        attrs.subrole.as_deref(),
    );
    let frame = desktop_frame(attrs);
    let offscreen = match (frame, window_frame) {
        (Some(frame), Some(window)) => !intersects(frame, window),
        _ => false,
    };
    let (name, name_source) = name_for(attrs);
    let text = text_readback(attrs);
    let numeric_value = numeric_readback(attrs);
    let value = value_for(attrs, name_source, text.as_ref(), numeric_value.as_ref());
    let supports_editable_text = value_editable(attrs)
        && attrs
            .role
            .as_deref()
            .is_some_and(|role| value_kind(role, attrs.subrole.as_deref()) == ValueKind::Text);
    NodeDraft {
        role,
        name,
        description: description_for(attrs, name_source),
        value,
        text,
        numeric_value,
        supports_editable_text,
        state_flags: state_flags(attrs, offscreen),
        semantic_actions: semantic_actions(attrs),
        frame,
    }
}

#[cfg(test)]
mod tests;
