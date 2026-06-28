#!/usr/bin/env python3
"""Generate Rust and Kotlin overlay spec constants from a canonical TOML source.

Usage:
    uv run python scripts/generate_overlay_spec.py
    uv run python scripts/generate_overlay_spec.py --check

The generator validates the TOML schema strictly, then emits:
    crates/sky-cua-platform/src/overlay_spec_generated.rs
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlaySpec.kt
"""

from __future__ import annotations

import argparse
import hashlib
import math
import sys
import tomllib
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC_PATH = REPO_ROOT / "resources" / "overlay" / "agent_overlay_spec.toml"
RUST_OUT = REPO_ROOT / "crates" / "sky-cua-platform" / "src" / "overlay_spec_generated.rs"
KT_OUT = (
    REPO_ROOT
    / "android"
    / "phone-companion"
    / "app"
    / "src"
    / "main"
    / "java"
    / "com"
    / "skycua"
    / "phonecompanion"
    / "overlay"
    / "OverlaySpec.kt"
)
GENERATOR_PATH = REPO_ROOT / "scripts" / "generate_overlay_spec.py"

# (type, min, max, optional). None means unbounded.
# Types: u8, u32, u64, f32, f64, bool, string.
# Schema nodes are either leaf tuples or nested dicts of schema nodes.
SCHEMA: dict[str, Any] = {
    "shared": {
        "colors": {
            "agent_pink_red_0_255": ("u8", 0, 255, False),
            "agent_pink_green_0_255": ("u8", 0, 255, False),
            "agent_pink_blue_0_255": ("u8", 0, 255, False),
            "agent_pink_light_red_0_255": ("u8", 0, 255, False),
            "agent_pink_light_green_0_255": ("u8", 0, 255, False),
            "agent_pink_light_blue_0_255": ("u8", 0, 255, False),
            "halo_inner_alpha_0_255": ("u8", 0, 255, False),
            "halo_inner_red_0_255": ("u8", 0, 255, False),
            "halo_inner_green_0_255": ("u8", 0, 255, False),
            "halo_inner_blue_0_255": ("u8", 0, 255, False),
            "halo_mid_alpha_0_255": ("u8", 0, 255, False),
            "halo_mid_red_0_255": ("u8", 0, 255, False),
            "halo_mid_green_0_255": ("u8", 0, 255, False),
            "halo_mid_blue_0_255": ("u8", 0, 255, False),
            "halo_outer_alpha_0_255": ("u8", 0, 255, False),
            "halo_outer_red_0_255": ("u8", 0, 255, False),
            "halo_outer_green_0_255": ("u8", 0, 255, False),
            "halo_outer_blue_0_255": ("u8", 0, 255, False),
        },
        "timing": {
            "min_gesture_duration_ms": ("u64", 1, None, False),
            "max_gesture_duration_ms": ("u64", 1, None, False),
            "breathe_period_ms": ("u64", 1, None, False),
            "wave_period_ms": ("u64", 1, None, False),
            "halo_breathe_period_ms": ("u64", 1, None, False),
            "click_feedback_ms": ("u64", 1, None, False),
            "ripple_burst_ms": ("u64", 1, None, False),
            "swipe_visual_min_ms": ("u64", 1, None, False),
            "no_no_wiggle_ms": ("u64", 1, None, False),
            "catcher_idle_ms": ("u64", 1, None, False),
        },
        "motion": {
            "cursor_max_speed_dp_per_s": ("f64", 0.0, None, False),
            "cursor_accel_dp_per_s2": ("f64", 0.0, None, False),
            "cursor_turn_rate_deg_per_s": ("f64", 0.0, None, False),
            "cursor_arrive_radius_dp": ("f64", 0.0, None, False),
            "cursor_homing_radius_dp": ("f64", 0.0, None, False),
            "cursor_homing_turn_boost": ("f64", 0.0, None, False),
            "cursor_max_step_s": ("f64", 0.0, None, False),
            "cursor_settle_px": ("f64", 0.0, None, False),
            "cursor_nose_deg": ("f64", None, None, False),
            "cursor_rotate_min_speed_dp_per_s": ("f64", 0.0, None, False),
            "cursor_rotate_rate_deg_per_s": ("f64", 0.0, None, False),
        },
        "effects": {
            "glow_baseline_min_alpha_0_1": ("f64", 0.0, 1.0, False),
            "glow_baseline_max_alpha_0_1": ("f64", 0.0, 1.0, False),
            "glow_pulse_peak_alpha_0_1": ("f64", 0.0, 1.0, False),
            "cursor_press_scale_fraction": ("f64", 0.0, None, False),
            "press_in_fraction": ("f64", 0.0, 1.0, False),
            "bounce_damp": ("f64", 0.0, None, False),
            "bounce_omega_pi_fraction": ("f64", 0.0, None, False),
            "no_no_shakes_fraction": ("f64", 0.0, None, False),
            "no_no_hold_fraction": ("f64", 0.0, 1.0, False),
            "no_no_wiggle_deg": ("f64", 0.0, None, False),
            "max_gesture_points": ("u32", 1, 1024, False),
            "capture_barrier_frames": ("u32", 1, None, False),
            "cursor_source_viewbox_width": ("u32", 1, None, False),
            "cursor_source_viewbox_height": ("u32", 1, None, False),
            "cursor_hotspot_fraction_x": ("f64", 0.0, 1.0, False),
            "cursor_hotspot_fraction_y": ("f64", 0.0, 1.0, False),
            "glyph_fill_red_0_1": ("f64", 0.0, 1.0, False),
            "glyph_fill_green_0_1": ("f64", 0.0, 1.0, False),
            "glyph_fill_blue_0_1": ("f64", 0.0, 1.0, False),
            "glyph_edge_white_mix_0_1": ("f64", 0.0, 1.0, False),
            "cursor_stroke_edge_0_1": ("f64", 0.0, 1.0, False),
            "cursor_smoke_offset_x_uv": ("f64", 0.0, 1.0, False),
            "cursor_smoke_offset_y_uv": ("f64", 0.0, 1.0, False),
            "cursor_shadow_reach_0_1": ("f64", 0.0, 1.0, False),
            "cursor_shadow_falloff_0_1": ("f64", 0.0, 1.0, False),
            "cursor_shadow_strength_0_1": ("f64", 0.0, 1.0, False),
            "cursor_shadow_lod": ("f64", 0.0, None, False),
        },
    },
    "desktop": {
        "geometry": {
            "cursor_height_logical_px": ("f64", 0.0, None, False),
            "cursor_halo_radius_logical_px": ("f64", 0.0, None, False),
            "ripple_min_logical_px": ("f64", 0.0, None, False),
            "ripple_max_logical_px": ("f64", 0.0, None, False),
            "gesture_arrive_logical_px": ("f64", 0.0, None, False),
            "catcher_logical_px": ("f64", 0.0, None, False),
            "glow_base_stroke_logical_px": ("f64", 0.0, None, False),
            "glow_base_blur_logical_px": ("f64", 0.0, None, False),
            "glow_core_stroke_logical_px": ("f64", 0.0, None, False),
            "glow_core_blur_logical_px": ("f64", 0.0, None, False),
            "glow_edge_inset_logical_px": ("f64", 0.0, None, False),
            "glow_corner_logical_px": ("f64", 0.0, None, False),
            "wave_stroke_logical_px": ("f64", 0.0, None, False),
            "wave_blur_logical_px": ("f64", 0.0, None, False),
            "ripple_stroke_logical_px": ("f64", 0.0, None, False),
            "ripple_blur_logical_px": ("f64", 0.0, None, False),
            "trail_stroke_logical_px": ("f64", 0.0, None, False),
        },
        "rendering": {
            "wave_count": ("u32", 1, None, False),
            "wave_travel_fraction": ("f64", 0.0, 1.0, False),
            "wave_fade_in_fraction": ("f64", 0.0, 1.0, False),
            "wave_max_alpha_0_255": ("u8", 0, 255, False),
            "max_base_alpha_0_255": ("u8", 0, 255, False),
            "max_core_alpha_0_255": ("u8", 0, 255, False),
            "max_ripple_alpha_0_255": ("u8", 0, 255, False),
            "max_trail_alpha_0_255": ("u8", 0, 255, False),
            "trail_max_points": ("u32", 1, None, False),
            "shadow_dx_viewbox_fraction": ("f64", None, None, False),
            "shadow_dy_viewbox_fraction": ("f64", None, None, False),
            "shadow_blur_viewbox_fraction": ("f64", 0.0, None, False),
            "shadow_alpha_0_1": ("f64", 0.0, 1.0, False),
            "halo_scale_min_fraction": ("f64", 0.0, None, False),
            "halo_scale_max_fraction": ("f64", 0.0, None, False),
            "halo_alpha_min_fraction": ("f64", 0.0, 1.0, False),
            "halo_alpha_max_fraction": ("f64", 0.0, 1.0, False),
            "viewbox_height": ("u32", 1, None, False),
        },
    },
    "android": {
        "geometry": {
            "cursor_height_dp": ("f64", 0.0, None, False),
            "cursor_halo_radius_dp": ("f64", 0.0, None, False),
            "ripple_min_dp": ("f64", 0.0, None, False),
            "ripple_max_dp": ("f64", 0.0, None, False),
            "gesture_arrive_dp": ("f64", 0.0, None, False),
            "catcher_dp": ("f64", 0.0, None, False),
            "glow_base_stroke_dp": ("f64", 0.0, None, False),
            "glow_base_blur_dp": ("f64", 0.0, None, False),
            "glow_core_stroke_dp": ("f64", 0.0, None, False),
            "glow_core_blur_dp": ("f64", 0.0, None, False),
            "glow_edge_inset_dp": ("f64", 0.0, None, False),
            "glow_corner_dp": ("f64", 0.0, None, False),
            "wave_stroke_dp": ("f64", 0.0, None, False),
            "wave_blur_dp": ("f64", 0.0, None, False),
            "ripple_stroke_dp": ("f64", 0.0, None, False),
            "ripple_blur_dp": ("f64", 0.0, None, False),
            "trail_stroke_dp": ("f64", 0.0, None, False),
        },
        "rendering": {
            "wave_count": ("u32", 1, None, False),
            "wave_travel_fraction": ("f64", 0.0, 1.0, False),
            "wave_fade_in_fraction": ("f64", 0.0, 1.0, False),
            "wave_max_alpha_0_255": ("u8", 0, 255, False),
            "max_base_alpha_0_255": ("u8", 0, 255, False),
            "max_core_alpha_0_255": ("u8", 0, 255, False),
            "max_ripple_alpha_0_255": ("u8", 0, 255, False),
            "max_trail_alpha_0_255": ("u8", 0, 255, False),
            "trail_max_points": ("u32", 1, None, False),
            "shadow_dx_viewbox_fraction": ("f64", None, None, False),
            "shadow_dy_viewbox_fraction": ("f64", None, None, False),
            "shadow_blur_viewbox_fraction": ("f64", 0.0, None, False),
            "shadow_alpha_0_1": ("f64", 0.0, 1.0, False),
            "halo_scale_min_fraction": ("f64", 0.0, None, False),
            "halo_scale_max_fraction": ("f64", 0.0, None, False),
            "halo_alpha_min_fraction": ("f64", 0.0, 1.0, False),
            "halo_alpha_max_fraction": ("f64", 0.0, 1.0, False),
            "viewbox_height": ("u32", 1, None, False),
        },
    },
    "sound": {
        "enabled": ("bool", None, None, False),
        "no_no_sound_asset": ("string", None, None, False),
    },
}

