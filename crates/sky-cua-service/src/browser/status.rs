use sky_cua_platform::model::{
    BrowserIntegrationReport, BrowserStatusReport, BrowserTargetAvailability, BrowserTargetKind,
    DiagnosticEntry,
};

pub(crate) async fn browser_status_from_doctor(
    integration: Option<BrowserIntegrationReport>,
) -> BrowserStatusReport {
    browser_status_from_integration(
        integration,
        DiagnosticEntry {
            code: "BrowserIntegrationUnavailable".to_string(),
            message: "The active backend did not report Chrome-family browser integration checks."
                .to_string(),
            details: None,
        },
    )
    .await
}

pub(crate) async fn browser_status_from_deferred_doctor() -> BrowserStatusReport {
    browser_status_from_integration(
        None,
        DiagnosticEntry {
            code: "BrowserIntegrationDeferred".to_string(),
            message: "Browser integration checks were deferred because another desktop request is active."
                .to_string(),
            details: None,
        },
    )
    .await
}

async fn browser_status_from_integration(
    integration: Option<BrowserIntegrationReport>,
    missing_integration_diagnostic: DiagnosticEntry,
) -> BrowserStatusReport {
    let mut diagnostics = crate::browser::browser_bridge_diagnostics().await;
    let bridge_ready = diagnostics.is_empty();

    let available_targets = match integration.as_ref() {
        Some(integration) => browser_target_availability(integration, bridge_ready),
        None => {
            diagnostics.push(missing_integration_diagnostic);
            vec![BrowserTargetAvailability {
                target: BrowserTargetKind::UserChrome,
                available: bridge_ready,
                detail: if bridge_ready {
                    "Chrome native-host browser bridge is responsive.".to_string()
                } else {
                    "No Chrome native host manifest check was reported.".to_string()
                },
            }]
        }
    };

    BrowserStatusReport {
        enabled: true,
        available_targets,
        tabs_known: None,
        browser_integration: integration,
        diagnostics,
    }
}

fn browser_target_availability(
    integration: &BrowserIntegrationReport,
    bridge_ready: bool,
) -> Vec<BrowserTargetAvailability> {
    let browser_checks = [
        &integration.chrome,
        &integration.chromium,
        &integration.brave,
    ];
    let available_browsers = browser_checks
        .iter()
        .filter(|check| check.ok)
        .map(|check| check.name.as_str())
        .collect::<Vec<_>>();
    let browser_binary_available = !available_browsers.is_empty();
    let user_available =
        bridge_ready || (browser_binary_available && integration.native_host_manifest.ok);

    vec![BrowserTargetAvailability {
        target: BrowserTargetKind::UserChrome,
        available: user_available,
        detail: if bridge_ready {
            "Chrome native-host browser bridge is responsive.".to_string()
        } else if user_available {
            format!(
                "Chrome native host manifest is installed: {}",
                integration.native_host_manifest.detail
            )
        } else if !browser_binary_available {
            "User Chrome automation requires a Chrome-family browser binary.".to_string()
        } else {
            format!(
                "Chrome native host manifest is not ready: {}",
                integration.native_host_manifest.detail
            )
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::browser_target_availability;
    use sky_cua_platform::model::{BrowserIntegrationReport, BrowserTargetKind, DoctorCheck};

    #[test]
    fn browser_status_reports_only_the_user_chrome_target() {
        let availability = browser_target_availability(
            &BrowserIntegrationReport {
                chrome: doctor_check("chrome", false, "missing"),
                chromium: doctor_check("chromium", true, "/usr/bin/chromium"),
                brave: doctor_check("brave", false, "missing"),
                native_host_manifest: doctor_check("native-host", true, "manifest installed"),
            },
            false,
        );

        assert_eq!(availability.len(), 1);
        assert_eq!(availability[0].target, BrowserTargetKind::UserChrome);
        assert!(availability[0].available);
    }

    #[test]
    fn browser_status_user_chrome_available_when_bridge_is_responsive() {
        let availability = browser_target_availability(
            &BrowserIntegrationReport {
                chrome: doctor_check("chrome", false, "missing"),
                chromium: doctor_check("chromium", false, "missing"),
                brave: doctor_check("brave", false, "missing"),
                native_host_manifest: doctor_check("native-host", false, "missing"),
            },
            true,
        );

        let user_chrome = availability
            .iter()
            .find(|target| target.target == BrowserTargetKind::UserChrome)
            .expect("user_chrome target should be reported");
        assert!(user_chrome.available);
        assert!(user_chrome.detail.contains("bridge is responsive"));
    }

    fn doctor_check(name: &str, ok: bool, detail: &str) -> DoctorCheck {
        DoctorCheck {
            name: name.to_string(),
            ok,
            detail: detail.to_string(),
        }
    }
}
