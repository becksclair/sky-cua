use super::*;
use crate::{PhoneCameraRequest, PhoneFeatureCall};
use serde_json::json;

fn sample_companion() -> PhoneCompanionCapabilities {
    PhoneCompanionCapabilities {
        installed: true,
        package_name: "com.skycua.phonecompanion".to_string(),
        installed_version: Some("1.0.0".to_string()),
        expected_version: Some("1.0.0".to_string()),
        installed_cert_sha256: Some("aa".to_string()),
        expected_cert_sha256: Some("aa".to_string()),
        apk_sha256: Some("bb".to_string()),
        signature_matches_expected: true,
        allow_downgrade: false,
        auto_install_attempted: true,
        rpc_reachable: true,
        rpc_token_expires_at_ms: Some(900_000),
        accessibility_enabled: true,
        can_perform_gestures: true,
        can_retrieve_window_content: true,
        can_take_screenshot: true,
        notification_listener_enabled: true,
        native_overlay: true,
        native_overlay_pass_through: true,
        gesture_dispatch: true,
        screenshot: true,
        accessibility_tree: true,
        notifications: true,
        privileged_setup: None,
    }
}

fn sample_profile() -> PhoneCapabilityProfile {
    PhoneCapabilityProfile {
        profile_id: "prof-1".to_string(),
        session_id: "sess-1".to_string(),
        serial: "ABC123".to_string(),
        detected_at_ms: 1_000,
        stale: false,
        refresh_state: PhoneCapabilityRefreshState::Detected,
        manufacturer: Some("Samsung".to_string()),
        brand: Some("samsung".to_string()),
        model: Some("Galaxy S26 Ultra".to_string()),
        device: Some("e3q".to_string()),
        target_device_kind: PhoneTargetDeviceKind::GalaxyS26Ultra,
        hyperos_version: None,
        android_sdk: Some(36),
        android_release: Some("16".to_string()),
        display_size: Some(PixelSize {
            width: 1440,
            height: 3120,
        }),
        density_dpi: Some(560),
        orientation: Some("portrait".to_string()),
        // Upside-down portrait: the coarse label is still "portrait", so the
        // exact 180 quarter only survives in `display_rotation_degrees`.
        display_rotation_degrees: Some(180),
        connection_kind: PhoneConnectionKind::WirelessDebugging,
        companion: sample_companion(),
        scrcpy: PhoneScrcpyCapabilities::absent(),
        root_available: false,
        shizuku_available: false,
        device_owner: false,
        available_actions: vec![PhoneAvailableAction {
            action: "phone_tap".to_string(),
            backend: PhoneBackendKind::Companion,
            detail: None,
        }],
        unavailable_actions: vec![PhoneUnavailableAction {
            action: "phone_notification_reply".to_string(),
            reason: "notification_listener_disabled".to_string(),
            detail: None,
        }],
        routes: Vec::new(),
    }
}

fn sample_backend_caps() -> PhoneBackendCapabilities {
    PhoneBackendCapabilities {
        adb: true,
        companion: true,
        scrcpy: false,
        screenshot: true,
        gestures: true,
        text_input: true,
        key_input: true,
        accessibility_tree: true,
        notifications: true,
        app_management: true,
        host_visible_overlay: false,
        screenshot_synthetic_cursor: true,
        phone_native_overlay: true,
    }
}

fn sample_session() -> PhoneSession {
    PhoneSession {
        session_id: "sess-1".to_string(),
        serial: "ABC123".to_string(),
        connection: Some(PhoneConnectionIdentity::Adb {
            serial: "ABC123".to_string(),
            name: None,
        }),
        connection_kind: PhoneConnectionKind::WirelessDebugging,
        backend: PhoneBackendKind::Companion,
        capabilities: sample_backend_caps(),
        capability_profile: sample_profile(),
        companion: Some(sample_companion()),
        managed_process: false,
        window_title: None,
        created_at_ms: 2_000,
    }
}

