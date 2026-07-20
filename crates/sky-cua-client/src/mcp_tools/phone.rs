//! Phone Use MCP tool surface.
//!
//! Mirrors `browser.rs`: `is_phone_tool` gates dispatch, and `handle_tool_call`
//! parses each tool's arguments into a `PhoneRequest`, sends it through the
//! service as `ServiceRequest::Phone`, and shapes the matching
//! `PhoneResponse` variant into the `{content, structuredContent, isError}` MCP
//! result. Screenshots return an image content block only when the model can
//! receive images, with `data_base64` stripped from `structuredContent`.

use anyhow::{Result, anyhow};
use serde_json::Value;
use sky_cua_platform::model::{
    PhoneAccessibilityTreeRequest, PhoneAppCurrentRequest, PhoneAppForceStopRequest,
    PhoneAppInstallRequest, PhoneAppLaunchRequest, PhoneAppListRequest, PhoneAppOpenIntentRequest,
    PhoneCompanionStatusRequest, PhoneConnectRequest, PhoneDisconnectRequest,
    PhoneInstallCompanionRequest, PhoneListDevicesRequest, PhoneNotificationActionRequest,
    PhoneNotificationDismissRequest, PhoneNotificationOpenRequest, PhoneNotificationReplyRequest,
    PhoneNotificationsRequest, PhoneObserveRequest, PhoneOpenSettingsRequest,
    PhonePairWirelessRequest, PhonePressKeyRequest, PhoneRefreshCapabilitiesRequest, PhoneRequest,
    PhoneResponse, PhoneScreenshotRequest, PhoneStatusRequest, PhoneSwipeRequest, PhoneTapRequest,
    PhoneTypeTextRequest, ServiceRequest, ServiceResponse,
};

mod args;
mod response;

use super::{McpService, invalid_request_tool_error, tool_error};
use args::{
    parse_optional_bool, parse_optional_duration_ms, parse_optional_limit, parse_phone_apk_paths,
    parse_phone_app_install_mode, parse_phone_backend, parse_phone_coordinate,
    parse_phone_selector, parse_phone_settings_screen, parse_required_literal_string,
    parse_required_string,
};
use response::{
    phone_accessibility_tree_result, phone_action_result, phone_app_result,
    phone_companion_status_result, phone_connected_result, phone_disconnect_result,
    phone_list_devices_result, phone_notifications_result, phone_observe_result,
    phone_pair_wireless_result, phone_screenshot_result, phone_status_result,
};

pub(super) fn is_phone_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "phone_observe"
            | "phone_status"
            | "phone_list_devices"
            | "phone_refresh_capabilities"
            | "phone_pair_wireless"
            | "phone_connect"
            | "phone_disconnect"
            | "phone_screenshot"
            | "phone_tap"
            | "phone_swipe"
            | "phone_type_text"
            | "phone_press_key"
            | "phone_install_companion"
            | "phone_companion_status"
            | "phone_accessibility_tree"
            | "phone_notifications"
            | "phone_notification_open"
            | "phone_notification_dismiss"
            | "phone_notification_action"
            | "phone_notification_reply"
            | "phone_app_current"
            | "phone_app_list"
            | "phone_app_launch"
            | "phone_app_open_intent"
            | "phone_app_force_stop"
            | "phone_app_install"
            | "phone_open_settings"
    )
}

/// Wrap a `PhoneRequest` into the service envelope.
fn phone_service_request(request: PhoneRequest) -> ServiceRequest {
    ServiceRequest::Phone {
        request,
        context: crate::mcp_server::current_phone_request_context(),
    }
}

/// Macro to short-circuit a parse error into an InvalidRequest tool error.
macro_rules! parse_or_invalid {
    ($expr:expr) => {
        match $expr {
            Ok(value) => value,
            Err(error) => return invalid_request_tool_error(error.to_string()),
        }
    };
}

