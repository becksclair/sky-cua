use std::ffi::c_void;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, CaptureInfo, CoordinateSpace, ElementNode,
    ElementNumericValueReadback, ElementTextReadback, RectF,
};
use windows::Win32::Foundation::{HWND as UiaHwnd, RECT as UiaRect, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, ExpandCollapseState, ExpandCollapseState_Collapsed,
    ExpandCollapseState_Expanded, ExpandCollapseState_LeafNode,
    ExpandCollapseState_PartiallyExpanded, IUIAutomation, IUIAutomationElement,
    IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationRangeValuePattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTextPattern, IUIAutomationTogglePattern,
    IUIAutomationTreeWalker, IUIAutomationValuePattern, ToggleState, ToggleState_Indeterminate,
    ToggleState_Off, ToggleState_On, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_ComboBoxControlTypeId, UIA_CustomControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_ExpandCollapsePatternId, UIA_GroupControlTypeId,
    UIA_HeaderControlTypeId, UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId,
    UIA_InvokePatternId, UIA_LegacyIAccessiblePatternId, UIA_ListControlTypeId,
    UIA_ListItemControlTypeId, UIA_MenuBarControlTypeId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_PaneControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_RangeValuePatternId, UIA_ScrollBarControlTypeId, UIA_SelectionItemPatternId,
    UIA_SemanticZoomControlTypeId, UIA_SeparatorControlTypeId, UIA_SliderControlTypeId,
    UIA_SpinnerControlTypeId, UIA_SplitButtonControlTypeId, UIA_StatusBarControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TextControlTypeId, UIA_TextPatternId,
    UIA_TogglePatternId, UIA_ToolBarControlTypeId, UIA_ToolTipControlTypeId, UIA_TreeControlTypeId,
    UIA_TreeItemControlTypeId, UIA_ValuePatternId, UIA_WindowControlTypeId,
};
use windows::core::BSTR;

const MAX_UIA_NODES: usize = 512;
const MAX_UIA_DEPTH: usize = 10;
const MAX_UIA_CHILDREN_PER_NODE: usize = 250;
/// Mirrors the AT-SPI text readback cap in `sky-cua-linux` (`MAX_TEXT_READBACK_CHARS`)
/// so both backends bound `ElementNode.text.content` to the same size.
const MAX_UIA_TEXT_READBACK_CHARS: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
struct ParsedUiaRef {
    hwnd: usize,
    path: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct UiaActionCall {
    target: ParsedUiaRef,
    action: UiaActionKind,
}

#[derive(Debug, Clone, PartialEq)]
enum UiaActionKind {
    Click,
    Focus,
    Activate,
    Select,
    Expand,
    Collapse,
    Toggle,
    SetValue(String),
}

impl UiaActionKind {
    fn requires_uia_success(&self) -> bool {
        matches!(
            self,
            Self::Focus
                | Self::Activate
                | Self::Select
                | Self::Expand
                | Self::Collapse
                | Self::Toggle
        )
    }

