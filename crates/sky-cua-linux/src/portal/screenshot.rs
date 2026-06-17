use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ashpd::desktop::screenshot::Screenshot;
use image::ImageEncoder;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::PixelSize;

use crate::portal::session::portal_u32_property;

const MODEL_SCREENSHOT_MAX_WIDTH: u32 = 1440;
const MODEL_SCREENSHOT_MAX_HEIGHT: u32 = 900;
const MODEL_SCREENSHOT_JPEG_QUALITY: u8 = 85;
const MODEL_SCREENSHOT_WEBP_QUALITY: u8 = 85;
const MODEL_SCREENSHOT_MIN_BOUND: u32 = 64;
const MODEL_SCREENSHOT_MAX_BOUND: u32 = 4096;
const MODEL_SCREENSHOT_MAX_WIDTH_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_MAX_WIDTH";
const MODEL_SCREENSHOT_MAX_HEIGHT_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_MAX_HEIGHT";
const MODEL_SCREENSHOT_FORMAT_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_FORMAT";
const MODEL_SCREENSHOT_JPEG_QUALITY_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY";
const MODEL_SCREENSHOT_WEBP_QUALITY_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCaptureImage {
    pub path: PathBuf,
    pub pixel_size: Option<PixelSize>,
    pub original_path: Option<PathBuf>,
    pub original_pixel_size: Option<PixelSize>,
    pub format: ModelScreenshotFormat,
    pub quality: u8,
    pub bytes: Option<u64>,
    pub encode_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelScreenshotFormat {
    Jpeg,
    Webp,
}

impl ModelScreenshotFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
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

pub(crate) fn cropped_capture_output_path(snapshot_id: &str) -> PathBuf {
    runtime_root()
        .join("captures")
        .join(format!("{snapshot_id}-window.png"))
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

pub(crate) fn model_capture_output_path_for_format(
    snapshot_id: &str,
    format: ModelScreenshotFormat,
) -> PathBuf {
    runtime_root()
        .join("captures")
        .join(format!("{snapshot_id}.{}", format.extension()))
}

pub fn prepare_model_capture(
    snapshot_id: &str,
    source_path: &Path,
) -> Result<ModelCaptureImage, BackendError> {
    prepare_model_capture_with_format(
        snapshot_id,
        source_path,
        model_screenshot_format(),
        model_screenshot_bounds(),
        None,
        None,
        None,
    )
}

pub(crate) fn prepare_model_capture_from_image(
    snapshot_id: &str,
    image: image::DynamicImage,
    source_path: &Path,
    original_pixel_size: Option<PixelSize>,
) -> Result<ModelCaptureImage, BackendError> {
    prepare_model_capture_with_format(
        snapshot_id,
        source_path,
        model_screenshot_format(),
        model_screenshot_bounds(),
        None,
        Some(image),
        original_pixel_size,
    )
}

fn prepare_model_capture_with_format(
    snapshot_id: &str,
    source_path: &Path,
    format: ModelScreenshotFormat,
    bounds: (u32, u32),
    quality_override: Option<u8>,
    image: Option<image::DynamicImage>,
    original_pixel_size_override: Option<PixelSize>,
) -> Result<ModelCaptureImage, BackendError> {
    let original_pixel_size =
        original_pixel_size_override.or_else(|| pixel_size_from_path(source_path));
    let quality = quality_override.unwrap_or_else(|| model_screenshot_quality(format));
    let target_path = model_capture_output_path_for_format(snapshot_id, format);
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

    let image = match image {
        Some(image) => image,
        None => image::ImageReader::open(source_path)
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
            })?,
    };

    let resized = image.resize(bounds.0, bounds.1, image::imageops::FilterType::Lanczos3);
    let rgb = resized.to_rgb8();
    let encode_started = Instant::now();
    let file = std::fs::File::create(&target_path).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "failed to create model screenshot {}: {error}",
                target_path.display()
            ),
        )
    })?;
    match format {
        ModelScreenshotFormat::Jpeg => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
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
        }
        ModelScreenshotFormat::Webp => {
            let encoder = webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height());
            let encoded = encoder.encode(f32::from(quality));
            let mut writer = std::io::BufWriter::new(file);
            writer.write_all(encoded.as_ref()).map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!(
                        "failed to write model WebP screenshot {}: {error}",
                        target_path.display()
                    ),
                )
            })?;
            writer.flush().map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!(
                        "failed to flush model WebP screenshot {}: {error}",
                        target_path.display()
                    ),
                )
            })?;
        }
    }
    let encode_ms = encode_started
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);
    let bytes = std::fs::metadata(&target_path)
        .map(|metadata| metadata.len())
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to stat model screenshot {} after encode: {error}",
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
        format,
        quality,
        bytes: Some(bytes),
        encode_ms,
    })
}

fn model_screenshot_bounds() -> (u32, u32) {
    (
        model_screenshot_bound_from_env(MODEL_SCREENSHOT_MAX_WIDTH_ENV, MODEL_SCREENSHOT_MAX_WIDTH),
        model_screenshot_bound_from_env(
            MODEL_SCREENSHOT_MAX_HEIGHT_ENV,
            MODEL_SCREENSHOT_MAX_HEIGHT,
        ),
    )
}

fn model_screenshot_bound_from_env(name: &str, default: u32) -> u32 {
    model_screenshot_bound_from_value(std::env::var(name).ok().as_deref(), default)
}

fn model_screenshot_bound_from_value(value: Option<&str>, default: u32) -> u32 {
    value
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| (MODEL_SCREENSHOT_MIN_BOUND..=MODEL_SCREENSHOT_MAX_BOUND).contains(value))
        .unwrap_or(default)
}

