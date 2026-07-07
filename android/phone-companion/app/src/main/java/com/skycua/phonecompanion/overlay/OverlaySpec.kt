package com.skycua.phonecompanion.overlay

/**
 * Generated overlay specification constants.
 * Source: resources/overlay/agent_overlay_spec.toml
 * Schema version: 1
 * Generator: scripts/generate_overlay_spec.py
 * Generator hash: d0bc5ab850e68ac4
 * GENERATED FILE - DO NOT EDIT
 * Run `uv run python scripts/generate_overlay_spec.py` to regenerate.
 */
object OverlaySpec {
    const val SCHEMA_VERSION: Int = 1

    /** `shared` constants. */
    object Shared {
        /** `colors` constants. */
        object Colors {
            /** `agent_pink_red_0_255` from `[shared.colors]`. */
            const val AGENT_PINK_RED_0_255: Int = 255
            /** `agent_pink_green_0_255` from `[shared.colors]`. */
            const val AGENT_PINK_GREEN_0_255: Int = 96
            /** `agent_pink_blue_0_255` from `[shared.colors]`. */
            const val AGENT_PINK_BLUE_0_255: Int = 172
            /** `agent_pink_light_red_0_255` from `[shared.colors]`. */
            const val AGENT_PINK_LIGHT_RED_0_255: Int = 255
            /** `agent_pink_light_green_0_255` from `[shared.colors]`. */
            const val AGENT_PINK_LIGHT_GREEN_0_255: Int = 150
            /** `agent_pink_light_blue_0_255` from `[shared.colors]`. */
            const val AGENT_PINK_LIGHT_BLUE_0_255: Int = 205
            /** `halo_inner_alpha_0_255` from `[shared.colors]`. */
            const val HALO_INNER_ALPHA_0_255: Int = 205
            /** `halo_inner_red_0_255` from `[shared.colors]`. */
            const val HALO_INNER_RED_0_255: Int = 255
            /** `halo_inner_green_0_255` from `[shared.colors]`. */
            const val HALO_INNER_GREEN_0_255: Int = 118
            /** `halo_inner_blue_0_255` from `[shared.colors]`. */
            const val HALO_INNER_BLUE_0_255: Int = 188
            /** `halo_mid_alpha_0_255` from `[shared.colors]`. */
            const val HALO_MID_ALPHA_0_255: Int = 90
            /** `halo_mid_red_0_255` from `[shared.colors]`. */
            const val HALO_MID_RED_0_255: Int = 255
            /** `halo_mid_green_0_255` from `[shared.colors]`. */
            const val HALO_MID_GREEN_0_255: Int = 118
            /** `halo_mid_blue_0_255` from `[shared.colors]`. */
            const val HALO_MID_BLUE_0_255: Int = 188
            /** `halo_outer_alpha_0_255` from `[shared.colors]`. */
            const val HALO_OUTER_ALPHA_0_255: Int = 0
            /** `halo_outer_red_0_255` from `[shared.colors]`. */
            const val HALO_OUTER_RED_0_255: Int = 255
            /** `halo_outer_green_0_255` from `[shared.colors]`. */
            const val HALO_OUTER_GREEN_0_255: Int = 118
            /** `halo_outer_blue_0_255` from `[shared.colors]`. */
            const val HALO_OUTER_BLUE_0_255: Int = 188
        }

        /** `timing` constants. */
        object Timing {
            /** `min_gesture_duration_ms` from `[shared.timing]`. */
            const val MIN_GESTURE_DURATION_MS: Long = 120L
            /** `max_gesture_duration_ms` from `[shared.timing]`. */
            const val MAX_GESTURE_DURATION_MS: Long = 60000L
            /** `breathe_period_ms` from `[shared.timing]`. */
            const val BREATHE_PERIOD_MS: Long = 1600L
            /** `wave_period_ms` from `[shared.timing]`. */
            const val WAVE_PERIOD_MS: Long = 2600L
            /** `halo_breathe_period_ms` from `[shared.timing]`. */
            const val HALO_BREATHE_PERIOD_MS: Long = 2000L
            /** `click_feedback_ms` from `[shared.timing]`. */
            const val CLICK_FEEDBACK_MS: Long = 380L
            /** `ripple_burst_ms` from `[shared.timing]`. */
            const val RIPPLE_BURST_MS: Long = 380L
            /** `swipe_visual_min_ms` from `[shared.timing]`. */
            const val SWIPE_VISUAL_MIN_MS: Long = 950L
            /** `no_no_wiggle_ms` from `[shared.timing]`. */
            const val NO_NO_WIGGLE_MS: Long = 760L
            /** `catcher_idle_ms` from `[shared.timing]`. */
            const val CATCHER_IDLE_MS: Long = 900L
        }

