import type { PhoneRequestContext } from "../context";

export type DiagnosticEntry = { code: string; message: string; details?: string };
export type PixelSize = { width: number; height: number };
export type RectF = { x: number; y: number; width: number; height: number };
export type PhoneBackendKind = "auto" | "adb" | "companion" | "scrcpy" | "none";
export type PhoneConnectionKind = "usb" | "emulator" | "legacy_tcpip" | "wireless_debugging" | "companion_direct" | "unknown";
export type PhoneTargetDeviceKind = "galaxy_s26_ultra" | "redmi_tablet" | "emulator" | "unknown_android";
export type PhoneCapabilityRefreshState = "detected" | "reused" | "refreshed" | "stale";
export type PhoneDeviceState = "device" | "unauthorized" | "offline" | "no_permissions" | "connecting" | "bootloader" | "recovery" | "unknown";
export type PhoneNotificationRedaction = "none" | "partial" | "full";
export type PhoneAppInstallMode = "single" | "multiple" | "multi_package";
export type PhoneSettingsScreen = "accessibility" | "notification_access" | "overlay_permission" | "app_details" | "wireless_debugging" | "battery_optimization";
export type PhoneAppResponseKind = "current" | "list" | "launch" | "open_intent" | "force_stop" | "install" | "open_settings";
export type PhoneInstallStrategy = "single" | "multiple" | "multi_package";

export type PhoneImage = { mime_type: string; data_base64: string; width?: number; height?: number };
export type PhoneAvailableAction = { action: string; backend: PhoneBackendKind; detail?: string };
export type PhoneUnavailableAction = { action: string; reason: string; detail?: string };
export type PhoneCompanionCapabilities = {
  installed: boolean; package_name: string; installed_version?: string; expected_version?: string;
  installed_cert_sha256?: string; expected_cert_sha256?: string; apk_sha256?: string;
  signature_matches_expected: boolean; allow_downgrade: boolean; auto_install_attempted: boolean;
  rpc_reachable: boolean; rpc_token_expires_at_ms?: number; accessibility_enabled: boolean;
  can_perform_gestures: boolean; can_retrieve_window_content: boolean; can_take_screenshot: boolean;
  notification_listener_enabled: boolean; native_overlay: boolean; native_overlay_pass_through: boolean;
  gesture_dispatch: boolean; screenshot: boolean; accessibility_tree: boolean; notifications: boolean;
  privileged_setup?: string;
};
export type PhoneScrcpyCapabilities = { installed: boolean; version?: string; active: boolean; host_window_mapped: boolean; window_title?: string; video_codec?: string; reason?: string };
export type PhoneBackendCapabilities = {
  adb: boolean; companion: boolean; scrcpy: boolean; screenshot: boolean; gestures: boolean;
  text_input: boolean; key_input: boolean; accessibility_tree: boolean; notifications: boolean;
  app_management: boolean; host_visible_overlay: boolean; screenshot_synthetic_cursor: boolean;
  phone_native_overlay: boolean;
};
export type PhoneCapabilityProfile = {
  profile_id: string; session_id: string; serial: string; detected_at_ms: number; stale: boolean;
  refresh_state: PhoneCapabilityRefreshState; manufacturer?: string; brand?: string; model?: string;
  device?: string; target_device_kind: PhoneTargetDeviceKind; hyperos_version?: string; android_sdk?: number;
  android_release?: string; display_size?: PixelSize; density_dpi?: number; orientation?: string;
  display_rotation_degrees?: number; connection_kind: PhoneConnectionKind;
  companion: PhoneCompanionCapabilities; scrcpy: PhoneScrcpyCapabilities; root_available: boolean;
  shizuku_available: boolean; device_owner: boolean; available_actions?: PhoneAvailableAction[];
  unavailable_actions?: PhoneUnavailableAction[];
};
export type PhoneConnectionIdentity = 
  { kind: "adb"; serial: string; name?: string }
  | { kind: "companion_direct"; device_id: string; link_epoch: number; name?: string }
  | { kind: "companion_v1"; serial: string }
  | { kind: "scrcpy"; serial: string };