    fn primitive_name(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Focus => "focus",
            Self::Activate => "activate",
            Self::Select => "select",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::Toggle => "toggle",
            Self::SetValue(_) => "set_value",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DesktopRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct UiaElementInfo {
    path: Vec<usize>,
    parent_index: Option<usize>,
    control_type: i32,
    localized_control_type: Option<String>,
    name: Option<String>,
    automation_id: Option<String>,
    class_name: Option<String>,
    framework_id: Option<String>,
    value: Option<String>,
    enabled: Option<bool>,
    focusable: Option<bool>,
    focused: Option<bool>,
    offscreen: Option<bool>,
    password: Option<bool>,
    readonly: Option<bool>,
    has_invoke: bool,
    has_selection_item: bool,
    has_expand_collapse: bool,
    expand_collapse_state: Option<&'static str>,
    has_toggle: bool,
    toggle_state: Option<&'static str>,
    has_legacy_default_action: bool,
    has_value: bool,
    text: Option<ElementTextReadback>,
    numeric_value: Option<ElementNumericValueReadback>,
    bounds: Option<DesktopRect>,
}

pub(super) fn is_available() -> bool {
    let Ok(_apartment) = ComApartment::initialize() else {
        return false;
    };
    create_automation().is_ok()
}

pub(super) fn collect_elements_for_hwnd(
    hwnd: usize,
    window_title: &str,
    window_bounds: &RectF,
    capture: Option<&CaptureInfo>,
) -> Result<Vec<ElementNode>, BackendError> {
    let _apartment = ComApartment::initialize()?;
    let automation = create_automation()?;
    let root = unsafe { automation.ElementFromHandle(uia_hwnd(hwnd)) }
        .map_err(|error| uia_error("UI Automation could not resolve the selected HWND", error))?;
    let walker = unsafe { automation.ControlViewWalker() }
        .map_err(|error| uia_error("UI Automation control-view walker is unavailable", error))?;

    let mut nodes = Vec::new();
    let mut stack = vec![PendingElement {
        element: root,
        path: Vec::new(),
        parent_index: None,
        depth: 0,
    }];

    while let Some(pending) = stack.pop() {
        if nodes.len() >= MAX_UIA_NODES {
            break;
        }
        let info = read_element_info(
            &pending.element,
            pending.path.clone(),
            pending.parent_index,
            window_title,
        );
        let keep = pending.parent_index.is_none() || is_interesting_node(&info);
        let current_parent = if keep {
            let index = nodes.len();
            nodes.push(element_node_from_info(
                index,
                hwnd,
                info,
                capture,
                window_bounds,
            ));
            Some(index)
        } else {
            pending.parent_index
        };

        if pending.depth < MAX_UIA_DEPTH {
            let mut children = children_for(&walker, &pending.element);
            children.reverse();
            for (child_ordinal, child) in children {
                let mut child_path = pending.path.clone();
                child_path.push(child_ordinal);
                stack.push(PendingElement {
                    element: child,
                    path: child_path,
                    parent_index: current_parent,
                    depth: pending.depth + 1,
                });
            }
        }
    }

    Ok(nodes)
}

pub(super) fn try_execute_semantic_action(
    request: &ActionRequest,
) -> Result<Option<ActionOutcome>, BackendError> {
    let Some(call) = uia_action_for_request(request)? else {
        return Ok(None);
    };

    match execute_uia_action(&call) {
        Ok(true) => Ok(Some(match call.action {
            UiaActionKind::Click => success("Activated the element through Windows UI Automation."),
            UiaActionKind::Focus => success("Focused the element through Windows UI Automation."),
            UiaActionKind::Activate => {
                success("Activated the element through Windows UI Automation.")
            }
            UiaActionKind::Select => success("Selected the element through Windows UI Automation."),
            UiaActionKind::Expand => success("Expanded the element through Windows UI Automation."),
            UiaActionKind::Collapse => {
                success("Collapsed the element through Windows UI Automation.")
            }
            UiaActionKind::Toggle => success("Toggled the element through Windows UI Automation."),
            UiaActionKind::SetValue(_) => {
                success("Set the value through Windows UI Automation ValuePattern.")
            }
        })),
        Ok(false) if call.action.requires_uia_success() => Err(BackendError::new(
            BackendErrorCode::ActionRequiresPhysicalInput,
            format!(
                "Windows UI Automation {} was unavailable for the selected element",
                call.action.primitive_name()
            ),
        )),
        Ok(false) => Ok(None),
        Err(error) => {
            if call.action.requires_uia_success() {
                return Err(error);
            }
            tracing::debug!(
                code = error.code,
                message = %error.message,
                "Windows UI Automation semantic action failed; falling back to physical input"
            );
            Ok(None)
        }
    }
}

fn create_automation() -> Result<IUIAutomation, BackendError> {
    unsafe {
        CoCreateInstance(
            &CUIAutomation,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
    }
    .map_err(|error| uia_error("failed to create CUIAutomation", error))
}

fn execute_uia_action(call: &UiaActionCall) -> Result<bool, BackendError> {
    let _apartment = ComApartment::initialize()?;
    let automation = create_automation()?;
    let element = resolve_element_by_path(&automation, &call.target)?;

    match &call.action {
        UiaActionKind::Click => execute_uia_click(&element),
        UiaActionKind::Focus => execute_uia_focus(&element),
        UiaActionKind::Activate => execute_uia_activate(&element),
        UiaActionKind::Select => execute_uia_select(&element),
        UiaActionKind::Expand => execute_uia_expand(&element),
        UiaActionKind::Collapse => execute_uia_collapse(&element),
        UiaActionKind::Toggle => execute_uia_toggle(&element),
        UiaActionKind::SetValue(value) => {
            let Ok(pattern) = (unsafe {
                element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            }) else {
                return Ok(false);
            };
            let readonly = unsafe { pattern.CurrentIsReadOnly() }
                .map(|value| value.as_bool())
                .unwrap_or(true);
            if readonly {
                return Ok(false);
            }
            let value = BSTR::from(value.as_str());
            unsafe { pattern.SetValue(&value) }
                .map_err(|error| uia_error("UI Automation ValuePattern SetValue failed", error))?;
            Ok(true)
        }
    }
}

fn execute_uia_focus(element: &IUIAutomationElement) -> Result<bool, BackendError> {
    unsafe { element.SetFocus() }
        .map_err(|error| uia_error("UI Automation SetFocus failed", error))?;
    Ok(true)
}

fn execute_uia_click(element: &IUIAutomationElement) -> Result<bool, BackendError> {
    if execute_uia_activate(element)? {
        return Ok(true);
    }

    if execute_uia_select(element)? {
        return Ok(true);
    }

    if let Ok(pattern) = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    } {
        let state = unsafe { pattern.CurrentExpandCollapseState() }
            .map_err(|error| uia_error("UI Automation ExpandCollapse state failed", error))?;
        if state == ExpandCollapseState_Expanded {
            unsafe { pattern.Collapse() }.map_err(|error| {
                uia_error("UI Automation ExpandCollapse collapse failed", error)
            })?;
            return Ok(true);
        }
        if state == ExpandCollapseState_Collapsed || state == ExpandCollapseState_PartiallyExpanded
        {
            unsafe { pattern.Expand() }
                .map_err(|error| uia_error("UI Automation ExpandCollapse expand failed", error))?;
            return Ok(true);
        }
        if state != ExpandCollapseState_LeafNode {
            return Ok(false);
        }
    }

    execute_uia_toggle(element)
}

fn execute_uia_activate(element: &IUIAutomationElement) -> Result<bool, BackendError> {
    if let Ok(pattern) =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) }
    {
        unsafe { pattern.Invoke() }
            .map_err(|error| uia_error("UI Automation InvokePattern failed", error))?;
        return Ok(true);
    }

    if let Ok(pattern) = unsafe {
        element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
            UIA_LegacyIAccessiblePatternId,
        )
    } {
        if bstr_property(|| unsafe { pattern.CurrentDefaultAction() }).is_some() {
            unsafe { pattern.DoDefaultAction() }.map_err(|error| {
                uia_error(
                    "UI Automation LegacyIAccessible default action failed",
                    error,
                )
            })?;
            return Ok(true);
        }
    }

    Ok(false)
}

fn execute_uia_select(element: &IUIAutomationElement) -> Result<bool, BackendError> {
    if let Ok(pattern) = unsafe {
        element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
    } {
        unsafe { pattern.Select() }
            .map_err(|error| uia_error("UI Automation SelectionItemPattern failed", error))?;
        return Ok(true);
    }

    Ok(false)
}

fn execute_uia_expand(element: &IUIAutomationElement) -> Result<bool, BackendError> {
    if let Ok(pattern) = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    } {
        let state = unsafe { pattern.CurrentExpandCollapseState() }
            .map_err(|error| uia_error("UI Automation ExpandCollapse state failed", error))?;
        if state == ExpandCollapseState_Collapsed || state == ExpandCollapseState_PartiallyExpanded
        {
            unsafe { pattern.Expand() }
                .map_err(|error| uia_error("UI Automation ExpandCollapse expand failed", error))?;
            return Ok(true);
        }
    }

    Ok(false)
}

