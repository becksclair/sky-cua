//! Request-to-AX action mapping (pure) and its execution on a lane.

use std::sync::atomic::{AtomicBool, Ordering};

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{ActionName, ActionOutcome, ActionRequest};

use super::element::{
    AxElement, AxErr, as_bool, as_f64, as_string, cf_number, cf_string, cf_strings, is_number,
};
use super::lane::JobGuard;
use super::refs::AxRef;
use super::tree::ax_error;
use super::vocab::{self, ValueKind};

/// What an `ActionRequest` asks AX to do.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AxAction {
    /// Element-targeted primary click: `AXPress`.
    Click,
    Focus,
    Activate,
    Select,
    Expand,
    Collapse,
    Toggle,
    SetValue(String),
    /// `PerformSecondaryAction`: `AXShowMenu`.
    ShowMenu,
    /// `PerformAction`: a named or indexed AX action of the element.
    Named(NamedAction),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NamedAction {
    Name(String),
    Index(usize),
    /// No name or index given: `AXPress` when offered, else the first action.
    Default,
}

impl AxAction {
    /// First-class semantic primitives have no faithful physical fallback, so
    /// an element that cannot perform them is an error rather than a silent
    /// fall-through (the same split as the Windows UIA backend).
    pub(crate) fn requires_ax_success(&self) -> bool {
        matches!(
            self,
            Self::Focus
                | Self::Activate
                | Self::Select
                | Self::Expand
                | Self::Collapse
                | Self::Toggle
                | Self::Named(_)
        )
    }

    pub(crate) fn primitive_name(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Focus => "focus",
            Self::Activate => "activate",
            Self::Select => "select",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::Toggle => "toggle",
            Self::SetValue(_) => "set_value",
            Self::ShowMenu => "secondary action",
            Self::Named(_) => "perform_action",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AxActionCall {
    pub target: AxRef,
    pub action: AxAction,
}

/// Maps a request to an AX call. `Ok(None)` means the request carries no AX
/// element reference or names an action AX does not model.
pub(crate) fn action_for_request(
    request: &ActionRequest,
) -> Result<Option<AxActionCall>, BackendError> {
    let Some(target) = request
        .resolved_element
        .as_ref()
        .and_then(|element| element.backend_ref.as_deref())
        .and_then(AxRef::parse)
    else {
        return Ok(None);
    };
    let arguments = &request.arguments;
    let action = match request.action {
        ActionName::Click => {
            let left = arguments
                .get("mouse_button")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|button| button.eq_ignore_ascii_case("left"));
            let single = arguments
                .get("click_count")
                .and_then(serde_json::Value::as_u64)
                .is_none_or(|count| count == 1);
            // A right or multi-click is not an AXPress; leave it to physical
            // input.
            if !(left && single) {
                return Ok(None);
            }
            AxAction::Click
        }
        ActionName::FocusElement => AxAction::Focus,
        ActionName::ActivateElement => AxAction::Activate,
        ActionName::SelectElement => AxAction::Select,
        ActionName::ExpandElement => AxAction::Expand,
        ActionName::CollapseElement => AxAction::Collapse,
        ActionName::ToggleElement => AxAction::Toggle,
        ActionName::PerformSecondaryAction => AxAction::ShowMenu,
        ActionName::SetValue => {
            let value = arguments
                .get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::InvalidRequest,
                        "set_value requires a string value argument",
                    )
                })?;
            AxAction::SetValue(value.to_string())
        }
        ActionName::PerformAction => AxAction::Named(named_action(arguments)?),
        ActionName::Move
        | ActionName::Scroll
        | ActionName::Drag
        | ActionName::TypeText
        | ActionName::PressKey => return Ok(None),
    };
    Ok(Some(AxActionCall { target, action }))
}