# Schema-level cross-field checks: (section_path, predicate_description).
CROSS_CHECKS: list[tuple[list[str], str]] = [
    (
        ["shared", "timing"],
        "min_gesture_duration_ms must be <= max_gesture_duration_ms",
    ),
    (
        ["shared", "effects"],
        "glow_baseline_min_alpha_0_1 must be <= glow_baseline_max_alpha_0_1",
    ),
    (
        ["shared", "effects"],
        "glow_baseline_max_alpha_0_1 must be <= glow_pulse_peak_alpha_0_1",
    ),
    (
        ["shared", "effects"],
        "cursor_shadow_falloff_0_1 must be >= cursor_shadow_reach_0_1",
    ),
    (
        ["desktop", "geometry"],
        "ripple_min_logical_px must be <= ripple_max_logical_px",
    ),
    (
        ["android", "geometry"],
        "ripple_min_dp must be <= ripple_max_dp",
    ),
    (
        ["desktop", "rendering"],
        "halo_scale_min_fraction must be <= halo_scale_max_fraction",
    ),
    (
        ["desktop", "rendering"],
        "halo_alpha_min_fraction must be <= halo_alpha_max_fraction",
    ),
    (
        ["android", "rendering"],
        "halo_scale_min_fraction must be <= halo_scale_max_fraction",
    ),
    (
        ["android", "rendering"],
        "halo_alpha_min_fraction must be <= halo_alpha_max_fraction",
    ),
]