fn sample_mapping() -> PhoneCoordinateMapping {
    PhoneCoordinateMapping {
        mapping_id: "map-1".to_string(),
        session_id: "sess-1".to_string(),
        serial: "ABC123".to_string(),
        device_rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 3120.0,
            space: crate::CoordinateSpace::StreamPixels,
        },
        screenshot_rect: RectF {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 3120.0,
            space: crate::CoordinateSpace::StreamPixels,
        },
        host_window_rect: None,
        host_content_rect: None,
        rotation_degrees: 0,
        captured_at_ms: 3_000,
    }
}

#[test]
fn phone_request_variants_preserve_type_tags() {
    let requests: Vec<(PhoneRequest, &str)> = vec![
        (
            PhoneRequest::SmsQuery(PhoneSmsQueryRequest {
                profile: "primary".to_string(),
                start_ms: 1_000,
                end_ms: 2_000,
                limit: 250,
                cursor: None,
            }),
            "sms_query",
        ),
        (
            PhoneRequest::Observe(PhoneObserveRequest::default()),
            "observe",
        ),
        (
            PhoneRequest::Status(PhoneStatusRequest::default()),
            "status",
        ),
        (
            PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
            "list_devices",
        ),
        (
            PhoneRequest::RefreshCapabilities(PhoneRefreshCapabilitiesRequest::default()),
            "refresh_capabilities",
        ),
        (
            PhoneRequest::PairWireless(PhonePairWirelessRequest {
                host_port: "10.0.0.5:37123".to_string(),
                pairing_code: "123456".to_string(),
            }),
            "pair_wireless",
        ),
        (
            PhoneRequest::Connect(PhoneConnectRequest::default()),
            "connect",
        ),
        (
            PhoneRequest::Disconnect(PhoneDisconnectRequest::default()),
            "disconnect",
        ),
        (
            PhoneRequest::Screenshot(PhoneScreenshotRequest {
                include_image_data: true,
                ..Default::default()
            }),
            "screenshot",
        ),
        (
            PhoneRequest::Tap(PhoneTapRequest {
                session: PhoneSessionSelector::default(),
                phone_snapshot_id: Some("snap-1".to_string()),
                x: 100.0,
                y: 200.0,
                use_device_coordinates: false,
            }),
            "tap",
        ),
        (
            PhoneRequest::Swipe(PhoneSwipeRequest {
                session: PhoneSessionSelector::default(),
                phone_snapshot_id: Some("snap-1".to_string()),
                start_x: 0.0,
                start_y: 0.0,
                end_x: 10.0,
                end_y: 10.0,
                duration_ms: Some(120),
                use_device_coordinates: false,
            }),
            "swipe",
        ),
        (
            PhoneRequest::TypeText(PhoneTypeTextRequest {
                session: PhoneSessionSelector::default(),
                text: "hello".to_string(),
            }),
            "type_text",
        ),
        (
            PhoneRequest::PressKey(PhonePressKeyRequest {
                session: PhoneSessionSelector::default(),
                key: "KEYCODE_BACK".to_string(),
            }),
            "press_key",
        ),
        (
            PhoneRequest::InstallCompanion(PhoneInstallCompanionRequest::default()),
            "install_companion",
        ),
        (
            PhoneRequest::CompanionStatus(PhoneCompanionStatusRequest::default()),
            "companion_status",
        ),
        (
            PhoneRequest::AccessibilityTree(PhoneAccessibilityTreeRequest::default()),
            "accessibility_tree",
        ),
        (
            PhoneRequest::Notifications(PhoneNotificationsRequest::default()),
            "notifications",
        ),
        (
            PhoneRequest::NotificationOpen(PhoneNotificationOpenRequest {
                session: PhoneSessionSelector::default(),
                event_id: "evt-1".to_string(),
            }),
            "notification_open",
        ),
        (
            PhoneRequest::NotificationDismiss(PhoneNotificationDismissRequest {
                session: PhoneSessionSelector::default(),
                event_id: "evt-1".to_string(),
            }),
            "notification_dismiss",
        ),
        (
            PhoneRequest::NotificationAction(PhoneNotificationActionRequest {
                session: PhoneSessionSelector::default(),
                event_id: "evt-1".to_string(),
                action_id: "act-1".to_string(),
            }),
            "notification_action",
        ),
        (
            PhoneRequest::NotificationReply(PhoneNotificationReplyRequest {
                session: PhoneSessionSelector::default(),
                event_id: "evt-1".to_string(),
                action_id: "act-1".to_string(),
                text: "ok".to_string(),
            }),
            "notification_reply",
        ),
        (
            PhoneRequest::AppCurrent(PhoneAppCurrentRequest::default()),
            "app_current",
        ),
        (
            PhoneRequest::AppList(PhoneAppListRequest::default()),
            "app_list",
        ),
        (
            PhoneRequest::AppLaunch(PhoneAppLaunchRequest {
                session: PhoneSessionSelector::default(),
                package_name: "com.example".to_string(),
            }),
            "app_launch",
        ),
        (
            PhoneRequest::AppOpenIntent(PhoneAppOpenIntentRequest {
                session: PhoneSessionSelector::default(),
                intent_uri: "https://example.test".to_string(),
                package_name: None,
            }),
            "app_open_intent",
        ),
        (
            PhoneRequest::AppForceStop(PhoneAppForceStopRequest {
                session: PhoneSessionSelector::default(),
                package_name: "com.example".to_string(),
            }),
            "app_force_stop",
        ),
        (
            PhoneRequest::AppInstall(PhoneAppInstallRequest {
                session: PhoneSessionSelector::default(),
                apk_paths: vec!["/tmp/app.apk".to_string()],
                mode: PhoneAppInstallMode::Single,
                reinstall: true,
                allow_downgrade: false,
                allow_test_apk: false,
                grant_runtime_permissions: false,
            }),
            "app_install",
        ),
        (
            PhoneRequest::OpenSettings(PhoneOpenSettingsRequest {
                session: PhoneSessionSelector::default(),
                screen: PhoneSettingsScreen::Accessibility,
                package_name: None,
            }),
            "open_settings",
        ),
    ];

    assert_eq!(requests.len(), 28, "all phone request variants are covered");

    for (request, expected) in requests {
        let rendered = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(rendered["type"], expected, "tag for {expected}");
        let parsed: PhoneRequest =
            serde_json::from_value(rendered).expect("request should round-trip");
        assert_eq!(parsed, request, "round-trip for {expected}");
    }
}

