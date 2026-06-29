use serde::{Deserialize, Serialize};

use crate::uinput::DesktopBounds;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HelperRequest {
    pub version: u32,
    #[serde(flatten)]
    pub command: HelperCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HelperCommand {
    Hello,
    ObservePointer {
        bounds: DesktopBounds,
    },
    KeyEvents {
        events: Vec<KeyEventCommand>,
    },
    PointerActions {
        bounds: DesktopBounds,
        actions: Vec<PointerAction>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyEventCommand {
    pub code: u16,
    pub pressed: bool,
}

/// A single absolute-pointer primitive replayed by the privileged helper. Button
/// indices are `0=left`, `1=right`, `2=middle`; coordinates are desktop-logical
/// and mapped to absolute device units by the helper's `DesktopBounds`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PointerAction {
    MoveAbsolute { x: f64, y: f64 },
    Button { button: u8, pressed: bool },
    ScrollVertical { steps: i32 },
    Settle { millis: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelperCapabilities {
    pub protocol_version: u32,
    pub pointer: bool,
    pub keyboard: bool,
}

impl Default for HelperCapabilities {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            pointer: true,
            keyboard: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelperErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelperResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<HelperCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HelperErrorInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HelperStreamEvent {
    PointerMoved {
        x: f64,
        y: f64,
        sequence: u64,
        coordinate_space: String,
        exact: bool,
    },
}

impl HelperResponse {
    pub fn ok() -> Self {
        Self {
            ok: true,
            capabilities: None,
            error: None,
        }
    }

    pub fn capabilities(capabilities: HelperCapabilities) -> Self {
        Self {
            ok: true,
            capabilities: Some(capabilities),
            error: None,
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            capabilities: None,
            error: Some(HelperErrorInfo {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

pub fn request_line(command: HelperCommand) -> Result<String, serde_json::Error> {
    let request = HelperRequest {
        version: PROTOCOL_VERSION,
        command,
    };
    let mut line = serde_json::to_string(&request)?;
    line.push('\n');
    Ok(line)
}

pub fn response_line(response: &HelperResponse) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(response)?;
    line.push('\n');
    Ok(line)
}

pub fn stream_event_line(event: &HelperStreamEvent) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    Ok(line)
}

pub fn parse_request_line(line: &str) -> Result<HelperRequest, serde_json::Error> {
    serde_json::from_str(line)
}

pub fn parse_response_line(line: &str) -> Result<HelperResponse, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::{
        HelperCommand, HelperStreamEvent, PROTOCOL_VERSION, PointerAction, parse_request_line,
        request_line, stream_event_line,
    };
    use crate::uinput::DesktopBounds;

    #[test]
    fn encodes_versioned_json_line_requests() {
        let line = request_line(HelperCommand::KeyEvents {
            events: vec![super::KeyEventCommand {
                code: 30,
                pressed: true,
            }],
        })
        .unwrap();

        assert!(line.ends_with('\n'));
        assert!(line.contains("\"version\":1"));
        assert!(line.contains("\"op\":\"key_events\""));

        let parsed = parse_request_line(&line).unwrap();
        assert_eq!(parsed.version, PROTOCOL_VERSION);
        assert!(matches!(parsed.command, HelperCommand::KeyEvents { .. }));
    }

    #[test]
    fn encodes_versioned_json_line_pointer_actions_requests() {
        let line = request_line(HelperCommand::PointerActions {
            bounds: DesktopBounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale_milli: 1000,
            },
            actions: vec![
                PointerAction::MoveAbsolute { x: 12.0, y: 34.0 },
                PointerAction::Button {
                    button: 0,
                    pressed: true,
                },
                PointerAction::Settle { millis: 120 },
                PointerAction::Button {
                    button: 0,
                    pressed: false,
                },
                PointerAction::ScrollVertical { steps: -2 },
            ],
        })
        .unwrap();

        assert!(line.ends_with('\n'));
        assert!(line.contains("\"version\":1"));
        assert!(line.contains("\"op\":\"pointer_actions\""));
        assert!(line.contains("\"kind\":\"move_absolute\""));

        let parsed = parse_request_line(&line).unwrap();
        assert_eq!(parsed.version, PROTOCOL_VERSION);
        let HelperCommand::PointerActions { bounds, actions } = parsed.command else {
            panic!("expected pointer_actions command");
        };
        assert_eq!(bounds.width, 1920);
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0], PointerAction::MoveAbsolute { x: 12.0, y: 34.0 });
    }

    #[test]
    fn encodes_pointer_observe_stream_events() {
        let request = request_line(HelperCommand::ObservePointer {
            bounds: DesktopBounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale_milli: 1000,
            },
        })
        .unwrap();
        assert!(request.contains("\"op\":\"observe_pointer\""));

        let event = stream_event_line(&HelperStreamEvent::PointerMoved {
            x: 10.0,
            y: 20.0,
            sequence: 7,
            coordinate_space: "desktop_logical".to_string(),
            exact: false,
        })
        .unwrap();
        assert!(event.ends_with('\n'));
        assert!(event.contains("\"event\":\"pointer_moved\""));
        assert!(event.contains("\"exact\":false"));
    }

    #[test]
    fn rejects_removed_shutdown_request() {
        let error = parse_request_line(r#"{"version":1,"op":"shutdown"}"#)
            .expect_err("shutdown must not be accepted on the production socket");

        assert!(error.to_string().contains("unknown variant"));
    }
}