        /** `motion` constants. */
        object Motion {
            /** `cursor_max_speed_dp_per_s` from `[shared.motion]`. */
            const val CURSOR_MAX_SPEED_DP_PER_S: Double = 950.0
            /** `cursor_accel_dp_per_s2` from `[shared.motion]`. */
            const val CURSOR_ACCEL_DP_PER_S2: Double = 5200.0
            /** `cursor_turn_rate_deg_per_s` from `[shared.motion]`. */
            const val CURSOR_TURN_RATE_DEG_PER_S: Double = 300.0
            /** `cursor_arrive_radius_dp` from `[shared.motion]`. */
            const val CURSOR_ARRIVE_RADIUS_DP: Double = 95.0
            /** `cursor_homing_radius_dp` from `[shared.motion]`. */
            const val CURSOR_HOMING_RADIUS_DP: Double = 240.0
            /** `cursor_homing_turn_boost` from `[shared.motion]`. */
            const val CURSOR_HOMING_TURN_BOOST: Double = 3.5
            /** `cursor_max_step_s` from `[shared.motion]`. */
            const val CURSOR_MAX_STEP_S: Double = 0.04
            /** `cursor_settle_px` from `[shared.motion]`. */
            const val CURSOR_SETTLE_PX: Double = 1.5
            /** `cursor_nose_deg` from `[shared.motion]`. */
            const val CURSOR_NOSE_DEG: Double = -135.0
            /** `cursor_rotate_min_speed_dp_per_s` from `[shared.motion]`. */
            const val CURSOR_ROTATE_MIN_SPEED_DP_PER_S: Double = 80.0
            /** `cursor_rotate_rate_deg_per_s` from `[shared.motion]`. */
            const val CURSOR_ROTATE_RATE_DEG_PER_S: Double = 520.0
        }

        /** `effects` constants. */
        object Effects {
            /** `glow_baseline_min_alpha_0_1` from `[shared.effects]`. */
            const val GLOW_BASELINE_MIN_ALPHA_0_1: Double = 0.55
            /** `glow_baseline_max_alpha_0_1` from `[shared.effects]`. */
            const val GLOW_BASELINE_MAX_ALPHA_0_1: Double = 0.92
            /** `glow_pulse_peak_alpha_0_1` from `[shared.effects]`. */
            const val GLOW_PULSE_PEAK_ALPHA_0_1: Double = 1.0
            /** `cursor_press_scale_fraction` from `[shared.effects]`. */
            const val CURSOR_PRESS_SCALE_FRACTION: Double = 0.6
            /** `press_in_fraction` from `[shared.effects]`. */
            const val PRESS_IN_FRACTION: Double = 0.14
            /** `bounce_damp` from `[shared.effects]`. */
            const val BOUNCE_DAMP: Double = 1.7
            /** `bounce_omega_pi_fraction` from `[shared.effects]`. */
            const val BOUNCE_OMEGA_PI_FRACTION: Double = 1.5
            /** `no_no_shakes_fraction` from `[shared.effects]`. */
            const val NO_NO_SHAKES_FRACTION: Double = 1.5
            /** `no_no_hold_fraction` from `[shared.effects]`. */
            const val NO_NO_HOLD_FRACTION: Double = 0.78
            /** `no_no_wiggle_deg` from `[shared.effects]`. */
            const val NO_NO_WIGGLE_DEG: Double = 20.0
            /** `max_gesture_points` from `[shared.effects]`. */
            const val MAX_GESTURE_POINTS: Int = 16
            /** `trail_samples` from `[shared.effects]`. */
            const val TRAIL_SAMPLES: Int = 12
            /** `capture_barrier_frames` from `[shared.effects]`. */
            const val CAPTURE_BARRIER_FRAMES: Int = 2
            /** `cursor_source_viewbox_width` from `[shared.effects]`. */
            const val CURSOR_SOURCE_VIEWBOX_WIDTH: Int = 23
            /** `cursor_source_viewbox_height` from `[shared.effects]`. */
            const val CURSOR_SOURCE_VIEWBOX_HEIGHT: Int = 24
            /** `cursor_hotspot_fraction_x` from `[shared.effects]`. */
            const val CURSOR_HOTSPOT_FRACTION_X: Double = 0.43478260869565216
            /** `cursor_hotspot_fraction_y` from `[shared.effects]`. */
            const val CURSOR_HOTSPOT_FRACTION_Y: Double = 0.4583333333333333
            /** `glyph_fill_red_0_1` from `[shared.effects]`. */
            const val GLYPH_FILL_RED_0_1: Double = 0.022
            /** `glyph_fill_green_0_1` from `[shared.effects]`. */
            const val GLYPH_FILL_GREEN_0_1: Double = 0.006
            /** `glyph_fill_blue_0_1` from `[shared.effects]`. */
            const val GLYPH_FILL_BLUE_0_1: Double = 0.038
            /** `glyph_edge_white_mix_0_1` from `[shared.effects]`. */
            const val GLYPH_EDGE_WHITE_MIX_0_1: Double = 0.5
            /** `cursor_stroke_edge_0_1` from `[shared.effects]`. */
            const val CURSOR_STROKE_EDGE_0_1: Double = 0.15
            /** `cursor_smoke_offset_x_uv` from `[shared.effects]`. */
            const val CURSOR_SMOKE_OFFSET_X_UV: Double = 0.018
            /** `cursor_smoke_offset_y_uv` from `[shared.effects]`. */
            const val CURSOR_SMOKE_OFFSET_Y_UV: Double = 0.022
            /** `cursor_shadow_reach_0_1` from `[shared.effects]`. */
            const val CURSOR_SHADOW_REACH_0_1: Double = 0.48
            /** `cursor_shadow_falloff_0_1` from `[shared.effects]`. */
            const val CURSOR_SHADOW_FALLOFF_0_1: Double = 0.62
            /** `cursor_shadow_strength_0_1` from `[shared.effects]`. */
            const val CURSOR_SHADOW_STRENGTH_0_1: Double = 0.5
            /** `cursor_shadow_lod` from `[shared.effects]`. */
            const val CURSOR_SHADOW_LOD: Double = 3.0
        }

    }

