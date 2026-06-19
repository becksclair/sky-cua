//! Coordinate-mapping transform tests (split from `mapping/mod.rs` to keep
//! each file under the god-file threshold).

use super::*;

fn pt(x: f64, y: f64) -> PhonePoint {
    PhonePoint { x, y }
}

fn host_rect(x: f64, y: f64, width: f64, height: f64) -> RectF {
    RectF {
        x,
        y,
        width,
        height,
        space: CoordinateSpace::DesktopLogical,
    }
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
}

#[test]
fn identity_mapping_is_one_to_one_zero_rotation() {
    let mapping = identity_mapping(
        "m",
        "s",
        "serial",
        PixelSize {
            width: 1080,
            height: 2400,
        },
        42,
    );
    assert_eq!(mapping.rotation_degrees, 0);
    assert_eq!(mapping.device_rect.width, 1080.0);
    assert_eq!(mapping.screenshot_rect.height, 2400.0);
    assert!(mapping.host_content_rect.is_none());

    let device = screenshot_to_device(&mapping, pt(540.0, 1200.0)).expect("in bounds");
    approx(device.x, 540.0);
    approx(device.y, 1200.0);
}

#[test]
fn screenshot_downscaled_by_max_size_scales_to_device() {
    // scrcpy --max-size 1200 on a 1080x2400 device: screenshot is 540x1200.
    let build = MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 1080,
            height: 2400,
        },
        screenshot_size: PixelSize {
            width: 540,
            height: 1200,
        },
        rotation_degrees: 0,
        host_window_rect: None,
        host_content_rect: None,
        captured_at_ms: 1,
    };
    let mapping = build_mapping(&build).expect("valid");

    let device = screenshot_to_device(&mapping, pt(270.0, 600.0)).expect("in bounds");
    approx(device.x, 540.0);
    approx(device.y, 1200.0);

    let back = device_to_screenshot(&mapping, device).expect("in bounds");
    approx(back.x, 270.0);
    approx(back.y, 600.0);
}

#[test]
fn host_content_rect_with_letterbox_maps_to_device() {
    // A 540x1200 portrait device shown in a 800x1200 content box would
    // letterbox horizontally; here the content box is the exact device
    // aspect placed at a window offset with fractional host scale 1.5.
    // device 540x1200 -> content box origin (130,40) size 405x900 (0.75x).
    let build = MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 540,
            height: 1200,
        },
        screenshot_size: PixelSize {
            width: 540,
            height: 1200,
        },
        rotation_degrees: 0,
        host_window_rect: Some(host_rect(100.0, 0.0, 600.0, 1000.0)),
        host_content_rect: Some(host_rect(130.0, 40.0, 405.0, 900.0)),
        captured_at_ms: 1,
    };
    let mapping = build_mapping(&build).expect("valid");

    // Top-left of content box maps to device origin.
    let dev_tl = host_to_device(&mapping, pt(130.0, 40.0)).expect("in bounds");
    approx(dev_tl.x, 0.0);
    approx(dev_tl.y, 0.0);

    // Center of content box maps to device center.
    let dev_center = host_to_device(&mapping, pt(130.0 + 202.5, 40.0 + 450.0)).expect("in bounds");
    approx(dev_center.x, 270.0);
    approx(dev_center.y, 600.0);

    // Round-trip device -> host.
    let host = device_to_host(&mapping, pt(270.0, 600.0)).expect("in bounds");
    approx(host.x, 130.0 + 202.5);
    approx(host.y, 40.0 + 450.0);
}

#[test]
fn screenshot_to_host_composes_both_scales() {
    // screenshot 270x600 (half device), content box 405x900 at (130,40).
    let build = MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 540,
            height: 1200,
        },
        screenshot_size: PixelSize {
            width: 270,
            height: 600,
        },
        rotation_degrees: 0,
        host_window_rect: None,
        host_content_rect: Some(host_rect(130.0, 40.0, 405.0, 900.0)),
        captured_at_ms: 1,
    };
    let mapping = build_mapping(&build).expect("valid");
    let host = screenshot_to_host(&mapping, pt(135.0, 300.0)).expect("in bounds");
    // screenshot center -> content box center.
    approx(host.x, 130.0 + 202.5);
    approx(host.y, 40.0 + 450.0);
}