fn execute_uia_collapse(element: &IUIAutomationElement) -> Result<bool, BackendError> {
    if let Ok(pattern) = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    } {
        let state = unsafe { pattern.CurrentExpandCollapseState() }
            .map_err(|error| uia_error("UI Automation ExpandCollapse state failed", error))?;
        if state == ExpandCollapseState_Expanded || state == ExpandCollapseState_PartiallyExpanded {
            unsafe { pattern.Collapse() }.map_err(|error| {
                uia_error("UI Automation ExpandCollapse collapse failed", error)
            })?;
            return Ok(true);
        }
    }

    Ok(false)
}

fn execute_uia_toggle(element: &IUIAutomationElement) -> Result<bool, BackendError> {
    if let Ok(pattern) =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) }
    {
        unsafe { pattern.Toggle() }
            .map_err(|error| uia_error("UI Automation TogglePattern failed", error))?;
        return Ok(true);
    }

    Ok(false)
}

fn resolve_element_by_path(
    automation: &IUIAutomation,
    target: &ParsedUiaRef,
) -> Result<IUIAutomationElement, BackendError> {
    let mut element = unsafe { automation.ElementFromHandle(uia_hwnd(target.hwnd)) }
        .map_err(|error| uia_error("UI Automation could not resolve the target HWND", error))?;
    let walker = unsafe { automation.ControlViewWalker() }
        .map_err(|error| uia_error("UI Automation control-view walker is unavailable", error))?;

    for child_ordinal in &target.path {
        element = nth_child(&walker, &element, *child_ordinal).ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionRequiresPhysicalInput,
                "UI Automation target path is no longer present in the current tree",
            )
        })?;
    }

    Ok(element)
}

fn nth_child(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
    index: usize,
) -> Option<IUIAutomationElement> {
    let mut current = unsafe { walker.GetFirstChildElement(element) }.ok()?;
    for _ in 0..index {
        current = unsafe { walker.GetNextSiblingElement(&current) }.ok()?;
    }
    Some(current)
}

struct PendingElement {
    element: IUIAutomationElement,
    path: Vec<usize>,
    parent_index: Option<usize>,
    depth: usize,
}

fn children_for(
    walker: &IUIAutomationTreeWalker,
    element: &IUIAutomationElement,
) -> Vec<(usize, IUIAutomationElement)> {
    let mut children = Vec::new();
    let Ok(mut child) = (unsafe { walker.GetFirstChildElement(element) }) else {
        return children;
    };
    let mut ordinal = 0usize;
    while children.len() < MAX_UIA_CHILDREN_PER_NODE {
        children.push((ordinal, child.clone()));
        ordinal += 1;
        let Ok(next) = (unsafe { walker.GetNextSiblingElement(&child) }) else {
            break;
        };
        child = next;
    }
    children
}

fn read_element_info(
    element: &IUIAutomationElement,
    path: Vec<usize>,
    parent_index: Option<usize>,
    window_title: &str,
) -> UiaElementInfo {
    let control_type = unsafe { element.CurrentControlType() }
        .map(|control_type| control_type.0)
        .unwrap_or(UIA_CustomControlTypeId.0);
    let password = bool_property(|| unsafe { element.CurrentIsPassword() });
    let value_pattern =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }
            .ok();
    let readonly = value_pattern
        .as_ref()
        .and_then(|pattern| unsafe { pattern.CurrentIsReadOnly() }.ok())
        .map(|value| value.as_bool());
    let value = if password == Some(true) {
        None
    } else {
        value_pattern
            .as_ref()
            .and_then(|pattern| bstr_property(|| unsafe { pattern.CurrentValue() }))
    };
    let expand_collapse_pattern = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    }
    .ok();
    let expand_collapse_state = expand_collapse_pattern
        .as_ref()
        .and_then(|pattern| unsafe { pattern.CurrentExpandCollapseState() }.ok())
        .and_then(expand_collapse_state_name);
    let toggle_pattern =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) }
            .ok();
    let toggle_state = toggle_pattern
        .as_ref()
        .and_then(|pattern| unsafe { pattern.CurrentToggleState() }.ok())
        .and_then(toggle_state_name);
    let range_value_pattern = unsafe {
        element.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId)
    }
    .ok();
    let numeric_value = if password == Some(true) {
        None
    } else {
        range_value_pattern
            .as_ref()
            .and_then(read_range_value_readback)
    };
    // Cheapest source first: an already-fetched ValuePattern string costs no
    // extra COM round trip. Only probe TextPattern (an extra live call) when
    // there is no ValuePattern value to fall back on.
    let text = if password == Some(true) {
        None
    } else if let Some(value) = value.as_ref() {
        Some(text_readback_from_value(value))
    } else {
        let text_pattern =
            unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
                .ok();
        text_pattern.as_ref().and_then(read_text_pattern_readback)
    };

    UiaElementInfo {
        path,
        parent_index,
        control_type,
        localized_control_type: bstr_property(|| unsafe { element.CurrentLocalizedControlType() }),
        name: bstr_property(|| unsafe { element.CurrentName() })
            .or_else(|| parent_index.is_none().then(|| window_title.to_string())),
        automation_id: bstr_property(|| unsafe { element.CurrentAutomationId() }),
        class_name: bstr_property(|| unsafe { element.CurrentClassName() }),
        framework_id: bstr_property(|| unsafe { element.CurrentFrameworkId() }),
        value,
        enabled: bool_property(|| unsafe { element.CurrentIsEnabled() }),
        focusable: bool_property(|| unsafe { element.CurrentIsKeyboardFocusable() }),
        focused: bool_property(|| unsafe { element.CurrentHasKeyboardFocus() }),
        offscreen: bool_property(|| unsafe { element.CurrentIsOffscreen() }),
        password,
        readonly,
        has_invoke: unsafe {
            element
                .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                .is_ok()
        },
        has_selection_item: unsafe {
            element
                .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                    UIA_SelectionItemPatternId,
                )
                .is_ok()
        },
        has_expand_collapse: expand_collapse_pattern.is_some(),
        expand_collapse_state,
        has_toggle: toggle_pattern.is_some(),
        toggle_state,
        has_legacy_default_action: unsafe {
            element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                UIA_LegacyIAccessiblePatternId,
            )
        }
        .ok()
        .and_then(|pattern| bstr_property(|| unsafe { pattern.CurrentDefaultAction() }))
        .is_some(),
        has_value: value_pattern.is_some(),
        text,
        numeric_value,
        bounds: unsafe { element.CurrentBoundingRectangle() }
            .ok()
            .and_then(desktop_rect_from_uia),
    }
}