#[test]
fn sms_query_response_keeps_nullable_raw_fields_and_scan_contract() {
    let response = PhoneResponse::SmsQuery(PhoneSmsQueryResponse {
        schema: PHONE_SMS_QUERY_SCHEMA.to_owned(),
        profile: "primary".to_owned(),
        device_id: Some("device-1".to_owned()),
        transport: Some("companion_direct".to_owned()),
        access: Some("observation_only".to_owned()),
        messages: vec![PhoneSmsRecord {
            id: Some(7),
            thread_id: None,
            address: Some("+34123".to_owned()),
            person: None,
            date: Some(1_000),
            date_sent: None,
            protocol: None,
            read: Some(1),
            status: None,
            message_type: Some(1),
            reply_path_present: None,
            subject: None,
            body: Some("hello".to_owned()),
            service_center: None,
            locked: None,
            sub_id: None,
            creator: None,
            seen: None,
            priority: None,
            subscription_id: None,
            error_code: None,
            message_class: None,
        }],
        next_cursor: None,
        scan: Some(PhoneSmsScan {
            has_more: false,
            exhausted_as_observed: true,
            snapshot: false,
            observed_at_ms: 2_000,
        }),
        error: None,
    });
    let value = serde_json::to_value(response).expect("SMS response serializes");
    assert_eq!(value["type"], "sms_query");
    assert!(value.get("thread_id").is_none());
    assert!(value["messages"][0]["thread_id"].is_null());
    assert_eq!(value["scan"]["snapshot"], false);
}