    /** `desktop` constants. */
    object Desktop {
        /** `geometry` constants. */
        object Geometry {
            /** `cursor_height_logical_px` from `[desktop.geometry]`. */
            const val CURSOR_HEIGHT_LOGICAL_PX: Double = 35.9375
            /** `cursor_halo_radius_logical_px` from `[desktop.geometry]`. */
            const val CURSOR_HALO_RADIUS_LOGICAL_PX: Double = 23.4375
            /** `ripple_min_logical_px` from `[desktop.geometry]`. */
            const val RIPPLE_MIN_LOGICAL_PX: Double = 20.0
            /** `ripple_max_logical_px` from `[desktop.geometry]`. */
            const val RIPPLE_MAX_LOGICAL_PX: Double = 64.0
            /** `gesture_arrive_logical_px` from `[desktop.geometry]`. */
            const val GESTURE_ARRIVE_LOGICAL_PX: Double = 10.0
            /** `catcher_logical_px` from `[desktop.geometry]`. */
            const val CATCHER_LOGICAL_PX: Double = 64.0
            /** `glow_base_stroke_logical_px` from `[desktop.geometry]`. */
            const val GLOW_BASE_STROKE_LOGICAL_PX: Double = 14.0
            /** `glow_base_blur_logical_px` from `[desktop.geometry]`. */
            const val GLOW_BASE_BLUR_LOGICAL_PX: Double = 52.0
            /** `glow_core_stroke_logical_px` from `[desktop.geometry]`. */
            const val GLOW_CORE_STROKE_LOGICAL_PX: Double = 4.0
            /** `glow_core_blur_logical_px` from `[desktop.geometry]`. */
            const val GLOW_CORE_BLUR_LOGICAL_PX: Double = 12.0
            /** `glow_edge_inset_logical_px` from `[desktop.geometry]`. */
            const val GLOW_EDGE_INSET_LOGICAL_PX: Double = 2.0
            /** `glow_corner_logical_px` from `[desktop.geometry]`. */
            const val GLOW_CORNER_LOGICAL_PX: Double = 46.0
            /** `wave_stroke_logical_px` from `[desktop.geometry]`. */
            const val WAVE_STROKE_LOGICAL_PX: Double = 4.0
            /** `wave_blur_logical_px` from `[desktop.geometry]`. */
            const val WAVE_BLUR_LOGICAL_PX: Double = 22.0
            /** `ripple_stroke_logical_px` from `[desktop.geometry]`. */
            const val RIPPLE_STROKE_LOGICAL_PX: Double = 16.0
            /** `ripple_blur_logical_px` from `[desktop.geometry]`. */
            const val RIPPLE_BLUR_LOGICAL_PX: Double = 14.0
            /** `trail_stroke_logical_px` from `[desktop.geometry]`. */
            const val TRAIL_STROKE_LOGICAL_PX: Double = 6.0
        }