pub(super) fn handle_tool_call(
    service: &impl McpService,
    tool_name: &str,
    arguments: Value,
    model: &crate::mcp_server::ModelSessionInfo,
) -> Result<Value> {
    let request = parse_or_invalid!(build_phone_request(tool_name, &arguments, model));
    let response = match service.call(&phone_service_request(request))? {
        ServiceResponse::Phone { response } => response,
        ServiceResponse::Error { code, message, .. } => return tool_error(code, message),
        other => return Err(anyhow!("unexpected response for {tool_name}: {other:?}")),
    };
    shape_phone_response(tool_name, response, model)
}

/// Parse a tool's arguments into the matching `PhoneRequest` variant.
fn build_phone_request(
    tool_name: &str,
    arguments: &Value,
    model: &crate::mcp_server::ModelSessionInfo,
) -> Result<PhoneRequest> {
    let request = match tool_name {
        "phone_observe" => PhoneRequest::Observe(PhoneObserveRequest {
            session: parse_phone_selector(arguments)?,
            backend: parse_phone_backend(arguments)?,
            include_image_data: model.can_receive_images(),
            include_accessibility: parse_optional_bool(arguments, "include_accessibility", false)?,
            include_notifications: parse_optional_bool(arguments, "include_notifications", false)?,
        }),
        "phone_status" => PhoneRequest::Status(PhoneStatusRequest {
            refresh_devices: parse_optional_bool(arguments, "refresh_devices", false)?,
        }),
        "phone_list_devices" => PhoneRequest::ListDevices(PhoneListDevicesRequest {
            include_mdns: parse_optional_bool(arguments, "include_mdns", false)?,
        }),
        "phone_refresh_capabilities" => {
            PhoneRequest::RefreshCapabilities(PhoneRefreshCapabilitiesRequest {
                session: parse_phone_selector(arguments)?,
            })
        }
        "phone_pair_wireless" => PhoneRequest::PairWireless(PhonePairWirelessRequest {
            host_port: parse_required_string(
                arguments,
                "host_port",
                "phone_pair_wireless host_port",
            )?,
            pairing_code: parse_required_literal_string(
                arguments,
                "pairing_code",
                "phone_pair_wireless pairing_code",
            )?,
        }),
        "phone_connect" => PhoneRequest::Connect(PhoneConnectRequest {
            serial: args::parse_optional_string(arguments, "serial", "phone_connect serial")?,
            backend: parse_phone_backend(arguments)?,
            install_companion: parse_optional_bool(arguments, "install_companion", false)?,
            start_scrcpy: parse_optional_bool(arguments, "start_scrcpy", false)?,
        }),
        "phone_disconnect" => PhoneRequest::Disconnect(PhoneDisconnectRequest {
            session: parse_phone_selector(arguments)?,
            keep_wireless: parse_optional_bool(arguments, "keep_wireless", false)?,
        }),
        "phone_screenshot" => PhoneRequest::Screenshot(PhoneScreenshotRequest {
            session: parse_phone_selector(arguments)?,
            backend: parse_phone_backend(arguments)?,
            include_image_data: model.can_receive_images(),
        }),
        "phone_tap" => PhoneRequest::Tap(PhoneTapRequest {
            session: parse_phone_selector(arguments)?,
            phone_snapshot_id: args::parse_optional_string(
                arguments,
                "phone_snapshot_id",
                "phone_tap phone_snapshot_id",
            )?,
            x: parse_phone_coordinate(arguments, "x", "phone_tap")?,
            y: parse_phone_coordinate(arguments, "y", "phone_tap")?,
            use_device_coordinates: parse_optional_bool(
                arguments,
                "use_device_coordinates",
                false,
            )?,
        }),
        "phone_swipe" => PhoneRequest::Swipe(PhoneSwipeRequest {
            session: parse_phone_selector(arguments)?,
            phone_snapshot_id: args::parse_optional_string(
                arguments,
                "phone_snapshot_id",
                "phone_swipe phone_snapshot_id",
            )?,
            start_x: parse_phone_coordinate(arguments, "start_x", "phone_swipe")?,
            start_y: parse_phone_coordinate(arguments, "start_y", "phone_swipe")?,
            end_x: parse_phone_coordinate(arguments, "end_x", "phone_swipe")?,
            end_y: parse_phone_coordinate(arguments, "end_y", "phone_swipe")?,
            duration_ms: parse_optional_duration_ms(arguments)?,
            use_device_coordinates: parse_optional_bool(
                arguments,
                "use_device_coordinates",
                false,
            )?,
        }),
        "phone_type_text" => PhoneRequest::TypeText(PhoneTypeTextRequest {
            session: parse_phone_selector(arguments)?,
            text: parse_required_literal_string(arguments, "text", "phone_type_text text")?,
        }),
        "phone_press_key" => PhoneRequest::PressKey(PhonePressKeyRequest {
            session: parse_phone_selector(arguments)?,
            key: parse_required_string(arguments, "key", "phone_press_key key")?,
        }),
        "phone_install_companion" => PhoneRequest::InstallCompanion(PhoneInstallCompanionRequest {
            session: parse_phone_selector(arguments)?,
            force_reinstall: parse_optional_bool(arguments, "force_reinstall", false)?,
            allow_downgrade: parse_optional_bool(arguments, "allow_downgrade", false)?,
        }),
        "phone_companion_status" => PhoneRequest::CompanionStatus(PhoneCompanionStatusRequest {
            session: parse_phone_selector(arguments)?,
        }),
        "phone_accessibility_tree" => {
            PhoneRequest::AccessibilityTree(PhoneAccessibilityTreeRequest {
                session: parse_phone_selector(arguments)?,
                node_limit: parse_optional_limit(
                    arguments,
                    "node_limit",
                    "phone_accessibility_tree node_limit",
                )?,
            })
        }
        "phone_notifications" => PhoneRequest::Notifications(PhoneNotificationsRequest {
            session: parse_phone_selector(arguments)?,
            limit: parse_optional_limit(arguments, "limit", "phone_notifications limit")?,
        }),
        "phone_notification_open" => PhoneRequest::NotificationOpen(PhoneNotificationOpenRequest {
            session: parse_phone_selector(arguments)?,
            event_id: parse_required_string(
                arguments,
                "event_id",
                "phone_notification_open event_id",
            )?,
        }),
        "phone_notification_dismiss" => {
            PhoneRequest::NotificationDismiss(PhoneNotificationDismissRequest {
                session: parse_phone_selector(arguments)?,
                event_id: parse_required_string(
                    arguments,
                    "event_id",
                    "phone_notification_dismiss event_id",
                )?,
            })
        }
        "phone_notification_action" => {
            PhoneRequest::NotificationAction(PhoneNotificationActionRequest {
                session: parse_phone_selector(arguments)?,
                event_id: parse_required_string(
                    arguments,
                    "event_id",
                    "phone_notification_action event_id",
                )?,
                action_id: parse_required_string(
                    arguments,
                    "action_id",
                    "phone_notification_action action_id",
                )?,
            })
        }
        "phone_notification_reply" => {
            PhoneRequest::NotificationReply(PhoneNotificationReplyRequest {
                session: parse_phone_selector(arguments)?,
                event_id: parse_required_string(
                    arguments,
                    "event_id",
                    "phone_notification_reply event_id",
                )?,
                action_id: parse_required_string(
                    arguments,
                    "action_id",
                    "phone_notification_reply action_id",
                )?,
                text: parse_required_literal_string(
                    arguments,
                    "text",
                    "phone_notification_reply text",
                )?,
            })
        }
        "phone_app_current" => PhoneRequest::AppCurrent(PhoneAppCurrentRequest {
            session: parse_phone_selector(arguments)?,
        }),
        "phone_app_list" => PhoneRequest::AppList(PhoneAppListRequest {
            session: parse_phone_selector(arguments)?,
            include_system: parse_optional_bool(arguments, "include_system", false)?,
            limit: parse_optional_limit(arguments, "limit", "phone_app_list limit")?,
        }),
        "phone_app_launch" => PhoneRequest::AppLaunch(PhoneAppLaunchRequest {
            session: parse_phone_selector(arguments)?,
            package_name: parse_required_string(
                arguments,
                "package_name",
                "phone_app_launch package_name",
            )?,
        }),
        "phone_app_open_intent" => PhoneRequest::AppOpenIntent(PhoneAppOpenIntentRequest {
            session: parse_phone_selector(arguments)?,
            intent_uri: parse_required_string(
                arguments,
                "intent_uri",
                "phone_app_open_intent intent_uri",
            )?,
            package_name: args::parse_optional_string(
                arguments,
                "package_name",
                "phone_app_open_intent package_name",
            )?,
        }),
        "phone_app_force_stop" => PhoneRequest::AppForceStop(PhoneAppForceStopRequest {
            session: parse_phone_selector(arguments)?,
            package_name: parse_required_string(
                arguments,
                "package_name",
                "phone_app_force_stop package_name",
            )?,
        }),
        "phone_app_install" => PhoneRequest::AppInstall(PhoneAppInstallRequest {
            session: parse_phone_selector(arguments)?,
            apk_paths: parse_phone_apk_paths(arguments)?,
            mode: parse_phone_app_install_mode(arguments)?,
            reinstall: parse_optional_bool(arguments, "reinstall", false)?,
            allow_downgrade: parse_optional_bool(arguments, "allow_downgrade", false)?,
            allow_test_apk: parse_optional_bool(arguments, "allow_test_apk", false)?,
            grant_runtime_permissions: parse_optional_bool(
                arguments,
                "grant_runtime_permissions",
                false,
            )?,
        }),
        "phone_open_settings" => PhoneRequest::OpenSettings(PhoneOpenSettingsRequest {
            session: parse_phone_selector(arguments)?,
            screen: parse_phone_settings_screen(arguments)?,
            package_name: args::parse_optional_string(
                arguments,
                "package_name",
                "phone_open_settings package_name",
            )?,
        }),
        other => return Err(anyhow!("unexpected phone tool name: {other}")),
    };
    Ok(request)
}

