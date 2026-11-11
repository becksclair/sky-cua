use std::path::PathBuf;
use std::time::Instant;

use super::*;

#[test]
fn empty_capture_reports_no_image() {
    let capture = empty_capture();
    assert_eq!(capture.backend, CaptureBackendKind::ScreenCaptureKit);
    assert_eq!(capture.image_backend, None);
    assert_eq!(capture.capture_scope, CaptureScope::Unknown);
    assert_eq!(
        capture.coordinate_space,
        Some(CoordinateSpace::StreamPixels)
    );
    assert_eq!(capture.logical_to_pixel_scale, Some(1.0));
    assert!(capture.logical_rect.is_none());
    assert!(capture.screenshot_path.is_none());
    assert!(capture.pixel_size.is_none());
}

fn probe_dir() -> PathBuf {
    std::env::var_os("SKY_CUA_MACOS_CAPTURE_PROBE_DIR").map_or_else(
        || std::env::temp_dir().join("sky-cua-macos-capture-probe"),
        PathBuf::from,
    )
}

fn report(
    label: &str,
    scale: f64,
    timings: sck::CaptureTimings,
    started: Instant,
    capture: &CaptureInfo,
) {
    let rect = capture.logical_rect.as_ref().unwrap();
    let original = capture.original_pixel_size.as_ref().unwrap();
    let model = capture.pixel_size.as_ref().unwrap();
    eprintln!(
        "{label}: rect {}x{}@({},{}) pt, scale {}, raw {}x{} px, model {}x{} px, \
         shareable {:?} (reused {}), screenshot {:?}, encode {} ms, end-to-end {:?}, {}",
        rect.width,
        rect.height,
        rect.x,
        rect.y,
        scale,
        original.width,
        original.height,
        model.width,
        model.height,
        timings.shareable_content,
        timings.content_reused,
        timings.screenshot,
        capture.model_image_encode_ms.unwrap_or_default(),
        started.elapsed(),
        capture.screenshot_path.as_deref().unwrap_or_default(),
    );
}

fn assert_exact_mapping(frame_scale: f64, capture: &CaptureInfo) {
    let rect = capture.logical_rect.as_ref().unwrap();
    let original = capture.original_pixel_size.as_ref().unwrap();
    assert_eq!(
        f64::from(original.width),
        (rect.width * frame_scale).round(),
        "raw width must equal logical width times scale"
    );
    assert_eq!(
        f64::from(original.height),
        (rect.height * frame_scale).round(),
        "raw height must equal logical height times scale"
    );
    let scale = capture.logical_to_pixel_scale.unwrap();
    let model = capture.pixel_size.as_ref().unwrap();
    assert!((f64::from(model.width) / rect.width - scale).abs() < 1e-9);
}

/// Captures the front-most normal window and the main display into
/// `SKY_CUA_MACOS_CAPTURE_PROBE_DIR`, three times each, and prints per-stage
/// timings. Read-only: no input, activation, or permission prompts.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live desktop probe"]
async fn live_capture_window_and_display() {
    assert!(
        permissions::status().screen_capture,
        "Screen Recording not granted"
    );
    let dir = probe_dir();
    std::fs::create_dir_all(&dir).unwrap();

    let windows = sck::probe_windows().await.unwrap();
    let window = windows
        .iter()
        .find(|window| {
            window.layer == 0 && window.frame.width >= 300.0 && window.frame.height >= 200.0
        })
        .expect("an on-screen normal window")
        .clone();
    eprintln!(
        "window target: {} {:?} '{}' {:?}",
        window.window_id, window.app, window.title, window.frame
    );
    for run in 0..3 {
        let started = Instant::now();
        let frame = sck::capture_window(window.window_id, window.pid)
            .await
            .unwrap();
        let scale = frame.point_pixel_scale;
        let convert_started = Instant::now();
        let rgb = pixels::cg_image_to_rgb(&frame.image).unwrap();
        eprintln!(
            "  convert {}x{} BGRA->RGB: {:?}",
            rgb.width(),
            rgb.height(),
            convert_started.elapsed()
        );
        let timings = frame.timings;
        let capture = build_capture_info(
            &dir,
            &format!("probe-window-{run}"),
            frame,
            CaptureScope::Window,
            None,
        )
        .await
        .unwrap();
        report("window", scale, timings, started, &capture);
        assert_exact_mapping(scale, &capture);
    }

    let main_display = objc2_core_graphics::CGMainDisplayID();
    for run in 0..3 {
        let started = Instant::now();
        let frame = sck::capture_display(main_display).await.unwrap();
        let scale = frame.point_pixel_scale;
        let convert_started = Instant::now();
        let rgb = pixels::cg_image_to_rgb(&frame.image).unwrap();
        eprintln!(
            "  convert {}x{} BGRA->RGB: {:?}",
            rgb.width(),
            rgb.height(),
            convert_started.elapsed()
        );
        let timings = frame.timings;
        let capture = build_capture_info(
            &dir,
            &format!("probe-display-{run}"),
            frame,
            CaptureScope::PrimaryDisplay,
            None,
        )
        .await
        .unwrap();
        report("display", scale, timings, started, &capture);
        assert_exact_mapping(scale, &capture);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live desktop probe"]
async fn live_missing_window_is_geometry_missing() {
    let error = sck::capture_window(u32::MAX - 1, 1)
        .await
        .err()
        .expect("a nonexistent window must not capture");
    assert_eq!(
        error.code, "CaptureSourceGeometryMissing",
        "{}",
        error.message
    );
    eprintln!("{}", error.message);
}
