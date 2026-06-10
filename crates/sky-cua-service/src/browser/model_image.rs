//! Post-processing for browser viewport captures.
//!
//! The CDP capture arrives as a PNG sized in device pixels. This module
//! normalizes it to CSS-pixel dimensions so image pixels, snapshot element
//! bounds, and pointer coordinates all share one space, re-encodes it with the
//! shared model-screenshot format/quality knobs, and persists it under the
//! per-user runtime captures directory.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::ImageEncoder;
use sky_cua_platform::model::DiagnosticEntry;

const MODEL_SCREENSHOT_FORMAT_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_FORMAT";
const MODEL_SCREENSHOT_JPEG_QUALITY_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY";
const MODEL_SCREENSHOT_WEBP_QUALITY_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY";
const DEFAULT_JPEG_QUALITY: u8 = 85;
const DEFAULT_WEBP_QUALITY: u8 = 85;
const KEPT_CAPTURES_PER_TAB: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserCaptureFormat {
    Jpeg,
    Webp,
}

impl BrowserCaptureFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    fn mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PreparedBrowserCapture {
    pub(super) data_base64: String,
    pub(super) mime_type: String,
    pub(super) screenshot_path: Option<String>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) diagnostics: Vec<DiagnosticEntry>,
}

/// Normalize a raw CDP PNG capture to CSS-pixel dimensions, re-encode it with
/// the model screenshot knobs, and write it to the captures directory.
///
/// Falls back to the raw PNG with a non-error diagnostic when processing
/// fails, so a decode hiccup degrades quality instead of losing the capture.
pub(super) fn prepare_browser_capture(
    tab_id: &str,
    png_base64: &str,
    css_width: f64,
    css_height: f64,
) -> PreparedBrowserCapture {
    match process_capture(tab_id, png_base64, css_width, css_height) {
        Ok(prepared) => prepared,
        Err(message) => {
            let mut fallback = PreparedBrowserCapture {
                data_base64: png_base64.to_string(),
                mime_type: "image/png".to_string(),
                screenshot_path: None,
                width: 0,
                height: 0,
                diagnostics: vec![DiagnosticEntry {
                    code: "BrowserScreenshotDegraded".to_string(),
                    message: format!(
                        "Browser screenshot post-processing failed; returning the raw PNG capture: {message}"
                    ),
                    details: None,
                }],
            };
            if let Ok(bytes) = BASE64.decode(png_base64)
                && let Ok(size) =
                    image::load_from_memory(&bytes).map(|img| (img.width(), img.height()))
            {
                fallback.width = size.0;
                fallback.height = size.1;
            }
            fallback
        }
    }
}

fn process_capture(
    tab_id: &str,
    png_base64: &str,
    css_width: f64,
    css_height: f64,
) -> Result<PreparedBrowserCapture, String> {
    let png_bytes = BASE64
        .decode(png_base64)
        .map_err(|error| format!("invalid base64 image data: {error}"))?;
    let decoded = image::load_from_memory(&png_bytes)
        .map_err(|error| format!("failed to decode PNG capture: {error}"))?;

    let target = css_target_dimensions(decoded.width(), decoded.height(), css_width, css_height);
    let mut diagnostics = Vec::new();
    let css_metrics_present = css_width.round() >= 1.0 && css_height.round() >= 1.0;
    if !css_metrics_present {
        // Without viewport metrics the capture was also taken without a clip
        // (full page, device pixels); the one-space guarantee does not hold.
        diagnostics.push(DiagnosticEntry {
            code: "BrowserScreenshotDegraded".to_string(),
            message: "Browser viewport metrics were unavailable; the capture may cover \
                      the full page at device-pixel dimensions and may not match \
                      CSS-pixel pointer coordinates."
                .to_string(),
            details: None,
        });
    } else if target != (css_width.round() as u32, css_height.round() as u32) {
        diagnostics.push(DiagnosticEntry {
            code: "BrowserScreenshotDegraded".to_string(),
            message: format!(
                "Browser capture could not be normalized to the reported CSS viewport \
                 ({css_width:.0}x{css_height:.0}); image pixels are {}x{} and may not \
                 match CSS-pixel pointer coordinates.",
                target.0, target.1
            ),
            details: None,
        });
    }
    let normalized = if target != (decoded.width(), decoded.height()) {
        decoded.resize_exact(target.0, target.1, image::imageops::FilterType::Lanczos3)
    } else {
        decoded
    };
    let rgb = normalized.to_rgb8();

    let format = capture_format_from_env();
    let quality = capture_quality_from_env(format);
    let mut encoded: Vec<u8> = Vec::new();
    match format {
        BrowserCaptureFormat::Jpeg => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, quality);
            encoder
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|error| format!("failed to encode JPEG capture: {error}"))?;
        }
        BrowserCaptureFormat::Webp => {
            let webp = webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height())
                .encode(f32::from(quality));
            encoded.extend_from_slice(webp.as_ref());
        }
    }

    let screenshot_path = match write_capture_file(tab_id, format, &encoded) {
        Ok(path) => Some(path.display().to_string()),
        Err(message) => {
            diagnostics.push(DiagnosticEntry {
                code: "BrowserScreenshotDegraded".to_string(),
                message: format!(
                    "Browser screenshot could not be persisted to disk; image data is still attached: {message}"
                ),
                details: None,
            });
            None
        }
    };

    Ok(PreparedBrowserCapture {
        data_base64: BASE64.encode(&encoded),
        mime_type: format.mime_type().to_string(),
        screenshot_path,
        width: rgb.width(),
        height: rgb.height(),
        diagnostics,
    })
}