#[test]
fn resized_window_recomputes_via_new_content_rect() {
    // Same device, two different content rects (window resized). The same
    // device center must land at each content box's center.
    let device = PixelSize {
        width: 1000,
        height: 2000,
    };
    for content in [
        host_rect(0.0, 0.0, 300.0, 600.0),
        host_rect(50.0, 25.0, 450.0, 900.0),
    ] {
        let build = MappingBuild {
            mapping_id: "m",
            session_id: "s",
            serial: "serial",
            device_size: device.clone(),
            screenshot_size: device.clone(),
            rotation_degrees: 0,
            host_window_rect: None,
            host_content_rect: Some(content.clone()),
            captured_at_ms: 1,
        };
        let mapping = build_mapping(&build).expect("valid");
        let host = device_to_host(&mapping, pt(500.0, 1000.0)).expect("in bounds");
        approx(host.x, content.x + content.width / 2.0);
        approx(host.y, content.y + content.height / 2.0);
    }
}

#[test]
fn rotation_natural_frame_landscape_90() {
    // Device naturally 1080x2400 (portrait). Displayed landscape at 90°, so
    // device_rect is 2400x1080.
    let build = MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 2400,
            height: 1080,
        },
        screenshot_size: PixelSize {
            width: 2400,
            height: 1080,
        },
        rotation_degrees: 90,
        host_window_rect: None,
        host_content_rect: None,
        captured_at_ms: 1,
    };
    let mapping = build_mapping(&build).expect("valid");

    // Displayed top-left (0,0) at 90°: natural = (y, w - x) = (0, 2400).
    let natural_tl = device_point_to_natural(&mapping, pt(0.0, 0.0)).expect("in bounds");
    approx(natural_tl.x, 0.0);
    approx(natural_tl.y, 2400.0);

    // Displayed top-right (2400,0): natural = (0, 0).
    let natural_tr = device_point_to_natural(&mapping, pt(2400.0, 0.0)).expect("in bounds");
    approx(natural_tr.x, 0.0);
    approx(natural_tr.y, 0.0);
}

#[test]
fn rotation_180_and_270_natural_frame() {
    let build_for = |rot: i32, dw: u32, dh: u32| {
        build_mapping(&MappingBuild {
            mapping_id: "m",
            session_id: "s",
            serial: "serial",
            device_size: PixelSize {
                width: dw,
                height: dh,
            },
            screenshot_size: PixelSize {
                width: dw,
                height: dh,
            },
            rotation_degrees: rot,
            host_window_rect: None,
            host_content_rect: None,
            captured_at_ms: 1,
        })
        .expect("valid")
    };

    // 180: device_rect same extent as natural (w=1080, h=2400). natural =
    // (w - x, h - y). The origin is a weak oracle (it just maps to (w, h)), so
    // assert at a non-origin interior point that actually pins the transform: a
    // wrong-sign or axis-swapped formula would land elsewhere.
    let m180 = build_for(180, 1080, 2400);
    let n = device_point_to_natural(&m180, pt(300.0, 700.0)).expect("ok");
    approx(n.x, 780.0); // 1080 - 300
    approx(n.y, 1700.0); // 2400 - 700
    // Corner still holds.
    let corner = device_point_to_natural(&m180, pt(0.0, 0.0)).expect("ok");
    approx(corner.x, 1080.0);
    approx(corner.y, 2400.0);

    // 270: landscape device_rect (w=2400, h=1080). natural = (h - y, x). Again
    // assert at an interior point: this is the quarter the live path used to
    // collapse into 90, and 90 would give (y, w - x) = (700, 2100) here, so the
    // interior point distinguishes the two.
    let m270 = build_for(270, 2400, 1080);
    let n = device_point_to_natural(&m270, pt(300.0, 700.0)).expect("ok");
    approx(n.x, 380.0); // 1080 - 700
    approx(n.y, 300.0); // x
    // Corner still holds.
    let corner = device_point_to_natural(&m270, pt(0.0, 0.0)).expect("ok");
    approx(corner.x, 1080.0);
    approx(corner.y, 0.0);
}

#[test]
fn out_of_bounds_screenshot_point_rejected() {
    let mapping = identity_mapping(
        "m",
        "s",
        "serial",
        PixelSize {
            width: 100,
            height: 100,
        },
        1,
    );
    let err = screenshot_to_device(&mapping, pt(150.0, 10.0)).expect_err("oob");
    assert!(matches!(
        err,
        MappingError::OutOfBounds {
            plane: "screenshot"
        }
    ));
    assert_eq!(err.code(), "PhoneMappingOutOfBounds");
}

