#[cfg(test)]
mod tests {
    use crate::overlay_spec;

    #[test]
    fn schema_version_matches() {
        assert_eq!(overlay_spec::SCHEMA_VERSION, 1);
    }

    #[test]
    fn shared_timing_positive() {
        assert!(overlay_spec::shared::timing::MIN_GESTURE_DURATION_MS > 0);
        assert!(
            overlay_spec::shared::timing::MIN_GESTURE_DURATION_MS
                <= overlay_spec::shared::timing::MAX_GESTURE_DURATION_MS
        );
    }

    #[test]
    fn shared_motion_speeds_finite() {
        assert!(overlay_spec::shared::motion::CURSOR_MAX_SPEED_DP_PER_S.is_finite());
        assert!(overlay_spec::shared::motion::CURSOR_ACCEL_DP_PER_S2.is_finite());
        assert!(overlay_spec::shared::motion::CURSOR_TURN_RATE_DEG_PER_S.is_finite());
    }

    #[test]
    fn alpha_ranges() {
        let min = overlay_spec::shared::effects::GLOW_BASELINE_MIN_ALPHA_0_1;
        let max = overlay_spec::shared::effects::GLOW_BASELINE_MAX_ALPHA_0_1;
        let peak = overlay_spec::shared::effects::GLOW_PULSE_PEAK_ALPHA_0_1;
        assert!((0.0..=1.0).contains(&min));
        assert!((0.0..=1.0).contains(&max));
        assert!((0.0..=1.0).contains(&peak));
        assert!(min <= max && max <= peak);
    }

    #[test]
    fn hotspot_fractions_inside_unit_square() {
        let hx = overlay_spec::shared::effects::CURSOR_HOTSPOT_FRACTION_X;
        let hy = overlay_spec::shared::effects::CURSOR_HOTSPOT_FRACTION_Y;
        assert!((0.0..=1.0).contains(&hx));
        assert!((0.0..=1.0).contains(&hy));
    }

    #[test]
    fn max_gesture_points_bounded() {
        assert!(overlay_spec::shared::effects::MAX_GESTURE_POINTS >= 1);
        assert!(overlay_spec::shared::effects::MAX_GESTURE_POINTS <= 1024);
    }

    #[test]
    fn android_desktop_geometry_match_in_dp_and_logical_px() {
        // The two platforms currently share the same visual proportions; this
        // test guards against accidental drift while the presentation units are
        // allowed to diverge in later work.
        assert!(
            (overlay_spec::android::geometry::CURSOR_HEIGHT_DP
                - overlay_spec::desktop::geometry::CURSOR_HEIGHT_LOGICAL_PX)
                .abs()
                < f64::EPSILON
        );
    }
}