/// Largest upscale factor tolerated when normalizing a capture to CSS
/// dimensions. Browser zoom-out drops the effective render scale below 1
/// (capture smaller than the CSS viewport), so upscaling must be allowed or
/// image pixels silently stop matching CSS-pixel pointer coordinates; the
/// bound keeps absurd viewport metrics from allocating enormous images.
const MAX_CSS_UPSCALE: f64 = 8.0;

/// Pick the output dimensions: the CSS viewport size reported by the page when
/// it is plausible, otherwise the capture's own dimensions. A capture equal to
/// the CSS size (device pixel ratio 1) passes through without resampling;
/// smaller captures (zoom-out) are upscaled so the coordinate space holds.
fn css_target_dimensions(
    capture_width: u32,
    capture_height: u32,
    css_width: f64,
    css_height: f64,
) -> (u32, u32) {
    let css_width = css_width.round();
    let css_height = css_height.round();
    let plausible = css_width >= 1.0
        && css_height >= 1.0
        && css_width <= f64::from(capture_width) * MAX_CSS_UPSCALE
        && css_height <= f64::from(capture_height) * MAX_CSS_UPSCALE;
    if plausible {
        (css_width as u32, css_height as u32)
    } else {
        (capture_width, capture_height)
    }
}

fn capture_format_from_env() -> BrowserCaptureFormat {
    capture_format_from_value(std::env::var(MODEL_SCREENSHOT_FORMAT_ENV).ok().as_deref())
}

fn capture_format_from_value(value: Option<&str>) -> BrowserCaptureFormat {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("webp") => BrowserCaptureFormat::Webp,
        _ => BrowserCaptureFormat::Jpeg,
    }
}

fn capture_quality_from_env(format: BrowserCaptureFormat) -> u8 {
    let (env, default) = match format {
        BrowserCaptureFormat::Jpeg => (MODEL_SCREENSHOT_JPEG_QUALITY_ENV, DEFAULT_JPEG_QUALITY),
        BrowserCaptureFormat::Webp => (MODEL_SCREENSHOT_WEBP_QUALITY_ENV, DEFAULT_WEBP_QUALITY),
    };
    capture_quality_from_value(std::env::var(env).ok().as_deref(), default)
}

fn capture_quality_from_value(value: Option<&str>, default: u8) -> u8 {
    value
        .map(str::trim)
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|quality| (1..=100).contains(quality))
        .unwrap_or(default)
}

fn captures_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("sky-cua")
        .join("captures")
}

fn write_capture_file(
    tab_id: &str,
    format: BrowserCaptureFormat,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let dir = captures_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;

    let tab_slug = sanitize_tab_id(tab_id);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    // Two captures of one tab can land in the same millisecond; a fixed-width
    // process counter keeps filenames unique and the lexicographic sort in
    // `prune_tab_captures` chronological.
    static CAPTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = CAPTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 10_000;
    let path = dir.join(format!(
        "browser-{tab_slug}-{millis}-{sequence:04}.{}",
        format.extension()
    ));

    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;

    prune_tab_captures(&dir, &tab_slug, &path);
    Ok(path)
}

fn sanitize_tab_id(tab_id: &str) -> String {
    let slug: String = tab_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug.chars().take(32).collect()
}