fn model_screenshot_format() -> ModelScreenshotFormat {
    model_screenshot_format_from_value(std::env::var(MODEL_SCREENSHOT_FORMAT_ENV).ok().as_deref())
}

fn model_screenshot_format_from_value(value: Option<&str>) -> ModelScreenshotFormat {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("webp") => ModelScreenshotFormat::Webp,
        Some("jpg" | "jpeg") | None => ModelScreenshotFormat::Jpeg,
        Some(_) => ModelScreenshotFormat::Jpeg,
    }
}

fn model_screenshot_quality(format: ModelScreenshotFormat) -> u8 {
    let (name, default) = match format {
        ModelScreenshotFormat::Jpeg => (
            MODEL_SCREENSHOT_JPEG_QUALITY_ENV,
            MODEL_SCREENSHOT_JPEG_QUALITY,
        ),
        ModelScreenshotFormat::Webp => (
            MODEL_SCREENSHOT_WEBP_QUALITY_ENV,
            MODEL_SCREENSHOT_WEBP_QUALITY,
        ),
    };
    model_screenshot_quality_from_value(std::env::var(name).ok().as_deref(), default)
}

fn model_screenshot_quality_from_value(value: Option<&str>, default: u8) -> u8 {
    value
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| (1..=100).contains(value))
        .unwrap_or(default)
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
    use super::{
        MODEL_SCREENSHOT_MAX_HEIGHT, MODEL_SCREENSHOT_MAX_WIDTH, ModelScreenshotFormat,
        file_path_from_uri, model_screenshot_bound_from_env, model_screenshot_bound_from_value,
        model_screenshot_format_from_value, model_screenshot_quality_from_value,
        prepare_model_capture, prepare_model_capture_with_format,
    };

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
        assert_eq!(pixel_size.width, 1440);
        assert_eq!(pixel_size.height, 810);
        assert_eq!(
            prepared.original_pixel_size.expect("source size").width,
            2560
        );
        assert_eq!(
            prepared.path.extension().and_then(|ext| ext.to_str()),
            Some("jpg")
        );
        assert_eq!(prepared.format, ModelScreenshotFormat::Jpeg);
        assert_eq!(prepared.quality, 85);
        assert!(prepared.bytes.is_some_and(|bytes| bytes > 0));

        let _ = std::fs::remove_file(prepared.path);
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_dir(temp_dir);
    }

    #[test]
    fn prepares_model_capture_as_bounded_webp() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-model-capture-webp-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let source_path = temp_dir.join("source.png");
        let image = image::RgbImage::from_pixel(2560, 1440, image::Rgb([32, 64, 96]));
        image
            .save(&source_path)
            .expect("source image should be saved");

        let prepared = prepare_model_capture_with_format(
            "bounded-webp-test",
            &source_path,
            ModelScreenshotFormat::Webp,
            (1440, 900),
            Some(80),
            None,
            None,
        )
        .expect("capture should scale");
        let pixel_size = prepared.pixel_size.expect("pixel size should be known");
        assert_eq!(pixel_size.width, 1440);
        assert_eq!(pixel_size.height, 810);
        assert_eq!(
            prepared.path.extension().and_then(|ext| ext.to_str()),
            Some("webp")
        );
        assert_eq!(prepared.format, ModelScreenshotFormat::Webp);
        assert_eq!(prepared.quality, 80);
        assert!(prepared.bytes.is_some_and(|bytes| bytes > 0));

        let _ = std::fs::remove_file(prepared.path);
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_dir(temp_dir);
    }

    #[test]
    fn model_screenshot_bound_env_parser_rejects_unsafe_values() {
        assert_eq!(
            model_screenshot_bound_from_env("__SKY_CUA_UNSET__", 1440),
            1440
        );
        assert_eq!(model_screenshot_bound_from_value(Some("1280"), 1440), 1280);
        assert_eq!(model_screenshot_bound_from_value(Some("63"), 1440), 1440);
        assert_eq!(model_screenshot_bound_from_value(Some("4097"), 1440), 1440);
        assert_eq!(model_screenshot_bound_from_value(Some("nope"), 1440), 1440);
        assert_eq!(
            model_screenshot_bound_from_env("__SKY_CUA_UNSET__", MODEL_SCREENSHOT_MAX_WIDTH),
            MODEL_SCREENSHOT_MAX_WIDTH
        );
        assert_eq!(
            model_screenshot_bound_from_env("__SKY_CUA_UNSET__", MODEL_SCREENSHOT_MAX_HEIGHT),
            MODEL_SCREENSHOT_MAX_HEIGHT
        );
    }

    #[test]
    fn model_screenshot_format_parser_defaults_to_jpeg() {
        assert_eq!(
            model_screenshot_format_from_value(None),
            ModelScreenshotFormat::Jpeg
        );
        assert_eq!(
            model_screenshot_format_from_value(Some("jpeg")),
            ModelScreenshotFormat::Jpeg
        );
        assert_eq!(
            model_screenshot_format_from_value(Some("jpg")),
            ModelScreenshotFormat::Jpeg
        );
        assert_eq!(
            model_screenshot_format_from_value(Some("webp")),
            ModelScreenshotFormat::Webp
        );
        assert_eq!(
            model_screenshot_format_from_value(Some("nope")),
            ModelScreenshotFormat::Jpeg
        );
    }

    #[test]
    fn model_screenshot_quality_parser_rejects_unsafe_values() {
        assert_eq!(model_screenshot_quality_from_value(None, 85), 85);
        assert_eq!(model_screenshot_quality_from_value(Some("70"), 85), 70);
        assert_eq!(model_screenshot_quality_from_value(Some("0"), 85), 85);
        assert_eq!(model_screenshot_quality_from_value(Some("101"), 85), 85);
        assert_eq!(model_screenshot_quality_from_value(Some("nope"), 85), 85);
    }
}