fn read_range_value_readback(
    pattern: &IUIAutomationRangeValuePattern,
) -> Option<ElementNumericValueReadback> {
    let current = unsafe { pattern.CurrentValue() }.ok()?;
    let minimum = unsafe { pattern.CurrentMinimum() }.ok()?;
    let maximum = unsafe { pattern.CurrentMaximum() }.ok()?;
    // CurrentSmallChange is not implemented by every RangeValuePattern provider;
    // 0.0 is the same "no defined increment" sentinel the Linux AT-SPI reader
    // and the scroll-step fallback in the Linux backend already use.
    let minimum_increment = unsafe { pattern.CurrentSmallChange() }.unwrap_or(0.0);
    Some(ElementNumericValueReadback {
        current,
        minimum,
        maximum,
        minimum_increment,
        text: None,
    })
}

fn text_readback_from_value(value: &str) -> ElementTextReadback {
    let full_len = value.chars().count();
    let truncated = full_len > MAX_UIA_TEXT_READBACK_CHARS;
    let content: String = value.chars().take(MAX_UIA_TEXT_READBACK_CHARS).collect();
    ElementTextReadback {
        character_count: i32::try_from(full_len).unwrap_or(i32::MAX),
        caret_offset: None,
        content: Some(content),
        content_suppressed: false,
        truncated,
        selections: Vec::new(),
    }
}

fn read_text_pattern_readback(pattern: &IUIAutomationTextPattern) -> Option<ElementTextReadback> {
    let document_range = unsafe { pattern.DocumentRange() }.ok()?;
    // Request one character past the cap so a full-length response can be told
    // apart from a response truncated at the cap, without fetching arbitrarily
    // large documents whole (unlike the ValuePattern path, TextPattern exposes
    // no lightweight character-count-only property).
    let capped_len = i32::try_from(MAX_UIA_TEXT_READBACK_CHARS + 1).unwrap_or(i32::MAX);
    let text = unsafe { document_range.GetText(capped_len) }
        .ok()?
        .to_string();
    let content: String = text.chars().take(MAX_UIA_TEXT_READBACK_CHARS).collect();
    let truncated = text.chars().count() > MAX_UIA_TEXT_READBACK_CHARS;
    Some(ElementTextReadback {
        // The true document length is unknown when truncated (see above); report
        // the bounded content length rather than overclaiming an exact count.
        character_count: i32::try_from(content.chars().count()).unwrap_or(i32::MAX),
        caret_offset: None,
        content: Some(content),
        content_suppressed: false,
        truncated,
        selections: Vec::new(),
    })
}

fn element_node_from_info(
    index: usize,
    hwnd: usize,
    info: UiaElementInfo,
    capture: Option<&CaptureInfo>,
    window_bounds: &RectF,
) -> ElementNode {
    let mut state_flags = vec!["uia".to_string()];
    push_bool_flag(&mut state_flags, "enabled", info.enabled);
    push_inverse_bool_flag(&mut state_flags, "disabled", info.enabled);
    push_bool_flag(&mut state_flags, "focusable", info.focusable);
    push_bool_flag(&mut state_flags, "focused", info.focused);
    push_bool_flag(&mut state_flags, "offscreen", info.offscreen);
    push_bool_flag(&mut state_flags, "password", info.password);
    push_bool_flag(&mut state_flags, "readonly", info.readonly);
    if let Some(state) = info.expand_collapse_state {
        state_flags.push(state.to_string());
    }
    if let Some(state) = info.toggle_state {
        state_flags.push(state.to_string());
    }

    let mut semantic_actions = Vec::new();
    if has_click_semantics(&info) {
        semantic_actions.push("click".to_string());
    }
    if info.focusable == Some(true) {
        semantic_actions.push("focus".to_string());
    }
    if info.has_invoke || info.has_legacy_default_action {
        semantic_actions.push("activate".to_string());
    }
    if info.has_selection_item {
        semantic_actions.push("select".to_string());
    }
    if info.has_expand_collapse {
        match info.expand_collapse_state {
            Some("collapsed") => semantic_actions.push("expand".to_string()),
            Some("expanded") => semantic_actions.push("collapse".to_string()),
            Some("partially_expanded") => {
                semantic_actions.push("expand".to_string());
                semantic_actions.push("collapse".to_string());
            }
            Some("leaf") => {}
            _ => {
                semantic_actions.push("expand".to_string());
                semantic_actions.push("collapse".to_string());
            }
        }
    }
    if info.has_toggle {
        semantic_actions.push("toggle".to_string());
    }
    let supports_editable_text =
        info.has_value && info.readonly != Some(true) && info.password != Some(true);
    if supports_editable_text {
        semantic_actions.push("set_value".to_string());
    }

    let description = description_for(&info);
    ElementNode {
        element_index: index,
        parent_index: info.parent_index,
        role: role_for_control_type(info.control_type, info.localized_control_type.as_deref()),
        name: info.name,
        description,
        value: info.value,
        text: info.text,
        numeric_value: info.numeric_value,
        supports_editable_text,
        state_flags,
        semantic_actions,
        bounds: info
            .bounds
            .and_then(|bounds| desktop_rect_to_stream_rect(bounds, capture, window_bounds)),
        backend_ref: Some(backend_ref(hwnd, &info.path)),
    }
}

fn is_interesting_node(info: &UiaElementInfo) -> bool {
    info.name.is_some()
        || info.value.is_some()
        || has_click_semantics(info)
        || info.has_value
        || info.focusable == Some(true)
        || matches!(
            info.control_type,
            value if value == UIA_ButtonControlTypeId.0
                || value == UIA_EditControlTypeId.0
                || value == UIA_MenuItemControlTypeId.0
                || value == UIA_TabItemControlTypeId.0
                || value == UIA_WindowControlTypeId.0
        )
}

