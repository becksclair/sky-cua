use sky_cua_platform::model::{CoordinateSpace, RectF};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CompatBounds {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
}

pub fn rect_from_i32(x: Option<i32>, y: Option<i32>, width: u32, height: u32) -> RectF {
    RectF {
        x: x.unwrap_or_default() as f64,
        y: y.unwrap_or_default() as f64,
        width: width as f64,
        height: height as f64,
        space: CoordinateSpace::DesktopLogical,
    }
}

impl From<CompatBounds> for RectF {
    fn from(bounds: CompatBounds) -> Self {
        rect_from_i32(bounds.x, bounds.y, bounds.width, bounds.height)
    }
}

pub fn backend_error(message: impl Into<String>) -> sky_cua_platform::diagnostics::BackendError {
    sky_cua_platform::diagnostics::BackendError::new(
        sky_cua_platform::diagnostics::BackendErrorCode::ActionUnsupportedForEnvironment,
        message,
    )
}

pub fn output_detail(stdout: &[u8], stderr: &[u8], fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    fallback.to_string()
}

pub fn command_exists(binary: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|entry| {
            let candidate = entry.join(binary);
            candidate.is_file() && is_executable(&candidate)
        })
    })
}

pub fn is_executable(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                metadata.is_file()
            }
        })
        .unwrap_or(false)
}

pub fn normalize_window_id(value: &serde_json::Value) -> Option<String> {
    value.as_u64().map(|id| id.to_string()).or_else(|| {
        value
            .as_str()
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    })
}
