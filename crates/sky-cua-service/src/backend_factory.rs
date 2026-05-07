use sky_cua_platform::backend::DesktopBackend;

#[cfg(target_os = "linux")]
pub fn create_backend() -> Box<dyn DesktopBackend> {
    Box::new(sky_cua_linux::LinuxDesktopBackend::new())
}

#[cfg(target_os = "windows")]
pub fn create_backend() -> Box<dyn DesktopBackend> {
    Box::new(sky_cua_windows::WindowsDesktopBackend::new())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn create_backend() -> Box<dyn DesktopBackend> {
    Box::new(UnsupportedDesktopBackend)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[derive(Debug)]
struct UnsupportedDesktopBackend;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[async_trait::async_trait]
impl DesktopBackend for UnsupportedDesktopBackend {
    async fn probe_environment(
        &self,
    ) -> Result<sky_cua_platform::model::EnvironmentInfo, sky_cua_platform::BackendError> {
        Err(sky_cua_platform::BackendError::new(
            sky_cua_platform::BackendErrorCode::UnsupportedEnvironment,
            "sky-cua has no desktop backend for this target",
        ))
    }

    async fn list_apps(
        &self,
    ) -> Result<Vec<sky_cua_platform::model::AppInfo>, sky_cua_platform::BackendError> {
        Err(sky_cua_platform::BackendError::new(
            sky_cua_platform::BackendErrorCode::UnsupportedEnvironment,
            "sky-cua has no desktop backend for this target",
        ))
    }

    async fn get_app_state(
        &self,
        _selector: Option<sky_cua_platform::model::AppSelector>,
    ) -> Result<sky_cua_platform::model::AppStateSnapshot, sky_cua_platform::BackendError> {
        Err(sky_cua_platform::BackendError::new(
            sky_cua_platform::BackendErrorCode::UnsupportedEnvironment,
            "sky-cua has no desktop backend for this target",
        ))
    }

    async fn execute_action(
        &self,
        _request: sky_cua_platform::model::ActionRequest,
    ) -> Result<sky_cua_platform::model::ActionOutcome, sky_cua_platform::BackendError> {
        Err(sky_cua_platform::BackendError::new(
            sky_cua_platform::BackendErrorCode::UnsupportedEnvironment,
            "sky-cua has no desktop backend for this target",
        ))
    }
}
