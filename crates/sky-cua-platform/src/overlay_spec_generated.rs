// GENERATED FILE - DO NOT EDIT
// Source: resources/overlay/agent_overlay_spec.toml
// Schema version: 1
// Generator: scripts/generate_overlay_spec.py
// Generator hash: e1f65e5d3f99748f

#![allow(missing_docs)]

/// Generated overlay specification constants.
pub mod overlay_spec {
    /// Schema version of the source spec.
    pub const SCHEMA_VERSION: u32 = 1;

    /// `shared` constants.
    pub mod shared {
        /// `colors` constants.
        pub mod colors {
            /// `agent_pink_red_0_255` from `[shared.colors]`.
            pub const AGENT_PINK_RED_0_255: u8 = 255;
            /// `agent_pink_green_0_255` from `[shared.colors]`.
            pub const AGENT_PINK_GREEN_0_255: u8 = 96;
            /// `agent_pink_blue_0_255` from `[shared.colors]`.
            pub const AGENT_PINK_BLUE_0_255: u8 = 172;
            /// `agent_pink_light_red_0_255` from `[shared.colors]`.
            pub const AGENT_PINK_LIGHT_RED_0_255: u8 = 255;
            /// `agent_pink_light_green_0_255` from `[shared.colors]`.
            pub const AGENT_PINK_LIGHT_GREEN_0_255: u8 = 150;
            /// `agent_pink_light_blue_0_255` from `[shared.colors]`.
            pub const AGENT_PINK_LIGHT_BLUE_0_255: u8 = 205;
            /// `halo_inner_alpha_0_255` from `[shared.colors]`.
            pub const HALO_INNER_ALPHA_0_255: u8 = 205;
            /// `halo_inner_red_0_255` from `[shared.colors]`.
            pub const HALO_INNER_RED_0_255: u8 = 255;
            /// `halo_inner_green_0_255` from `[shared.colors]`.
            pub const HALO_INNER_GREEN_0_255: u8 = 118;
            /// `halo_inner_blue_0_255` from `[shared.colors]`.
            pub const HALO_INNER_BLUE_0_255: u8 = 188;
            /// `halo_mid_alpha_0_255` from `[shared.colors]`.
            pub const HALO_MID_ALPHA_0_255: u8 = 90;
            /// `halo_mid_red_0_255` from `[shared.colors]`.
            pub const HALO_MID_RED_0_255: u8 = 255;
            /// `halo_mid_green_0_255` from `[shared.colors]`.
            pub const HALO_MID_GREEN_0_255: u8 = 118;
            /// `halo_mid_blue_0_255` from `[shared.colors]`.
            pub const HALO_MID_BLUE_0_255: u8 = 188;
            /// `halo_outer_alpha_0_255` from `[shared.colors]`.
            pub const HALO_OUTER_ALPHA_0_255: u8 = 0;
            /// `halo_outer_red_0_255` from `[shared.colors]`.
            pub const HALO_OUTER_RED_0_255: u8 = 255;
            /// `halo_outer_green_0_255` from `[shared.colors]`.
            pub const HALO_OUTER_GREEN_0_255: u8 = 118;
            /// `halo_outer_blue_0_255` from `[shared.colors]`.
            pub const HALO_OUTER_BLUE_0_255: u8 = 188;
        }
        /// `timing` constants.
        pub mod timing {
            /// `min_gesture_duration_ms` from `[shared.timing]`.
            pub const MIN_GESTURE_DURATION_MS: u64 = 120;
            /// `max_gesture_duration_ms` from `[shared.timing]`.
            pub const MAX_GESTURE_DURATION_MS: u64 = 60000;
            /// `breathe_period_ms` from `[shared.timing]`.
            pub const BREATHE_PERIOD_MS: u64 = 1600;
            /// `wave_period_ms` from `[shared.timing]`.
            pub const WAVE_PERIOD_MS: u64 = 2600;
            /// `halo_breathe_period_ms` from `[shared.timing]`.
            pub const HALO_BREATHE_PERIOD_MS: u64 = 2000;
            /// `click_feedback_ms` from `[shared.timing]`.
            pub const CLICK_FEEDBACK_MS: u64 = 380;
            /// `ripple_burst_ms` from `[shared.timing]`.
            pub const RIPPLE_BURST_MS: u64 = 380;
            /// `swipe_visual_min_ms` from `[shared.timing]`.
            pub const SWIPE_VISUAL_MIN_MS: u64 = 950;
            /// `no_no_wiggle_ms` from `[shared.timing]`.
            pub const NO_NO_WIGGLE_MS: u64 = 760;
            /// `catcher_idle_ms` from `[shared.timing]`.
            pub const CATCHER_IDLE_MS: u64 = 900;
        }
        /// `motion` constants.
        pub mod motion {
            /// `cursor_max_speed_dp_per_s` from `[shared.motion]`.
            pub const CURSOR_MAX_SPEED_DP_PER_S: f64 = 950.0;
            /// `cursor_accel_dp_per_s2` from `[shared.motion]`.
            pub const CURSOR_ACCEL_DP_PER_S2: f64 = 5200.0;
            /// `cursor_turn_rate_deg_per_s` from `[shared.motion]`.
            pub const CURSOR_TURN_RATE_DEG_PER_S: f64 = 300.0;
            /// `cursor_arrive_radius_dp` from `[shared.motion]`.
            pub const CURSOR_ARRIVE_RADIUS_DP: f64 = 95.0;
            /// `cursor_homing_radius_dp` from `[shared.motion]`.
            pub const CURSOR_HOMING_RADIUS_DP: f64 = 240.0;
            /// `cursor_homing_turn_boost` from `[shared.motion]`.
            pub const CURSOR_HOMING_TURN_BOOST: f64 = 3.5;
            /// `cursor_max_step_s` from `[shared.motion]`.
            pub const CURSOR_MAX_STEP_S: f64 = 0.04;
            /// `cursor_settle_px` from `[shared.motion]`.
            pub const CURSOR_SETTLE_PX: f64 = 1.5;
            /// `cursor_nose_deg` from `[shared.motion]`.
            pub const CURSOR_NOSE_DEG: f64 = -135.0;
            /// `cursor_rotate_min_speed_dp_per_s` from `[shared.motion]`.
            pub const CURSOR_ROTATE_MIN_SPEED_DP_PER_S: f64 = 80.0;
            /// `cursor_rotate_rate_deg_per_s` from `[shared.motion]`.
            pub const CURSOR_ROTATE_RATE_DEG_PER_S: f64 = 520.0;
        }
        /// `effects` constants.
        pub mod effects {
            /// `glow_baseline_min_alpha_0_1` from `[shared.effects]`.
            pub const GLOW_BASELINE_MIN_ALPHA_0_1: f64 = 0.55;
            /// `glow_baseline_max_alpha_0_1` from `[shared.effects]`.
            pub const GLOW_BASELINE_MAX_ALPHA_0_1: f64 = 0.92;
            /// `glow_pulse_peak_alpha_0_1` from `[shared.effects]`.
            pub const GLOW_PULSE_PEAK_ALPHA_0_1: f64 = 1.0;
            /// `cursor_press_scale_fraction` from `[shared.effects]`.
            pub const CURSOR_PRESS_SCALE_FRACTION: f64 = 0.6;
            /// `press_in_fraction` from `[shared.effects]`.
            pub const PRESS_IN_FRACTION: f64 = 0.14;
            /// `bounce_damp` from `[shared.effects]`.
            pub const BOUNCE_DAMP: f64 = 1.7;
            /// `bounce_omega_pi_fraction` from `[shared.effects]`.
            pub const BOUNCE_OMEGA_PI_FRACTION: f64 = 1.5;
            /// `no_no_shakes_fraction` from `[shared.effects]`.
            pub const NO_NO_SHAKES_FRACTION: f64 = 1.5;
            /// `no_no_hold_fraction` from `[shared.effects]`.
            pub const NO_NO_HOLD_FRACTION: f64 = 0.78;
            /// `no_no_wiggle_deg` from `[shared.effects]`.
            pub const NO_NO_WIGGLE_DEG: f64 = 20.0;
            /// `max_gesture_points` from `[shared.effects]`.
            pub const MAX_GESTURE_POINTS: u32 = 16;
            /// `capture_barrier_frames` from `[shared.effects]`.
            pub const CAPTURE_BARRIER_FRAMES: u32 = 2;
            /// `cursor_source_viewbox_width` from `[shared.effects]`.
            pub const CURSOR_SOURCE_VIEWBOX_WIDTH: u32 = 23;
            /// `cursor_source_viewbox_height` from `[shared.effects]`.
            pub const CURSOR_SOURCE_VIEWBOX_HEIGHT: u32 = 24;
            /// `cursor_hotspot_fraction_x` from `[shared.effects]`.
            pub const CURSOR_HOTSPOT_FRACTION_X: f64 = 0.43478260869565216;
            /// `cursor_hotspot_fraction_y` from `[shared.effects]`.
            pub const CURSOR_HOTSPOT_FRACTION_Y: f64 = 0.4583333333333333;
            /// `glyph_fill_red_0_1` from `[shared.effects]`.
            pub const GLYPH_FILL_RED_0_1: f64 = 0.022;
            /// `glyph_fill_green_0_1` from `[shared.effects]`.
            pub const GLYPH_FILL_GREEN_0_1: f64 = 0.006;
            /// `glyph_fill_blue_0_1` from `[shared.effects]`.
            pub const GLYPH_FILL_BLUE_0_1: f64 = 0.038;
            /// `glyph_edge_white_mix_0_1` from `[shared.effects]`.
            pub const GLYPH_EDGE_WHITE_MIX_0_1: f64 = 0.5;
            /// `cursor_stroke_edge_0_1` from `[shared.effects]`.
            pub const CURSOR_STROKE_EDGE_0_1: f64 = 0.15;
            /// `cursor_smoke_offset_x_uv` from `[shared.effects]`.
            pub const CURSOR_SMOKE_OFFSET_X_UV: f64 = 0.018;
            /// `cursor_smoke_offset_y_uv` from `[shared.effects]`.
            pub const CURSOR_SMOKE_OFFSET_Y_UV: f64 = 0.022;
            /// `cursor_shadow_reach_0_1` from `[shared.effects]`.
            pub const CURSOR_SHADOW_REACH_0_1: f64 = 0.48;
            /// `cursor_shadow_falloff_0_1` from `[shared.effects]`.
            pub const CURSOR_SHADOW_FALLOFF_0_1: f64 = 0.62;
            /// `cursor_shadow_strength_0_1` from `[shared.effects]`.
            pub const CURSOR_SHADOW_STRENGTH_0_1: f64 = 0.5;
            /// `cursor_shadow_lod` from `[shared.effects]`.
            pub const CURSOR_SHADOW_LOD: f64 = 3.0;
        }
    }

