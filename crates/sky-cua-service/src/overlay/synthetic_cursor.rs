use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Instant;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use sky_cua_overlay_host::cursor_asset;
use sky_cua_platform::model::{
    AgentCursorPoint, CaptureInfo, CoordinateSpace, DiagnosticEntry, ModelImageFormat,
};

const DEFAULT_JPEG_QUALITY: u8 = 85;
const DEFAULT_WEBP_QUALITY: u8 = 75;

static AGENT_CURSOR_IMAGE: LazyLock<Result<RgbaImage, String>> = LazyLock::new(|| {
    let image = image::load_from_memory(cursor_asset::AGENT_CURSOR_PNG)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    if image.width() != cursor_asset::AGENT_CURSOR_SOURCE_WIDTH
        || image.height() != cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT
    {
        return Err(format!(
            "expected {}x{} cursor asset, got {}x{}",
            cursor_asset::AGENT_CURSOR_SOURCE_WIDTH,
            cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT,
            image.width(),
            image.height()
        ));
    }
    Ok(image::imageops::resize(
        &image,
        cursor_asset::AGENT_CURSOR_WIDTH,
        cursor_asset::AGENT_CURSOR_HEIGHT,
        FilterType::Lanczos3,
    ))
});

pub(super) fn compose_synthetic_cursor(
    capture: &CaptureInfo,
    point: &AgentCursorPoint,
) -> Result<Option<CaptureInfo>, DiagnosticEntry> {
    compose_synthetic_cursor_with_size(capture, point, None)
}

pub(super) fn compose_synthetic_cursor_with_size(
    capture: &CaptureInfo,
    point: &AgentCursorPoint,
    cursor_size_px: Option<u32>,
) -> Result<Option<CaptureInfo>, DiagnosticEntry> {
    if point.coordinate_space != CoordinateSpace::StreamPixels {
        return Ok(None);
    }
    if cursor_size_px == Some(0) {
        return Ok(None);
    }

    let Some(screenshot_path) = capture.screenshot_path.as_ref() else {
        return Ok(None);
    };
    let screenshot_path = Path::new(screenshot_path);
    let source_path = source_screenshot_path(screenshot_path);
    let started = Instant::now();
    let image = image::open(&source_path).map_err(|error| {
        diagnostic(
            "AgentCursorSyntheticFailed",
            "Failed to open screenshot for agent cursor compositing.",
            Some(format!("path={} error={error}", source_path.display())),
        )
    })?;
    let (width, height) = image.dimensions();
    let mut rgba = image.to_rgba8();
    let cursor = agent_cursor_image().map_err(|error| {
        diagnostic(
            "AgentCursorSyntheticFailed",
            "Failed to decode bundled agent cursor image.",
            Some(error),
        )
    })?;
    let resized_cursor = cursor_size_px.map(|size| {
        let width = ((f64::from(cursor.width()) * f64::from(size) / f64::from(cursor.height()))
            .round() as u32)
            .max(1);
        image::imageops::resize(cursor, width, size.max(1), FilterType::Lanczos3)
    });
    let (hotspot_x, hotspot_y) = match cursor_size_px {
        Some(size) => (
            (f64::from(cursor_asset::AGENT_CURSOR_HOTSPOT_X) * f64::from(size)
                / f64::from(cursor.height()))
            .round() as i32,
            (f64::from(cursor_asset::AGENT_CURSOR_HOTSPOT_Y) * f64::from(size)
                / f64::from(cursor.height()))
            .round() as i32,
        ),
        None => (
            cursor_asset::AGENT_CURSOR_HOTSPOT_X,
            cursor_asset::AGENT_CURSOR_HOTSPOT_Y,
        ),
    };
    let cursor = resized_cursor.as_ref().unwrap_or(cursor);
    if !draw_cursor_image(&mut rgba, cursor, point.x, point.y, hotspot_x, hotspot_y) {
        return Err(diagnostic(
            "AgentCursorSyntheticOutOfBounds",
            "Agent cursor point did not overlap the screenshot.",
            Some(format!(
                "point=({}, {}) image={}x{} path={}",
                point.x,
                point.y,
                width,
                height,
                source_path.display()
            )),
        ));
    }

    let format = output_format(capture, &source_path);
    let output_path = cursor_output_path(&source_path, format.extension());
    encode_cursor_image(&rgba, &output_path, format, capture).map_err(|error| {
        diagnostic(
            "AgentCursorSyntheticFailed",
            "Failed to write agent cursor screenshot.",
            Some(format!("path={} error={error}", output_path.display())),
        )
    })?;

    let mut updated = capture.clone();
    updated.screenshot_path = Some(output_path.display().to_string());
    updated.model_image_bytes = fs::metadata(&output_path)
        .ok()
        .map(|metadata| metadata.len());
    updated.model_image_encode_ms =
        Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
    if format == CursorImageFormat::Jpeg {
        updated.model_image_format = Some(ModelImageFormat::Jpeg);
    } else if format == CursorImageFormat::Webp {
        updated.model_image_format = Some(ModelImageFormat::Webp);
    }
    Ok(Some(updated))
}