fn has_click_semantics(info: &UiaElementInfo) -> bool {
    info.has_invoke
        || info.has_selection_item
        || info.has_expand_collapse
        || info.has_toggle
        || info.has_legacy_default_action
}

fn expand_collapse_state_name(state: ExpandCollapseState) -> Option<&'static str> {
    if state == ExpandCollapseState_Collapsed {
        Some("collapsed")
    } else if state == ExpandCollapseState_Expanded {
        Some("expanded")
    } else if state == ExpandCollapseState_PartiallyExpanded {
        Some("partially_expanded")
    } else if state == ExpandCollapseState_LeafNode {
        Some("leaf")
    } else {
        None
    }
}

fn toggle_state_name(state: ToggleState) -> Option<&'static str> {
    if state == ToggleState_Off {
        Some("toggle_off")
    } else if state == ToggleState_On {
        Some("toggle_on")
    } else if state == ToggleState_Indeterminate {
        Some("toggle_indeterminate")
    } else {
        None
    }
}

fn description_for(info: &UiaElementInfo) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(localized) = info.localized_control_type.as_ref() {
        parts.push(format!("control_type={localized}"));
    }
    if let Some(automation_id) = info.automation_id.as_ref() {
        parts.push(format!("automation_id={automation_id}"));
    }
    if let Some(class_name) = info.class_name.as_ref() {
        parts.push(format!("class={class_name}"));
    }
    if let Some(framework_id) = info.framework_id.as_ref() {
        parts.push(format!("framework={framework_id}"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn backend_ref(hwnd: usize, path: &[usize]) -> String {
    let path = path
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("/");
    format!("uia:hwnd=0x{hwnd:x};path={path}")
}

fn uia_action_for_request(request: &ActionRequest) -> Result<Option<UiaActionCall>, BackendError> {
    let Some(target) = request
        .resolved_element
        .as_ref()
        .and_then(|element| element.backend_ref.as_deref())
        .and_then(parse_backend_ref)
    else {
        return Ok(None);
    };

    let action = match request.action {
        ActionName::Click => UiaActionKind::Click,
        ActionName::FocusElement => UiaActionKind::Focus,
        ActionName::ActivateElement => UiaActionKind::Activate,
        ActionName::SelectElement => UiaActionKind::Select,
        ActionName::ExpandElement => UiaActionKind::Expand,
        ActionName::CollapseElement => UiaActionKind::Collapse,
        ActionName::ToggleElement => UiaActionKind::Toggle,
        ActionName::SetValue => {
            let value = request
                .arguments
                .get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::InvalidRequest,
                        "set_value requires a string value argument",
                    )
                })?;
            UiaActionKind::SetValue(value.to_string())
        }
        _ => return Ok(None),
    };

    Ok(Some(UiaActionCall { target, action }))
}

fn parse_backend_ref(value: &str) -> Option<ParsedUiaRef> {
    let rest = value.strip_prefix("uia:")?;
    let mut hwnd = None;
    let mut path = Vec::new();
    for part in rest.split(';') {
        let (key, value) = part.split_once('=')?;
        match key {
            "hwnd" => hwnd = parse_usize(value),
            "path" => {
                path = if value.is_empty() {
                    Vec::new()
                } else {
                    value
                        .split('/')
                        .map(str::parse)
                        .collect::<Result<Vec<_>, _>>()
                        .ok()?
                };
            }
            _ => {}
        }
    }
    Some(ParsedUiaRef { hwnd: hwnd?, path })
}

fn parse_usize(value: &str) -> Option<usize> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|hex| usize::from_str_radix(hex, 16).ok())
        .unwrap_or_else(|| value.parse().ok())
}

fn desktop_rect_from_uia(rect: UiaRect) -> Option<DesktopRect> {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(DesktopRect {
        x: f64::from(rect.left),
        y: f64::from(rect.top),
        width: f64::from(width),
        height: f64::from(height),
    })
}

fn desktop_rect_to_stream_rect(
    rect: DesktopRect,
    capture: Option<&CaptureInfo>,
    window_bounds: &RectF,
) -> Option<RectF> {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let logical_rect = capture
        .and_then(|capture| capture.logical_rect.as_ref())
        .unwrap_or(window_bounds);
    let scale = capture
        .and_then(|capture| capture.logical_to_pixel_scale)
        .unwrap_or(1.0);
    if scale <= 0.0 {
        return None;
    }

    Some(RectF {
        x: (rect.x - logical_rect.x) * scale,
        y: (rect.y - logical_rect.y) * scale,
        width: rect.width * scale,
        height: rect.height * scale,
        space: CoordinateSpace::StreamPixels,
    })
}

