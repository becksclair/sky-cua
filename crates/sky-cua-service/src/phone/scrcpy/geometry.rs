//! scrcpy host-window content-rect geometry.
//!
//! From the host window rect, device pixel size, and rotation this computes the
//! letterboxed video content rectangle so tap coordinates survive letterboxing,
//! rotation, and fractional host scale. The host-visible cursor overlay is
//! enabled only when that mapping is current.

use sky_cua_platform::model::{CoordinateSpace, PixelSize, RectF};

/// The video content rectangle inside the host window, plus the device size and
/// rotation it was computed against. Used to map between host-desktop pixels and
/// device pixels for the host-visible overlay.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::phone) struct ScrcpyContentRect {
    /// The letterboxed content rectangle in host-desktop pixels.
    pub(in crate::phone) content_rect: RectF,
    /// The device pixel size after applying rotation (the orientation scrcpy is
    /// actually rendering).
    pub(in crate::phone) rotated_device_size: PixelSize,
    /// Clockwise rotation in degrees (0/90/180/270) applied to the device frame.
    pub(in crate::phone) rotation_degrees: i32,
    /// Uniform scale from rotated-device pixels to host content pixels.
    pub(in crate::phone) host_scale: f64,
}

/// Compute the letterboxed video content rectangle inside `host_window`.
///
/// scrcpy preserves aspect ratio, so the rotated device frame is scaled
/// uniformly to fit the host window and centered, leaving letterbox bars on the
/// constraining axis. `host_scale` is the single uniform factor mapping
/// rotated-device pixels to host pixels; fractional values (HiDPI / fractional
/// desktop scaling) are preserved exactly.
///
/// Returns `None` for degenerate inputs (zero-area window or device) so callers
/// fall back to ADB coordinates instead of dividing by zero, and for a host
/// window whose coordinate space is not [`CoordinateSpace::DesktopLogical`]: the
/// content-rect output is asserted to be in that plane, so a mismatched input
/// space surfaces as a failed mapping (overlay disabled, ADB fallback) rather
/// than a silently-misplaced cursor. The Linux backend always emits
/// `DesktopLogical`, so the live path is unchanged.
pub(in crate::phone) fn content_rect(
    host_window: &RectF,
    device_size: PixelSize,
    rotation_degrees: i32,
) -> Option<ScrcpyContentRect> {
    if host_window.space != CoordinateSpace::DesktopLogical {
        return None;
    }
    if host_window.width <= 0.0 || host_window.height <= 0.0 {
        return None;
    }
    if device_size.width == 0 || device_size.height == 0 {
        return None;
    }

    let rotation = normalize_rotation(rotation_degrees);
    let rotated = rotate_size(device_size, rotation);
    let dev_w = f64::from(rotated.width);
    let dev_h = f64::from(rotated.height);

    // Uniform fit: the smaller of the two axis ratios drives the scale.
    let scale_x = host_window.width / dev_w;
    let scale_y = host_window.height / dev_h;
    let host_scale = scale_x.min(scale_y);

    let content_w = dev_w * host_scale;
    let content_h = dev_h * host_scale;
    // Center the content; floor of the half-gap is fine because the rect is f64.
    let offset_x = (host_window.width - content_w) / 2.0;
    let offset_y = (host_window.height - content_h) / 2.0;

    Some(ScrcpyContentRect {
        content_rect: RectF {
            x: host_window.x + offset_x,
            y: host_window.y + offset_y,
            width: content_w,
            height: content_h,
            space: CoordinateSpace::DesktopLogical,
        },
        rotated_device_size: rotated,
        rotation_degrees: rotation,
        host_scale,
    })
}

impl ScrcpyContentRect {
    /// Map a device-pixel point into host-desktop pixels using the content rect
    /// and uniform scale. Rotation is already baked into `rotated_device_size`, so
    /// the caller passes a point already expressed in the rotated device frame.
    ///
    /// The production host-visible cursor overlay that consumed this was removed
    /// (the agent cursor is drawn on the device by the companion overlay), so this
    /// stays exercised by the window-mapping drift tests until a host-side consumer
    /// (e.g. host-click-to-device mapping) wires it again.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(in crate::phone) fn device_to_host(&self, device_x: f64, device_y: f64) -> (f64, f64) {
        (
            self.content_rect.x + device_x * self.host_scale,
            self.content_rect.y + device_y * self.host_scale,
        )
    }

    /// Map a host-desktop-pixel point back into the rotated device frame.
    /// Returns `None` when the point lies in the letterbox bars (outside the
    /// content rect) so out-of-frame host clicks are rejected rather than mapped
    /// to a bogus device coordinate.
    ///
    /// The host-to-device direction (acting on a click made directly on the
    /// scrcpy window) is a later increment; only the device-to-host overlay-draw
    /// direction is wired now, so this stays test-exercised until then.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(in crate::phone) fn host_to_device(&self, host_x: f64, host_y: f64) -> Option<(f64, f64)> {
        if self.host_scale <= 0.0 {
            return None;
        }
        let rel_x = host_x - self.content_rect.x;
        let rel_y = host_y - self.content_rect.y;
        if rel_x < 0.0
            || rel_y < 0.0
            || rel_x > self.content_rect.width
            || rel_y > self.content_rect.height
        {
            return None;
        }
        Some((rel_x / self.host_scale, rel_y / self.host_scale))
    }
}