/// Same argument keys the Linux AT-SPI backend accepts: `action_index`
/// (number or numeric string), `action_name`, or `action` (a name, or an
/// index as a numeric string).
fn named_action(arguments: &serde_json::Value) -> Result<NamedAction, BackendError> {
    let index = arguments
        .get("action_index")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        })
        .or_else(|| {
            arguments
                .get("action")
                .and_then(serde_json::Value::as_str)
                .and_then(|text| text.trim().parse().ok())
        });
    if let Some(index) = index {
        return usize::try_from(index).map(NamedAction::Index).map_err(|_| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!("action_index {index} is not a valid AX action index"),
            )
        });
    }
    Ok(arguments
        .get("action_name")
        .or_else(|| arguments.get("action"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or(NamedAction::Default, |name| {
            NamedAction::Name(name.to_string())
        }))
}

/// Normalizes an action name for comparison: `AXShowMenu`, `showmenu`,
/// `show_menu`, and `Show Menu` are equal.
fn normalize_action(name: &str) -> String {
    let lower: String = name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    lower
        .strip_prefix("ax")
        .map(str::to_string)
        .unwrap_or(lower)
}

/// Shared-vocabulary aliases for AX action names.
fn alias(normalized: &str) -> &str {
    match normalized {
        "click" | "activate" | "invoke" | "default" => "press",
        "secondary" | "contextmenu" | "menu" => "showmenu",
        "select" => "pick",
        other => other,
    }
}

/// Resolves a named action against the element's live action list.
pub(crate) fn resolve_named(
    named: &NamedAction,
    available: &[String],
) -> Result<Option<String>, BackendError> {
    match named {
        NamedAction::Default => Ok(available
            .iter()
            .find(|name| name.as_str() == "AXPress")
            .or_else(|| available.first())
            .cloned()),
        NamedAction::Index(index) => available.get(*index).cloned().map(Some).ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!(
                    "action_index {index} is out of range; the element exposes [{}]",
                    available.join(", ")
                ),
            )
        }),
        NamedAction::Name(name) => {
            let wanted = normalize_action(name);
            let wanted = alias(&wanted);
            available
                .iter()
                .find(|candidate| normalize_action(candidate) == wanted)
                .cloned()
                .map(Some)
                .ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::InvalidRequest,
                        format!(
                            "the element exposes AX actions [{}], but none matched requested action {name:?}",
                            available.join(", ")
                        ),
                    )
                })
        }
    }
}

/// The result of running an action on a live element.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Executed {
    Done(String),
    /// The live element offers no AX path for this action.
    Unsupported,
}

/// Marks the action as possibly sent, unless its caller has already stopped
/// waiting, in which case nothing is written.
///
/// The caller closes its side before it reads `mutating`, and this stores
/// `mutating` before its final guard check, so the two cannot both miss each
/// other: either the write is skipped, or the caller sees `mutating` and
/// reports an unknown outcome instead of repeating the action physically.
fn begin_write(mutating: &AtomicBool, guard: &JobGuard<'_>) -> Result<(), BackendError> {
    guard.ensure_wanted()?;
    mutating.store(true, Ordering::SeqCst);
    guard.ensure_wanted()
}