export type PhoneSession = {
  session_id: string; serial: string; connection_kind: PhoneConnectionKind; backend: PhoneBackendKind;
  capabilities: PhoneBackendCapabilities; capability_profile: PhoneCapabilityProfile;
  companion?: PhoneCompanionCapabilities; managed_process: boolean; window_title?: string; created_at_ms: number;
  connection?: PhoneConnectionIdentity;
};
export type PhonePoint = { x: number; y: number };
export type PhoneCursorCapabilities = { host_visible_overlay: boolean; screenshot_synthetic_cursor: boolean; phone_native_overlay: boolean; visible_overlay_reason?: string };
export type PhoneCursorState = { visible: boolean; sequence: number; device_point?: PhonePoint; screenshot_point?: PhonePoint; snapshot_id?: string; source_action?: string; updated_at_ms: number };
export type PhoneCoordinateMapping = { mapping_id: string; session_id: string; serial: string; device_rect: RectF; screenshot_rect: RectF; host_window_rect?: RectF; host_content_rect?: RectF; rotation_degrees: number; captured_at_ms: number };
export type PhoneDevice = { serial: string; state: PhoneDeviceState; connection_kind: PhoneConnectionKind; model?: string; product?: string; device?: string; transport_id?: string; primary?: boolean; device_id?: string; link_epoch?: number; connection?: PhoneConnectionIdentity };
export type PhoneAccessibilitySummary = { package_name?: string; activity?: string; node_count: number; headline_texts?: string[]; truncated: boolean; redacted: boolean };
export type PhoneAccessibilityNode = { node_index: number; parent_index?: number; class_name?: string; package_name?: string; text?: string; content_description?: string; bounds?: RectF; clickable: boolean; focusable: boolean; enabled: boolean; redacted: boolean };
export type PhoneNotificationAction = { action_id: string; title?: string; supports_inline_reply: boolean };
export type PhoneNotificationEvent = { event_id: string; key?: string; package_name: string; channel?: string; title?: string; body?: string; redaction: PhoneNotificationRedaction; rank?: number; ongoing: boolean; can_open: boolean; can_dismiss: boolean; actions?: PhoneNotificationAction[]; posted_at_ms: number };
export type PhoneAppInfo = { package_name: string; label?: string; activity?: string; version_name?: string; version_code?: number; launchable: boolean; system_app: boolean };