#[test]
fn phone_response_variants_preserve_type_tags() {
    let responses: Vec<(PhoneResponse, &str)> = vec![
        (
            PhoneResponse::Observe(PhoneObserveResponse {
                session: sample_session(),
                appshot: None,
                phone_snapshot_id: Some("snap-1".to_string()),
                screenshot_path: None,
                inline_image: Some(PhoneImage {
                    mime_type: "image/png".to_string(),
                    data_base64: "AAAA".to_string(),
                    width: Some(1440),
                    height: Some(3120),
                }),
                current_app: None,
                accessibility_summary: None,
                recent_notifications: Vec::new(),
                cursor: None,
                backend: PhoneBackendKind::Companion,
                capability_profile_id: "prof-1".to_string(),
                profile_refresh_state: PhoneCapabilityRefreshState::Reused,
                available_actions: Vec::new(),
                unavailable_actions: Vec::new(),
                diagnostics: Vec::new(),
            }),
            "observe",
        ),
        (
            PhoneResponse::Status(PhoneStatusReport {
                enabled: true,
                adb_available: true,
                adb_path: Some("/usr/bin/adb".to_string()),
                adb_version: Some("1.0.41".to_string()),
                adb_server_running: Some(true),
                scrcpy_available: false,
                scrcpy_path: None,
                scrcpy_version: None,
                companion_enabled: true,
                mdns_available: true,
                default_serial: None,
                default_backend: PhoneBackendKind::Auto,
                sessions: Vec::new(),
                devices: Vec::new(),
                diagnostics: Vec::new(),
            }),
            "status",
        ),
        (
            PhoneResponse::Devices(PhoneListDevicesResponse {
                devices: vec![PhoneDevice {
                    serial: "ABC123".to_string(),
                    device_id: None,
                    link_epoch: None,
                    connection: Some(PhoneConnectionIdentity::Adb {
                        serial: "ABC123".to_string(),
                        name: Some("SM-S938B".to_string()),
                    }),
                    state: PhoneDeviceState::Device,
                    connection_kind: PhoneConnectionKind::Usb,
                    model: Some("SM-S938B".to_string()),
                    product: None,
                    device: None,
                    transport_id: Some("3".to_string()),
                    primary: false,
                    alias: None,
                }],
                adb_path: Some("/usr/bin/adb".to_string()),
                adb_version: None,
                diagnostics: Vec::new(),
            }),
            "devices",
        ),
        (
            PhoneResponse::Capabilities(sample_profile()),
            "capabilities",
        ),
        (
            PhoneResponse::PairedWireless(PhonePairWirelessResponse {
                paired: true,
                host_port: "10.0.0.5:5555".to_string(),
                serial: Some("10.0.0.5:5555".to_string()),
                diagnostics: Vec::new(),
            }),
            "paired_wireless",
        ),
        (PhoneResponse::Connected(sample_session()), "connected"),
        (
            PhoneResponse::Disconnected(PhoneDisconnectResponse {
                session_id: "sess-1".to_string(),
                serial: "ABC123".to_string(),
                disconnected: true,
                diagnostics: Vec::new(),
            }),
            "disconnected",
        ),
        (
            PhoneResponse::Screenshot(PhoneScreenshotResponse {
                session_id: "sess-1".to_string(),
                serial: "ABC123".to_string(),
                phone_snapshot_id: "snap-1".to_string(),
                backend: PhoneBackendKind::Adb,
                capability_profile_id: "prof-1".to_string(),
                profile_refresh_state: PhoneCapabilityRefreshState::Detected,
                screenshot_path: None,
                inline_image: None,
                device_size: PixelSize {
                    width: 1440,
                    height: 3120,
                },
                coordinate_mapping: sample_mapping(),
                cursor: None,
                cursor_capabilities: PhoneCursorCapabilities {
                    host_visible_overlay: false,
                    screenshot_synthetic_cursor: true,
                    phone_native_overlay: false,
                    visible_overlay_reason: None,
                },
                capture_contains_native_overlay: false,
                diagnostics: Vec::new(),
            }),
            "screenshot",
        ),
        (
            PhoneResponse::Action(PhoneActionResponse {
                session_id: "sess-1".to_string(),
                serial: "ABC123".to_string(),
                action: "phone_tap".to_string(),
                backend: PhoneBackendKind::Companion,
                capability_profile_id: "prof-1".to_string(),
                profile_refresh_state: PhoneCapabilityRefreshState::Reused,
                phone_snapshot_id: Some("snap-1".to_string()),
                cursor: None,
                diagnostics: Vec::new(),
            }),
            "action",
        ),
        (
            PhoneResponse::CompanionStatus(PhoneCompanionStatusResponse {
                session_id: "sess-1".to_string(),
                serial: "ABC123".to_string(),
                companion: sample_companion(),
                diagnostics: Vec::new(),
            }),
            "companion_status",
        ),
        (
            PhoneResponse::AccessibilityTree(PhoneAccessibilityTreeResponse {
                session_id: "sess-1".to_string(),
                serial: "ABC123".to_string(),
                backend: PhoneBackendKind::Companion,
                package_name: Some("com.example".to_string()),
                activity: None,
                nodes: Vec::new(),
                truncated: false,
                redacted: false,
                diagnostics: Vec::new(),
            }),
            "accessibility_tree",
        ),
        (
            PhoneResponse::Notifications(PhoneNotificationsResponse {
                session_id: "sess-1".to_string(),
                serial: "ABC123".to_string(),
                backend: PhoneBackendKind::Companion,
                listener_enabled: true,
                events: Vec::new(),
                truncated: false,
                diagnostics: Vec::new(),
            }),
            "notifications",
        ),
        (
            PhoneResponse::App(PhoneAppResponse {
                session_id: "sess-1".to_string(),
                serial: "ABC123".to_string(),
                kind: PhoneAppResponseKind::Launch,
                backend: PhoneBackendKind::Adb,
                success: true,
                destination_appshot: None,
                current_app: None,
                apps: Vec::new(),
                truncated: false,
                install_strategy: None,
                diagnostics: Vec::new(),
            }),
            "app",
        ),
    ];

    for (response, expected) in responses {
        let rendered = serde_json::to_value(&response).expect("response should serialize");
        assert_eq!(rendered["type"], expected, "tag for {expected}");
        let parsed: PhoneResponse =
            serde_json::from_value(rendered).expect("response should round-trip");
        assert_eq!(parsed, response, "round-trip for {expected}");
    }
}

