use std::process::ExitCode;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    AppSelector, CaptureScreenMode, ServiceRequest, ServiceResponse,
};

use crate::heuristics::HeuristicsRegistry;
use crate::mcp_server::enrich_snapshot;
use crate::output_shapes::{
    AppStateDetail, compact_snapshot, list_apps_error_diagnostic, setup_accessibility_is_error,
    setup_window_targeting_is_error,
};
use crate::service_launcher::ServiceClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliMode {
    Mcp,
    ClearPortalTokens,
    Operator(OperatorCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OperatorCommand {
    Health,
    Doctor,
    SetupAccessibility,
    SetupWindowTargeting,
    ListApps,
    ListWindows,
    FocusedWindow,
    GetAppState(GetAppStateArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GetAppStateArgs {
    selector: Option<AppSelector>,
    detail: AppStateDetail,
    capture_screen: CaptureScreenMode,
}

#[derive(Debug, Clone, PartialEq)]
struct RenderedResponse {
    payload: Value,
    exit_code: ExitCode,
}

impl OperatorCommand {
    pub(crate) fn service_request(&self) -> ServiceRequest {
        match self {
            Self::Health => ServiceRequest::Health,
            Self::Doctor => ServiceRequest::Doctor,
            Self::SetupAccessibility => ServiceRequest::SetupAccessibility,
            Self::SetupWindowTargeting => ServiceRequest::SetupWindowTargeting,
            Self::ListApps => ServiceRequest::ListApps,
            Self::ListWindows => ServiceRequest::ListWindows,
            Self::FocusedWindow => ServiceRequest::FocusedWindow,
            Self::GetAppState(args) => ServiceRequest::GetAppState {
                selector: args.selector.clone(),
                capture_screen: args.capture_screen,
            },
        }
    }
}

pub(crate) fn parse_cli_mode<I>(args: I) -> Result<CliMode>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let Some(mode) = args.next() else {
        return Ok(CliMode::Mcp);
    };
    let rest: Vec<String> = args.collect();
    match mode.as_str() {
        "mcp" => {
            ensure_no_extra_args(&mode, &rest)?;
            Ok(CliMode::Mcp)
        }
        "clear-portal-tokens" => {
            ensure_no_extra_args(&mode, &rest)?;
            Ok(CliMode::ClearPortalTokens)
        }
        "health" => parse_simple_mode(rest, OperatorCommand::Health),
        "doctor" => parse_simple_mode(rest, OperatorCommand::Doctor),
        "setup-accessibility" => parse_simple_mode(rest, OperatorCommand::SetupAccessibility),
        "setup-window-targeting" => {
            parse_simple_mode(rest, OperatorCommand::SetupWindowTargeting)
        }
        "list-apps" => parse_simple_mode(rest, OperatorCommand::ListApps),
        "list-windows" => parse_simple_mode(rest, OperatorCommand::ListWindows),
        "focused-window" => parse_simple_mode(rest, OperatorCommand::FocusedWindow),
        "get-app-state" => Ok(CliMode::Operator(OperatorCommand::GetAppState(
            parse_get_app_state_args(&rest)?,
        ))),
        other => bail!("unsupported sky-cua-client mode: {other}"),
    }
}

pub(crate) fn run_clear_portal_tokens() -> Result<ExitCode> {
    let service = ServiceClient::connect_or_spawn()?;
    match service.clear_portal_tokens()? {
        ServiceResponse::ResetPortalTokens {
            cleared,
            token_path,
            dropped_cached_session,
        } => {
            println!(
                "{}",
                format_clear_portal_tokens(cleared, dropped_cached_session, &token_path)
            );
            Ok(ExitCode::SUCCESS)
        }
        other => bail!("unexpected response for clear-portal-tokens mode: {other:?}"),
    }
}

pub(crate) fn run_operator_command(command: OperatorCommand) -> Result<ExitCode> {
    let service = ServiceClient::connect_or_spawn()?;
    let request = command.service_request();
    let response = service.call(&request)?;
    let heuristics = match (&command, &response) {
        (OperatorCommand::GetAppState(_), ServiceResponse::GetAppState { .. }) => {
            Some(HeuristicsRegistry::load_from_repo()?)
        }
        _ => None,
    };
    let rendered = render_operator_response(&command, response, heuristics.as_ref())?;
    print_json(&rendered.payload)?;
    Ok(rendered.exit_code)
}

pub(crate) fn format_clear_portal_tokens(
    cleared: bool,
    dropped_cached_session: bool,
    token_path: &str,
) -> String {
    format!(
        "cleared={} dropped_cached_session={} token_path={}",
        cleared, dropped_cached_session, token_path
    )
}

fn parse_simple_mode(args: Vec<String>, command: OperatorCommand) -> Result<CliMode> {
    ensure_no_extra_args(command_name(&command), &args)?;
    Ok(CliMode::Operator(command))
}

fn ensure_no_extra_args(mode: &str, args: &[String]) -> Result<()> {
    if let Some(arg) = args.first() {
        bail!("unexpected argument for {mode}: {arg}");
    }
    Ok(())
}

fn command_name(command: &OperatorCommand) -> &'static str {
    match command {
        OperatorCommand::Health => "health",
        OperatorCommand::Doctor => "doctor",
        OperatorCommand::SetupAccessibility => "setup-accessibility",
        OperatorCommand::SetupWindowTargeting => "setup-window-targeting",
        OperatorCommand::ListApps => "list-apps",
        OperatorCommand::ListWindows => "list-windows",
        OperatorCommand::FocusedWindow => "focused-window",
        OperatorCommand::GetAppState(_) => "get-app-state",
    }
}

fn parse_get_app_state_args(args: &[String]) -> Result<GetAppStateArgs> {
    let mut selector = AppSelector::default();
    let mut detail = AppStateDetail::default();
    let mut capture_screen = CaptureScreenMode::default();
    let mut index = 0;

    while index < args.len() {
        let flag = args[index].as_str();
        let value = next_arg_value(args, index, flag)?;
        match flag {
            "--app-id" => selector.app_id = Some(value.to_string()),
            "--desktop-file-id" => selector.desktop_file_id = Some(value.to_string()),
            "--window-title" => selector.window_title = Some(value.to_string()),
            "--name" => selector.name = Some(value.to_string()),
            "--detail" => detail = parse_app_state_detail(value)?,
            "--capture-screen" => capture_screen = parse_capture_screen_mode(value)?,
            other => bail!("unsupported get-app-state flag: {other}"),
        }
        index += 2;
    }

    let selector = if selector == AppSelector::default() {
        None
    } else {
        Some(selector)
    };

    Ok(GetAppStateArgs {
        selector,
        detail,
        capture_screen,
    })
}

fn next_arg_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("missing value for {flag}"))
}