export type PhoneSessionSelector = { session_id?: string; serial?: string; device_id?: string; appshot_id?: string };
export type PhoneObserveRequest = { type: "observe"; session_id?: string; serial?: string; backend?: PhoneBackendKind; include_image_data?: boolean; include_accessibility?: boolean; include_notifications?: boolean };
export type PhoneStatusRequest = { type: "status"; refresh_devices?: boolean };
export type PhoneListDevicesRequest = { type: "list_devices"; include_mdns?: boolean };
export type PhoneRefreshCapabilitiesRequest = { type: "refresh_capabilities" } & PhoneSessionSelector;
export type PhonePairWirelessRequest = { type: "pair_wireless"; host_port: string; pairing_code: string };
export type PhoneConnectRequest = { type: "connect"; serial?: string; device_id?: string; backend?: PhoneBackendKind; install_companion?: boolean; start_scrcpy?: boolean };
export type PhoneDisconnectRequest = { type: "disconnect"; keep_wireless?: boolean } & PhoneSessionSelector;
export type PhoneScreenshotRequest = { type: "screenshot"; backend?: PhoneBackendKind; include_image_data?: boolean } & PhoneSessionSelector;
export type PhoneTapRequest = { type: "tap"; phone_snapshot_id?: string; x: number; y: number; use_device_coordinates?: boolean } & PhoneSessionSelector;
export type PhoneSwipeRequest = { type: "swipe"; phone_snapshot_id?: string; start_x: number; start_y: number; end_x: number; end_y: number; duration_ms?: number; use_device_coordinates?: boolean } & PhoneSessionSelector;
export type PhoneTypeTextRequest = { type: "type_text"; text: string } & PhoneSessionSelector;
export type PhonePressKeyRequest = { type: "press_key"; key: string } & PhoneSessionSelector;
export type PhoneInstallCompanionRequest = { type: "install_companion"; force_reinstall?: boolean; allow_downgrade?: boolean } & PhoneSessionSelector;
export type PhoneCompanionStatusRequest = { type: "companion_status" } & PhoneSessionSelector;
export type PhoneAccessibilityTreeRequest = { type: "accessibility_tree"; node_limit?: number } & PhoneSessionSelector;
export type PhoneNotificationsRequest = { type: "notifications"; limit?: number } & PhoneSessionSelector;
export type PhoneNotificationOpenRequest = { type: "notification_open"; event_id: string } & PhoneSessionSelector;
export type PhoneNotificationDismissRequest = { type: "notification_dismiss"; event_id: string } & PhoneSessionSelector;
export type PhoneNotificationActionRequest = { type: "notification_action"; event_id: string; action_id: string } & PhoneSessionSelector;
export type PhoneNotificationReplyRequest = { type: "notification_reply"; event_id: string; action_id: string; text: string } & PhoneSessionSelector;
export type PhoneAppCurrentRequest = { type: "app_current" } & PhoneSessionSelector;
export type PhoneAppListRequest = { type: "app_list"; include_system?: boolean; limit?: number } & PhoneSessionSelector;
export type PhoneAppLaunchRequest = { type: "app_launch"; package_name: string } & PhoneSessionSelector;
export type PhoneAppOpenIntentRequest = { type: "app_open_intent"; intent_uri: string; package_name?: string } & PhoneSessionSelector;
export type PhoneAppForceStopRequest = { type: "app_force_stop"; package_name: string } & PhoneSessionSelector;
export type PhoneAppInstallRequest = { type: "app_install"; apk_paths: string[]; mode?: PhoneAppInstallMode; reinstall?: boolean; allow_downgrade?: boolean; allow_test_apk?: boolean; grant_runtime_permissions?: boolean } & PhoneSessionSelector;
export type PhoneOpenSettingsRequest = { type: "open_settings"; screen: PhoneSettingsScreen; package_name?: string } & PhoneSessionSelector;
export type PhoneRequest = PhoneObserveRequest | PhoneStatusRequest | PhoneListDevicesRequest | PhoneRefreshCapabilitiesRequest | PhonePairWirelessRequest | PhoneConnectRequest | PhoneDisconnectRequest | PhoneScreenshotRequest | PhoneTapRequest | PhoneSwipeRequest | PhoneTypeTextRequest | PhonePressKeyRequest | PhoneInstallCompanionRequest | PhoneCompanionStatusRequest | PhoneAccessibilityTreeRequest | PhoneNotificationsRequest | PhoneNotificationOpenRequest | PhoneNotificationDismissRequest | PhoneNotificationActionRequest | PhoneNotificationReplyRequest | PhoneAppCurrentRequest | PhoneAppListRequest | PhoneAppLaunchRequest | PhoneAppOpenIntentRequest | PhoneAppForceStopRequest | PhoneAppInstallRequest | PhoneOpenSettingsRequest;