fn role_for_control_type(control_type: i32, localized: Option<&str>) -> String {
    if control_type == UIA_ButtonControlTypeId.0 {
        "button".to_string()
    } else if control_type == UIA_CheckBoxControlTypeId.0 {
        "checkbox".to_string()
    } else if control_type == UIA_ComboBoxControlTypeId.0 {
        "combo_box".to_string()
    } else if control_type == UIA_EditControlTypeId.0 {
        "text".to_string()
    } else if control_type == UIA_DocumentControlTypeId.0 {
        "document".to_string()
    } else if control_type == UIA_HyperlinkControlTypeId.0 {
        "link".to_string()
    } else if control_type == UIA_ImageControlTypeId.0 {
        "image".to_string()
    } else if control_type == UIA_ListControlTypeId.0 {
        "list".to_string()
    } else if control_type == UIA_ListItemControlTypeId.0 {
        "list_item".to_string()
    } else if control_type == UIA_MenuControlTypeId.0 {
        "menu".to_string()
    } else if control_type == UIA_MenuBarControlTypeId.0 {
        "menu_bar".to_string()
    } else if control_type == UIA_MenuItemControlTypeId.0 {
        "menu_item".to_string()
    } else if control_type == UIA_PaneControlTypeId.0 {
        "pane".to_string()
    } else if control_type == UIA_RadioButtonControlTypeId.0 {
        "radio_button".to_string()
    } else if control_type == UIA_ScrollBarControlTypeId.0 {
        "scroll_bar".to_string()
    } else if control_type == UIA_SliderControlTypeId.0 {
        "slider".to_string()
    } else if control_type == UIA_SpinnerControlTypeId.0 {
        "spinner".to_string()
    } else if control_type == UIA_SplitButtonControlTypeId.0 {
        "split_button".to_string()
    } else if control_type == UIA_StatusBarControlTypeId.0 {
        "status_bar".to_string()
    } else if control_type == UIA_TabControlTypeId.0 {
        "tab_list".to_string()
    } else if control_type == UIA_TabItemControlTypeId.0 {
        "tab".to_string()
    } else if control_type == UIA_TextControlTypeId.0 {
        "text".to_string()
    } else if control_type == UIA_ToolBarControlTypeId.0 {
        "tool_bar".to_string()
    } else if control_type == UIA_ToolTipControlTypeId.0 {
        "tool_tip".to_string()
    } else if control_type == UIA_TreeControlTypeId.0 {
        "tree".to_string()
    } else if control_type == UIA_TreeItemControlTypeId.0 {
        "tree_item".to_string()
    } else if control_type == UIA_WindowControlTypeId.0 {
        "window".to_string()
    } else if control_type == UIA_GroupControlTypeId.0 {
        "group".to_string()
    } else if control_type == UIA_HeaderControlTypeId.0 {
        "header".to_string()
    } else if control_type == UIA_SemanticZoomControlTypeId.0 {
        "semantic_zoom".to_string()
    } else if control_type == UIA_SeparatorControlTypeId.0 {
        "separator".to_string()
    } else {
        localized
            .filter(|value| !value.trim().is_empty())
            .map(normalize_role)
            .unwrap_or_else(|| "custom".to_string())
    }
}

fn normalize_role(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn bstr_property<F>(read: F) -> Option<String>
where
    F: FnOnce() -> windows::core::Result<BSTR>,
{
    read()
        .ok()
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
}

fn bool_property<F>(read: F) -> Option<bool>
where
    F: FnOnce() -> windows::core::Result<windows::core::BOOL>,
{
    read().ok().map(|value| value.as_bool())
}

fn push_bool_flag(flags: &mut Vec<String>, flag: &str, value: Option<bool>) {
    if value == Some(true) {
        flags.push(flag.to_string());
    }
}

fn push_inverse_bool_flag(flags: &mut Vec<String>, flag: &str, value: Option<bool>) {
    if value == Some(false) {
        flags.push(flag.to_string());
    }
}

fn uia_hwnd(hwnd: usize) -> UiaHwnd {
    UiaHwnd(hwnd as *mut c_void)
}

fn success(message: &str) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message: message.to_string(),
        code: "Completed".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    }
}

fn uia_error(message: &str, error: impl std::fmt::Display) -> BackendError {
    BackendError::new(
        BackendErrorCode::AccessibilityUnavailable,
        format!("{message}: {error}"),
    )
}

struct ComApartment {
    uninitialize: bool,
}