    /// `desktop` constants.
    pub mod desktop {
        /// `geometry` constants.
        pub mod geometry {
            /// `cursor_height_logical_px` from `[desktop.geometry]`.
            pub const CURSOR_HEIGHT_LOGICAL_PX: f64 = 35.9375;
            /// `cursor_halo_radius_logical_px` from `[desktop.geometry]`.
            pub const CURSOR_HALO_RADIUS_LOGICAL_PX: f64 = 23.4375;
            /// `ripple_min_logical_px` from `[desktop.geometry]`.
            pub const RIPPLE_MIN_LOGICAL_PX: f64 = 20.0;
            /// `ripple_max_logical_px` from `[desktop.geometry]`.
            pub const RIPPLE_MAX_LOGICAL_PX: f64 = 64.0;
            /// `gesture_arrive_logical_px` from `[desktop.geometry]`.
            pub const GESTURE_ARRIVE_LOGICAL_PX: f64 = 10.0;
            /// `catcher_logical_px` from `[desktop.geometry]`.
            pub const CATCHER_LOGICAL_PX: f64 = 64.0;
            /// `glow_base_stroke_logical_px` from `[desktop.geometry]`.
            pub const GLOW_BASE_STROKE_LOGICAL_PX: f64 = 14.0;
            /// `glow_base_blur_logical_px` from `[desktop.geometry]`.
            pub const GLOW_BASE_BLUR_LOGICAL_PX: f64 = 52.0;
            /// `glow_core_stroke_logical_px` from `[desktop.geometry]`.
            pub const GLOW_CORE_STROKE_LOGICAL_PX: f64 = 4.0;
            /// `glow_core_blur_logical_px` from `[desktop.geometry]`.
            pub const GLOW_CORE_BLUR_LOGICAL_PX: f64 = 12.0;
            /// `glow_edge_inset_logical_px` from `[desktop.geometry]`.
            pub const GLOW_EDGE_INSET_LOGICAL_PX: f64 = 2.0;
            /// `glow_corner_logical_px` from `[desktop.geometry]`.
            pub const GLOW_CORNER_LOGICAL_PX: f64 = 46.0;
            /// `wave_stroke_logical_px` from `[desktop.geometry]`.
            pub const WAVE_STROKE_LOGICAL_PX: f64 = 4.0;
            /// `wave_blur_logical_px` from `[desktop.geometry]`.
            pub const WAVE_BLUR_LOGICAL_PX: f64 = 22.0;
            /// `ripple_stroke_logical_px` from `[desktop.geometry]`.
            pub const RIPPLE_STROKE_LOGICAL_PX: f64 = 16.0;
            /// `ripple_blur_logical_px` from `[desktop.geometry]`.
            pub const RIPPLE_BLUR_LOGICAL_PX: f64 = 14.0;
            /// `trail_stroke_logical_px` from `[desktop.geometry]`.
            pub const TRAIL_STROKE_LOGICAL_PX: f64 = 6.0;
        }
        /// `rendering` constants.
        pub mod rendering {
            /// `wave_count` from `[desktop.rendering]`.
            pub const WAVE_COUNT: u32 = 2;
            /// `wave_travel_fraction` from `[desktop.rendering]`.
            pub const WAVE_TRAVEL_FRACTION: f64 = 0.05;
            /// `wave_fade_in_fraction` from `[desktop.rendering]`.
            pub const WAVE_FADE_IN_FRACTION: f64 = 0.18;
            /// `wave_max_alpha_0_255` from `[desktop.rendering]`.
            pub const WAVE_MAX_ALPHA_0_255: u8 = 25;
            /// `max_base_alpha_0_255` from `[desktop.rendering]`.
            pub const MAX_BASE_ALPHA_0_255: u8 = 200;
            /// `max_core_alpha_0_255` from `[desktop.rendering]`.
            pub const MAX_CORE_ALPHA_0_255: u8 = 220;
            /// `max_ripple_alpha_0_255` from `[desktop.rendering]`.
            pub const MAX_RIPPLE_ALPHA_0_255: u8 = 215;
            /// `max_trail_alpha_0_255` from `[desktop.rendering]`.
            pub const MAX_TRAIL_ALPHA_0_255: u8 = 190;
            /// `trail_max_points` from `[desktop.rendering]`.
            pub const TRAIL_MAX_POINTS: u32 = 24;
            /// `shadow_dx_viewbox_fraction` from `[desktop.rendering]`.
            pub const SHADOW_DX_VIEWBOX_FRACTION: f64 = 0.5;
            /// `shadow_dy_viewbox_fraction` from `[desktop.rendering]`.
            pub const SHADOW_DY_VIEWBOX_FRACTION: f64 = 1.3;
            /// `shadow_blur_viewbox_fraction` from `[desktop.rendering]`.
            pub const SHADOW_BLUR_VIEWBOX_FRACTION: f64 = 1.1;
            /// `shadow_alpha_0_1` from `[desktop.rendering]`.
            pub const SHADOW_ALPHA_0_1: f64 = 0.58;
            /// `halo_scale_min_fraction` from `[desktop.rendering]`.
            pub const HALO_SCALE_MIN_FRACTION: f64 = 0.85;
            /// `halo_scale_max_fraction` from `[desktop.rendering]`.
            pub const HALO_SCALE_MAX_FRACTION: f64 = 1.1;
            /// `halo_alpha_min_fraction` from `[desktop.rendering]`.
            pub const HALO_ALPHA_MIN_FRACTION: f64 = 0.5;
            /// `halo_alpha_max_fraction` from `[desktop.rendering]`.
            pub const HALO_ALPHA_MAX_FRACTION: f64 = 1.0;
            /// `viewbox_height` from `[desktop.rendering]`.
            pub const VIEWBOX_HEIGHT: u32 = 48;
        }
    }