export type PhoneObserveResponse = { type: "observe"; session: PhoneSession; phone_snapshot_id?: string; screenshot_path?: string; inline_image?: PhoneImage; current_app?: PhoneAppInfo; accessibility_summary?: PhoneAccessibilitySummary; recent_notifications?: PhoneNotificationEvent[]; cursor?: PhoneCursorState; backend: PhoneBackendKind; capability_profile_id: string; profile_refresh_state: PhoneCapabilityRefreshState; available_actions?: PhoneAvailableAction[]; unavailable_actions?: PhoneUnavailableAction[]; diagnostics?: DiagnosticEntry[] };
export type PhoneStatusReport = { type: "status"; enabled: boolean; adb_available: boolean; adb_path?: string; adb_version?: string; adb_server_running?: boolean; scrcpy_available: boolean; scrcpy_path?: string; scrcpy_version?: string; companion_enabled: boolean; mdns_available: boolean; default_serial?: string; default_backend: PhoneBackendKind; sessions?: PhoneSession[]; devices?: PhoneDevice[]; diagnostics?: DiagnosticEntry[] };
export type PhoneListDevicesResponse = { type: "devices"; devices: PhoneDevice[]; adb_path?: string; adb_version?: string; diagnostics?: DiagnosticEntry[] };
export type PhoneCapabilitiesResponse = { type: "capabilities" } & PhoneCapabilityProfile;
export type PhonePairWirelessResponse = { type: "paired_wireless"; paired: boolean; host_port: string; serial?: string; diagnostics?: DiagnosticEntry[] };
export type PhoneConnectedResponse = { type: "connected" } & PhoneSession;
export type PhoneDisconnectResponse = { type: "disconnected"; session_id: string; serial: string; disconnected: boolean; diagnostics?: DiagnosticEntry[] };
export type PhoneScreenshotResponse = { type: "screenshot"; session_id: string; serial: string; phone_snapshot_id: string; backend: PhoneBackendKind; capability_profile_id: string; profile_refresh_state: PhoneCapabilityRefreshState; screenshot_path?: string; inline_image?: PhoneImage; device_size: PixelSize; coordinate_mapping: PhoneCoordinateMapping; cursor?: PhoneCursorState; cursor_capabilities: PhoneCursorCapabilities; capture_contains_native_overlay: boolean; diagnostics?: DiagnosticEntry[] };
export type PhoneActionResponse = { type: "action"; session_id: string; serial: string; action: string; backend: PhoneBackendKind; capability_profile_id: string; profile_refresh_state: PhoneCapabilityRefreshState; phone_snapshot_id?: string; cursor?: PhoneCursorState; diagnostics?: DiagnosticEntry[] };
export type PhoneCompanionStatusResponse = { type: "companion_status"; session_id: string; serial: string; companion: PhoneCompanionCapabilities; diagnostics?: DiagnosticEntry[] };
export type PhoneAccessibilityTreeResponse = { type: "accessibility_tree"; session_id: string; serial: string; backend: PhoneBackendKind; package_name?: string; activity?: string; nodes?: PhoneAccessibilityNode[]; truncated: boolean; redacted: boolean; diagnostics?: DiagnosticEntry[] };
export type PhoneNotificationsResponse = { type: "notifications"; session_id: string; serial: string; backend: PhoneBackendKind; listener_enabled: boolean; events?: PhoneNotificationEvent[]; truncated: boolean; diagnostics?: DiagnosticEntry[] };
export type PhoneAppResponse = { type: "app"; session_id: string; serial: string; kind: PhoneAppResponseKind; backend: PhoneBackendKind; success: boolean; current_app?: PhoneAppInfo; apps?: PhoneAppInfo[]; truncated: boolean; install_strategy?: PhoneInstallStrategy; destination_appshot?: { appshot_id: string; action_snapshot: { session_id?: string } } & Record<string, unknown>; diagnostics?: DiagnosticEntry[] };
export type PhoneAppShotRequiredResponse = { type: "appshot_required"; code: string; reason: string; message: string; fresh_appshot?: { appshot_id: string } & Record<string, unknown> };
export type PhoneFeatureErrorResponse = { type: "feature_error"; code: string; message: string };
export type PhoneContentResponse = { type: "content"; result: { success: boolean; diagnostics?: DiagnosticEntry[] } };
export type PhoneClipboardResponse = { type: "clipboard"; result: { success: boolean; diagnostics?: DiagnosticEntry[] } };
export type PhoneEditorResponse = { type: "editor"; result: { success: boolean; diagnostics?: DiagnosticEntry[] } };
export type PhoneCameraResponse = { type: "camera"; result: { success: boolean; diagnostics?: DiagnosticEntry[] } };
export type PhoneStorageResponse = { type: "storage"; result: { success: boolean; diagnostics?: DiagnosticEntry[] } };
export type PhoneResponse = PhoneObserveResponse | PhoneStatusReport | PhoneListDevicesResponse | PhoneCapabilitiesResponse | PhonePairWirelessResponse | PhoneConnectedResponse | PhoneDisconnectResponse | PhoneScreenshotResponse | PhoneActionResponse | PhoneCompanionStatusResponse | PhoneAccessibilityTreeResponse | PhoneNotificationsResponse | PhoneAppResponse | PhoneAppShotRequiredResponse | PhoneFeatureErrorResponse | PhoneContentResponse | PhoneClipboardResponse | PhoneEditorResponse | PhoneCameraResponse | PhoneStorageResponse;

export type PhoneServiceRequest = { type: "phone"; request: PhoneRequest; context?: PhoneRequestContext };
export type PhoneServiceResponse = { type: "phone"; response: PhoneResponse };