pub(super) fn remove_synthetic_cursor(capture: &mut CaptureInfo) -> bool {
    let Some(screenshot_path) = capture.screenshot_path.as_deref().map(Path::new) else {
        return false;
    };
    let Some(raw_path) = decomposited_screenshot_path(screenshot_path) else {
        return false;
    };
    if !raw_path.is_file() {
        return false;
    }

    capture.screenshot_path = Some(raw_path.display().to_string());
    capture.model_image_bytes = fs::metadata(&raw_path).ok().map(|metadata| metadata.len());
    capture.model_image_encode_ms = None;
    true
}

fn source_screenshot_path(screenshot_path: &Path) -> PathBuf {
    if let Some(raw_path) = decomposited_screenshot_path(screenshot_path)
        && raw_path.is_file()
    {
        return raw_path;
    }
    screenshot_path.to_path_buf()
}

pub(super) fn decomposited_screenshot_path(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let raw_stem = stem.strip_suffix(".agent-cursor")?;
    let extension = path.extension()?;
    Some(path.with_file_name(Path::new(raw_stem).with_extension(extension)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorImageFormat {
    Jpeg,
    Png,
    Webp,
}

impl CursorImageFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }
}

fn output_format(capture: &CaptureInfo, path: &Path) -> CursorImageFormat {
    match capture.model_image_format {
        Some(ModelImageFormat::Jpeg) => CursorImageFormat::Jpeg,
        Some(ModelImageFormat::Webp) => CursorImageFormat::Webp,
        None => match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => CursorImageFormat::Png,
            Some("webp") => CursorImageFormat::Webp,
            _ => CursorImageFormat::Jpeg,
        },
    }
}

fn cursor_output_path(path: &Path, extension: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("screenshot");
    parent.join(format!("{stem}.agent-cursor.{extension}"))
}

fn encode_cursor_image(
    rgba: &RgbaImage,
    path: &Path,
    format: CursorImageFormat,
    capture: &CaptureInfo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    match format {
        CursorImageFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
            let quality = capture.model_image_quality.unwrap_or(DEFAULT_JPEG_QUALITY);
            JpegEncoder::new_with_quality(&mut writer, quality).encode_image(&rgb)?;
        }
        CursorImageFormat::Png => {
            DynamicImage::ImageRgba8(rgba.clone()).write_to(&mut writer, ImageFormat::Png)?;
        }
        CursorImageFormat::Webp => {
            let rgb = DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
            let quality = f32::from(capture.model_image_quality.unwrap_or(DEFAULT_WEBP_QUALITY));
            let encoded =
                webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()).encode(quality);
            writer.write_all(&encoded)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn agent_cursor_image() -> Result<&'static RgbaImage, String> {
    match AGENT_CURSOR_IMAGE.as_ref() {
        Ok(image) => Ok(image),
        Err(error) => Err(error.clone()),
    }
}