    /// `android` constants.
    pub mod android {
        /// `geometry` constants.
        pub mod geometry {
            /// `cursor_height_dp` from `[android.geometry]`.
            pub const CURSOR_HEIGHT_DP: f64 = 35.9375;
            /// `cursor_halo_radius_dp` from `[android.geometry]`.
            pub const CURSOR_HALO_RADIUS_DP: f64 = 23.4375;
            /// `ripple_min_dp` from `[android.geometry]`.
            pub const RIPPLE_MIN_DP: f64 = 20.0;
            /// `ripple_max_dp` from `[android.geometry]`.
            pub const RIPPLE_MAX_DP: f64 = 64.0;
            /// `gesture_arrive_dp` from `[android.geometry]`.
            pub const GESTURE_ARRIVE_DP: f64 = 10.0;
            /// `catcher_dp` from `[android.geometry]`.
            pub const CATCHER_DP: f64 = 64.0;
            /// `glow_base_stroke_dp` from `[android.geometry]`.
            pub const GLOW_BASE_STROKE_DP: f64 = 22.0;
            /// `glow_base_blur_dp` from `[android.geometry]`.
            pub const GLOW_BASE_BLUR_DP: f64 = 22.0;
            /// `glow_core_stroke_dp` from `[android.geometry]`.
            pub const GLOW_CORE_STROKE_DP: f64 = 6.0;
            /// `glow_core_blur_dp` from `[android.geometry]`.
            pub const GLOW_CORE_BLUR_DP: f64 = 9.0;
            /// `glow_edge_inset_dp` from `[android.geometry]`.
            pub const GLOW_EDGE_INSET_DP: f64 = 2.0;
            /// `glow_corner_dp` from `[android.geometry]`.
            pub const GLOW_CORNER_DP: f64 = 46.0;
            /// `wave_stroke_dp` from `[android.geometry]`.
            pub const WAVE_STROKE_DP: f64 = 5.0;
            /// `wave_blur_dp` from `[android.geometry]`.
            pub const WAVE_BLUR_DP: f64 = 9.0;
            /// `ripple_stroke_dp` from `[android.geometry]`.
            pub const RIPPLE_STROKE_DP: f64 = 16.0;
            /// `ripple_blur_dp` from `[android.geometry]`.
            pub const RIPPLE_BLUR_DP: f64 = 14.0;
            /// `trail_stroke_dp` from `[android.geometry]`.
            pub const TRAIL_STROKE_DP: f64 = 6.0;
        }
        /// `rendering` constants.
        pub mod rendering {
            /// `wave_count` from `[android.rendering]`.
            pub const WAVE_COUNT: u32 = 3;
            /// `wave_travel_fraction` from `[android.rendering]`.
            pub const WAVE_TRAVEL_FRACTION: f64 = 0.2;
            /// `wave_fade_in_fraction` from `[android.rendering]`.
            pub const WAVE_FADE_IN_FRACTION: f64 = 0.12;
            /// `wave_max_alpha_0_255` from `[android.rendering]`.
            pub const WAVE_MAX_ALPHA_0_255: u8 = 165;
            /// `max_base_alpha_0_255` from `[android.rendering]`.
            pub const MAX_BASE_ALPHA_0_255: u8 = 200;
            /// `max_core_alpha_0_255` from `[android.rendering]`.
            pub const MAX_CORE_ALPHA_0_255: u8 = 220;
            /// `max_ripple_alpha_0_255` from `[android.rendering]`.
            pub const MAX_RIPPLE_ALPHA_0_255: u8 = 215;
            /// `max_trail_alpha_0_255` from `[android.rendering]`.
            pub const MAX_TRAIL_ALPHA_0_255: u8 = 190;
            /// `trail_max_points` from `[android.rendering]`.
            pub const TRAIL_MAX_POINTS: u32 = 24;
            /// `shadow_dx_viewbox_fraction` from `[android.rendering]`.
            pub const SHADOW_DX_VIEWBOX_FRACTION: f64 = 0.5;
            /// `shadow_dy_viewbox_fraction` from `[android.rendering]`.
            pub const SHADOW_DY_VIEWBOX_FRACTION: f64 = 1.3;
            /// `shadow_blur_viewbox_fraction` from `[android.rendering]`.
            pub const SHADOW_BLUR_VIEWBOX_FRACTION: f64 = 1.1;
            /// `shadow_alpha_0_1` from `[android.rendering]`.
            pub const SHADOW_ALPHA_0_1: f64 = 0.58;
            /// `halo_scale_min_fraction` from `[android.rendering]`.
            pub const HALO_SCALE_MIN_FRACTION: f64 = 0.85;
            /// `halo_scale_max_fraction` from `[android.rendering]`.
            pub const HALO_SCALE_MAX_FRACTION: f64 = 1.1;
            /// `halo_alpha_min_fraction` from `[android.rendering]`.
            pub const HALO_ALPHA_MIN_FRACTION: f64 = 0.5;
            /// `halo_alpha_max_fraction` from `[android.rendering]`.
            pub const HALO_ALPHA_MAX_FRACTION: f64 = 1.0;
            /// `viewbox_height` from `[android.rendering]`.
            pub const VIEWBOX_HEIGHT: u32 = 48;
        }
    }

    /// `sound` constants.
    pub mod sound {
        /// `enabled` from `[sound]`.
        pub const ENABLED: bool = false;
        /// `no_no_sound_asset` from `[sound]`.
        pub const NO_NO_SOUND_ASSET: &'static str = "";
    }
}