/// Keep the most recent captures per tab so long sessions do not accumulate
/// unbounded image files in the runtime directory.
fn prune_tab_captures(dir: &std::path::Path, tab_slug: &str, just_written: &std::path::Path) {
    let prefix = format!("browser-{tab_slug}-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut matching: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path != just_written
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect();
    matching.sort();
    // `matching` excludes `just_written`, so retain KEPT - 1 prior captures:
    // the inclusive upper bound deletes one extra entry to make room for the
    // file that was just written, keeping KEPT files total per tab.
    if matching.len() >= KEPT_CAPTURES_PER_TAB {
        for stale in &matching[..=matching.len() - KEPT_CAPTURES_PER_TAB] {
            let _ = std::fs::remove_file(stale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_base64(width: u32, height: u32) -> String {
        let mut bytes = Vec::new();
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([120, 30, 200]));
        image::DynamicImage::ImageRgb8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode test png");
        BASE64.encode(&bytes)
    }

    #[test]
    fn prune_keeps_newest_captures_including_just_written() {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-prune-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut paths = Vec::new();
        for index in 0..11 {
            let path = dir.join(format!("browser-tab1-{index:02}.jpg"));
            std::fs::write(&path, b"x").unwrap();
            paths.push(path);
        }
        let just_written = paths.last().unwrap().clone();
        prune_tab_captures(&dir, "tab1", &just_written);

        let remaining: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        assert_eq!(remaining.len(), KEPT_CAPTURES_PER_TAB);
        // The oldest captures are pruned; the just-written file survives.
        for stale in &paths[..11 - KEPT_CAPTURES_PER_TAB] {
            assert!(!stale.exists(), "expected pruned: {}", stale.display());
        }
        for kept in &paths[11 - KEPT_CAPTURES_PER_TAB..] {
            assert!(kept.exists(), "expected kept: {}", kept.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalizes_hidpi_capture_to_css_dimensions() {
        let prepared = prepare_browser_capture("tab-1", &png_base64(200, 160), 100.0, 80.0);
        assert_eq!(prepared.width, 100);
        assert_eq!(prepared.height, 80);
        assert_eq!(prepared.mime_type, "image/jpeg");
        assert!(!prepared.data_base64.is_empty());
        let decoded = image::load_from_memory(&BASE64.decode(&prepared.data_base64).unwrap())
            .expect("decode prepared capture");
        assert_eq!((decoded.width(), decoded.height()), (100, 80));
        if let Some(path) = prepared.screenshot_path.as_deref() {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn upscales_zoomed_out_capture_to_css_dimensions() {
        // Browser zoom-out renders the capture smaller than the CSS viewport;
        // it must be upscaled so image pixels keep matching CSS-pixel
        // pointer coordinates.
        let prepared = prepare_browser_capture("tab-zoom", &png_base64(50, 40), 100.0, 80.0);
        assert_eq!(prepared.width, 100);
        assert_eq!(prepared.height, 80);
        assert!(prepared.diagnostics.is_empty());
        if let Some(path) = prepared.screenshot_path.as_deref() {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn implausible_css_metrics_keep_capture_dimensions_with_diagnostic() {
        // Metrics beyond the upscale bound abandon normalization, and the
        // broken coordinate-space guarantee must be surfaced, not silent.
        let prepared = prepare_browser_capture("tab-wild", &png_base64(64, 48), 6400.0, 4800.0);
        assert_eq!(prepared.width, 64);
        assert_eq!(prepared.height, 48);
        assert_eq!(prepared.diagnostics.len(), 1);
        assert_eq!(prepared.diagnostics[0].code, "BrowserScreenshotDegraded");
        assert!(prepared.diagnostics[0].message.contains("CSS-pixel"));
        if let Some(path) = prepared.screenshot_path.as_deref() {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn keeps_capture_dimensions_when_css_metrics_are_missing() {
        let prepared = prepare_browser_capture("tab-2", &png_base64(64, 48), 0.0, 0.0);
        assert_eq!(prepared.width, 64);
        assert_eq!(prepared.height, 48);
        // The missing-metrics capture is unclipped and device-pixel sized;
        // the broken coordinate-space guarantee must be surfaced.
        assert_eq!(prepared.diagnostics.len(), 1);
        assert_eq!(prepared.diagnostics[0].code, "BrowserScreenshotDegraded");
        assert!(prepared.diagnostics[0].message.contains("viewport metrics"));
        if let Some(path) = prepared.screenshot_path.as_deref() {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn invalid_image_data_falls_back_to_raw_payload_with_diagnostic() {
        let prepared = prepare_browser_capture("tab-3", "not-base64!!!", 100.0, 80.0);
        assert_eq!(prepared.data_base64, "not-base64!!!");
        assert_eq!(prepared.mime_type, "image/png");
        assert!(prepared.screenshot_path.is_none());
        assert_eq!(prepared.diagnostics.len(), 1);
        assert_eq!(prepared.diagnostics[0].code, "BrowserScreenshotDegraded");
    }

    #[test]
    fn capture_format_parses_webp_and_defaults_to_jpeg() {
        assert_eq!(
            capture_format_from_value(Some("webp")),
            BrowserCaptureFormat::Webp
        );
        assert_eq!(
            capture_format_from_value(Some("WEBP ")),
            BrowserCaptureFormat::Webp
        );
        assert_eq!(
            capture_format_from_value(Some("png")),
            BrowserCaptureFormat::Jpeg
        );
        assert_eq!(capture_format_from_value(None), BrowserCaptureFormat::Jpeg);
    }

    #[test]
    fn capture_quality_rejects_out_of_range_values() {
        assert_eq!(capture_quality_from_value(Some("70"), 85), 70);
        assert_eq!(capture_quality_from_value(Some("0"), 85), 85);
        assert_eq!(capture_quality_from_value(Some("101"), 85), 85);
        assert_eq!(capture_quality_from_value(Some("junk"), 85), 85);
        assert_eq!(capture_quality_from_value(None, 85), 85);
    }

    #[test]
    fn sanitizes_tab_ids_for_filenames() {
        assert_eq!(sanitize_tab_id("tab/../12:34"), "tab----12-34");
        assert!(sanitize_tab_id(&"x".repeat(64)).len() <= 32);
    }
}