        /** `rendering` constants. */
        object Rendering {
            /** `wave_count` from `[desktop.rendering]`. */
            const val WAVE_COUNT: Int = 2
            /** `wave_travel_fraction` from `[desktop.rendering]`. */
            const val WAVE_TRAVEL_FRACTION: Double = 0.05
            /** `wave_fade_in_fraction` from `[desktop.rendering]`. */
            const val WAVE_FADE_IN_FRACTION: Double = 0.18
            /** `wave_max_alpha_0_255` from `[desktop.rendering]`. */
            const val WAVE_MAX_ALPHA_0_255: Int = 25
            /** `max_base_alpha_0_255` from `[desktop.rendering]`. */
            const val MAX_BASE_ALPHA_0_255: Int = 200
            /** `max_core_alpha_0_255` from `[desktop.rendering]`. */
            const val MAX_CORE_ALPHA_0_255: Int = 220
            /** `max_ripple_alpha_0_255` from `[desktop.rendering]`. */
            const val MAX_RIPPLE_ALPHA_0_255: Int = 215
            /** `max_trail_alpha_0_255` from `[desktop.rendering]`. */
            const val MAX_TRAIL_ALPHA_0_255: Int = 190
            /** `trail_max_points` from `[desktop.rendering]`. */
            const val TRAIL_MAX_POINTS: Int = 24
            /** `shadow_dx_viewbox_fraction` from `[desktop.rendering]`. */
            const val SHADOW_DX_VIEWBOX_FRACTION: Double = 0.5
            /** `shadow_dy_viewbox_fraction` from `[desktop.rendering]`. */
            const val SHADOW_DY_VIEWBOX_FRACTION: Double = 1.3
            /** `shadow_blur_viewbox_fraction` from `[desktop.rendering]`. */
            const val SHADOW_BLUR_VIEWBOX_FRACTION: Double = 1.1
            /** `shadow_alpha_0_1` from `[desktop.rendering]`. */
            const val SHADOW_ALPHA_0_1: Double = 0.58
            /** `halo_scale_min_fraction` from `[desktop.rendering]`. */
            const val HALO_SCALE_MIN_FRACTION: Double = 0.85
            /** `halo_scale_max_fraction` from `[desktop.rendering]`. */
            const val HALO_SCALE_MAX_FRACTION: Double = 1.1
            /** `halo_alpha_min_fraction` from `[desktop.rendering]`. */
            const val HALO_ALPHA_MIN_FRACTION: Double = 0.5
            /** `halo_alpha_max_fraction` from `[desktop.rendering]`. */
            const val HALO_ALPHA_MAX_FRACTION: Double = 1.0
            /** `viewbox_height` from `[desktop.rendering]`. */
            const val VIEWBOX_HEIGHT: Int = 48
        }

    }