fn parse_app_state_detail(value: &str) -> Result<AppStateDetail> {
    match value {
        "full" => Ok(AppStateDetail::Full),
        "compact" => Ok(AppStateDetail::Compact),
        _ => bail!("unsupported detail value: {value}"),
    }
}

fn parse_capture_screen_mode(value: &str) -> Result<CaptureScreenMode> {
    match value {
        "auto" => Ok(CaptureScreenMode::Auto),
        "if-changed" | "if_changed" => Ok(CaptureScreenMode::IfChanged),
        "always" => Ok(CaptureScreenMode::Always),
        "never" => Ok(CaptureScreenMode::Never),
        _ => bail!("unsupported capture-screen value: {value}"),
    }
}

fn render_operator_response(
    command: &OperatorCommand,
    response: ServiceResponse,
    heuristics: Option<&HeuristicsRegistry>,
) -> Result<RenderedResponse> {
    match (command, response) {
        (
            OperatorCommand::Health,
            ServiceResponse::Health {
                ok,
                service_socket,
                desktop_env,
            },
        ) => Ok(RenderedResponse {
            payload: json!({
                "ok": ok,
                "service_socket": service_socket,
                "desktop_env": desktop_env,
            }),
            exit_code: ExitCode::SUCCESS,
        }),
        (OperatorCommand::Doctor, ServiceResponse::Doctor { report }) => Ok(RenderedResponse {
            payload: serde_json::to_value(report)?,
            exit_code: ExitCode::SUCCESS,
        }),
        (OperatorCommand::SetupAccessibility, ServiceResponse::SetupAccessibility { report }) => {
            Ok(RenderedResponse {
                payload: serde_json::to_value(&report)?,
                exit_code: exit_code(!setup_accessibility_is_error(&report)),
            })
        }
        (
            OperatorCommand::SetupWindowTargeting,
            ServiceResponse::SetupWindowTargeting { report },
        ) => Ok(RenderedResponse {
            payload: serde_json::to_value(&report)?,
            exit_code: exit_code(!setup_window_targeting_is_error(&report)),
        }),
        (
            OperatorCommand::ListApps,
            ServiceResponse::ListApps {
                environment,
                apps,
                diagnostics,
            },
        ) => Ok(RenderedResponse {
            payload: json!({
                "environment": environment,
                "apps": apps,
                "diagnostics": diagnostics,
            }),
            exit_code: exit_code(list_apps_error_diagnostic(&diagnostics).is_none()),
        }),
        (
            OperatorCommand::ListWindows,
            ServiceResponse::ListWindows {
                environment,
                windows,
                diagnostics,
            },
        ) => Ok(RenderedResponse {
            payload: json!({
                "environment": environment,
                "windows": windows,
                "diagnostics": diagnostics,
            }),
            exit_code: exit_code(diagnostics.is_empty()),
        }),
        (
            OperatorCommand::FocusedWindow,
            ServiceResponse::FocusedWindow {
                environment,
                window,
                diagnostics,
            },
        ) => Ok(RenderedResponse {
            payload: json!({
                "environment": environment,
                "window": window,
                "diagnostics": diagnostics,
            }),
            exit_code: exit_code(diagnostics.is_empty()),
        }),
        (OperatorCommand::GetAppState(args), ServiceResponse::GetAppState { mut snapshot }) => {
            if let Some(heuristics) = heuristics {
                enrich_snapshot(heuristics, &mut snapshot);
            }
            let payload = match args.detail {
                AppStateDetail::Full => serde_json::to_value(&snapshot)?,
                AppStateDetail::Compact => compact_snapshot(&snapshot),
            };
            Ok(RenderedResponse {
                payload,
                exit_code: ExitCode::SUCCESS,
            })
        }
        (_, ServiceResponse::Error { code, message }) => Ok(RenderedResponse {
            payload: json!({
                "code": code,
                "message": message,
            }),
            exit_code: ExitCode::from(1),
        }),
        (other_command, other_response) => bail!(
            "unexpected response for {}: {other_response:?}",
            command_name(other_command)
        ),
    }
}

