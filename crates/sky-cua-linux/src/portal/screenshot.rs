use std::path::{Path, PathBuf};

use ashpd::desktop::screenshot::Screenshot;
use image::ImageEncoder;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::PixelSize;

use crate::portal::session::portal_u32_property;

const MODEL_SCREENSHOT_MAX_WIDTH: u32 = 1920;
const MODEL_SCREENSHOT_MAX_HEIGHT: u32 = 1080;
const MODEL_SCREENSHOT_JPEG_QUALITY: u8 = 85;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCaptureImage {
    pub path: PathBuf,
    pub pixel_size: Option<PixelSize>,
    pub original_path: Option<PathBuf>,
    pub original_pixel_size: Option<PixelSize>,
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
    runtime_root()
        .join("captures")
        .join(format!("{snapshot_id}.png"))
}

pub(crate) fn model_capture_output_path(snapshot_id: &str) -> PathBuf {
    runtime_root()
        .join("captures")
        .join(format!("{snapshot_id}.jpg"))
}

pub fn prepare_model_capture(
    snapshot_id: &str,
    source_path: &Path,
) -> Result<ModelCaptureImage, BackendError> {
    let original_pixel_size = pixel_size_from_path(source_path);
    let target_path = model_capture_output_path(snapshot_id);
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to create model capture directory {}: {error}",
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
                    "failed to open raw capture {} for model image preparation: {error}",
                    source_path.display()
                ),
            )
        })?
        .decode()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to decode raw capture {} for model image preparation: {error}",
                    source_path.display()
                ),
            )
        })?;

    let resized = image.resize(
        MODEL_SCREENSHOT_MAX_WIDTH,
        MODEL_SCREENSHOT_MAX_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let rgb = resized.to_rgb8();
    let file = std::fs::File::create(&target_path).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "failed to create model screenshot {}: {error}",
                target_path.display()
            ),
        )
    })?;
    let encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(file, MODEL_SCREENSHOT_JPEG_QUALITY);
    encoder
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to write model JPEG screenshot {}: {error}",
                    target_path.display()
                ),
            )
        })?;

    Ok(ModelCaptureImage {
        pixel_size: Some(PixelSize {
            width: rgb.width(),
            height: rgb.height(),
        }),
        path: target_path,
        original_path: Some(source_path.to_path_buf()),
        original_pixel_size,
    })
}

fn runtime_root() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("sky-cua")
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

pub fn pixel_size_from_path(path: &Path) -> Option<PixelSize> {
    let (width, height) = image::image_dimensions(path).ok()?;
    Some(PixelSize { width, height })
}

#[cfg(test)]
mod tests {
    use super::{file_path_from_uri, prepare_model_capture};

    #[test]
    fn parses_file_uri_to_path() {
        let path = file_path_from_uri("file:///tmp/demo.png").expect("uri should parse");
        assert_eq!(path, std::path::PathBuf::from("/tmp/demo.png"));
    }

    #[test]
    fn prepares_model_capture_as_bounded_jpeg() {
        let temp_dir =
            std::env::temp_dir().join(format!("sky-cua-model-capture-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let source_path = temp_dir.join("source.png");
        let image = image::RgbImage::from_pixel(2560, 1440, image::Rgb([32, 64, 96]));
        image
            .save(&source_path)
            .expect("source image should be saved");

        let prepared =
            prepare_model_capture("bounded-jpeg-test", &source_path).expect("capture should scale");
        let pixel_size = prepared.pixel_size.expect("pixel size should be known");
        assert_eq!(pixel_size.width, 1920);
        assert_eq!(pixel_size.height, 1080);
        assert_eq!(
            prepared.original_pixel_size.expect("source size").width,
            2560
        );
        assert_eq!(
            prepared.path.extension().and_then(|ext| ext.to_str()),
            Some("jpg")
        );

        let _ = std::fs::remove_file(prepared.path);
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_dir(temp_dir);
    }
}
