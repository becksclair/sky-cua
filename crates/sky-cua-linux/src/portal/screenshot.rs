use std::path::{Path, PathBuf};

use ashpd::desktop::screenshot::Screenshot;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::PixelSize;

use crate::portal::session::portal_u32_property;

// The model-image preparation core (downscale, encode, format/quality/bounds
// resolution) is shared with the Windows backend via `sky-cua-capture`. This
// module keeps the portal-specific raw capture and crop logic, plus thin
// wrappers that supply the Linux captures directory.
pub use sky_cua_capture::{ModelCaptureImage, ModelScreenshotFormat, pixel_size_from_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub async fn version() -> Result<u32, BackendError> {
    portal_u32_property("org.freedesktop.portal.Screenshot", "version").await
}

pub async fn capture_still(snapshot_id: &str) -> Result<PathBuf, BackendError> {
    let response = Screenshot::request()
        .interactive(false)
        .modal(false)
        .send()
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("failed to request a portal screenshot: {error}"),
            )
        })?
        .response()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("portal screenshot request did not complete successfully: {error}"),
            )
        })?;

    let source_path = file_path_from_uri(response.uri().as_str())?;
    let target_path = capture_output_path(snapshot_id);
    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to create capture directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    tokio::fs::copy(&source_path, &target_path)
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to copy portal screenshot from {} to {}: {error}",
                    source_path.display(),
                    target_path.display()
                ),
            )
        })?;

    Ok(target_path)
}

pub(crate) fn capture_output_path(snapshot_id: &str) -> PathBuf {
    captures_dir().join(format!("{snapshot_id}.png"))
}

pub(crate) fn cropped_capture_output_path(snapshot_id: &str) -> PathBuf {
    captures_dir().join(format!("{snapshot_id}-window.png"))
}

pub(crate) fn crop_capture(
    snapshot_id: &str,
    source_path: &Path,
    crop: PixelRect,
) -> Result<(PathBuf, image::DynamicImage), BackendError> {
    let target_path = cropped_capture_output_path(snapshot_id);
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to create cropped capture directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }

    let image = image::ImageReader::open(source_path)
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to open raw capture {} for window crop: {error}",
                    source_path.display()
                ),
            )
        })?
        .decode()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to decode raw capture {} for window crop: {error}",
                    source_path.display()
                ),
            )
        })?;

    let cropped = image.crop_imm(crop.x, crop.y, crop.width, crop.height);
    cropped.save(&target_path).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "failed to write cropped window capture {}: {error}",
                target_path.display()
            ),
        )
    })?;
    Ok((target_path, cropped))
}

/// Prepare a model image from a raw capture file under the Linux captures dir.
pub fn prepare_model_capture(
    snapshot_id: &str,
    source_path: &Path,
) -> Result<ModelCaptureImage, BackendError> {
    sky_cua_capture::prepare_model_capture(&captures_dir(), snapshot_id, source_path)
}

/// Prepare a model image from an already-decoded capture under the Linux
/// captures dir.
pub(crate) fn prepare_model_capture_from_image(
    snapshot_id: &str,
    image: image::DynamicImage,
    source_path: &Path,
    original_pixel_size: Option<PixelSize>,
) -> Result<ModelCaptureImage, BackendError> {
    sky_cua_capture::prepare_model_capture_from_image(
        &captures_dir(),
        snapshot_id,
        image,
        source_path,
        original_pixel_size,
    )
}

fn captures_dir() -> PathBuf {
    sky_cua_platform::capture_artifacts_dir()
}

fn file_path_from_uri(uri: &str) -> Result<PathBuf, BackendError> {
    let url = url::Url::parse(uri).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("portal screenshot returned an invalid URI {uri:?}: {error}"),
        )
    })?;
    url.to_file_path().map_err(|_| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("portal screenshot URI did not point to a local file: {uri}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::file_path_from_uri;

    #[test]
    fn parses_file_uri_to_path() {
        let path = file_path_from_uri("file:///tmp/demo.png").expect("uri should parse");
        assert_eq!(path, std::path::PathBuf::from("/tmp/demo.png"));
    }
}