/// Runs `action` on `element`. `mutating` is set just before the first call
/// that can change application state, so a lost answer can be reported as
/// an unknown outcome rather than as "not performed".
pub(crate) fn execute(
    element: &AxElement,
    action: &AxAction,
    mutating: &AtomicBool,
    guard: &JobGuard<'_>,
) -> Result<Executed, BackendError> {
    let actions = || {
        element
            .action_names()
            .map_err(|error| read_error(error, "action names"))
    };
    let perform = |name: &str| -> Result<(), BackendError> {
        begin_write(mutating, guard)?;
        element
            .perform(&cf_string(name))
            .map_err(|error| write_error(error, name))
    };
    let set_bool = |attribute: &str, value: bool| -> Result<(), BackendError> {
        begin_write(mutating, guard)?;
        element
            .set_bool(&cf_string(attribute), value)
            .map_err(|error| write_error(error, attribute))
    };
    let settable = |attribute: &str| -> Result<bool, BackendError> {
        match element.is_settable(&cf_string(attribute)) {
            Ok(settable) => Ok(settable),
            Err(error) if error.is_unsupported() => Ok(false),
            Err(error) => Err(read_error(error, attribute)),
        }
    };
    let done = |message: &str| Ok(Executed::Done(message.to_string()));

    match action {
        AxAction::Click | AxAction::Activate => {
            if !actions()?.iter().any(|name| name == "AXPress") {
                return Ok(Executed::Unsupported);
            }
            perform("AXPress")?;
            done("Activated the element through macOS Accessibility.")
        }
        AxAction::Focus => {
            if !settable("AXFocused")? {
                return Ok(Executed::Unsupported);
            }
            set_bool("AXFocused", true)?;
            done("Focused the element through macOS Accessibility.")
        }
        AxAction::Select => {
            if settable("AXSelected")? {
                set_bool("AXSelected", true)?;
                return done("Selected the element through macOS Accessibility.");
            }
            let available = actions()?;
            if available.iter().any(|name| name == "AXPick") {
                perform("AXPick")?;
                return done("Selected the element through macOS Accessibility.");
            }
            let (role, subrole) = role_of(element)?;
            if vocab::role_name(&role, subrole.as_deref()) == "tab"
                && available.iter().any(|name| name == "AXPress")
            {
                perform("AXPress")?;
                return done("Selected the element through macOS Accessibility.");
            }
            Ok(Executed::Unsupported)
        }
        AxAction::Expand | AxAction::Collapse => {
            let expand = matches!(action, AxAction::Expand);
            let message = if expand {
                "Expanded the element through macOS Accessibility."
            } else {
                "Collapsed the element through macOS Accessibility."
            };
            for attribute in ["AXExpanded", "AXDisclosing"] {
                if settable(attribute)? {
                    set_bool(attribute, expand)?;
                    return done(message);
                }
            }
            // Pop-ups, combo boxes, menu buttons, and disclosure triangles
            // open and close with AXPress; read the current state first so a
            // press never inverts it.
            let (role, _) = role_of(element)?;
            let state_attribute = match role.as_str() {
                "AXDisclosureTriangle" => "AXValue",
                "AXPopUpButton" | "AXComboBox" | "AXMenuButton" => "AXExpanded",
                _ => return Ok(Executed::Unsupported),
            };
            if !actions()?.iter().any(|name| name == "AXPress") {
                return Ok(Executed::Unsupported);
            }
            let open = element
                .copy_attribute(&cf_string(state_attribute))
                .ok()
                .and_then(|value| as_bool(&value));
            match open {
                Some(open) if open == expand => {}
                Some(_) => perform("AXPress")?,
                None => return Ok(Executed::Unsupported),
            }
            done(message)
        }
        AxAction::Toggle => {
            let (role, subrole) = role_of(element)?;
            if vocab::value_kind(&role, subrole.as_deref()) != ValueKind::Toggle
                || !actions()?.iter().any(|name| name == "AXPress")
            {
                return Ok(Executed::Unsupported);
            }
            perform("AXPress")?;
            done("Toggled the element through macOS Accessibility.")
        }
        AxAction::ShowMenu => {
            if !actions()?.iter().any(|name| name == "AXShowMenu") {
                return Ok(Executed::Unsupported);
            }
            perform("AXShowMenu")?;
            done("Performed the secondary action through macOS Accessibility.")
        }
        AxAction::Named(named) => {
            let available = actions()?;
            let Some(name) = resolve_named(named, &available)? else {
                return Ok(Executed::Unsupported);
            };
            perform(&name)?;
            Ok(Executed::Done(format!(
                "Performed AX action {name} through macOS Accessibility."
            )))
        }
        AxAction::SetValue(value) => set_value(element, value, mutating, guard),
    }
}

fn set_value(
    element: &AxElement,
    value: &str,
    mutating: &AtomicBool,
    guard: &JobGuard<'_>,
) -> Result<Executed, BackendError> {
    let attribute = cf_string("AXValue");
    match element.is_settable(&attribute) {
        Ok(true) => {}
        Ok(false) => return Ok(Executed::Unsupported),
        Err(error) if error.is_unsupported() => return Ok(Executed::Unsupported),
        Err(error) => return Err(read_error(error, "AXValue")),
    }
    let values = element
        .copy_multiple(&cf_strings(&["AXRole", "AXSubrole", "AXValue"]))
        .map_err(|error| read_error(error, "AXRole"))?;
    let get = |index: usize| values.get(index).and_then(|value| value.as_ref().ok());
    let role = get(0)
        .and_then(|value| as_string(value))
        .unwrap_or_default();
    let subrole = get(1).and_then(|value| as_string(value));
    let numeric = vocab::value_kind(&role, subrole.as_deref()) == ValueKind::Range
        || get(2).is_some_and(|value| is_number(value) && as_f64(value).is_some());
    if numeric {
        let number: f64 = value.trim().parse().map_err(|_| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!("set_value on a numeric {role} element requires a number, got {value:?}"),
            )
        })?;
        begin_write(mutating, guard)?;
        element
            .set_attribute(&attribute, &cf_number(number))
            .map_err(|error| write_error(error, "AXValue"))?;
        return Ok(Executed::Done(format!(
            "Set numeric value to {number} through macOS Accessibility AXValue."
        )));
    }
    // Text controls in Chromium and AppKit commit edits more reliably when
    // focused first; focus is best effort.
    begin_write(mutating, guard)?;
    let _ = element.set_bool(&cf_string("AXFocused"), true);
    element
        .set_attribute(&attribute, &cf_string(value))
        .map_err(|error| write_error(error, "AXValue"))?;
    Ok(Executed::Done(
        "Set the value through macOS Accessibility AXValue.".to_string(),
    ))
}

