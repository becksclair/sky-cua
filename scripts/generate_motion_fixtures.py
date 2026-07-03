#!/usr/bin/env python3
"""Generate cross-language motion fixture families from the canonical overlay spec.

Usage:
    uv run python scripts/generate_motion_fixtures.py
    uv run python scripts/generate_motion_fixtures.py --check

The generator splices four generated fixture families (`mover_trajectory`,
`approach_angle`, `wrap_radians`, `trail_resample`) into the hand-maintained
`resources/overlay/agent_overlay_motion_fixtures.json`, preserving the existing
families, key order, and formatting byte-for-byte, and writes the result to both
the canonical file and the Android test-classpath copy
(`android/phone-companion/app/src/test/resources/overlay/...`), which
`scripts/test_overlay_spec_codegen.py` guards for byte equality.

Motion constants come from `resources/overlay/agent_overlay_spec.toml`
(`[shared.motion]`, `[shared.effects].trail_samples`); nothing numeric is
hardcoded here.

Behavioral reference and canonicality
-------------------------------------
Kotlin `OverlayMath.Mover2D` (and its helpers) is the behavioral reference for
these fixtures. The JSON stays canonical, this script is the authoring tool: if
a Kotlin fixture consumer ever disagrees with a generated value, the GENERATOR
is wrong and gets fixed; the Kotlin implementation is never adjusted to match.

Kotlin Float emulation
----------------------
Kotlin stores mover state as `Float` (IEEE-754 binary32) and computes
transcendentals in f64 via `java.lang.Math`, truncating back to `Float` per
assignment. The reference implementation mirrors that recipe exactly:

- `f32(v)` round-trips a Python float through binary32 (`struct.pack('<f')`).
  Every f32 arithmetic result goes through it. Sums/differences/products of two
  binary32 values are exact in f64, so `f32(a + b)` equals a native f32 add;
  f64 division/f64-int division carry a theoretical double-rounding ulp risk
  that the fixture tolerances absorb.
- Transcendentals (`math.sqrt`, `math.atan2`, `math.cos`, `math.sin`) run in
  f64 and are then `f32()`-rounded, mirroring
  `Math.sqrt((dx * dx + dy * dy).toDouble()).toFloat()` and friends.
- `java_to_radians` reproduces `java.lang.Math.toRadians` (`deg / 180.0 * PI`);
  Python's `math.radians` multiplies by a precomputed factor and can round
  differently in the last f64 ulp.
- Kotlin `%` on `Float` is JVM `frem` (exact); mirrored with `math.fmod`.
- `OverlayMath.wrapRadians` compares a `Float` against `Math.PI` (a `Double`),
  so those comparisons are done in f64 (Kotlin numeric promotion). Note the
  consequence: f32(pi) > f64 pi, so `wrap_radians(f32(pi))` folds to -f32(pi).

Expression-by-expression sources (android/phone-companion/.../overlay/):
- `wrap_radians`        <- OverlayMath.kt:158-164 (`wrapRadians`)
- `approach_angle_deg`  <- OverlayMath.kt:171-180 (`approachAngleDeg`)
- `path_length`         <- OverlayMath.kt:314-321 (`pathLength`)
- `point_at_progress`   <- OverlayMath.kt:329-351 (`pointAtProgress`)
- `lerp` / `distance`   <- OverlayMath.kt:353-364
- `Mover2D`             <- OverlayMath.kt:392-496 (setBounds/snapTo/step,
  including both stop branches: settle resets the heading to the resting nose,
  pass-target lands exactly WITHOUT resetting the heading)
- `sample_trail`        <- AgentOverlayController.kt:719-729 (`sampleTrail`)

Number formatting
-----------------
Outputs (sample/expected values) are rounded to 6 decimal places per the file's
convention and compared with the family tolerance. Inputs (start, bounds, dt,
segment targets, progress, wrap/approach arguments) are emitted as decimals
that reparse to the exact binary32 value the generator used, so consumers can
replay trajectories bit-exactly.

Tolerances (documented in the JSON description as well)
--------------------------------------------------------
- Heading comparisons must assert `|wrap_radians(expected - actual)| <= tol`,
  never raw subtraction (avoids +/-pi flips).
- Mid-flight `mover_trajectory` samples use `tolerance.mover`; settled/landed
  samples (step >= settled_step) and the scalar families use
  `tolerance.default`.
- Samples are never placed within +/-2 steps of a settle/land branch flip;
  the generator detects branch flips and enforces this when selecting samples.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import struct
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
SPEC_PATH = REPO_ROOT / "resources" / "overlay" / "agent_overlay_spec.toml"
CANONICAL_OUT = REPO_ROOT / "resources" / "overlay" / "agent_overlay_motion_fixtures.json"
ANDROID_OUT = (
    REPO_ROOT
    / "android"
    / "phone-companion"
    / "app"
    / "src"
    / "test"
    / "resources"
    / "overlay"
    / "agent_overlay_motion_fixtures.json"
)

GENERATED_FAMILIES = ("mover_trajectory", "approach_angle", "wrap_radians", "trail_resample")

# java.lang.Float.MAX_VALUE, the Kotlin Mover2D default bound.
FLOAT32_MAX = 3.4028234663852886e38

# Splice anchors: the generated block is always the last content inside the
# "fixtures" object, immediately before the closing braces of the document.
_TAIL = "\n\t}\n}\n"
_FAMILY_ANCHOR = ',\n\t\t"mover_trajectory"'

_MOVER_TOLERANCE = "0.002"

_DESCRIPTION = (
    "Canonical motion/animation math fixtures for Rust/Kotlin/WGSL parity. Generated values are "
    "rounded to 6 decimal places; consumers should compare with a small epsilon. The "
    "mover_trajectory, approach_angle, wrap_radians, and trail_resample families are generated "
    "by scripts/generate_motion_fixtures.py from resources/overlay/agent_overlay_spec.toml; the "
    "other families are hand-maintained. Heading comparisons must assert "
    "|wrap_radians(expected - actual)| <= tol, never raw subtraction. Mid-flight mover_trajectory "
    "samples use tolerance.mover; settled/landed samples (step >= settled_step) and the scalar "
    "families use tolerance.default. Trajectory inputs (start, bounds, dt, segment targets) are "
    "exact float32 decimals and must be replayed bit-exactly; cases without bounds use the Kotlin "
    "Mover2D defaults (each axis clamped to [0, Float.MAX_VALUE])."
)

BRANCH_SNAP = "snap"
BRANCH_NOOP = "noop"
BRANCH_SETTLE = "settle"
BRANCH_PASS_TARGET = "pass_target"
BRANCH_INTEGRATE = "integrate"

# Branches where the mover stops on the target; flips into/out of these are the
# frames where a one-ulp cross-language divergence becomes a whole-step offset.
STOP_BRANCHES = frozenset({BRANCH_SETTLE, BRANCH_PASS_TARGET})

# Steps between first settle and the recorded settled sample; must exceed the
# +/-2 flip margin so the sample sits in provable steady state.
SETTLED_SAMPLE_OFFSET = 5

FLIP_MARGIN_STEPS = 2

Point = tuple[float, float]


def _die(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


# --- Kotlin Float numeric primitives ---------------------------------------


def f32(value: float) -> float:
    """Round a Python float (f64) to IEEE-754 binary32, as a Kotlin Float."""
    return float(struct.unpack("<f", struct.pack("<f", value))[0])


def java_to_radians(degrees: float) -> float:
    """java.lang.Math.toRadians: `deg / 180.0 * PI` in f64 (rounding order matters)."""
    return degrees / 180.0 * math.pi


def coerce_in(value: float, low: float, high: float) -> float:
    """Kotlin Float.coerceIn for finite inputs."""
    if value < low:
        return low
    if value > high:
        return high
    return value


def clamp01(value: float) -> float:
    """OverlayMath.clamp01 (OverlayMath.kt:195-200)."""
    if value < 0.0:
        return 0.0
    if value > 1.0:
        return 1.0
    return value


# --- OverlayMath reference implementations ----------------------------------


def wrap_radians(angle: float) -> float:
    """OverlayMath.wrapRadians (OverlayMath.kt:158-164); Math.PI comparisons in f64."""
    two_pi = f32(2.0 * math.pi)
    a = f32(math.fmod(angle, two_pi))
    if a <= -math.pi:
        a = f32(a + two_pi)
    if a > math.pi:
        a = f32(a - two_pi)
    return a


def approach_angle_deg(current: float, target: float, max_delta: float) -> float:
    """OverlayMath.approachAngleDeg (OverlayMath.kt:171-180); all-f32 arithmetic."""
    diff = f32(math.fmod(f32(target - current), 360.0))
    if diff < -180.0:
        diff = f32(diff + 360.0)
    if diff > 180.0:
        diff = f32(diff - 360.0)
    step = coerce_in(diff, -max_delta, max_delta)
    result = f32(math.fmod(f32(current + step), 360.0))
    if result <= -180.0:
        result = f32(result + 360.0)
    if result > 180.0:
        result = f32(result - 360.0)
    return result


def distance(a: Point, b: Point) -> float:
    """OverlayMath.distance (OverlayMath.kt:360-364): f32 sums, f64 sqrt, f32 result."""
    dx = f32(b[0] - a[0])
    dy = f32(b[1] - a[1])
    return f32(math.sqrt(f32(f32(dx * dx) + f32(dy * dy))))


def path_length(points: list[Point]) -> float:
    """OverlayMath.pathLength (OverlayMath.kt:314-321)."""
    if len(points) < 2:
        return 0.0
    total = 0.0
    for i in range(1, len(points)):
        total = f32(total + distance(points[i - 1], points[i]))
    return total


def lerp_point(a: Point, b: Point, t: float) -> Point:
    """OverlayMath.lerp (OverlayMath.kt:353-357)."""
    x = clamp01(t)
    return (
        f32(a[0] + f32(f32(b[0] - a[0]) * x)),
        f32(a[1] + f32(f32(b[1] - a[1]) * x)),
    )


def point_at_progress(points: list[Point], progress: float) -> Point:
    """OverlayMath.pointAtProgress (OverlayMath.kt:329-351)."""
    if not points:
        return (0.0, 0.0)
    if len(points) == 1:
        return points[0]
    p = clamp01(progress)
    if p <= 0.0:
        return points[0]
    if p >= 1.0:
        return points[-1]
    total = path_length(points)
    if total <= 0.0:
        return points[0]
    target = f32(total * p)
    travelled = 0.0
    for i in range(1, len(points)):
        seg = distance(points[i - 1], points[i])
        if seg <= 0.0:
            continue
        if f32(travelled + seg) >= target:
            local_t = f32(f32(target - travelled) / seg)
            return lerp_point(points[i - 1], points[i], local_t)
        travelled = f32(travelled + seg)
    return points[-1]


def sample_trail(points: list[Point], progress: float, sample_count: int) -> list[Point]:
    """AgentOverlayController.sampleTrail (AgentOverlayController.kt:719-729)."""
    samples: list[Point] = []
    for i in range(sample_count):
        frac = f32(progress * f32(float(i) / float(sample_count - 1)))
        samples.append(point_at_progress(points, frac))
    return samples


# --- Mover2D reference (OverlayMath.kt:392-496) ------------------------------


@dataclass(frozen=True)
class MoverParams:
    max_speed: float
    accel: float
    turn_rate_rad: float
    arrive_radius: float
    homing_radius: float
    homing_boost: float
    default_heading_rad: float
    max_step_s: float
    settle_px: float


def mover_params_from_spec(spec: dict[str, Any]) -> MoverParams:
    """Production mover parameters at density 1 (AgentOverlayController.kt:71-79)."""
    motion = spec["shared"]["motion"]
    return MoverParams(
        max_speed=f32(motion["cursor_max_speed_dp_per_s"]),
        accel=f32(motion["cursor_accel_dp_per_s2"]),
        turn_rate_rad=f32(java_to_radians(f32(motion["cursor_turn_rate_deg_per_s"]))),
        arrive_radius=f32(motion["cursor_arrive_radius_dp"]),
        homing_radius=f32(motion["cursor_homing_radius_dp"]),
        homing_boost=f32(motion["cursor_homing_turn_boost"]),
        default_heading_rad=f32(java_to_radians(f32(motion["cursor_nose_deg"]))),
        max_step_s=f32(motion["cursor_max_step_s"]),
        settle_px=f32(motion["cursor_settle_px"]),
    )


class Mover2D:
    """Reference port of OverlayMath.Mover2D, f32 state with f64 transcendentals."""

    def __init__(self, params: MoverParams) -> None:
        self.params = params
        self.x = 0.0
        self.y = 0.0
        self.heading_rad = 0.0
        self.speed = 0.0
        self.initialized = False
        self.max_x = FLOAT32_MAX
        self.max_y = FLOAT32_MAX

    def set_bounds(self, width: float, height: float) -> None:
        self.max_x = width
        self.max_y = height
        self.x = coerce_in(self.x, 0.0, self.max_x)
        self.y = coerce_in(self.y, 0.0, self.max_y)

    def snap_to(self, tx: float, ty: float) -> None:
        self.x = coerce_in(tx, 0.0, self.max_x)
        self.y = coerce_in(ty, 0.0, self.max_y)
        self.speed = 0.0
        self.heading_rad = self.params.default_heading_rad
        self.initialized = True

    def step(self, tx: float, ty: float, dt_seconds: float) -> str:
        """One integration step; returns the branch taken (for flip detection)."""
        p = self.params
        if not self.initialized:
            self.snap_to(tx, ty)
            return BRANCH_SNAP
        dt = coerce_in(dt_seconds, 0.0, p.max_step_s)
        if dt <= 0.0:
            return BRANCH_NOOP
        dx = f32(tx - self.x)
        dy = f32(ty - self.y)
        dist = f32(math.sqrt(f32(f32(dx * dx) + f32(dy * dy))))
        if dist <= p.settle_px:
            # Settle branch: exact landing, stop, heading reset to the resting
            # nose (OverlayMath.kt:453-461). No bounds clamp, Kotlin parity.
            self.x = tx
            self.y = ty
            self.speed = 0.0
            self.heading_rad = p.default_heading_rad
            return BRANCH_SETTLE
        if p.homing_radius > 0.0 and dist < p.homing_radius:
            homing = f32(p.homing_boost * f32(1.0 - f32(dist / p.homing_radius)))
        else:
            homing = 0.0
        target_angle = f32(math.atan2(dy, dx))
        max_turn = f32(f32(p.turn_rate_rad * f32(1.0 + homing)) * dt)
        turn = coerce_in(wrap_radians(f32(target_angle - self.heading_rad)), -max_turn, max_turn)
        self.heading_rad = wrap_radians(f32(self.heading_rad + turn))
        if dist < p.arrive_radius:
            desired_speed = f32(p.max_speed * f32(dist / p.arrive_radius))
        else:
            desired_speed = p.max_speed
        accel_dt = f32(p.accel * dt)
        ds = coerce_in(f32(desired_speed - self.speed), -accel_dt, accel_dt)
        self.speed = max(f32(self.speed + ds), 0.0)
        step_len = f32(self.speed * dt)
        if step_len >= dist:
            # Pass-target branch: exact landing and stop, heading NOT reset
            # (OverlayMath.kt:485-491); the reset happens on the next step's
            # settle branch. Both branches must stay distinct for parity.
            self.x = tx
            self.y = ty
            self.speed = 0.0
            return BRANCH_PASS_TARGET
        self.x = coerce_in(
            f32(self.x + f32(f32(math.cos(self.heading_rad)) * step_len)), 0.0, self.max_x
        )
        self.y = coerce_in(
            f32(self.y + f32(f32(math.sin(self.heading_rad)) * step_len)), 0.0, self.max_y
        )
        return BRANCH_INTEGRATE


# --- Trajectory simulation and sample selection ------------------------------


@dataclass(frozen=True)
class StepState:
    x: float
    y: float
    heading_rad: float
    speed: float
    branch: str


@dataclass(frozen=True)
class TrajectoryCase:
    name: str
    start: Point | None
    bounds: tuple[float, float] | None  # (max_x, max_y); min is 0 per Kotlin
    dt: float
    segments: list[tuple[Point, int]]
    samples: list[tuple[int, StepState]]  # (1-based step, state after that step)
    settled_step: int | None


def run_trajectory(
    params: MoverParams,
    start: Point | None,
    bounds: tuple[float, float] | None,
    dt: float,
    segments: list[tuple[Point, int]],
) -> list[StepState]:
    mover = Mover2D(params)
    if bounds is not None:
        mover.set_bounds(bounds[0], bounds[1])
    if start is not None:
        mover.snap_to(start[0], start[1])
    states: list[StepState] = []
    for (tx, ty), steps in segments:
        for _ in range(steps):
            branch = mover.step(tx, ty, dt)
            states.append(StepState(mover.x, mover.y, mover.heading_rad, mover.speed, branch))
    return states


def flip_steps(branches: list[str]) -> list[int]:
    """1-based steps where the branch flips into or out of a stop branch."""
    flips: list[int] = []
    for i in range(1, len(branches)):
        a, b = branches[i - 1], branches[i]
        if a != b and (a in STOP_BRANCHES or b in STOP_BRANCHES):
            flips.append(i + 1)
    return flips


def assert_samples_clear_of_flips(
    branches: list[str],
    sample_steps: list[int],
    margin: int = FLIP_MARGIN_STEPS,
) -> None:
    """Reject samples within `margin` steps of a settle/land branch flip."""
    flips = flip_steps(branches)
    for step in sample_steps:
        for flip in flips:
            if abs(step - flip) <= margin:
                raise ValueError(
                    f"sample at step {step} is within +/-{margin} steps of the "
                    f"settle/land branch flip at step {flip}"
                )


def _first_settled_step(states: list[StepState], target: Point) -> int | None:
    for idx, st in enumerate(states):
        if (
            st.branch in STOP_BRANCHES
            and st.x == target[0]
            and st.y == target[1]
            and st.speed == 0.0
        ):
            return idx + 1
    return None


def _assert_steady_after_settle(states: list[StepState], settled_step: int) -> None:
    steady = states[settled_step:]
    if not steady:
        raise ValueError("no steady-state steps recorded after the settle step")
    head = steady[0]
    for st in steady[1:]:
        if st != head:
            raise ValueError("mover state changed after settling; settled sample would be unstable")


def _pick_samples(states: list[StepState], sample_steps: list[int]) -> list[tuple[int, StepState]]:
    branches = [s.branch for s in states]
    assert_samples_clear_of_flips(branches, sample_steps)
    return [(step, states[step - 1]) for step in sample_steps]


# --- Trajectory cases ---------------------------------------------------------


def _settling_case(
    params: MoverParams,
    name: str,
    start: Point,
    target: Point,
    dt: float,
    mid_flight_steps: list[int],
) -> TrajectoryCase:
    probe = run_trajectory(params, start, None, dt, [(target, 2000)])
    settled = _first_settled_step(probe, target)
    if settled is None:
        raise ValueError(f"{name}: mover never settled within the probe budget")
    settled_sample = settled + SETTLED_SAMPLE_OFFSET
    total_steps = max([settled_sample, *mid_flight_steps])
    states = probe[:total_steps]
    _assert_steady_after_settle(states, settled)
    sample_steps = sorted({*mid_flight_steps, settled_sample})
    return TrajectoryCase(
        name=name,
        start=start,
        bounds=None,
        dt=dt,
        segments=[(target, total_steps)],
        samples=_pick_samples(states, sample_steps),
        settled_step=settled,
    )


def _case_redirect_mid_flight(params: MoverParams, dt: float) -> TrajectoryCase:
    start: Point = (0.0, 0.0)
    seg1_target: Point = (4000.0, 0.0)
    first_leg = run_trajectory(params, start, None, dt, [(seg1_target, 30)])
    x_at_30 = first_leg[-1].x
    if f32(float(format_input(x_at_30))) != x_at_30:
        raise ValueError("redirect target x does not round-trip through its JSON encoding")
    seg2_target: Point = (x_at_30, 4000.0)
    segments: list[tuple[Point, int]] = [(seg1_target, 30), (seg2_target, 15)]
    states = run_trajectory(params, start, None, dt, segments)
    final = states[-1]
    if not (final.x > x_at_30 and final.y > 1.0):
        raise ValueError("redirect case lost its momentum-carry signature")
    return TrajectoryCase(
        name="redirect_mid_flight",
        start=start,
        bounds=None,
        dt=dt,
        segments=segments,
        samples=_pick_samples(states, [32, 36, 45]),
        settled_step=None,
    )


def _case_bounds_clamp(params: MoverParams, dt: float) -> TrajectoryCase:
    start: Point = (500.0, 500.0)
    bounds = (1000.0, 2000.0)
    target: Point = (9000.0, 9000.0)
    states = run_trajectory(params, start, bounds, dt, [(target, 100)])
    for st in states:
        if not (0.0 <= st.x <= bounds[0] and 0.0 <= st.y <= bounds[1]):
            raise ValueError("bounds_clamp case escaped its bounds")
    if states[-1].speed <= 0.0:
        raise ValueError("bounds_clamp case unexpectedly settled")
    return TrajectoryCase(
        name="bounds_clamp",
        start=start,
        bounds=bounds,
        dt=dt,
        segments=[(target, 100)],
        samples=_pick_samples(states, [30, 100]),
        settled_step=None,
    )


def _case_dt_clamp(params: MoverParams) -> TrajectoryCase:
    start: Point = (0.0, 0.0)
    target: Point = (500.0, 0.0)
    dt = 10.0
    states = run_trajectory(params, start, None, dt, [(target, 1)])
    st = states[0]
    # An unclamped 10 s step would land on the target; the clamp must leave the
    # mover mid-flight with exactly one accel-limited speed increment.
    if st.branch != BRANCH_INTEGRATE or st.speed != f32(f32(params.accel) * params.max_step_s):
        raise ValueError("dt_clamp case did not exhibit the clamped single-step signature")
    return TrajectoryCase(
        name="dt_clamp",
        start=start,
        bounds=None,
        dt=dt,
        segments=[(target, 1)],
        samples=_pick_samples(states, [1]),
        settled_step=None,
    )


def _case_first_step_snaps(params: MoverParams, dt: float) -> TrajectoryCase:
    target: Point = (321.5, 87.25)
    states = run_trajectory(params, None, None, dt, [(target, 1)])
    st = states[0]
    if (
        st.branch != BRANCH_SNAP
        or st.x != target[0]
        or st.y != target[1]
        or st.speed != 0.0
        or st.heading_rad != params.default_heading_rad
    ):
        raise ValueError("first_step_snaps case did not snap exactly")
    return TrajectoryCase(
        name="first_step_snaps",
        start=None,
        bounds=None,
        dt=dt,
        segments=[(target, 1)],
        samples=_pick_samples(states, [1]),
        settled_step=None,
    )


def build_trajectory_cases(params: MoverParams) -> list[TrajectoryCase]:
    dt = f32(1.0 / 60.0)
    return [
        _settling_case(params, "straight_arrival", (0.0, 0.0), (500.0, 0.0), dt, [5, 15, 30]),
        _settling_case(
            params, "homing_convergence", (500.0, 500.0), (120.0, 470.0), dt, [30, 120, 300]
        ),
        _case_redirect_mid_flight(params, dt),
        _case_bounds_clamp(params, dt),
        _case_dt_clamp(params),
        _case_first_step_snaps(params, dt),
    ]


# --- Number and JSON formatting -----------------------------------------------


def format_value(value: float) -> str:
    """Output values: 6-decimal rounding, trailing zeros stripped, -0 normalized."""
    rounded = round(value, 6)
    if rounded == 0.0:
        rounded = 0.0
    text = f"{rounded:.6f}".rstrip("0")
    if text.endswith("."):
        text += "0"
    return text


def format_input(value: float) -> str:
    """Input values: the shortest fixed-point decimal that reparses to the same f32."""
    for places in range(1, 18):
        text = f"{value:.{places}f}"
        if f32(float(text)) == value:
            return text
    return repr(value)


def _point_inline(point: Point, fmt: Any) -> str:
    return f'{{ "x": {fmt(point[0])}, "y": {fmt(point[1])} }}'


def _join_parts(parts: list[list[str]], indent: str) -> list[str]:
    lines: list[str] = []
    for index, part in enumerate(parts):
        part = list(part)
        if index < len(parts) - 1:
            part[-1] += ","
        lines.extend(indent + line for line in part)
    return lines


def _trajectory_entry(case: TrajectoryCase) -> list[str]:
    parts: list[list[str]] = [[f'"name": "{case.name}"']]
    if case.start is not None:
        parts.append([f'"start": {_point_inline(case.start, format_input)}'])
    if case.bounds is not None:
        max_x, max_y = case.bounds
        parts.append(
            [
                '"bounds": { "min_x": 0.0, "min_y": 0.0, '
                f'"max_x": {format_input(max_x)}, "max_y": {format_input(max_y)} }}'
            ]
        )
    parts.append([f'"dt": {format_input(case.dt)}'])

    seg_lines = ['"segments": [']
    for index, (target, steps) in enumerate(case.segments):
        comma = "," if index < len(case.segments) - 1 else ""
        seg_lines.append(
            f'\t{{ "target": {_point_inline(target, format_input)}, "steps": {steps} }}{comma}'
        )
    seg_lines.append("]")
    parts.append(seg_lines)

    sample_lines = ['"samples": [']
    for index, (step, st) in enumerate(case.samples):
        comma = "," if index < len(case.samples) - 1 else ""
        sample_lines.append(
            f'\t{{ "step": {step}, "x": {format_value(st.x)}, "y": {format_value(st.y)}, '
            f'"heading_rad": {format_value(st.heading_rad)}, "speed": {format_value(st.speed)} }}'
            f"{comma}"
        )
    sample_lines.append("]")
    parts.append(sample_lines)

    if case.settled_step is not None:
        parts.append([f'"settled_step": {case.settled_step}'])

    return ["\t\t\t{", *_join_parts(parts, "\t\t\t\t"), "\t\t\t}"]


def _scalar_entry(pairs: list[tuple[str, str]]) -> list[str]:
    lines = ["\t\t\t{"]
    for index, (key, value) in enumerate(pairs):
        comma = "," if index < len(pairs) - 1 else ""
        lines.append(f'\t\t\t\t"{key}": {value}{comma}')
    lines.append("\t\t\t}")
    return lines


def _trail_entry(points: list[Point], progress: float, sample_count: int) -> list[str]:
    expected = sample_trail(points, f32(progress), sample_count)
    parts: list[list[str]] = []

    point_lines = ['"points": [']
    for index, point in enumerate(points):
        comma = "," if index < len(points) - 1 else ""
        point_lines.append(f"\t{_point_inline(point, format_input)}{comma}")
    point_lines.append("]")
    parts.append(point_lines)

    parts.append([f'"progress": {format_input(f32(progress))}'])
    parts.append([f'"sample_count": {sample_count}'])

    expected_lines = ['"expected": [']
    for index, point in enumerate(expected):
        comma = "," if index < len(expected) - 1 else ""
        expected_lines.append(f"\t{_point_inline(point, format_value)}{comma}")
    expected_lines.append("]")
    parts.append(expected_lines)

    return ["\t\t\t{", *_join_parts(parts, "\t\t\t\t"), "\t\t\t}"]


def _family_lines(name: str, entries: list[list[str]]) -> list[str]:
    lines = [f'\t\t"{name}": [']
    for index, entry in enumerate(entries):
        entry = list(entry)
        if index < len(entries) - 1:
            entry[-1] += ","
        lines.extend(entry)
    lines.append("\t\t]")
    return lines


def build_families_block(spec: dict[str, Any]) -> str:
    """The generated families as text, spliced verbatim into the fixtures object."""
    params = mover_params_from_spec(spec)
    trail_samples = int(spec["shared"]["effects"]["trail_samples"])

    trajectory_entries = [_trajectory_entry(case) for case in build_trajectory_cases(params)]

    approach_cases: list[tuple[float, float, float]] = [
        (170.0, -170.0, 5.0),  # shortest path across the wrap
        (0.0, 90.0, 10.0),  # clamped to max_delta
        (-170.0, 170.0, 5.0),  # shortest path, negative direction
        (350.0, 10.0, 30.0),  # result folds across 360
        (10.0, 40.0, 90.0),  # unclamped
    ]
    approach_entries = [
        _scalar_entry(
            [
                ("current", format_input(current)),
                ("target", format_input(target)),
                ("max_delta", format_input(max_delta)),
                ("expected", format_value(approach_angle_deg(current, target, max_delta))),
            ]
        )
        for current, target, max_delta in approach_cases
    ]

    wrap_values = [
        f32(2.0 * math.pi),
        f32(math.pi),
        f32(-math.pi),
        f32(1.5 * math.pi),
    ]
    wrap_entries = [
        _scalar_entry(
            [
                ("value", format_input(value)),
                ("expected", format_value(wrap_radians(value))),
            ]
        )
        for value in wrap_values
    ]

    trail_entries = [
        _trail_entry([(0.0, 0.0), (300.0, 150.0)], 0.5, trail_samples),
        _trail_entry([(0.0, 0.0), (10.0, 0.0), (40.0, 0.0)], 0.75, trail_samples),
    ]

    families = [
        _family_lines("mover_trajectory", trajectory_entries),
        _family_lines("approach_angle", approach_entries),
        _family_lines("wrap_radians", wrap_entries),
        _family_lines("trail_resample", trail_entries),
    ]
    lines: list[str] = []
    for index, family in enumerate(families):
        family = list(family)
        if index < len(families) - 1:
            family[-1] += ","
        lines.extend(family)
    return "\n".join(lines)


# --- Splicing into the hand-maintained JSON -----------------------------------


def _strip_generated_families(text: str) -> str:
    anchor = text.find(_FAMILY_ANCHOR)
    if anchor == -1:
        return text
    if not text.endswith(_TAIL):
        _die("fixtures file has generated families but an unexpected document tail")
    return text[:anchor] + _TAIL


def _strip_mover_tolerance(text: str) -> str:
    return re.sub(r',\n\t\t"mover": [^\n]+', "", text, count=1)


def _set_description(text: str) -> str:
    replacement = f'\t"description": "{_DESCRIPTION}",'
    new_text, count = re.subn(
        r'(?m)^\t"description": .*$', lambda _match: replacement, text, count=1
    )
    if count != 1:
        _die("fixtures file is missing its description line")
    return new_text


def _insert_mover_tolerance(text: str) -> str:
    new_text, count = re.subn(
        r'(\n\t\t"loose": [^\n,]+)\n',
        rf'\1,\n\t\t"mover": {_MOVER_TOLERANCE}\n',
        text,
        count=1,
    )
    if count != 1:
        _die("fixtures file is missing the tolerance map loose entry")
    return new_text


def build_canonical_text(existing_text: str, spec: dict[str, Any]) -> str:
    """Regenerate the full fixtures document from the hand-maintained base."""
    base = _strip_generated_families(existing_text)
    base = _strip_mover_tolerance(base)
    base = _set_description(base)
    base = _insert_mover_tolerance(base)
    if '"path_sampling"' not in base:
        _die("fixtures file is missing the hand-maintained path_sampling family")
    if not base.endswith(_TAIL):
        _die("fixtures file has an unexpected document tail")
    content = base[: -len(_TAIL)] + ",\n" + build_families_block(spec) + _TAIL

    parsed = json.loads(content)
    families = list(parsed["fixtures"].keys())
    if families[-len(GENERATED_FAMILIES) :] != list(GENERATED_FAMILIES):
        _die("generated families did not land at the end of the fixtures object")
    return content


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


def check_files(content: str) -> bool:
    """Return True if both fixture copies match the supplied content."""
    canonical_ok = CANONICAL_OUT.exists() and CANONICAL_OUT.read_text(encoding="utf-8") == content
    android_ok = ANDROID_OUT.exists() and ANDROID_OUT.read_text(encoding="utf-8") == content
    return canonical_ok and android_ok


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate cross-language motion fixture families from the overlay spec TOML."
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify both fixture copies are up to date without writing them.",
    )
    parser.add_argument(
        "--spec",
        type=Path,
        default=SPEC_PATH,
        help="Path to the source TOML spec.",
    )
    args = parser.parse_args()

    spec = load_spec(args.spec)
    try:
        if not CANONICAL_OUT.exists():
            _die(f"missing canonical fixtures file {CANONICAL_OUT.relative_to(REPO_ROOT)}")
        existing = CANONICAL_OUT.read_text(encoding="utf-8")
        content = build_canonical_text(existing, spec)
    except KeyError as error:
        _die(f"spec is missing a required key: {error}")
        raise
    except ValueError as error:
        _die(str(error))
        raise

    if args.check:
        if check_files(content):
            print("motion fixtures are up to date")
            return 0
        print("error: motion fixtures are stale or missing", file=sys.stderr)
        return 1

    canonical_changed = write_if_changed(CANONICAL_OUT, content)
    android_changed = write_if_changed(ANDROID_OUT, content)

    if canonical_changed:
        print(f"updated {CANONICAL_OUT.relative_to(REPO_ROOT)}")
    if android_changed:
        print(f"updated {ANDROID_OUT.relative_to(REPO_ROOT)}")
    if not canonical_changed and not android_changed:
        print("motion fixtures already up to date")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