#[test]
fn observe_request_flattens_session_selector() {
    let parsed: PhoneObserveRequest = serde_json::from_value(json!({
        "session_id": "sess-1",
        "include_notifications": true
    }))
    .expect("observe request should deserialize");
    assert_eq!(parsed.session.session_id.as_deref(), Some("sess-1"));
    assert!(parsed.include_notifications);
    assert!(parsed.include_image_data, "image data defaults to on");
}

#[test]
fn camera_follow_up_keeps_phone_and_camera_session_ids_distinct() {
    let parsed: PhoneFeatureCall<PhoneCameraRequest> = serde_json::from_value(json!({
        "session_id": "phone-session-1",
        "appshot_id": "shot-1",
        "operation": "preview_stop",
        "camera_session_id": "camera-session-1"
    }))
    .expect("camera follow-up should deserialize");
    assert_eq!(
        parsed.session.session_id.as_deref(),
        Some("phone-session-1")
    );
    assert_eq!(
        parsed.request,
        PhoneCameraRequest::PreviewStop {
            camera_session_id: "camera-session-1".to_string()
        }
    );

    let rendered = serde_json::to_value(parsed).expect("camera follow-up should serialize");
    assert_eq!(rendered["session_id"], "phone-session-1");
    assert_eq!(rendered["camera_session_id"], "camera-session-1");
}