#[test]
fn negative_coordinate_rejected() {
    let mapping = identity_mapping(
        "m",
        "s",
        "serial",
        PixelSize {
            width: 100,
            height: 100,
        },
        1,
    );
    let err = screenshot_to_device(&mapping, pt(-1.0, 10.0)).expect_err("negative");
    assert!(matches!(
        err,
        MappingError::OutOfBounds {
            plane: "screenshot"
        }
    ));
}

#[test]
fn nan_and_infinite_coordinates_rejected() {
    let mapping = identity_mapping(
        "m",
        "s",
        "serial",
        PixelSize {
            width: 100,
            height: 100,
        },
        1,
    );
    let nan = screenshot_to_device(&mapping, pt(f64::NAN, 10.0)).expect_err("nan");
    assert!(matches!(
        nan,
        MappingError::NonFinite {
            plane: "screenshot"
        }
    ));
    assert_eq!(nan.code(), "PhoneMappingNonFinite");

    let inf = screenshot_to_device(&mapping, pt(10.0, f64::INFINITY)).expect_err("inf");
    assert!(matches!(
        inf,
        MappingError::NonFinite {
            plane: "screenshot"
        }
    ));

    let neg_inf =
        device_point_to_natural(&mapping, pt(f64::NEG_INFINITY, 10.0)).expect_err("neg inf");
    assert!(matches!(
        neg_inf,
        MappingError::NonFinite { plane: "device" }
    ));
}

#[test]
fn host_translation_without_surface_errors() {
    let mapping = identity_mapping(
        "m",
        "s",
        "serial",
        PixelSize {
            width: 100,
            height: 100,
        },
        1,
    );
    let err = host_to_device(&mapping, pt(10.0, 10.0)).expect_err("no host");
    assert_eq!(err, MappingError::NoHostMapping);
    assert_eq!(err.code(), "PhoneMappingNoHostSurface");

    let err = device_to_host(&mapping, pt(10.0, 10.0)).expect_err("no host");
    assert_eq!(err, MappingError::NoHostMapping);
}

#[test]
fn unsupported_rotation_rejected_at_build() {
    let err = build_mapping(&MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 100,
            height: 100,
        },
        screenshot_size: PixelSize {
            width: 100,
            height: 100,
        },
        rotation_degrees: 45,
        host_window_rect: None,
        host_content_rect: None,
        captured_at_ms: 1,
    })
    .expect_err("bad rotation");
    assert!(matches!(
        err,
        MappingError::UnsupportedRotation {
            rotation_degrees: 45
        }
    ));
    assert_eq!(err.code(), "PhoneMappingUnsupportedRotation");
}

#[test]
fn degenerate_extents_rejected_at_build() {
    let err = build_mapping(&MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 0,
            height: 100,
        },
        screenshot_size: PixelSize {
            width: 100,
            height: 100,
        },
        rotation_degrees: 0,
        host_window_rect: None,
        host_content_rect: None,
        captured_at_ms: 1,
    })
    .expect_err("degenerate device");
    assert!(matches!(
        err,
        MappingError::DegenerateRect { plane: "device" }
    ));
}

#[test]
fn zero_extent_host_content_rect_is_treated_as_no_surface() {
    let mapping = build_mapping(&MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 100,
            height: 100,
        },
        screenshot_size: PixelSize {
            width: 100,
            height: 100,
        },
        rotation_degrees: 0,
        host_window_rect: None,
        host_content_rect: Some(host_rect(0.0, 0.0, 0.0, 0.0)),
        captured_at_ms: 1,
    })
    .expect("valid");
    assert!(mapping.host_content_rect.is_none());
    assert!(matches!(
        device_to_host(&mapping, pt(10.0, 10.0)),
        Err(MappingError::NoHostMapping)
    ));
}

#[test]
fn negative_rotation_normalizes() {
    let mapping = build_mapping(&MappingBuild {
        mapping_id: "m",
        session_id: "s",
        serial: "serial",
        device_size: PixelSize {
            width: 2400,
            height: 1080,
        },
        screenshot_size: PixelSize {
            width: 2400,
            height: 1080,
        },
        rotation_degrees: -90,
        host_window_rect: None,
        host_content_rect: None,
        captured_at_ms: 1,
    })
    .expect("valid");
    assert_eq!(mapping.rotation_degrees, 270);
}