/// Shape a `PhoneResponse` into the MCP result for the originating tool. The
/// service follows the Phase 1 routing honesty rule, so several request tools
/// (connect/observe/screenshot/refresh) come back as `Status` until a real
/// device path lands; the response shaping keys off the actual variant, which
/// is always one of the contracted mappings for each tool.
fn shape_phone_response(
    tool_name: &str,
    response: PhoneResponse,
    model: &crate::mcp_server::ModelSessionInfo,
) -> Result<Value> {
    match response {
        PhoneResponse::Observe(response) => {
            phone_observe_result(response, model.can_receive_images())
        }
        PhoneResponse::Status(report) => phone_status_result(report),
        PhoneResponse::Devices(response) => phone_list_devices_result(response),
        PhoneResponse::Capabilities(profile) => Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "Phone capability profile {} for session {} (serial {}).",
                    profile.profile_id, profile.session_id, profile.serial
                )
            }],
            "structuredContent": profile,
            "isError": false
        })),
        PhoneResponse::PairedWireless(response) => phone_pair_wireless_result(response),
        PhoneResponse::Connected(session) => phone_connected_result(session),
        PhoneResponse::Disconnected(response) => phone_disconnect_result(response),
        PhoneResponse::Screenshot(response) => {
            phone_screenshot_result(response, model.can_receive_images())
        }
        PhoneResponse::Action(response) => phone_action_result(response),
        PhoneResponse::CompanionStatus(response) => phone_companion_status_result(response),
        PhoneResponse::AccessibilityTree(response) => phone_accessibility_tree_result(response),
        PhoneResponse::Notifications(response) => phone_notifications_result(response),
        PhoneResponse::App(response) => phone_app_result(response),
    }
    .map_err(|error| anyhow!("failed to shape {tool_name} response: {error}"))
}