#[test]
fn ambiguous_camera_follow_up_without_camera_session_id_is_rejected() {
    let parsed = serde_json::from_value::<PhoneFeatureCall<PhoneCameraRequest>>(json!({
        "session_id": "phone-session-1",
        "appshot_id": "shot-1",
        "operation": "preview_stop"
    }));
    assert!(parsed.is_err());
}

#[test]
fn pairing_code_is_not_emitted_for_response() {
    // The pairing request carries the code, but no response type echoes it.
    let response = PhonePairWirelessResponse {
        paired: true,
        host_port: "10.0.0.5:5555".to_string(),
        serial: None,
        diagnostics: Vec::new(),
    };
    let rendered = serde_json::to_value(&response).expect("serialize");
    assert!(rendered.get("pairing_code").is_none());
}

#[test]
fn capability_profile_carries_backend_and_action_lists() {
    let profile = sample_profile();
    let rendered = serde_json::to_value(&profile).expect("serialize");
    assert_eq!(rendered["target_device_kind"], "galaxy_s26_ultra");
    assert_eq!(rendered["refresh_state"], "detected");
    assert_eq!(rendered["available_actions"][0]["action"], "phone_tap");
    assert_eq!(rendered["available_actions"][0]["backend"], "companion");
    assert_eq!(
        rendered["unavailable_actions"][0]["reason"],
        "notification_listener_disabled"
    );
    // The exact rotation quarter survives serialization (the coarse label cannot
    // express upside-down portrait), and round-trips back to the same value.
    assert_eq!(rendered["display_rotation_degrees"], 180);
    let parsed: PhoneCapabilityProfile =
        serde_json::from_value(rendered).expect("profile should round-trip");
    assert_eq!(parsed.display_rotation_degrees, Some(180));
}

#[test]
fn companion_absent_helper_reports_uninstalled() {
    let companion = PhoneCompanionCapabilities::absent("com.skycua.phonecompanion");
    assert!(!companion.installed);
    assert!(!companion.gesture_dispatch);
    assert_eq!(companion.package_name, "com.skycua.phonecompanion");
}