fn exit_code(success: bool) -> ExitCode {
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn print_json(value: &Value) -> Result<()> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    use std::io::Write;
    writeln!(&mut stdout)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use sky_cua_platform::model::{
        AccessibilitySetupReport, CaptureBackendKind, DiagnosticEntry, DoctorCheck,
        DoctorReadiness, DoctorReport, EnvironmentInfo, FocusedApp, InputBackendKind,
        PortalCapabilities, SemanticBackendKind, SessionKind, SetupCommandReport,
        ToolAvailability, ToolCapabilities, WindowTargetingSetupReport,
    };

    use super::*;

    #[test]
    fn defaults_to_mcp_mode() {
        assert_eq!(parse_cli_mode(Vec::<String>::new()).unwrap(), CliMode::Mcp);
    }

    #[test]
    fn parses_get_app_state_defaults() {
        let mode = parse_cli_mode(["get-app-state"].into_iter().map(str::to_string)).unwrap();
        assert_eq!(
            mode,
            CliMode::Operator(OperatorCommand::GetAppState(GetAppStateArgs {
                selector: None,
                detail: AppStateDetail::Full,
                capture_screen: CaptureScreenMode::IfChanged,
            }))
        );
    }

    #[test]
    fn parses_get_app_state_flags() {
        let mode = parse_cli_mode(
            [
                "get-app-state",
                "--app-id",
                "org.kde.kate",
                "--detail",
                "compact",
                "--capture-screen",
                "always",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(
            mode,
            CliMode::Operator(OperatorCommand::GetAppState(GetAppStateArgs {
                selector: Some(AppSelector {
                    app_id: Some("org.kde.kate".to_string()),
                    desktop_file_id: None,
                    window_title: None,
                    name: None,
                }),
                detail: AppStateDetail::Compact,
                capture_screen: CaptureScreenMode::Always,
            }))
        );
    }

    #[test]
    fn rejects_unsupported_mode() {
        let error = parse_cli_mode(["wat"].into_iter().map(str::to_string)).unwrap_err();
        assert!(error.to_string().contains("unsupported sky-cua-client mode"));
    }

    #[test]
    fn rejects_missing_flag_value() {
        let error = parse_cli_mode(
            ["get-app-state", "--app-id"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap_err();
        assert!(error.to_string().contains("missing value for --app-id"));
    }

    #[test]
    fn rejects_extra_args_for_simple_mode() {
        let error = parse_cli_mode(["health", "extra"].into_iter().map(str::to_string)).unwrap_err();
        assert!(error.to_string().contains("unexpected argument for health"));
    }

    #[test]
    fn maps_each_operator_command_to_service_request() {
        let cases = vec![
            (OperatorCommand::Health, ServiceRequest::Health),
            (OperatorCommand::Doctor, ServiceRequest::Doctor),
            (
                OperatorCommand::SetupAccessibility,
                ServiceRequest::SetupAccessibility,
            ),
            (
                OperatorCommand::SetupWindowTargeting,
                ServiceRequest::SetupWindowTargeting,
            ),
            (OperatorCommand::ListApps, ServiceRequest::ListApps),
            (OperatorCommand::ListWindows, ServiceRequest::ListWindows),
            (OperatorCommand::FocusedWindow, ServiceRequest::FocusedWindow),
            (
                OperatorCommand::GetAppState(GetAppStateArgs {
                    selector: Some(AppSelector {
                        app_id: Some("app".to_string()),
                        desktop_file_id: None,
                        window_title: None,
                        name: None,
                    }),
                    detail: AppStateDetail::Compact,
                    capture_screen: CaptureScreenMode::Never,
                }),
                ServiceRequest::GetAppState {
                    selector: Some(AppSelector {
                        app_id: Some("app".to_string()),
                        desktop_file_id: None,
                        window_title: None,
                        name: None,
                    }),
                    capture_screen: CaptureScreenMode::Never,
                },
            ),
        ];

        for (command, expected) in cases {
            assert_eq!(command.service_request(), expected);
        }
    }

    #[test]
    fn full_get_app_state_returns_full_snapshot_json() {
        let snapshot = sample_snapshot();
        let expected = serde_json::to_value(&snapshot).unwrap();
        let rendered = render_operator_response(
            &OperatorCommand::GetAppState(GetAppStateArgs {
                selector: None,
                detail: AppStateDetail::Full,
                capture_screen: CaptureScreenMode::IfChanged,
            }),
            ServiceResponse::GetAppState {
                snapshot: Box::new(snapshot),
            },
            None,
        )
        .unwrap();

        assert_eq!(rendered.payload, expected);
        assert_eq!(rendered.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    fn compact_get_app_state_matches_shared_compact_shape() {
        let snapshot = sample_snapshot();
        let expected = compact_snapshot(&snapshot);
        let rendered = render_operator_response(
            &OperatorCommand::GetAppState(GetAppStateArgs {
                selector: None,
                detail: AppStateDetail::Compact,
                capture_screen: CaptureScreenMode::IfChanged,
            }),
            ServiceResponse::GetAppState {
                snapshot: Box::new(snapshot),
            },
            None,
        )
        .unwrap();

        assert_eq!(rendered.payload, expected);
        assert_eq!(rendered.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    fn capture_screen_flag_maps_correctly() {
        let cases = vec![
            ("auto", CaptureScreenMode::Auto),
            ("if-changed", CaptureScreenMode::IfChanged),
            ("always", CaptureScreenMode::Always),
            ("never", CaptureScreenMode::Never),
        ];

        for (value, expected) in cases {
            let mode = parse_capture_screen_mode(value).unwrap();
            assert_eq!(mode, expected);
        }
    }

    #[test]
    fn setup_accessibility_failure_exits_non_zero() {
        let rendered = render_operator_response(
            &OperatorCommand::SetupAccessibility,
            ServiceResponse::SetupAccessibility {
                report: Box::new(AccessibilitySetupReport {
                    before: Box::new(sample_doctor_report(true)),
                    accessibility_command: SetupCommandReport {
                        ok: false,
                        detail: "failed".to_string(),
                    },
                    after: Box::new(sample_doctor_report(false)),
                    changed: false,
                    requires_restart: true,
                }),
            },
            None,
        )
        .unwrap();

        assert_eq!(rendered.exit_code, ExitCode::from(1));
    }

    #[test]
    fn setup_window_targeting_failure_exits_non_zero() {
        let rendered = render_operator_response(
            &OperatorCommand::SetupWindowTargeting,
            ServiceResponse::SetupWindowTargeting {
                report: Box::new(WindowTargetingSetupReport {
                    extension_dir: "/tmp/ext".to_string(),
                    wrote_files: true,
                    enable_command: SetupCommandReport {
                        ok: true,
                        detail: "enabled".to_string(),
                    },
                    windows: vec![],
                    windows_error: Some("extension unavailable".to_string()),
                    requires_shell_reload: false,
                    message: "extension unavailable".to_string(),
                    permissions_hint: None,
                }),
            },
            None,
        )
        .unwrap();

        assert_eq!(rendered.exit_code, ExitCode::from(1));
    }

    #[test]
    fn degraded_list_apps_exits_non_zero() {
        let rendered = render_operator_response(
            &OperatorCommand::ListApps,
            ServiceResponse::ListApps {
                environment: sample_environment(),
                apps: vec![],
                diagnostics: vec![DiagnosticEntry {
                    code: "WindowBackendUnavailable".to_string(),
                    message: "window backend unavailable".to_string(),
                    details: None,
                }],
            },
            None,
        )
        .unwrap();

        assert_eq!(rendered.exit_code, ExitCode::from(1));
    }

    #[test]
    fn degraded_list_windows_exits_non_zero() {
        let rendered = render_operator_response(
            &OperatorCommand::ListWindows,
            ServiceResponse::ListWindows {
                environment: sample_environment(),
                windows: vec![],
                diagnostics: vec![DiagnosticEntry {
                    code: "WindowBackendUnavailable".to_string(),
                    message: "window backend unavailable".to_string(),
                    details: None,
                }],
            },
            None,
        )
        .unwrap();

        assert_eq!(rendered.exit_code, ExitCode::from(1));
    }

    #[test]
    fn degraded_focused_window_exits_non_zero() {
        let rendered = render_operator_response(
            &OperatorCommand::FocusedWindow,
            ServiceResponse::FocusedWindow {
                environment: sample_environment(),
                window: None,
                diagnostics: vec![DiagnosticEntry {
                    code: "WindowBackendUnavailable".to_string(),
                    message: "window backend unavailable".to_string(),
                    details: None,
                }],
            },
            None,
        )
        .unwrap();

        assert_eq!(rendered.exit_code, ExitCode::from(1));
    }

    #[test]
    fn error_response_exits_non_zero_and_prints_json_payload() {
        let rendered = render_operator_response(
            &OperatorCommand::Doctor,
            ServiceResponse::Error {
                code: "Boom".to_string(),
                message: "service failed".to_string(),
            },
            None,
        )
        .unwrap();

        assert_eq!(
            rendered.payload,
            json!({
                "code": "Boom",
                "message": "service failed",
            })
        );
        assert_eq!(rendered.exit_code, ExitCode::from(1));
    }

    #[test]
    fn mcp_and_clear_portal_modes_still_parse() {
        assert_eq!(
            parse_cli_mode(["mcp"].into_iter().map(str::to_string)).unwrap(),
            CliMode::Mcp
        );
        assert_eq!(
            parse_cli_mode(["clear-portal-tokens"].into_iter().map(str::to_string)).unwrap(),
            CliMode::ClearPortalTokens
        );
    }

    #[test]
    fn clear_portal_tokens_output_shape_is_unchanged() {
        assert_eq!(
            format_clear_portal_tokens(true, false, "/tmp/tokens.json"),
            "cleared=true dropped_cached_session=false token_path=/tmp/tokens.json"
        );
    }

    #[test]
    fn get_app_state_service_error_does_not_require_heuristics_loading() {
        let rendered = render_operator_response(
            &OperatorCommand::GetAppState(GetAppStateArgs {
                selector: None,
                detail: AppStateDetail::Full,
                capture_screen: CaptureScreenMode::IfChanged,
            }),
            ServiceResponse::Error {
                code: "ServiceUnavailable".to_string(),
                message: "backend not ready".to_string(),
            },
            None,
        )
        .unwrap();

        assert_eq!(
            rendered.payload,
            json!({
                "code": "ServiceUnavailable",
                "message": "backend not ready",
            })
        );
        assert_eq!(rendered.exit_code, ExitCode::from(1));
    }

    fn sample_snapshot() -> sky_cua_platform::model::AppStateSnapshot {
        sky_cua_platform::model::AppStateSnapshot {
            snapshot_id: "snap-1".to_string(),
            created_at: Utc::now(),
            environment: sample_environment(),
            capabilities: sample_capabilities(),
            focused_app: Some(FocusedApp {
                app_id: "org.kde.kate".to_string(),
                name: "Kate".to_string(),
                pid: Some(100),
                desktop_file_id: Some("org.kde.kate.desktop".to_string()),
                app_user_model_id: None,
                window_handle: None,
                toolkit_guess: Some("Qt".to_string()),
                window_title: Some("notes.txt".to_string()),
            }),
            capture: None,
            elements: vec![],
            diagnostics: vec![],
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        }
    }

    fn sample_capabilities() -> ToolCapabilities {
        fn available() -> ToolAvailability {
            ToolAvailability {
                available: true,
                reason: None,
            }
        }

        ToolCapabilities {
            list_apps: available(),
            get_app_state: available(),
            focus_element: available(),
            activate_element: available(),
            select_element: available(),
            expand_element: available(),
            collapse_element: available(),
            toggle_element: available(),
            click: available(),
            perform_action: available(),
            perform_secondary_action: available(),
            scroll: available(),
            drag: available(),
            type_text: available(),
            press_key: available(),
            set_value: available(),
        }
    }

    fn sample_environment() -> EnvironmentInfo {
        EnvironmentInfo {
            session_kind: SessionKind::Wayland,
            compositor: Some("kde-kwin-wayland".to_string()),
            desktop_environment: Some("KDE".to_string()),
            capture_backend: CaptureBackendKind::PortalPipeWire,
            input_backend: InputBackendKind::PortalRemoteDesktop,
            semantic_backend: SemanticBackendKind::Atspi,
            portal_capabilities: PortalCapabilities {
                screencast_version: Some(5),
                remote_desktop_version: Some(2),
                screenshot_version: Some(2),
                available_source_types: None,
                available_cursor_modes: None,
                available_device_types: None,
            },
            xdg_session_type: Some("wayland".to_string()),
            display: None,
            wayland_display: Some("wayland-0".to_string()),
        }
    }

    fn sample_doctor_report(can_build_accessibility_tree: bool) -> DoctorReport {
        DoctorReport {
            environment: sample_environment(),
            checks: vec![DoctorCheck {
                name: "atspi".to_string(),
                ok: can_build_accessibility_tree,
                detail: "check".to_string(),
            }],
            readiness: DoctorReadiness {
                can_register_mcp_tools: true,
                can_build_accessibility_tree,
                can_capture_screen: true,
                can_send_input: true,
                can_list_windows: true,
                can_target_windows: true,
                recommended_next_step: "ready".to_string(),
                blockers: vec![],
            },
            platform: None,
            portal: None,
            accessibility: None,
            windowing: None,
            input: None,
            browser_integration: None,
        }
    }
}