    /** `android` constants. */
    object Android {
        /** `geometry` constants. */
        object Geometry {
            /** `cursor_height_dp` from `[android.geometry]`. */
            const val CURSOR_HEIGHT_DP: Double = 35.9375
            /** `cursor_halo_radius_dp` from `[android.geometry]`. */
            const val CURSOR_HALO_RADIUS_DP: Double = 23.4375
            /** `ripple_min_dp` from `[android.geometry]`. */
            const val RIPPLE_MIN_DP: Double = 20.0
            /** `ripple_max_dp` from `[android.geometry]`. */
            const val RIPPLE_MAX_DP: Double = 64.0
            /** `gesture_arrive_dp` from `[android.geometry]`. */
            const val GESTURE_ARRIVE_DP: Double = 10.0
            /** `catcher_dp` from `[android.geometry]`. */
            const val CATCHER_DP: Double = 64.0
            /** `glow_base_stroke_dp` from `[android.geometry]`. */
            const val GLOW_BASE_STROKE_DP: Double = 22.0
            /** `glow_base_blur_dp` from `[android.geometry]`. */
            const val GLOW_BASE_BLUR_DP: Double = 22.0
            /** `glow_core_stroke_dp` from `[android.geometry]`. */
            const val GLOW_CORE_STROKE_DP: Double = 6.0
            /** `glow_core_blur_dp` from `[android.geometry]`. */
            const val GLOW_CORE_BLUR_DP: Double = 9.0
            /** `glow_edge_inset_dp` from `[android.geometry]`. */
            const val GLOW_EDGE_INSET_DP: Double = 2.0
            /** `glow_corner_dp` from `[android.geometry]`. */
            const val GLOW_CORNER_DP: Double = 46.0
            /** `wave_stroke_dp` from `[android.geometry]`. */
            const val WAVE_STROKE_DP: Double = 5.0
            /** `wave_blur_dp` from `[android.geometry]`. */
            const val WAVE_BLUR_DP: Double = 9.0
            /** `ripple_stroke_dp` from `[android.geometry]`. */
            const val RIPPLE_STROKE_DP: Double = 16.0
            /** `ripple_blur_dp` from `[android.geometry]`. */
            const val RIPPLE_BLUR_DP: Double = 14.0
            /** `trail_stroke_dp` from `[android.geometry]`. */
            const val TRAIL_STROKE_DP: Double = 6.0
        }

        /** `rendering` constants. */
        object Rendering {
            /** `wave_count` from `[android.rendering]`. */
            const val WAVE_COUNT: Int = 3
            /** `wave_travel_fraction` from `[android.rendering]`. */
            const val WAVE_TRAVEL_FRACTION: Double = 0.2
            /** `wave_fade_in_fraction` from `[android.rendering]`. */
            const val WAVE_FADE_IN_FRACTION: Double = 0.12
            /** `wave_max_alpha_0_255` from `[android.rendering]`. */
            const val WAVE_MAX_ALPHA_0_255: Int = 165
            /** `max_base_alpha_0_255` from `[android.rendering]`. */
            const val MAX_BASE_ALPHA_0_255: Int = 200
            /** `max_core_alpha_0_255` from `[android.rendering]`. */
            const val MAX_CORE_ALPHA_0_255: Int = 220
            /** `max_ripple_alpha_0_255` from `[android.rendering]`. */
            const val MAX_RIPPLE_ALPHA_0_255: Int = 215
            /** `max_trail_alpha_0_255` from `[android.rendering]`. */
            const val MAX_TRAIL_ALPHA_0_255: Int = 190
            /** `trail_max_points` from `[android.rendering]`. */
            const val TRAIL_MAX_POINTS: Int = 24
            /** `shadow_dx_viewbox_fraction` from `[android.rendering]`. */
            const val SHADOW_DX_VIEWBOX_FRACTION: Double = 0.5
            /** `shadow_dy_viewbox_fraction` from `[android.rendering]`. */
            const val SHADOW_DY_VIEWBOX_FRACTION: Double = 1.3
            /** `shadow_blur_viewbox_fraction` from `[android.rendering]`. */
            const val SHADOW_BLUR_VIEWBOX_FRACTION: Double = 1.1
            /** `shadow_alpha_0_1` from `[android.rendering]`. */
            const val SHADOW_ALPHA_0_1: Double = 0.58
            /** `halo_scale_min_fraction` from `[android.rendering]`. */
            const val HALO_SCALE_MIN_FRACTION: Double = 0.85
            /** `halo_scale_max_fraction` from `[android.rendering]`. */
            const val HALO_SCALE_MAX_FRACTION: Double = 1.1
            /** `halo_alpha_min_fraction` from `[android.rendering]`. */
            const val HALO_ALPHA_MIN_FRACTION: Double = 0.5
            /** `halo_alpha_max_fraction` from `[android.rendering]`. */
            const val HALO_ALPHA_MAX_FRACTION: Double = 1.0
            /** `viewbox_height` from `[android.rendering]`. */
            const val VIEWBOX_HEIGHT: Int = 48
        }

    }

    /** `sound` constants. */
    object Sound {
        /** `enabled` from `[sound]`. */
        const val ENABLED: Boolean = false
        /** `no_no_sound_asset` from `[sound]`. */
        const val NO_NO_SOUND_ASSET: String = ""
    }
}