#[test]
fn phone_request_idempotency_matches_the_classification_table() {
    let idempotent = [
        PhoneRequest::Observe(PhoneObserveRequest::default()),
        PhoneRequest::Status(PhoneStatusRequest::default()),
        PhoneRequest::ListDevices(PhoneListDevicesRequest::default()),
        PhoneRequest::RefreshCapabilities(PhoneRefreshCapabilitiesRequest::default()),
        PhoneRequest::Connect(PhoneConnectRequest::default()),
        PhoneRequest::Disconnect(PhoneDisconnectRequest::default()),
        PhoneRequest::Screenshot(PhoneScreenshotRequest::default()),
        PhoneRequest::CompanionStatus(PhoneCompanionStatusRequest::default()),
        PhoneRequest::AccessibilityTree(PhoneAccessibilityTreeRequest::default()),
        PhoneRequest::Notifications(PhoneNotificationsRequest::default()),
        PhoneRequest::AppCurrent(PhoneAppCurrentRequest::default()),
        PhoneRequest::AppList(PhoneAppListRequest::default()),
    ];
    for request in idempotent {
        assert!(request.is_idempotent(), "expected idempotent: {request:?}");
    }

    let non_idempotent = [
        PhoneRequest::PairWireless(PhonePairWirelessRequest {
            host_port: "10.0.0.5:5555".to_string(),
            pairing_code: "123456".to_string(),
        }),
        PhoneRequest::Tap(PhoneTapRequest {
            session: PhoneSessionSelector::default(),
            phone_snapshot_id: None,
            x: 10.0,
            y: 10.0,
            use_device_coordinates: false,
        }),
        PhoneRequest::Swipe(PhoneSwipeRequest {
            session: PhoneSessionSelector::default(),
            phone_snapshot_id: None,
            start_x: 0.0,
            start_y: 0.0,
            end_x: 10.0,
            end_y: 10.0,
            duration_ms: None,
            use_device_coordinates: false,
        }),
        PhoneRequest::TypeText(PhoneTypeTextRequest {
            session: PhoneSessionSelector::default(),
            text: "hello".to_string(),
        }),
        PhoneRequest::PressKey(PhonePressKeyRequest {
            session: PhoneSessionSelector::default(),
            key: "KEYCODE_BACK".to_string(),
        }),
        PhoneRequest::InstallCompanion(PhoneInstallCompanionRequest::default()),
        PhoneRequest::NotificationOpen(PhoneNotificationOpenRequest {
            session: PhoneSessionSelector::default(),
            event_id: "evt-1".to_string(),
        }),
        PhoneRequest::NotificationDismiss(PhoneNotificationDismissRequest {
            session: PhoneSessionSelector::default(),
            event_id: "evt-1".to_string(),
        }),
        PhoneRequest::NotificationAction(PhoneNotificationActionRequest {
            session: PhoneSessionSelector::default(),
            event_id: "evt-1".to_string(),
            action_id: "act-1".to_string(),
        }),
        PhoneRequest::NotificationReply(PhoneNotificationReplyRequest {
            session: PhoneSessionSelector::default(),
            event_id: "evt-1".to_string(),
            action_id: "act-1".to_string(),
            text: "ok".to_string(),
        }),
        PhoneRequest::AppLaunch(PhoneAppLaunchRequest {
            session: PhoneSessionSelector::default(),
            package_name: "com.example.app".to_string(),
        }),
        PhoneRequest::AppOpenIntent(PhoneAppOpenIntentRequest {
            session: PhoneSessionSelector::default(),
            intent_uri: "myapp://open".to_string(),
            package_name: None,
        }),
        PhoneRequest::AppForceStop(PhoneAppForceStopRequest {
            session: PhoneSessionSelector::default(),
            package_name: "com.example.app".to_string(),
        }),
        PhoneRequest::AppInstall(PhoneAppInstallRequest {
            session: PhoneSessionSelector::default(),
            apk_paths: vec!["/tmp/app.apk".to_string()],
            mode: PhoneAppInstallMode::Single,
            reinstall: false,
            allow_downgrade: false,
            allow_test_apk: false,
            grant_runtime_permissions: false,
        }),
        PhoneRequest::OpenSettings(PhoneOpenSettingsRequest {
            session: PhoneSessionSelector::default(),
            screen: PhoneSettingsScreen::Accessibility,
            package_name: None,
        }),
    ];
    for request in non_idempotent {
        assert!(
            !request.is_idempotent(),
            "expected non-idempotent: {request:?}"
        );
    }
}

#[test]
fn phone_caller_provenance_values_have_exact_wire_names() {
    for (value, expected) in [
        (PhoneCallerProvenance::CodexDesktop, "codex_desktop"),
        (PhoneCallerProvenance::OpenClaw, "openclaw"),
        (PhoneCallerProvenance::OpenCode, "opencode"),
        (PhoneCallerProvenance::DirectMcp, "direct_mcp"),
    ] {
        let rendered = serde_json::to_value(value).expect("provenance should serialize");
        assert_eq!(rendered, json!(expected));
        assert_eq!(
            serde_json::from_value::<PhoneCallerProvenance>(rendered)
                .expect("provenance should deserialize"),
            value
        );
    }
}
