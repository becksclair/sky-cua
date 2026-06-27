//! Shared model-facing screenshot preparation for the desktop backends.
//!
//! Both the Linux portal backend and the Windows GDI backend acquire raw
//! captures in platform-specific ways, but the bounded, re-encoded image handed
//! to the model must be identical: downscaled (never upscaled) to fit the model
//! bounds, encoded as WebP by default, and described with honest pixel and
//! encoding metadata so action coordinates can be mapped back to the underlying
//! display. That platform-agnostic core lives here so neither backend drifts.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::ImageEncoder;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{CaptureInfo, PixelSize};

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

/// Resampling filter for shrinking oversized captures. Lanczos3 is the sharpest
/// filter the `image` crate provides; it preserves edge and text detail on
/// downscaled UI better than Triangle, CatmullRom, or Gaussian. It is only ever
/// applied to downscale — captures already within the model bounds pass through
/// untouched (see `prepare_model_capture_with_format`).
const MODEL_SCREENSHOT_FILTER: image::imageops::FilterType = image::imageops::FilterType::Lanczos3;

/// Metadata describing a prepared model-facing capture image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCaptureImage {
    /// Path to the bounded, re-encoded model image.
    pub path: PathBuf,
    /// Pixel dimensions of the bounded model image.
    pub pixel_size: Option<PixelSize>,
    /// Path to the raw capture the model image was derived from, when retained.
    pub original_path: Option<PathBuf>,
    /// Pixel dimensions of the raw capture.
    pub original_pixel_size: Option<PixelSize>,
    /// Encoding container the model image was written with.
    pub format: ModelScreenshotFormat,
    /// Encoder quality the model image was written with.
    pub quality: u8,
    /// On-disk size of the model image, when stat succeeds.
    pub bytes: Option<u64>,
    /// Wall-clock encode time in milliseconds.
    pub encode_ms: u64,
}

/// Encoding container for the model-facing capture image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelScreenshotFormat {
    Jpeg,
    Webp,
}