def _generator_hash() -> str:
    data = GENERATOR_PATH.read_bytes()
    return hashlib.sha256(data).hexdigest()[:16]


def _die(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def _is_leaf(node: Any) -> bool:
    return isinstance(node, tuple)


def _check_finite(value: float, key_path: str) -> None:
    if not math.isfinite(value):
        _die(f"{key_path} must be finite, got {value}")


def _validate_value(
    key_path: str,
    value: Any,
    expected_type: str,
    min_value: Any,
    max_value: Any,
) -> None:
    if expected_type in {"u8", "u32", "u64"}:
        if not isinstance(value, int) or isinstance(value, bool):
            _die(f"{key_path} must be an integer, got {type(value).__name__}")
        if value < 0:
            _die(f"{key_path} must be non-negative, got {value}")
        if expected_type == "u8" and value > 255:
            _die(f"{key_path} must fit in u8, got {value}")
        if min_value is not None and value < min_value:
            _die(f"{key_path} must be >= {min_value}, got {value}")
        if max_value is not None and value > max_value:
            _die(f"{key_path} must be <= {max_value}, got {value}")
    elif expected_type in {"f32", "f64"}:
        if isinstance(value, bool) or not isinstance(value, float):
            _die(f"{key_path} must be a float, got {type(value).__name__}")
        _check_finite(value, key_path)
        if min_value is not None and value < min_value:
            _die(f"{key_path} must be >= {min_value}, got {value}")
        if max_value is not None and value > max_value:
            _die(f"{key_path} must be <= {max_value}, got {value}")
    elif expected_type == "bool":
        if not isinstance(value, bool):
            _die(f"{key_path} must be a boolean, got {type(value).__name__}")
    elif expected_type == "string":
        if not isinstance(value, str):
            _die(f"{key_path} must be a string, got {type(value).__name__}")


def _validate_node(
    data: Any,
    schema: Any,
    path: list[str],
) -> None:
    if _is_leaf(schema):
        assert isinstance(schema, tuple)
        expected_type, min_value, max_value, optional = schema
        key_path = ".".join(path)
        if data is None and optional:
            return
        _validate_value(key_path, data, expected_type, min_value, max_value)
        return

    assert isinstance(schema, dict)
    key_path = ".".join(path) if path else "<root>"
    if not isinstance(data, dict):
        _die(f"{key_path} must be a table")

    unknown = set(data.keys()) - set(schema.keys())
    if unknown:
        _die(f"unknown keys in [{key_path}]: {sorted(unknown)}")

    missing = {k for k, v in schema.items() if k not in data and (not _is_leaf(v) or not v[3])}
    if missing:
        _die(f"missing required keys in [{key_path}]: {sorted(missing)}")

    for key, child_schema in schema.items():
        if key not in data:
            continue
        _validate_node(data[key], child_schema, [*path, key])


def _validate_geometry_consistency(data: dict[str, Any]) -> None:
    shared = data.get("shared", {})
    effects = shared.get("effects", {})
    hotspot_x = effects.get("cursor_hotspot_fraction_x", 0.0)
    hotspot_y = effects.get("cursor_hotspot_fraction_y", 0.0)
    viewbox_w = effects.get("cursor_source_viewbox_width", 1)
    viewbox_h = effects.get("cursor_source_viewbox_height", 1)

    if not (0.0 <= hotspot_x <= 1.0):
        _die("shared.effects.cursor_hotspot_fraction_x must be in [0, 1]")
    if not (0.0 <= hotspot_y <= 1.0):
        _die("shared.effects.cursor_hotspot_fraction_y must be in [0, 1]")
    if viewbox_w <= 0 or viewbox_h <= 0:
        _die("shared.effects cursor_source_viewbox dimensions must be positive")


def _validate_cross_checks(data: dict[str, Any]) -> None:
    for path, message in CROSS_CHECKS:
        node = data
        for part in path:
            if not isinstance(node, dict) or part not in node:
                continue
            node = node[part]
        if not isinstance(node, dict):
            continue

        if path == ["shared", "timing"]:
            if node.get("min_gesture_duration_ms", 0) > node.get("max_gesture_duration_ms", 0):
                _die(message)
        elif path == ["shared", "effects"]:
            if node.get("glow_baseline_min_alpha_0_1", 0.0) > node.get(
                "glow_baseline_max_alpha_0_1", 0.0
            ):
                _die(message)
            if node.get("glow_baseline_max_alpha_0_1", 0.0) > node.get(
                "glow_pulse_peak_alpha_0_1", 0.0
            ):
                _die(message)
            if node.get("cursor_shadow_falloff_0_1", 0.0) < node.get(
                "cursor_shadow_reach_0_1", 0.0
            ):
                _die(message)
        elif path == ["desktop", "geometry"]:
            if node.get("ripple_min_logical_px", 0.0) > node.get("ripple_max_logical_px", 0.0):
                _die(message)
        elif path == ["android", "geometry"]:
            if node.get("ripple_min_dp", 0.0) > node.get("ripple_max_dp", 0.0):
                _die(message)
        elif path in (
            ["desktop", "rendering"],
            ["android", "rendering"],
        ):
            if node.get("halo_scale_min_fraction", 0.0) > node.get("halo_scale_max_fraction", 0.0):
                _die(message)
            if node.get("halo_alpha_min_fraction", 0.0) > node.get("halo_alpha_max_fraction", 0.0):
                _die(message)


def validate(data: dict[str, Any]) -> None:
    """Strictly validate the parsed TOML against the frozen schema."""
    if data.get("schema_version") != 1:
        _die(f"schema_version must be 1, got {data.get('schema_version')}")

    unknown_top = set(data.keys()) - {"schema_version"} - set(SCHEMA.keys())
    if unknown_top:
        _die(f"unknown top-level keys: {sorted(unknown_top)}")

    for section, node_schema in SCHEMA.items():
        sec_data = data.get(section)
        if sec_data is None:
            _die(f"missing required section [{section}]")
        _validate_node(sec_data, node_schema, [section])

    _validate_geometry_consistency(data)
    _validate_cross_checks(data)


def _rust_const_type(t: str) -> str:
    return {
        "u8": "u8",
        "u32": "u32",
        "u64": "u64",
        "f32": "f32",
        "f64": "f64",
        "bool": "bool",
        "string": "&'static str",
    }[t]


def _rust_literal(value: Any, t: str) -> str:
    if t == "string":
        return f'"{value}"'
    if t in {"u8", "u32", "u64"}:
        return str(value)
    if t in {"f32", "f64"}:
        s = repr(value)
        if t == "f32":
            return f"{s}f32"
        return s
    if t == "bool":
        return str(value).lower()
    raise ValueError(f"unknown type {t}")


def _kt_const_type(t: str) -> str:
    return {
        "u8": "Int",
        "u32": "Int",
        "u64": "Long",
        "f32": "Float",
        "f64": "Double",
        "bool": "Boolean",
        "string": "String",
    }[t]


def _kt_literal(value: Any, t: str) -> str:
    if t == "string":
        return f'"{value}"'
    if t in {"u8", "u32"}:
        return str(value)
    if t == "u64":
        return f"{value}L"
    if t == "f32":
        return f"{value}f"
    if t == "f64":
        return repr(value)
    if t == "bool":
        return str(value).lower()
    raise ValueError(f"unknown type {t}")


def _identifier(key: str) -> str:
    return key.upper()


def _emit_rust_node(
    data: Any,
    schema: Any,
    lines: list[str],
    indent: str,
    path: list[str],
) -> None:
    if _is_leaf(schema):
        assert isinstance(schema, tuple)
        t = schema[0]
        key = path[-1]
        const_type = _rust_const_type(t)
        literal = _rust_literal(data, t)
        ident = _identifier(key)
        lines.append(f"{indent}/// `{key}` from `[{'.'.join(path[:-1])}]`.")
        lines.append(f"{indent}pub const {ident}: {const_type} = {literal};")
        return

    assert isinstance(schema, dict)
    section_name = path[-1] if path else "root"
    lines.append(f"{indent}/// `{section_name}` constants.")
    lines.append(f"{indent}pub mod {section_name} {{")
    for key, child_schema in schema.items():
        _emit_rust_node(data[key], child_schema, lines, indent + "    ", [*path, key])
    lines.append(f"{indent}}}")


def generate_rust(data: dict[str, Any]) -> str:
    lines: list[str] = [
        "// GENERATED FILE - DO NOT EDIT",
        "// Source: resources/overlay/agent_overlay_spec.toml",
        "// Schema version: 1",
        f"// Generator: {GENERATOR_PATH.relative_to(REPO_ROOT).as_posix()}",
        f"// Generator hash: {_generator_hash()}",
        "",
        "#![allow(missing_docs)]",
        "",
        "/// Generated overlay specification constants.",
        "pub mod overlay_spec {",
        "    /// Schema version of the source spec.",
        "    pub const SCHEMA_VERSION: u32 = 1;",
        "",
    ]

    sections: list[str] = []
    for section, node_schema in SCHEMA.items():
        section_lines: list[str] = []
        _emit_rust_node(data[section], node_schema, section_lines, "    ", [section])
        sections.append("\n".join(section_lines))
    lines.append("\n\n".join(sections))
    lines.append("}")
    return "\n".join(lines) + "\n"


def _emit_kt_node(
    data: Any,
    schema: Any,
    lines: list[str],
    indent: str,
    path: list[str],
) -> None:
    if _is_leaf(schema):
        assert isinstance(schema, tuple)
        t = schema[0]
        key = path[-1]
        const_type = _kt_const_type(t)
        literal = _kt_literal(data, t)
        ident = _identifier(key)
        lines.append(f"{indent}/** `{key}` from `[{'.'.join(path[:-1])}]`. */")
        lines.append(f"{indent}const val {ident}: {const_type} = {literal}")
        return

    assert isinstance(schema, dict)
    section_name = path[-1] if path else "root"
    class_name = section_name.capitalize()
    lines.append(f"{indent}/** `{section_name}` constants. */")
    lines.append(f"{indent}object {class_name} {{")
    for key, child_schema in schema.items():
        _emit_kt_node(data[key], child_schema, lines, indent + "    ", [*path, key])
    lines.append(f"{indent}}}")
    lines.append("")


def generate_kotlin(data: dict[str, Any]) -> str:
    lines: list[str] = [
        "package com.skycua.phonecompanion.overlay",
        "",
        "/**",
        " * Generated overlay specification constants.",
        " * Source: resources/overlay/agent_overlay_spec.toml",
        " * Schema version: 1",
        f" * Generator: {GENERATOR_PATH.relative_to(REPO_ROOT).as_posix()}",
        f" * Generator hash: {_generator_hash()}",
        " * GENERATED FILE - DO NOT EDIT",
        " * Run `uv run python scripts/generate_overlay_spec.py` to regenerate.",
        " */",
        "object OverlaySpec {",
        "    const val SCHEMA_VERSION: Int = 1",
        "",
    ]

    sections: list[str] = []
    for section, node_schema in SCHEMA.items():
        section_lines: list[str] = []
        _emit_kt_node(data[section], node_schema, section_lines, "    ", [section])
        sections.append("\n".join(section_lines).rstrip("\n"))
    lines.append("\n\n".join(sections))
    lines.append("}")
    return "\n".join(lines) + "\n"


def load_spec(path: Path) -> dict[str, Any]:
    with path.open("rb") as f:
        return tomllib.load(f)


def write_if_changed(path: Path, content: str) -> bool:
    """Write content and return whether the file changed."""
    if path.exists() and path.read_text(encoding="utf-8") == content:
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    return True


def check_files(rust_content: str, kt_content: str) -> bool:
    """Return True if both generated files match the supplied content."""
    rust_ok = RUST_OUT.exists() and RUST_OUT.read_text(encoding="utf-8") == rust_content
    kt_ok = KT_OUT.exists() and KT_OUT.read_text(encoding="utf-8") == kt_content
    return rust_ok and kt_ok


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate Rust/Kotlin overlay spec constants from TOML."
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify generated files are up to date without writing them.",
    )
    parser.add_argument(
        "--spec",
        type=Path,
        default=SPEC_PATH,
        help="Path to the source TOML spec.",
    )
    args = parser.parse_args()

    data = load_spec(args.spec)
    validate(data)

    rust_content = generate_rust(data)
    kt_content = generate_kotlin(data)

    if args.check:
        if check_files(rust_content, kt_content):
            print("generated files are up to date")
            return 0
        print("error: generated files are stale or missing", file=sys.stderr)
        return 1

    rust_changed = write_if_changed(RUST_OUT, rust_content)
    kt_changed = write_if_changed(KT_OUT, kt_content)

    if rust_changed:
        print(f"updated {RUST_OUT.relative_to(REPO_ROOT)}")
    if kt_changed:
        print(f"updated {KT_OUT.relative_to(REPO_ROOT)}")
    if not rust_changed and not kt_changed:
        print("generated files already up to date")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
