use sky_cua_platform::model::{CoordinateSpace, PixelSize, RectF};

#[must_use]
pub fn center_of(bounds: &RectF) -> (f64, f64) {
    (
        bounds.x + (bounds.width / 2.0),
        bounds.y + (bounds.height / 2.0),
    )
}

#[must_use]
pub fn logical_to_pixel(
    point: (f64, f64),
    logical_rect: &RectF,
    pixel_size: &PixelSize,
) -> Option<(f64, f64)> {
    if logical_rect.space != CoordinateSpace::DesktopLogical
        && logical_rect.space != CoordinateSpace::StreamLogical
    {
        return None;
    }
    if logical_rect.width <= 0.0 || logical_rect.height <= 0.0 {
        return None;
    }
    let rel_x = (point.0 - logical_rect.x) / logical_rect.width;
    let rel_y = (point.1 - logical_rect.y) / logical_rect.height;
    Some((
        rel_x * f64::from(pixel_size.width),
        rel_y * f64::from(pixel_size.height),
    ))
}

#[must_use]
pub fn desktop_to_stream(point: (f64, f64), logical_rect: &RectF) -> Option<(f64, f64)> {
    if logical_rect.space != CoordinateSpace::DesktopLogical {
        return None;
    }
    Some((point.0 - logical_rect.x, point.1 - logical_rect.y))
}

#[cfg(test)]
mod tests {
    use super::{center_of, desktop_to_stream, logical_to_pixel};
    use sky_cua_platform::model::{CoordinateSpace, PixelSize, RectF};

    #[test]
    fn maps_center_into_pixels() {
        let rect = RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let center = center_of(&rect);
        let pixel = logical_to_pixel(
            center,
            &rect,
            &PixelSize {
                width: 1000,
                height: 500,
            },
        )
        .expect("mapping should exist");
        assert_eq!(pixel, (500.0, 250.0));
    }

    #[test]
    fn maps_desktop_point_into_stream_coordinates() {
        let rect = RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        };
        let stream = desktop_to_stream((125.0, 70.0), &rect).expect("mapping should exist");
        assert_eq!(stream, (25.0, 20.0));
    }
}