fn draw_cursor_image(
    destination: &mut RgbaImage,
    cursor: &RgbaImage,
    x: f64,
    y: f64,
    hotspot_x: i32,
    hotspot_y: i32,
) -> bool {
    if !x.is_finite() || !y.is_finite() {
        return false;
    }

    let left = x.round() as i32 - hotspot_x;
    let top = y.round() as i32 - hotspot_y;
    let width = i32::try_from(destination.width()).unwrap_or(i32::MAX);
    let height = i32::try_from(destination.height()).unwrap_or(i32::MAX);
    let mut changed = false;

    for source_y in 0..cursor.height() {
        for source_x in 0..cursor.width() {
            let source = *cursor.get_pixel(source_x, source_y);
            if source[3] == 0 {
                continue;
            }
            let px = left + source_x as i32;
            let py = top + source_y as i32;
            if px < 0 || py < 0 || px >= width || py >= height {
                continue;
            }

            blend_pixel(destination.get_pixel_mut(px as u32, py as u32), source);
            changed = true;
        }
    }

    changed
}

fn blend_pixel(destination: &mut Rgba<u8>, source: Rgba<u8>) {
    let alpha = f32::from(source[3]) / 255.0;
    for channel in 0..3 {
        destination[channel] = ((f32::from(source[channel]) * alpha)
            + (f32::from(destination[channel]) * (1.0 - alpha)))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    destination[3] = 255;
}

fn diagnostic(code: &str, message: &str, details: Option<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Rgba};
    use sky_cua_overlay_host::cursor_asset;
    use sky_cua_platform::model::{
        AgentCursorPoint, CaptureBackendKind, CaptureInfo, CaptureScope, CoordinateSpace,
        ModelImageFormat, PixelSize,
    };

    use super::{compose_synthetic_cursor, compose_synthetic_cursor_with_size};

    #[test]
    fn composites_chrome_cursor_asset_into_png_screenshot_near_requested_point() {
        let dir = unique_temp_dir("compose-center");
        let path = dir.join("capture.png");
        let image = ImageBuffer::from_pixel(96, 96, Rgba([240u8, 240, 240, 255]));
        image.save(&path).expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(48.0, 48.0);

        let updated = compose_synthetic_cursor(&capture, &point)
            .expect("composite should succeed")
            .expect("capture should update");

        let output_path = updated.screenshot_path.expect("updated path");
        assert!(output_path.ends_with("capture.agent-cursor.png"));
        let rendered = image::open(&output_path).expect("open output").to_rgba8();
        let black_source_x = 8_i32;
        let black_source_y = 8_i32;
        let black_dest_x = 48_i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_X + black_source_x;
        let black_dest_y = 48_i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_Y + black_source_y;
        // The cursor's opaque body is composited at the hotspot-offset location.
        // Assert it is opaque and near-black rather than byte-exact (0,0,0): the
        // vector cursor glyph anti-aliases its black fill against the thicker
        // white outline, so an interior pixel reads a few levels off pure black
        // while staying strongly distinct from the 240 background and 255 outline.
        let body = rendered.get_pixel(black_dest_x as u32, black_dest_y as u32);
        assert_eq!(body.0[3], 255, "cursor body is opaque: {body:?}");
        assert!(
            body.0[0] < 32 && body.0[1] < 32 && body.0[2] < 32,
            "cursor body is near-black at the hotspot offset: {body:?}"
        );
        assert_eq!(rendered.get_pixel(95, 95), &Rgba([240u8, 240, 240, 255]));
        assert!(updated.model_image_bytes.unwrap_or_default() > 0);
    }

    #[test]
    fn requested_cursor_size_scales_the_synthetic_cursor_and_zero_disables_it() {
        let dir = unique_temp_dir("compose-sized");
        let path = dir.join("capture.png");
        let image = ImageBuffer::from_pixel(96, 96, Rgba([240u8, 240, 240, 255]));
        image.save(&path).expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(48.0, 48.0);

        let updated = compose_synthetic_cursor_with_size(&capture, &point, Some(12))
            .expect("sized composite should succeed")
            .expect("sized capture should update");
        let rendered = image::open(updated.screenshot_path.expect("sized path"))
            .expect("open sized output")
            .to_rgba8();
        let changed = rendered
            .enumerate_pixels()
            .filter_map(|(x, y, pixel)| (pixel != &Rgba([240u8, 240, 240, 255])).then_some((x, y)));
        let changed = changed.collect::<Vec<_>>();
        let min_x = changed
            .iter()
            .map(|(x, _)| *x)
            .min()
            .expect("cursor pixels");
        let max_x = changed
            .iter()
            .map(|(x, _)| *x)
            .max()
            .expect("cursor pixels");
        let min_y = changed
            .iter()
            .map(|(_, y)| *y)
            .min()
            .expect("cursor pixels");
        let max_y = changed
            .iter()
            .map(|(_, y)| *y)
            .max()
            .expect("cursor pixels");
        assert!(max_x - min_x < 12);
        assert!(max_y - min_y < 12);

        assert!(
            compose_synthetic_cursor_with_size(&capture, &point, Some(0))
                .expect("zero-sized cursor should be accepted")
                .is_none()
        );
    }

    #[test]
    fn composites_chrome_cursor_asset_when_hotspot_is_on_image_edge() {
        let dir = unique_temp_dir("compose-edge");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(16, 16, Rgba([240u8, 240, 240, 255]))
            .save(&path)
            .expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(0.0, 0.0);

        let updated = compose_synthetic_cursor(&capture, &point)
            .expect("edge composite should not fail")
            .expect("capture should update");

        let rendered = image::open(updated.screenshot_path.expect("path"))
            .expect("open output")
            .to_rgba8();
        assert!(
            rendered
                .pixels()
                .any(|pixel| pixel != &Rgba([240u8, 240, 240, 255]))
        );
    }

    #[test]
    fn out_of_bounds_synthetic_point_returns_diagnostic() {
        let dir = unique_temp_dir("compose-oob");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(8, 8, Rgba([0u8, 0, 0, 255]))
            .save(&path)
            .expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(100.0, 100.0);

        let diagnostic = compose_synthetic_cursor(&capture, &point)
            .expect_err("out-of-bounds cursor should produce diagnostic");

        assert_eq!(diagnostic.code, "AgentCursorSyntheticOutOfBounds");
    }

    #[test]
    fn webp_capture_keeps_webp_format_for_cursor_output() {
        let dir = unique_temp_dir("compose-webp");
        let path = dir.join("capture.webp");
        let rgba = ImageBuffer::from_pixel(16, 16, Rgba([0u8, 0, 0, 255]));
        let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
        let encoded = webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()).encode(75.0);
        std::fs::write(&path, &*encoded).expect("write source webp");
        let capture = capture_with_path(&path, Some(ModelImageFormat::Webp));
        let point = synthetic_point(8.0, 8.0);

        let updated = compose_synthetic_cursor(&capture, &point)
            .expect("webp composite should succeed")
            .expect("capture should update");

        assert!(
            updated
                .screenshot_path
                .expect("path")
                .ends_with("capture.agent-cursor.webp")
        );
        assert_eq!(updated.model_image_format, Some(ModelImageFormat::Webp));
    }

    #[test]
    fn recomposing_from_previous_cursor_output_is_idempotent() {
        let dir = unique_temp_dir("compose-idempotent");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(32, 32, Rgba([240u8, 240, 240, 255]))
            .save(&path)
            .expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(16.0, 16.0);

        let first = compose_synthetic_cursor(&capture, &point)
            .expect("first composite should succeed")
            .expect("first capture should update");
        let second = compose_synthetic_cursor(&first, &point)
            .expect("second composite should succeed")
            .expect("second capture should update");

        assert_eq!(first.screenshot_path, second.screenshot_path);
        assert!(
            !second
                .screenshot_path
                .as_deref()
                .expect("path")
                .contains("agent-cursor.agent-cursor")
        );
    }

    fn synthetic_point(x: f64, y: f64) -> AgentCursorPoint {
        AgentCursorPoint {
            x,
            y,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: Some("stream".to_string()),
        }
    }

    fn capture_with_path(path: &std::path::Path, format: Option<ModelImageFormat>) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            capture_scope: CaptureScope::Unknown,
            display: None,
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: Some("mapping".to_string()),
            source_logical_rect: None,
            logical_rect: None,
            pixel_size: Some(PixelSize {
                width: 31,
                height: 31,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: Some(path.display().to_string()),
            original_screenshot_path: None,
            model_image_format: format,
            model_image_quality: Some(85),
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-agent-cursor-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