impl ModelScreenshotFormat {
    /// File extension for this format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    /// Canonical lowercase name for this format.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

/// Resolve the on-disk path for a model image of the given format under
/// `captures_dir`.
pub fn model_capture_output_path_for_format(
    captures_dir: &Path,
    snapshot_id: &str,
    format: ModelScreenshotFormat,
) -> PathBuf {
    captures_dir.join(format!("{snapshot_id}.{}", format.extension()))
}

/// Prepare a model image from a raw capture file, using the environment-resolved
/// format, bounds, and quality.
pub fn prepare_model_capture(
    captures_dir: &Path,
    snapshot_id: &str,
    source_path: &Path,
) -> Result<ModelCaptureImage, BackendError> {
    prepare_model_capture_with_format(
        captures_dir,
        snapshot_id,
        source_path,
        model_screenshot_format(),
        model_screenshot_bounds(),
        None,
        None,
        None,
    )
}

/// Prepare a model image from an already-decoded capture, using the
/// environment-resolved format, bounds, and quality.
pub fn prepare_model_capture_from_image(
    captures_dir: &Path,
    snapshot_id: &str,
    image: image::DynamicImage,
    source_path: &Path,
    original_pixel_size: Option<PixelSize>,
) -> Result<ModelCaptureImage, BackendError> {
    prepare_model_capture_with_format(
        captures_dir,
        snapshot_id,
        source_path,
        model_screenshot_format(),
        model_screenshot_bounds(),
        None,
        Some(image),
        original_pixel_size,
    )
}

/// Prepare a model image with explicit format, bounds, and optional quality.
///
/// The image is only ever downscaled to fit `bounds`; a capture already within
/// the bounds is written at native size rather than upscaled. The result is
/// re-encoded into `format` and described with honest pixel/byte/timing metadata.
///
/// This is the flexible core behind the thin `prepare_model_capture*` wrappers;
/// the argument count reflects the knobs those wrappers default, not a missing
/// abstraction.
#[allow(clippy::too_many_arguments)]
pub fn prepare_model_capture_with_format(
    captures_dir: &Path,
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
    let target_path = model_capture_output_path_for_format(captures_dir, snapshot_id, format);
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

    // Only ever downscale. A capture that already fits within the model bounds is
    // handed through untouched: upscaling would invent detail and inflate the
    // payload for no fidelity gain, and resampling at 1:1 would just soften it.
    let resized = if image.width() <= bounds.0 && image.height() <= bounds.1 {
        image
    } else {
        image.resize(bounds.0, bounds.1, MODEL_SCREENSHOT_FILTER)
    };
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

/// Derive `logical_to_pixel_scale` from the model image and logical rect on a
/// capture, so action coordinates expressed in model-image pixels map back to the
/// underlying display. Both backends call this after populating `pixel_size` and
/// `logical_rect` so the downscale stays coordinate-safe.
pub fn update_model_capture_scale(capture_info: &mut CaptureInfo) {
    capture_info.logical_to_pixel_scale = None;
    if let (Some(pixel_size), Some(logical_rect)) = (
        capture_info.pixel_size.as_ref(),
        capture_info.logical_rect.as_ref(),
    ) && logical_rect.width > 0.0
    {
        capture_info.logical_to_pixel_scale =
            Some(f64::from(pixel_size.width) / logical_rect.width);
    }
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
        Some("jpg" | "jpeg") => ModelScreenshotFormat::Jpeg,
        // WebP is the default: at equal quality it is meaningfully smaller than
        // JPEG, which trims payload and upload latency. Unknown values also fall
        // back to WebP rather than failing the capture.
        Some("webp") | None => ModelScreenshotFormat::Webp,
        Some(_) => ModelScreenshotFormat::Webp,
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

/// Read the pixel dimensions of an image file without fully decoding it.
pub fn pixel_size_from_path(path: &Path) -> Option<PixelSize> {
    let (width, height) = image::image_dimensions(path).ok()?;
    Some(PixelSize { width, height })
}

#[cfg(test)]
mod tests {
    use super::{
        ModelScreenshotFormat, model_screenshot_bound_from_value,
        model_screenshot_format_from_value, model_screenshot_quality_from_value,
        prepare_model_capture, prepare_model_capture_with_format,
    };

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-model-capture-{tag}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[test]
    fn prepares_model_capture_defaults_to_bounded_webp() {
        let dir = temp_dir("default");
        let source_path = dir.join("source.png");
        let image = image::RgbImage::from_pixel(2560, 1440, image::Rgb([32, 64, 96]));
        image
            .save(&source_path)
            .expect("source image should be saved");

        let prepared = prepare_model_capture(&dir, "bounded-default-test", &source_path)
            .expect("capture should scale");
        let pixel_size = prepared.pixel_size.expect("pixel size should be known");
        assert_eq!(pixel_size.width, 1440);
        assert_eq!(pixel_size.height, 810);
        assert_eq!(
            prepared.original_pixel_size.expect("source size").width,
            2560
        );
        assert_eq!(
            prepared.path.extension().and_then(|ext| ext.to_str()),
            Some("webp")
        );
        assert_eq!(prepared.format, ModelScreenshotFormat::Webp);
        assert_eq!(prepared.quality, 85);
        assert!(prepared.bytes.is_some_and(|bytes| bytes > 0));

        let _ = std::fs::remove_file(prepared.path);
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn prepares_model_capture_as_bounded_jpeg_when_requested() {
        let dir = temp_dir("jpeg");
        let source_path = dir.join("source.png");
        let image = image::RgbImage::from_pixel(2560, 1440, image::Rgb([32, 64, 96]));
        image
            .save(&source_path)
            .expect("source image should be saved");

        let prepared = prepare_model_capture_with_format(
            &dir,
            "bounded-jpeg-test",
            &source_path,
            ModelScreenshotFormat::Jpeg,
            (1440, 900),
            Some(85),
            None,
            None,
        )
        .expect("capture should scale");
        let pixel_size = prepared.pixel_size.expect("pixel size should be known");
        assert_eq!(pixel_size.width, 1440);
        assert_eq!(pixel_size.height, 810);
        assert_eq!(
            prepared.path.extension().and_then(|ext| ext.to_str()),
            Some("jpg")
        );
        assert_eq!(prepared.format, ModelScreenshotFormat::Jpeg);
        assert_eq!(prepared.quality, 85);
        assert!(prepared.bytes.is_some_and(|bytes| bytes > 0));

        let _ = std::fs::remove_file(prepared.path);
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn prepares_model_capture_as_bounded_webp() {
        let dir = temp_dir("webp");
        let source_path = dir.join("source.png");
        let image = image::RgbImage::from_pixel(2560, 1440, image::Rgb([32, 64, 96]));
        image
            .save(&source_path)
            .expect("source image should be saved");

        let prepared = prepare_model_capture_with_format(
            &dir,
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
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn prepares_model_capture_does_not_upscale_small_capture() {
        let dir = temp_dir("noupscale");
        let source_path = dir.join("source.png");
        // Smaller than the model bounds in both dimensions: must pass through at
        // native size rather than being upscaled to fill the box.
        let image = image::RgbImage::from_pixel(800, 600, image::Rgb([10, 20, 30]));
        image
            .save(&source_path)
            .expect("source image should be saved");

        let prepared = prepare_model_capture_with_format(
            &dir,
            "no-upscale-test",
            &source_path,
            ModelScreenshotFormat::Webp,
            (1440, 900),
            Some(85),
            None,
            None,
        )
        .expect("capture should pass through");
        let pixel_size = prepared.pixel_size.expect("pixel size should be known");
        assert_eq!(pixel_size.width, 800);
        assert_eq!(pixel_size.height, 600);

        let _ = std::fs::remove_file(prepared.path);
        let _ = std::fs::remove_file(source_path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn model_screenshot_format_parser_defaults_to_webp() {
        assert_eq!(
            model_screenshot_format_from_value(None),
            ModelScreenshotFormat::Webp
        );
        assert_eq!(
            model_screenshot_format_from_value(Some("webp")),
            ModelScreenshotFormat::Webp
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
            model_screenshot_format_from_value(Some("nope")),
            ModelScreenshotFormat::Webp
        );
    }

    #[test]
    fn model_screenshot_bound_parser_rejects_unsafe_values() {
        assert_eq!(model_screenshot_bound_from_value(Some("1280"), 1440), 1280);
        assert_eq!(model_screenshot_bound_from_value(Some("63"), 1440), 1440);
        assert_eq!(model_screenshot_bound_from_value(Some("4097"), 1440), 1440);
        assert_eq!(model_screenshot_bound_from_value(Some("nope"), 1440), 1440);
        assert_eq!(model_screenshot_bound_from_value(None, 1440), 1440);
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