/// Clamp an arbitrary degree value to one of 0/90/180/270 clockwise.
fn normalize_rotation(degrees: i32) -> i32 {
    let normalized = degrees.rem_euclid(360);
    // Snap to the nearest quarter turn; scrcpy only renders quarter rotations.
    match normalized {
        0..=44 | 315..=359 => 0,
        45..=134 => 90,
        135..=224 => 180,
        _ => 270,
    }
}

/// Apply a quarter-turn rotation to a device size (90/270 swap width/height).
fn rotate_size(size: PixelSize, rotation_degrees: i32) -> PixelSize {
    match rotation_degrees {
        90 | 270 => PixelSize {
            width: size.height,
            height: size.width,
        },
        _ => size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_rect_letterboxes_tall_device_in_wide_window() {
        // 1080x2400 portrait device into a 1000x1000 square window.
        let window = RectF {
            x: 100.0,
            y: 50.0,
            width: 1000.0,
            height: 1000.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let rect = content_rect(
            &window,
            PixelSize {
                width: 1080,
                height: 2400,
            },
            0,
        )
        .expect("content rect");
        // Height-constrained: scale = 1000/2400.
        let expected_scale = 1000.0 / 2400.0;
        assert!((rect.host_scale - expected_scale).abs() < 1e-9);
        // Content width is narrower than the window: horizontal letterbox.
        let expected_w = 1080.0 * expected_scale;
        assert!((rect.content_rect.width - expected_w).abs() < 1e-6);
        assert!((rect.content_rect.height - 1000.0).abs() < 1e-6);
        // Centered horizontally, full height, offset by window origin.
        let expected_x = 100.0 + (1000.0 - expected_w) / 2.0;
        assert!((rect.content_rect.x - expected_x).abs() < 1e-6);
        assert!((rect.content_rect.y - 50.0).abs() < 1e-6);
    }

    #[test]
    fn content_rect_rotation_swaps_axes() {
        // 1080x2400 device rotated 90deg renders as 2400x1080 landscape.
        let window = RectF {
            x: 0.0,
            y: 0.0,
            width: 2400.0,
            height: 1080.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let rect = content_rect(
            &window,
            PixelSize {
                width: 1080,
                height: 2400,
            },
            90,
        )
        .expect("content rect");
        assert_eq!(rect.rotation_degrees, 90);
        assert_eq!(
            rect.rotated_device_size,
            PixelSize {
                width: 2400,
                height: 1080
            }
        );
        // Exact fit: scale 1.0, no letterbox.
        assert!((rect.host_scale - 1.0).abs() < 1e-9);
        assert!((rect.content_rect.width - 2400.0).abs() < 1e-6);
        assert!((rect.content_rect.height - 1080.0).abs() < 1e-6);
    }

    #[test]
    fn content_rect_at_180_keeps_portrait_extent_and_maps_interior_point() {
        // 180 is upside-down portrait: the rotated extent equals the natural
        // extent (no axis swap), but the quarter must still be honored as 180,
        // not collapsed to 0. Use a non-origin window and an interior device
        // point so the offset and scale are both exercised (the origin alone is a
        // weak oracle).
        let window = RectF {
            x: 200.0,
            y: 100.0,
            width: 540.0,
            height: 1200.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let rect = content_rect(
            &window,
            PixelSize {
                width: 1080,
                height: 2400,
            },
            180,
        )
        .expect("content rect");
        assert_eq!(rect.rotation_degrees, 180);
        // No axis swap at 180: rotated extent is the natural portrait size.
        assert_eq!(
            rect.rotated_device_size,
            PixelSize {
                width: 1080,
                height: 2400
            }
        );
        // Exact fit: scale 0.5, no letterbox, content origin at the window origin.
        assert!((rect.host_scale - 0.5).abs() < 1e-9);
        assert!((rect.content_rect.x - 200.0).abs() < 1e-6);
        assert!((rect.content_rect.y - 100.0).abs() < 1e-6);
        // Interior device point -> host: (200 + 600*0.5, 100 + 800*0.5).
        let (hx, hy) = rect.device_to_host(600.0, 800.0);
        assert!((hx - 500.0).abs() < 1e-6);
        assert!((hy - 500.0).abs() < 1e-6);
    }

    #[test]
    fn content_rect_at_270_swaps_axes_and_maps_interior_point() {
        // 270 shares the landscape axis-swap with 90, so a label-derived path
        // that collapsed 270 -> 90 would still swap axes and look "right" at the
        // origin. The content-rect geometry is identical for 90 and 270 (both
        // rotate the size), so this asserts 270 is honored as 270 here and that
        // an interior point maps through the offset/scale correctly; the 90-vs-270
        // *direction* difference is pinned by the natural-frame mapping test.
        let window = RectF {
            x: 300.0,
            y: 50.0,
            width: 1200.0,
            height: 540.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let rect = content_rect(
            &window,
            PixelSize {
                width: 1080,
                height: 2400,
            },
            270,
        )
        .expect("content rect");
        assert_eq!(rect.rotation_degrees, 270);
        // 270 swaps axes: 1080x2400 portrait renders as 2400x1080 landscape.
        assert_eq!(
            rect.rotated_device_size,
            PixelSize {
                width: 2400,
                height: 1080
            }
        );
        // Exact fit: scale 0.5, no letterbox, content origin at the window origin.
        assert!((rect.host_scale - 0.5).abs() < 1e-9);
        assert!((rect.content_rect.x - 300.0).abs() < 1e-6);
        assert!((rect.content_rect.y - 50.0).abs() < 1e-6);
        // Interior point in the rotated frame -> host: (300 + 800*0.5, 50 + 400*0.5).
        let (hx, hy) = rect.device_to_host(800.0, 400.0);
        assert!((hx - 700.0).abs() < 1e-6);
        assert!((hy - 250.0).abs() < 1e-6);
    }

    #[test]
    fn content_rect_preserves_fractional_host_scale() {
        // 1000x2000 device into a 150x300 window: scale 0.15 (fractional).
        let window = RectF {
            x: 0.0,
            y: 0.0,
            width: 150.0,
            height: 300.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let rect = content_rect(
            &window,
            PixelSize {
                width: 1000,
                height: 2000,
            },
            0,
        )
        .expect("content rect");
        assert!((rect.host_scale - 0.15).abs() < 1e-9);
    }

    #[test]
    fn content_rect_rejects_degenerate_inputs() {
        let zero_window = RectF {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        };
        assert!(
            content_rect(
                &zero_window,
                PixelSize {
                    width: 100,
                    height: 100
                },
                0
            )
            .is_none()
        );

        let good_window = RectF {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        };
        assert!(
            content_rect(
                &good_window,
                PixelSize {
                    width: 0,
                    height: 100
                },
                0
            )
            .is_none()
        );
    }

    #[test]
    fn device_host_round_trips_inside_content() {
        let window = RectF {
            x: 100.0,
            y: 50.0,
            width: 1000.0,
            height: 1000.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let rect = content_rect(
            &window,
            PixelSize {
                width: 1080,
                height: 2400,
            },
            0,
        )
        .expect("content rect");
        // Device center maps to host, then back to the same device point.
        let (hx, hy) = rect.device_to_host(540.0, 1200.0);
        let (dx, dy) = rect.host_to_device(hx, hy).expect("inside content");
        assert!((dx - 540.0).abs() < 1e-6);
        assert!((dy - 1200.0).abs() < 1e-6);
    }

    #[test]
    fn host_to_device_rejects_letterbox_clicks() {
        let window = RectF {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let rect = content_rect(
            &window,
            PixelSize {
                width: 1080,
                height: 2400,
            },
            0,
        )
        .expect("content rect");
        // A click in the left letterbox bar (x=0) is outside the centered content.
        assert!(rect.host_to_device(0.0, 500.0).is_none());
        // A click well inside the content maps successfully.
        assert!(
            rect.host_to_device(rect.content_rect.x + 5.0, 500.0)
                .is_some()
        );
    }

    #[test]
    fn content_rect_rejects_non_desktop_logical_space() {
        // A host window in any space other than DesktopLogical (the plane the
        // overlay draws on) must yield no mapping, so a space mismatch fails the
        // mapping instead of misplacing the cursor.
        for space in [
            CoordinateSpace::StreamPixels,
            CoordinateSpace::StreamLogical,
        ] {
            let window = RectF {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0,
                space: space.clone(),
            };
            assert!(
                content_rect(
                    &window,
                    PixelSize {
                        width: 1080,
                        height: 2400,
                    },
                    0,
                )
                .is_none(),
                "non-DesktopLogical space {space:?} must not produce a content rect"
            );
        }
    }

    #[test]
    fn normalize_rotation_snaps_to_quarter_turns() {
        assert_eq!(normalize_rotation(0), 0);
        assert_eq!(normalize_rotation(10), 0);
        assert_eq!(normalize_rotation(90), 90);
        assert_eq!(normalize_rotation(180), 180);
        assert_eq!(normalize_rotation(270), 270);
        assert_eq!(normalize_rotation(-90), 270);
        assert_eq!(normalize_rotation(450), 90);
    }
}