fn role_of(element: &AxElement) -> Result<(String, Option<String>), BackendError> {
    let values = element
        .copy_multiple(&cf_strings(&["AXRole", "AXSubrole"]))
        .map_err(|error| read_error(error, "AXRole"))?;
    let get = |index: usize| {
        values
            .get(index)
            .and_then(|value| value.as_ref().ok())
            .and_then(|value| as_string(value))
    };
    Ok((get(0).unwrap_or_default(), get(1)))
}

fn read_error(error: AxErr, what: &str) -> BackendError {
    ax_error(error, &format!("macOS Accessibility could not read {what}"))
}

/// A write whose reply was lost may still have taken effect (an `AXPress`
/// that opens a modal does not answer until the modal closes).
fn write_error(error: AxErr, what: &str) -> BackendError {
    if error.is_timeout() {
        return BackendError::new(
            BackendErrorCode::CuaActionOutcomeUnknown,
            format!(
                "macOS Accessibility {what} was sent but the application did not confirm it within the messaging timeout ({error}); it may have taken effect"
            ),
        );
    }
    ax_error(error, &format!("macOS Accessibility {what} failed"))
}

pub(crate) fn success(message: String) -> ActionOutcome {
    ActionOutcome {
        success: true,
        message,
        code: "Completed".to_string(),
        diagnostics: Vec::new(),
        agent_cursor: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::ElementNode;

    const REF: &str = "ax:pid=42;gen=7;idx=3";

    fn request(
        action: ActionName,
        arguments: serde_json::Value,
        backend_ref: Option<&str>,
    ) -> ActionRequest {
        ActionRequest {
            action,
            appshot_id: None,
            snapshot_id: Some("snapshot".to_string()),
            element_index: backend_ref.map(|_| 3),
            arguments,
            resolved_element: backend_ref.map(|backend_ref| ElementNode {
                element_index: 3,
                parent_index: Some(0),
                role: "button".to_string(),
                name: Some("Target".to_string()),
                description: None,
                value: None,
                text: None,
                numeric_value: None,
                supports_editable_text: false,
                state_flags: vec!["ax".to_string()],
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

    fn action_of(action: ActionName, arguments: serde_json::Value) -> Option<AxAction> {
        action_for_request(&request(action, arguments, Some(REF)))
            .unwrap()
            .map(|call| {
                assert_eq!(
                    call.target,
                    AxRef {
                        pid: 42,
                        generation: 7,
                        index: 3
                    }
                );
                call.action
            })
    }

    #[test]
    fn first_class_primitives_route_to_ax() {
        for (name, expected) in [
            (ActionName::FocusElement, AxAction::Focus),
            (ActionName::ActivateElement, AxAction::Activate),
            (ActionName::SelectElement, AxAction::Select),
            (ActionName::ExpandElement, AxAction::Expand),
            (ActionName::CollapseElement, AxAction::Collapse),
            (ActionName::ToggleElement, AxAction::Toggle),
            (ActionName::PerformSecondaryAction, AxAction::ShowMenu),
        ] {
            assert_eq!(action_of(name, serde_json::json!({})), Some(expected));
        }
    }

    #[test]
    fn only_plain_left_clicks_become_axpress() {
        assert_eq!(
            action_of(ActionName::Click, serde_json::json!({})),
            Some(AxAction::Click)
        );
        assert_eq!(
            action_of(
                ActionName::Click,
                serde_json::json!({"mouse_button": "left", "click_count": 1})
            ),
            Some(AxAction::Click)
        );
        assert_eq!(
            action_of(
                ActionName::Click,
                serde_json::json!({"mouse_button": "right"})
            ),
            None
        );
        assert_eq!(
            action_of(ActionName::Click, serde_json::json!({"click_count": 2})),
            None
        );
    }

    #[test]
    fn pointer_and_keyboard_actions_fall_through() {
        for name in [
            ActionName::Move,
            ActionName::Scroll,
            ActionName::Drag,
            ActionName::TypeText,
            ActionName::PressKey,
        ] {
            assert_eq!(action_of(name, serde_json::json!({})), None);
        }
    }

    #[test]
    fn requests_without_an_ax_reference_fall_through() {
        let none = request(ActionName::Click, serde_json::json!({}), None);
        assert_eq!(action_for_request(&none).unwrap(), None);
        let uia = request(
            ActionName::Click,
            serde_json::json!({}),
            Some("uia:hwnd=0x2a;path=0"),
        );
        assert_eq!(action_for_request(&uia).unwrap(), None);
    }

    #[test]
    fn set_value_requires_a_string_value() {
        assert_eq!(
            action_of(ActionName::SetValue, serde_json::json!({"value": "hello"})),
            Some(AxAction::SetValue("hello".to_string()))
        );
        let error = action_for_request(&request(
            ActionName::SetValue,
            serde_json::json!({"value": 3}),
            Some(REF),
        ))
        .unwrap_err();
        assert_eq!(error.code, "InvalidRequest");
    }

    #[test]
    fn perform_action_accepts_the_linux_argument_keys() {
        assert_eq!(
            action_of(
                ActionName::PerformAction,
                serde_json::json!({"action_index": 2})
            ),
            Some(AxAction::Named(NamedAction::Index(2)))
        );
        assert_eq!(
            action_of(
                ActionName::PerformAction,
                serde_json::json!({"action_index": "1"})
            ),
            Some(AxAction::Named(NamedAction::Index(1)))
        );
        assert_eq!(
            action_of(
                ActionName::PerformAction,
                serde_json::json!({"action": "0"})
            ),
            Some(AxAction::Named(NamedAction::Index(0)))
        );
        assert_eq!(
            action_of(
                ActionName::PerformAction,
                serde_json::json!({"action_name": "AXIncrement"})
            ),
            Some(AxAction::Named(NamedAction::Name(
                "AXIncrement".to_string()
            )))
        );
        assert_eq!(
            action_of(
                ActionName::PerformAction,
                serde_json::json!({"action": "show menu"})
            ),
            Some(AxAction::Named(NamedAction::Name("show menu".to_string())))
        );
        assert_eq!(
            action_of(ActionName::PerformAction, serde_json::json!({})),
            Some(AxAction::Named(NamedAction::Default))
        );
        assert!(
            action_for_request(&request(
                ActionName::PerformAction,
                serde_json::json!({"action_index": -1}),
                Some(REF)
            ))
            .is_err()
        );
    }

    #[test]
    fn named_actions_resolve_against_the_live_list() {
        let available: Vec<String> = ["AXShowMenu", "AXIncrement", "AXPress"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let resolve = |named: NamedAction| resolve_named(&named, &available);
        assert_eq!(
            resolve(NamedAction::Default).unwrap().as_deref(),
            Some("AXPress")
        );
        assert_eq!(
            resolve(NamedAction::Index(1)).unwrap().as_deref(),
            Some("AXIncrement")
        );
        assert!(resolve(NamedAction::Index(9)).is_err());
        for name in ["AXIncrement", "increment", "Increment"] {
            assert_eq!(
                resolve(NamedAction::Name(name.to_string()))
                    .unwrap()
                    .as_deref(),
                Some("AXIncrement")
            );
        }
        for name in ["showmenu", "show_menu", "Show Menu", "context menu"] {
            assert_eq!(
                resolve(NamedAction::Name(name.to_string()))
                    .unwrap()
                    .as_deref(),
                Some("AXShowMenu")
            );
        }
        assert_eq!(
            resolve(NamedAction::Name("click".to_string()))
                .unwrap()
                .as_deref(),
            Some("AXPress")
        );
        let error = resolve(NamedAction::Name("AXConfirm".to_string())).unwrap_err();
        assert_eq!(error.code, "InvalidRequest");
        assert!(error.message.contains("AXShowMenu, AXIncrement, AXPress"));
        assert_eq!(resolve_named(&NamedAction::Default, &[]).unwrap(), None);
    }

    #[test]
    fn semantic_primitives_require_ax_success_like_the_windows_backend() {
        for action in [
            AxAction::Focus,
            AxAction::Activate,
            AxAction::Select,
            AxAction::Expand,
            AxAction::Collapse,
            AxAction::Toggle,
            AxAction::Named(NamedAction::Default),
        ] {
            assert!(action.requires_ax_success(), "{action:?}");
        }
        for action in [
            AxAction::Click,
            AxAction::ShowMenu,
            AxAction::SetValue(String::new()),
        ] {
            assert!(!action.requires_ax_success(), "{action:?}");
        }
    }

    #[test]
    fn lost_write_replies_are_unknown_outcomes() {
        assert_eq!(
            write_error(AxErr::CANNOT_COMPLETE, "AXPress").code,
            "CuaActionOutcomeUnknown"
        );
        assert_eq!(
            write_error(AxErr::INVALID_ELEMENT, "AXPress").code,
            "SnapshotStale"
        );
    }
}