impl ComApartment {
    fn initialize() -> Result<Self, BackendError> {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self {
                uninitialize: false,
            });
        }
        if result.is_err() {
            return Err(uia_error(
                "failed to initialize COM for UI Automation",
                result,
            ));
        }
        Ok(Self { uninitialize: true })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopRect, UiaActionCall, UiaActionKind, UiaElementInfo, backend_ref,
        desktop_rect_to_stream_rect, element_node_from_info, parse_backend_ref,
        role_for_control_type, uia_action_for_request,
    };
    use sky_cua_platform::model::{
        ActionName, ActionRequest, CaptureBackendKind, CaptureInfo, CaptureScope, CoordinateSpace,
        ElementNode, PixelSize, RectF,
    };
    use windows::Win32::UI::Accessibility::{
        UIA_ButtonControlTypeId, UIA_EditControlTypeId, UIA_PaneControlTypeId,
        UIA_TabItemControlTypeId,
    };

    #[test]
    fn maps_uia_desktop_bounds_to_screenshot_local_element_bounds() {
        let info = UiaElementInfo {
            path: vec![0, 2],
            parent_index: Some(0),
            control_type: UIA_ButtonControlTypeId.0,
            localized_control_type: Some("button".to_string()),
            name: Some("Settings".to_string()),
            automation_id: Some("settingsButton".to_string()),
            class_name: Some("Button".to_string()),
            framework_id: Some("Win32".to_string()),
            value: None,
            enabled: Some(true),
            focusable: Some(true),
            focused: Some(false),
            offscreen: Some(false),
            password: Some(false),
            readonly: None,
            has_invoke: true,
            has_selection_item: false,
            has_expand_collapse: false,
            expand_collapse_state: None,
            has_toggle: false,
            toggle_state: None,
            has_legacy_default_action: false,
            has_value: false,
            text: None,
            numeric_value: None,
            bounds: Some(DesktopRect {
                x: 430.0,
                y: 204.0,
                width: 100.0,
                height: 40.0,
            }),
        };

        let node = element_node_from_info(
            1,
            0x10,
            info,
            Some(&capture_with_rect(420.0, 184.0, 1732.0, 1070.0, 1.0)),
            &window_bounds(),
        );

        assert_eq!(node.element_index, 1);
        assert_eq!(node.parent_index, Some(0));
        assert_eq!(node.role, "button");
        assert_eq!(node.name.as_deref(), Some("Settings"));
        assert_eq!(node.semantic_actions, vec!["click", "focus", "activate"]);
        assert!(node.state_flags.iter().any(|flag| flag == "uia"));
        assert!(node.state_flags.iter().any(|flag| flag == "enabled"));
        assert_eq!(
            node.bounds,
            Some(RectF {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
                space: CoordinateSpace::StreamPixels,
            })
        );
        assert_eq!(node.backend_ref.as_deref(), Some("uia:hwnd=0x10;path=0/2"));
        assert!(
            node.description
                .as_deref()
                .is_some_and(|description| description.contains("automation_id=settingsButton"))
        );
    }

    #[test]
    fn editable_uia_nodes_advertise_set_value_when_not_readonly_or_password() {
        let node = element_node_from_info(
            2,
            0x10,
            UiaElementInfo {
                path: vec![1],
                parent_index: Some(0),
                control_type: UIA_EditControlTypeId.0,
                localized_control_type: Some("edit".to_string()),
                name: Some("Address and search bar".to_string()),
                automation_id: None,
                class_name: None,
                framework_id: None,
                value: Some("https://example.test".to_string()),
                enabled: Some(true),
                focusable: Some(true),
                focused: Some(true),
                offscreen: Some(false),
                password: Some(false),
                readonly: Some(false),
                has_invoke: false,
                has_selection_item: false,
                has_expand_collapse: false,
                expand_collapse_state: None,
                has_toggle: false,
                toggle_state: None,
                has_legacy_default_action: false,
                has_value: true,
                text: None,
                numeric_value: None,
                bounds: None,
            },
            None,
            &window_bounds(),
        );

        assert_eq!(node.role, "text");
        assert_eq!(
            node.semantic_actions,
            vec!["focus".to_string(), "set_value".to_string()]
        );
        assert_eq!(node.value.as_deref(), Some("https://example.test"));
    }

    #[test]
    fn selection_item_uia_nodes_advertise_click_for_tab_like_controls() {
        let node = element_node_from_info(
            2,
            0x10,
            UiaElementInfo {
                path: vec![2],
                parent_index: Some(0),
                control_type: UIA_TabItemControlTypeId.0,
                localized_control_type: Some("tab item".to_string()),
                name: Some("Example Domain".to_string()),
                automation_id: None,
                class_name: None,
                framework_id: None,
                value: None,
                enabled: Some(true),
                focusable: Some(true),
                focused: Some(false),
                offscreen: Some(false),
                password: Some(false),
                readonly: None,
                has_invoke: false,
                has_selection_item: true,
                has_expand_collapse: false,
                expand_collapse_state: None,
                has_toggle: false,
                toggle_state: None,
                has_legacy_default_action: false,
                has_value: false,
                text: None,
                numeric_value: None,
                bounds: None,
            },
            None,
            &window_bounds(),
        );

        assert_eq!(node.role, "tab");
        assert!(node.semantic_actions.iter().any(|action| action == "click"));
        assert!(
            node.semantic_actions
                .iter()
                .any(|action| action == "select")
        );
    }

    #[test]
    fn app_chrome_uia_patterns_advertise_click() {
        for info in [
            UiaElementInfo {
                has_expand_collapse: true,
                expand_collapse_state: Some("collapsed"),
                ..minimal_info(UIA_ButtonControlTypeId.0)
            },
            UiaElementInfo {
                has_toggle: true,
                toggle_state: Some("toggle_off"),
                ..minimal_info(UIA_ButtonControlTypeId.0)
            },
            UiaElementInfo {
                has_legacy_default_action: true,
                ..minimal_info(UIA_ButtonControlTypeId.0)
            },
        ] {
            let node = element_node_from_info(2, 0x10, info, None, &window_bounds());
            assert!(node.semantic_actions.iter().any(|action| action == "click"));
        }
    }

    #[test]
    fn app_chrome_uia_patterns_advertise_specific_primitives() {
        let expand = element_node_from_info(
            2,
            0x10,
            UiaElementInfo {
                has_expand_collapse: true,
                expand_collapse_state: Some("collapsed"),
                ..minimal_info(UIA_ButtonControlTypeId.0)
            },
            None,
            &window_bounds(),
        );
        let collapse = element_node_from_info(
            3,
            0x10,
            UiaElementInfo {
                has_expand_collapse: true,
                expand_collapse_state: Some("expanded"),
                ..minimal_info(UIA_ButtonControlTypeId.0)
            },
            None,
            &window_bounds(),
        );
        let toggle = element_node_from_info(
            4,
            0x10,
            UiaElementInfo {
                has_toggle: true,
                toggle_state: Some("toggle_on"),
                ..minimal_info(UIA_ButtonControlTypeId.0)
            },
            None,
            &window_bounds(),
        );

        assert!(
            expand
                .semantic_actions
                .iter()
                .any(|action| action == "expand")
        );
        assert!(expand.state_flags.iter().any(|flag| flag == "collapsed"));
        assert!(
            collapse
                .semantic_actions
                .iter()
                .any(|action| action == "collapse")
        );
        assert!(collapse.state_flags.iter().any(|flag| flag == "expanded"));
        assert!(
            toggle
                .semantic_actions
                .iter()
                .any(|action| action == "toggle")
        );
        assert!(toggle.state_flags.iter().any(|flag| flag == "toggle_on"));
    }

    #[test]
    fn readonly_or_password_uia_nodes_do_not_advertise_set_value() {
        let readonly = element_node_from_info(
            1,
            0x10,
            UiaElementInfo {
                readonly: Some(true),
                has_value: true,
                text: None,
                numeric_value: None,
                has_selection_item: false,
                has_expand_collapse: false,
                expand_collapse_state: None,
                has_toggle: false,
                toggle_state: None,
                has_legacy_default_action: false,
                ..minimal_info(UIA_EditControlTypeId.0)
            },
            None,
            &window_bounds(),
        );
        let password = element_node_from_info(
            1,
            0x10,
            UiaElementInfo {
                password: Some(true),
                has_value: true,
                text: None,
                numeric_value: None,
                has_selection_item: false,
                has_expand_collapse: false,
                expand_collapse_state: None,
                has_toggle: false,
                toggle_state: None,
                has_legacy_default_action: false,
                ..minimal_info(UIA_EditControlTypeId.0)
            },
            None,
            &window_bounds(),
        );

        assert!(
            !readonly
                .semantic_actions
                .iter()
                .any(|action| action == "set_value")
        );
        assert!(
            !password
                .semantic_actions
                .iter()
                .any(|action| action == "set_value")
        );
    }

    #[test]
    fn parses_uia_backend_refs_for_action_replay() {
        assert_eq!(
            parse_backend_ref("uia:hwnd=0x2a;path=0/1/4"),
            Some(super::ParsedUiaRef {
                hwnd: 0x2a,
                path: vec![0, 1, 4],
            })
        );
        assert_eq!(
            parse_backend_ref("uia:hwnd=42;path="),
            Some(super::ParsedUiaRef {
                hwnd: 42,
                path: Vec::new(),
            })
        );
        assert!(parse_backend_ref("hwnd:0x2a").is_none());
    }

    #[test]
    fn routes_only_supported_actions_to_uia_and_leaves_others_for_fallback() {
        let click = action_request(
            ActionName::Click,
            serde_json::json!({}),
            Some("uia:hwnd=0x2a;path=0"),
        );
        assert_eq!(
            uia_action_for_request(&click).unwrap(),
            Some(UiaActionCall {
                target: super::ParsedUiaRef {
                    hwnd: 0x2a,
                    path: vec![0],
                },
                action: UiaActionKind::Click,
            })
        );

        let scroll = action_request(
            ActionName::Scroll,
            serde_json::json!({ "direction": "down" }),
            Some("uia:hwnd=0x2a;path=0"),
        );
        assert!(uia_action_for_request(&scroll).unwrap().is_none());

        let physical_only = action_request(ActionName::Click, serde_json::json!({}), None);
        assert!(uia_action_for_request(&physical_only).unwrap().is_none());
    }

    #[test]
    fn routes_set_value_to_uia_value_pattern_call() {
        let request = action_request(
            ActionName::SetValue,
            serde_json::json!({ "value": "hello" }),
            Some("uia:hwnd=0x2a;path=1/3"),
        );

        assert_eq!(
            uia_action_for_request(&request).unwrap(),
            Some(UiaActionCall {
                target: super::ParsedUiaRef {
                    hwnd: 0x2a,
                    path: vec![1, 3],
                },
                action: UiaActionKind::SetValue("hello".to_string()),
            })
        );
    }

    #[test]
    fn routes_first_class_semantic_primitives_to_uia() {
        for (action, expected) in [
            (ActionName::FocusElement, UiaActionKind::Focus),
            (ActionName::ActivateElement, UiaActionKind::Activate),
            (ActionName::SelectElement, UiaActionKind::Select),
            (ActionName::ExpandElement, UiaActionKind::Expand),
            (ActionName::CollapseElement, UiaActionKind::Collapse),
            (ActionName::ToggleElement, UiaActionKind::Toggle),
        ] {
            let request =
                action_request(action, serde_json::json!({}), Some("uia:hwnd=0x2a;path=4"));

            assert_eq!(
                uia_action_for_request(&request).unwrap(),
                Some(UiaActionCall {
                    target: super::ParsedUiaRef {
                        hwnd: 0x2a,
                        path: vec![4],
                    },
                    action: expected,
                })
            );
        }
    }

    #[test]
    fn first_class_primitives_require_uia_success_instead_of_physical_fallback() {
        for action in [
            UiaActionKind::Focus,
            UiaActionKind::Activate,
            UiaActionKind::Select,
            UiaActionKind::Expand,
            UiaActionKind::Collapse,
            UiaActionKind::Toggle,
        ] {
            assert!(action.requires_uia_success());
        }

        assert!(!UiaActionKind::Click.requires_uia_success());
        assert!(!UiaActionKind::SetValue("hello".to_string()).requires_uia_success());
    }

    #[test]
    fn maps_unknown_localized_control_type_to_stable_role_slug() {
        assert_eq!(
            role_for_control_type(UIA_PaneControlTypeId.0, Some("pane")),
            "pane"
        );
        assert_eq!(
            role_for_control_type(999_999, Some("Fancy Thing")),
            "fancy_thing"
        );
        assert_eq!(role_for_control_type(999_999, None), "custom");
    }

    #[test]
    fn backend_ref_round_trips_empty_root_paths() {
        assert_eq!(backend_ref(0x10, &[]), "uia:hwnd=0x10;path=");
    }

    #[test]
    fn coordinate_mapping_uses_window_bounds_when_capture_is_missing() {
        assert_eq!(
            desktop_rect_to_stream_rect(
                DesktopRect {
                    x: 430.0,
                    y: 204.0,
                    width: 100.0,
                    height: 40.0,
                },
                None,
                &window_bounds(),
            ),
            Some(RectF {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
                space: CoordinateSpace::StreamPixels,
            })
        );
    }

    fn action_request(
        action: ActionName,
        arguments: serde_json::Value,
        backend_ref: Option<&str>,
    ) -> ActionRequest {
        ActionRequest {
            action,
            snapshot_id: Some("snapshot".to_string()),
            element_index: backend_ref.map(|_| 1),
            arguments,
            resolved_element: backend_ref.map(|backend_ref| ElementNode {
                element_index: 1,
                parent_index: Some(0),
                role: "button".to_string(),
                name: Some("Target".to_string()),
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: vec!["uia".to_string()],
                semantic_actions: vec!["click".to_string()],
                bounds: None,
                backend_ref: Some(backend_ref.to_string()),
            }),
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        }
    }

    fn minimal_info(control_type: i32) -> UiaElementInfo {
        UiaElementInfo {
            path: vec![0],
            parent_index: Some(0),
            control_type,
            localized_control_type: None,
            name: Some("Target".to_string()),
            automation_id: None,
            class_name: None,
            framework_id: None,
            value: None,
            enabled: Some(true),
            focusable: Some(true),
            focused: Some(false),
            offscreen: Some(false),
            password: Some(false),
            readonly: None,
            has_invoke: false,
            has_selection_item: false,
            has_expand_collapse: false,
            expand_collapse_state: None,
            has_toggle: false,
            toggle_state: None,
            has_legacy_default_action: false,
            has_value: false,
            text: None,
            numeric_value: None,
            bounds: None,
        }
    }

    fn window_bounds() -> RectF {
        RectF {
            x: 420.0,
            y: 184.0,
            width: 1732.0,
            height: 1070.0,
            space: CoordinateSpace::DesktopLogical,
        }
    }

    fn capture_with_rect(x: f64, y: f64, width: f64, height: f64, scale: f64) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::WindowsGdi,
            image_backend: Some(CaptureBackendKind::WindowsGdi),
            capture_scope: CaptureScope::Window,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: None,
            logical_rect: Some(RectF {
                x,
                y,
                width,
                height,
                space: CoordinateSpace::DesktopLogical,
            }),
            source_logical_rect: None,
            pixel_size: Some(PixelSize {
                width: width as u32,
                height: height as u32,
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
