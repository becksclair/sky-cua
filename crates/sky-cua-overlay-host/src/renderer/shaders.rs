//! WGSL shader source and pipeline factory for GPU-rendered overlay effects.
//!
//! The shader treats generated TOML color channels as normalized sRGB values
//! and outputs premultiplied alpha. The current surface policy prefers sRGB
//! swapchain formats, so texture sampling and presentation use WGPU's sRGB
//! conversions while analytic effect colors stay in the authored visual space.

pub const EFFECT_SHADER: &str = r#"
const PI: f32 = 3.141592653589793;
const KIND_NONE: u32 = 0u;
const KIND_TAP: u32 = 1u;
const KIND_DRAG: u32 = 2u;
const KIND_SWIPE: u32 = 3u;
const KIND_NO_NO: u32 = 4u;
const MAX_POINTS: u32 = 16u;
// White-outline ring half-width, in normalized SDF units (the cursor texture's R
// channel is a signed distance field where 0.5 = on the glyph path). MUST match
// the Rust `CURSOR_STROKE_EDGE` (guarded by a test).
const CURSOR_STROKE_EDGE: f32 = 0.15;
// Soft shadow reconstruction (sampled from the same distance field): centered on
// the glyph (no offset) so it haloes the cursor on ALL sides, a blurred mip
// level, a wide soft falloff that fades out before the SDF saturates (0.5) to
// avoid a ghost edge, and an opacity. Reach/falloff are normalized SDF units, so
// the shadow's screen size scales with SDF_RANGE_TEXELS.
const CURSOR_SHADOW_OFFSET_X: f32 = 0.0;
const CURSOR_SHADOW_OFFSET_Y: f32 = 0.0;
const CURSOR_SHADOW_LOD: f32 = 3.0;
const CURSOR_SHADOW_REACH: f32 = 0.48;
// Falloff >= reach: no full-strength plateau, just a soft gradient that fades
// from the silhouette outward, so the shadow reads as a diffuse halo rather than
// a hard dark band hugging the outline.
const CURSOR_SHADOW_FALLOFF: f32 = 0.62;
const CURSOR_SHADOW_STRENGTH: f32 = 0.5;
// Up-left shift (uv units) of the smoke cloud so it centres on the cursor
// hotspot rather than the glyph centroid (which sits down-right of the tip).
const CURSOR_SMOKE_OFFSET_X: f32 = 0.018;
const CURSOR_SMOKE_OFFSET_Y: f32 = 0.022;

struct AgentEffectUniform {
    surface_size_px: vec4<f32>,
    cursor: vec4<f32>,
    cursor_metrics: vec4<f32>,
    timing: vec4<f32>,
    effect: vec4<f32>,
    color_agent_pink: vec4<f32>,
    color_agent_pink_light: vec4<f32>,
    color_halo_inner: vec4<f32>,
    glow: vec4<f32>,
    wave: vec4<f32>,
    halo: vec4<f32>,
    ripple: vec4<f32>,
    trail: vec4<f32>,
    no_no: vec4<f32>,
    flags: vec4<u32>,
};

struct AgentEffectPoints {
    points: array<vec4<f32>, 16>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
};

struct ConformanceOut {
    values: array<vec4<f32>, 6>,
};

@group(0) @binding(0) var<uniform> frame: AgentEffectUniform;
@group(0) @binding(1) var<storage, read> effect_points: AgentEffectPoints;
@group(0) @binding(2) var cursor_texture: texture_2d<f32>;
@group(0) @binding(3) var cursor_sampler: sampler;
@group(1) @binding(0) var<storage, read_write> conformance_out: ConformanceOut;

fn saturate(value: f32) -> f32 {
    return clamp(value, 0.0, 1.0);
}

fn safe_div(numerator: f32, denominator: f32) -> f32 {
    return numerator / max(denominator, 0.0001);
}

fn effect_progress() -> f32 {
    return saturate(safe_div(frame.effect.x, frame.effect.y));
}

fn ease_in_out(value: f32) -> f32 {
    let t = saturate(value);
    return t * t * (3.0 - 2.0 * t);
}

fn breathing(elapsed_ms: f32, period_ms: f32) -> f32 {
    let phase = fract(safe_div(elapsed_ms, period_ms));
    // Eased triangle ramp, matching Android `OverlayMath.breathingIntensity`:
    // a linear 0->1->0 sweep across the period, softened by `ease_in_out`.
    let triangle = select((1.0 - phase) * 2.0, phase * 2.0, phase < 0.5);
    return ease_in_out(triangle);
}

fn effect_point(index: u32) -> vec2<f32> {
    return effect_points.points[min(index, MAX_POINTS - 1u)].xy;
}

fn active_point_count() -> u32 {
    return min(frame.flags.z, MAX_POINTS);
}

fn first_effect_point_or_cursor() -> vec2<f32> {
    if active_point_count() > 0u {
        return effect_point(0u);
    }
    return frame.cursor.xy;
}

fn last_effect_point_or_cursor() -> vec2<f32> {
    let count = active_point_count();
    if count > 0u {
        return effect_point(count - 1u);
    }
    return frame.cursor.xy;
}

fn animated_cursor_position() -> vec2<f32> {
    let kind = frame.flags.y;
    let progress = ease_in_out(effect_progress());
    if (kind == KIND_DRAG || kind == KIND_SWIPE) && active_point_count() >= 2u {
        return mix(first_effect_point_or_cursor(), last_effect_point_or_cursor(), progress);
    }
    if kind == KIND_TAP || kind == KIND_NO_NO {
        return first_effect_point_or_cursor();
    }
    return frame.cursor.xy;
}

fn no_no_rotation_offset(progress: f32) -> f32 {
    // Matches Android `OverlayMath.noNoWiggleDeg`: the oscillation runs on raw
    // progress across [0, 1]; the envelope holds at 1 until `hold_fraction`,
    // then `ease_in_out`-ramps to 0 over the tail. The math already yields 0 at
    // p=0 (sin 0) and p=1 (envelope 0), so no early-return guards are needed.
    let p = saturate(progress);
    let hold_fraction = frame.no_no.z;
    var envelope = 1.0;
    if p >= hold_fraction {
        envelope = ease_in_out(safe_div(1.0 - p, 1.0 - hold_fraction));
    }
    return frame.no_no.x * envelope * sin(2.0 * PI * frame.no_no.y * p);
}

fn cursor_rotation_deg() -> f32 {
    let kind = frame.flags.y;
    var rotation = 0.0;
    if (kind == KIND_DRAG || kind == KIND_SWIPE) && active_point_count() >= 2u {
        let direction = last_effect_point_or_cursor() - first_effect_point_or_cursor();
        if length(direction) > 0.001 {
            rotation = degrees(atan2(direction.y, direction.x)) - frame.trail.z;
        }
    }
    if kind == KIND_NO_NO {
        rotation = rotation + no_no_rotation_offset(effect_progress());
    }
    return rotation;
}

fn cursor_scale() -> f32 {
    // Matches Android `OverlayMath.clickScale`: an eased squash into the press,
    // then a damped-cosine spring back that overshoots past 1.0. `frame.cursor.z`
    // carries BOUNCE_DAMP and `frame.cursor.w` carries BOUNCE_OMEGA_PI_FRACTION
    // (the spring is `cos(omega_fraction * PI * t)`).
    let kind = frame.flags.y;
    if kind != KIND_TAP {
        return 1.0;
    }
    let press_fraction = frame.no_no.w;
    let depth = 1.0 - frame.trail.w;
    let progress = effect_progress();
    if progress < press_fraction {
        return 1.0 - depth * ease_in_out(safe_div(progress, press_fraction));
    }
    let t = safe_div(progress - press_fraction, 1.0 - press_fraction);
    let envelope = exp(-frame.cursor.z * t);
    let wave = envelope * cos(frame.cursor.w * PI * t);
    return 1.0 - depth * wave;
}

fn premul(color: vec3<f32>, alpha: f32) -> vec4<f32> {
    let a = saturate(alpha);
    return vec4<f32>(color * a, a);
}

fn over(src: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
    return src + dst * (1.0 - src.a);
}

fn edge_distance(pixel: vec2<f32>) -> f32 {
    let size = frame.surface_size_px.xy;
    return min(min(pixel.x, size.x - pixel.x), min(pixel.y, size.y - pixel.y));
}

// --- Value-noise fbm used to break the edge glow into uneven, smoky tendrils ---
// There is no built-in noise in WGSL; this is the standard hash -> value noise
// -> fractal-Brownian-motion stack. Inputs are kept at small magnitudes (a few
// cells) so `fract`/`floor` stay bit-stable across GPUs for the conformance
// fixture.
fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    // Smootherstep interpolation for C2-continuous cells (no grid creases).
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var freq = p;
    for (var i = 0u; i < 4u; i = i + 1u) {
        value = value + amplitude * value_noise(freq);
        freq = freq * 2.0;
        amplitude = amplitude * 0.5;
    }
    return value;
}

// Domain-warped fbm: folds the noise into drifting, smoky tendrils. Shared by
// the edge glow, the cursor halo, and the drag/swipe trail so the whole overlay
// speaks one cohesive smoky language. `q` is in noise-cell units; `t` is the
// time drift, also in cell units.
fn smoke_fbm(q: vec2<f32>, t: f32) -> f32 {
    let drift = vec2<f32>(t, -t * 0.55);
    let warp = vec2<f32>(fbm(q + drift + 19.3), fbm(q - drift * 0.8 + 4.1));
    return fbm(q + warp * 1.25 + drift * 0.5);
}

fn edge_glow(pixel: vec2<f32>) -> vec4<f32> {
    if frame.flags.w == 0u {
        return vec4<f32>(0.0);
    }
    let distance = edge_distance(pixel);
    // Real-world sizing: `surface_size_px.z` is logical px per physical mm.
    let px_per_mm = max(frame.surface_size_px.z, 0.5);
    // Band depth is 2.5cm physical, but capped to 8% of the smaller screen
    // dimension so it stays proportional on small/low-DPI panels where a fixed
    // 2.5cm would swallow too much of the screen. Large monitors are well under
    // the cap and keep the full physical 2.5cm.
    let min_dim = min(frame.surface_size_px.x, frame.surface_size_px.y);
    let reach = min(25.0 * px_per_mm, 0.08 * min_dim);
    if distance > reach {
        return vec4<f32>(0.0); // fully contained to the border
    }

    // Drifting domain-warped fbm: an uneven, smoky field that crawls over time
    // like a cast spell. The smoke grain scales with the band depth so its
    // density reads the same whether the band is deep (large monitor) or shallow
    // (small monitor) rather than looking coarse in a narrow band.
    let t = frame.timing.x * 0.00017;
    let cell = max(reach * 0.55, 1.0);
    let q = pixel / cell;
    let smoke = smoke_fbm(q, t);
    // Border weighting: 1 at the very edge, fading to 0 over the outer ~40% of
    // the band. Drives a fuller smoke layer where it meets the border.
    let border = exp(-distance / max(reach * 0.40, 1.0));
    // Smoke coverage is fuller near the border (a lower noise threshold lets
    // more of the field register) and breaks into tighter wisps further inward.
    // Never floored to a solid, so the layer stays smoky and translucent.
    let density = smoothstep(mix(0.34, 0.08, border), 0.88, smoke);

    // Bright rim hugging the very edge (~0.8mm), the highest-intensity zone,
    // lightly shimmered by the smoke so it is alive but still crisp.
    let rim = exp(-distance / (0.8 * px_per_mm)) * mix(0.82, 1.0, density);

    // Smoky body: tendrils lick inward where the noise is dense, kept tight to
    // the border (fast falloff plus an early containment ramp) so the smoke ends
    // close to the rim instead of drifting deep into the screen. The base is
    // thicker — extra smoke amplitude in the outer band tapering inward — while
    // staying noise-gated, so it reads as thick wisps rather than a wall.
    let d_eff = max(distance - density * reach * 0.5, 0.0);
    let body = exp(-d_eff / max(reach * 0.18, 1.0)) * density * (1.0 + 1.3 * border);
    let contain = 1.0 - smoothstep(reach * 0.58, reach * 0.94, distance);

    let breathe = breathing(frame.timing.x, frame.timing.y);
    let base_alpha = mix(frame.color_agent_pink.a, frame.color_agent_pink_light.a, breathe);
    // Hot tendrils and the bright rim run lighter pink; thin haze stays deep.
    let tint = mix(
        frame.color_agent_pink.xyz,
        frame.color_agent_pink_light.xyz,
        max(density, rim * 0.6),
    );
    let alpha = base_alpha * (rim * 0.9 + body * 0.62) * contain;
    return premul(tint, alpha);
}

fn inward_waves(pixel: vec2<f32>) -> vec4<f32> {
    // The desktop edge glow folds wave-like motion into its drifting turbulent
    // field (see `edge_glow`), so the separate concentric travelling rings that
    // read as a mechanical "tunnel" are retired here. The function is retained
    // as a no-op for the compositor chain and the Android-parity spec surface;
    // Android still renders discrete inward waves from `OverlaySpec.Android`.
    return vec4<f32>(0.0);
}

// The cursor's aura is the screen edge-glow recipe with the GLYPH silhouette as
// the anchor instead of the screen border. A point anchor is radially symmetric
// and always reads as a disc; the cursor texture instead bakes a distance field
// in its G channel (1 on the glyph outline -> 0 at the smoke margin), so the
// smoke clings to the arrow shape and billows outward exactly like the border
// smoke clings to the screen edge and billows inward.
fn cursor_smoke(pixel: vec2<f32>, cursor_pos: vec2<f32>) -> vec4<f32> {
    if frame.flags.x == 0u {
        return vec4<f32>(0.0);
    }
    // Sample the enlarged cursor texture with the same transform as the glyph.
    let angle = -radians(cursor_rotation_deg());
    let s = sin(angle);
    let c = cos(angle);
    let local = (pixel - cursor_pos) / cursor_scale();
    let rotated = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    let uv = (rotated + frame.cursor_metrics.zw) / frame.cursor_metrics.xy;
    // Shift the smoke sample up-and-left so the cloud centres on the cursor
    // hotspot (the arrow body sits down-right of it) rather than the glyph
    // centroid. A positive uv offset moves the sampled anchor down-right, so the
    // displayed cloud peak lands up-left.
    let smoke_uv = uv + vec2<f32>(CURSOR_SMOKE_OFFSET_X, CURSOR_SMOKE_OFFSET_Y);
    // Sample the anchor BEFORE the bounds branch so this implicit-LOD sample
    // stays in uniform control flow (matching `cursor_sample`); a sample after a
    // pixel-dependent early-return is a WGSL derivative-uniformity violation.
    // ClampToEdge keeps out-of-bounds reads at the transparent margin.
    let border = textureSample(cursor_texture, cursor_sampler, smoke_uv).g; // 1 on outline -> 0 at reach
    if smoke_uv.x < 0.0 || smoke_uv.y < 0.0 || smoke_uv.x > 1.0 || smoke_uv.y > 1.0 {
        return vec4<f32>(0.0);
    }
    if border <= 0.004 {
        return vec4<f32>(0.0); // out in the dead margin; skip the noise field
    }
    // Mask the smoke off the arrow using the ACTUAL (unshifted) glyph coverage.
    // Sampling it at the shifted uv would punch a smoke gap up-left of the real
    // arrow and expose the shadow underneath as a dark band.
    let glyph_a = textureSampleLevel(cursor_texture, cursor_sampler, uv, 0.0).a;

    // Drifting domain-warped fbm in screen space — the same smoky field as the
    // border, so the whole overlay speaks one language. Grain scales with the
    // footprint so it reads the same regardless of cursor scale. The cursor
    // cloud drifts faster than the border (2.6x): the border crawls across a
    // deep band where slow motion reads, but the cursor cloud is small and
    // parked, so it needs a brisker churn to read as alive rather than frozen.
    let t = frame.timing.x * 0.00045;
    let cell = max(frame.cursor_metrics.x * 0.16, 1.0);
    let smoke = smoke_fbm(pixel / cell, t);
    // Fuller coverage near the outline (lower threshold), tighter wisps outward.
    // Higher thresholds keep the cloud thinner / less dense.
    let density = smoothstep(mix(0.42, 0.16, border), 0.9, smoke);
    // `d_norm` is the normalized outward distance: 0 on the outline, 1 at reach.
    let d_norm = 1.0 - border;
    // Bright rim hugging the very outline, lightly shimmered by the smoke.
    let rim = exp(-d_norm / 0.05) * mix(0.82, 1.0, density);
    // Smoky body: tendrils lick outward where the noise is dense (the dense
    // smoke pushes the effective edge out), kept tight to the outline otherwise.
    let d_eff = max(d_norm - density * 0.5, 0.0);
    let body = exp(-d_eff / 0.18) * density * (0.5 + border);
    // Keep the smoke in the margin, off the solid arrow body (it would be hidden
    // anyway, but this stops the rim bleeding through the glyph's AA edge).
    let outside = smoothstep(0.35, 0.8, 1.0 - glyph_a);
    // Feather the OUTER boundary of the cloud to nothing. The anchor falls off
    // linearly, which leaves a faint but detectable line where it reaches zero;
    // easing the outer third of the band out with a smoothstep makes the cloud
    // dissolve into the desktop with no visible edge.
    let edge_fade = smoothstep(0.0, 0.34, border);

    // Deepen the breath around its midpoint so the cursor cloud's pulse reads
    // more clearly than the border's gentle swing (the small parked cloud needs
    // a stronger tell). Gain steepens the curve toward both extremes without
    // introducing new alpha constants.
    let breathe = clamp((breathing(frame.timing.x, frame.timing.y) - 0.5) * 1.5 + 0.5, 0.0, 1.0);
    let base_alpha = mix(frame.color_agent_pink.a, frame.color_agent_pink_light.a, breathe);
    let tint = mix(
        frame.color_agent_pink.xyz,
        frame.color_agent_pink_light.xyz,
        max(density, rim * 0.6),
    );
    let alpha = base_alpha * (rim * 0.72 + body * 0.48) * outside * edge_fade * frame.halo.w;
    return premul(tint, alpha);
}

fn ripple(pixel: vec2<f32>) -> vec4<f32> {
    let kind = frame.flags.y;
    if kind == KIND_NONE || active_point_count() == 0u {
        return vec4<f32>(0.0);
    }
    // Magical energy discharge: a crackling shockwave, a white-hot core flash,
    // and thin radiating rays, instead of a clean expanding ring.
    let progress = saturate(safe_div(frame.effect.x, frame.effect.z));
    let center = first_effect_point_or_cursor();
    let to = pixel - center;
    let dist = length(to);
    let angle = atan2(to.y, to.x);
    let fade = 1.0 - progress;

    // Expanding shockwave whose radius eases outward (Android `rippleRadius`),
    // broken into an uneven discharge by angular smoke noise.
    let radius = mix(frame.ripple.x, frame.ripple.y, ease_in_out(progress));
    let crackle = smoke_fbm(vec2<f32>(angle * 2.4, dist * 0.06), frame.timing.x * 0.003);
    let ring_w = frame.ripple.z * (0.7 + 0.9 * crackle);
    let ring = (1.0 - smoothstep(ring_w * 0.25, ring_w, abs(dist - radius))) * (0.55 + 0.8 * crackle);

    // White-hot core flash that pops on contact and fades fast.
    let flash = exp(-dist / max(frame.ripple.x * 0.55, 1.0)) * pow(fade, 3.0);

    // Thin energy rays radiating from the burst, slowly rotating.
    let ray = pow(0.5 + 0.5 * sin(angle * 7.0 + frame.timing.x * 0.004), 6.0);
    let rays = ray * (1.0 - smoothstep(radius * 0.2, radius * 1.35, dist)) * 0.6;

    let energy = (ring + rays) * fade + flash * 1.4;
    let tint = mix(
        frame.color_agent_pink_light.xyz,
        vec3<f32>(1.0, 0.9, 0.96),
        saturate(flash * 1.5),
    );
    return premul(tint, energy * frame.ripple.w);
}

fn segment_distance(pixel: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let ab = b - a;
    let denom = max(dot(ab, ab), 0.0001);
    let t = saturate(dot(pixel - a, ab) / denom);
    return vec2<f32>(length(pixel - (a + ab * t)), t);
}

fn trail(pixel: vec2<f32>, cursor_pos: vec2<f32>) -> vec4<f32> {
    let kind = frame.flags.y;
    if !(kind == KIND_DRAG || kind == KIND_SWIPE) || active_point_count() < 2u {
        return vec4<f32>(0.0);
    }
    let segment = segment_distance(pixel, first_effect_point_or_cursor(), cursor_pos);
    let width = max(frame.trail.x, 1.0);
    if segment.x > width * 2.5 {
        return vec4<f32>(0.0); // beyond the wispy band; skip the noise field
    }
    // Smoky, wispy stroke: noise flows along the trail (`segment.y`, origin->
    // cursor) and across it, pushing the band wider unevenly so it billows like
    // the border smoke rather than reading as a clean line.
    let t = frame.timing.x * 0.0004;
    let smoke = smoke_fbm(vec2<f32>(segment.y * 7.0, segment.x / width), t);
    let density = smoothstep(0.30, 0.85, smoke);
    let eff = max(segment.x - density * width * 0.7, 0.0);
    let band = 1.0 - smoothstep(width * 0.4, width, eff);
    let tint = mix(frame.color_agent_pink.xyz, frame.color_agent_pink_light.xyz, density);
    let alpha = band * segment.y * mix(0.55, 1.1, density) * frame.trail.y;
    return premul(tint, alpha);
}

fn no_no_mark(pixel: vec2<f32>, cursor_pos: vec2<f32>) -> vec4<f32> {
    if frame.flags.y != KIND_NO_NO {
        return vec4<f32>(0.0);
    }
    let local = pixel - cursor_pos;
    let radius = frame.halo.x * 0.95;
    let ring = 1.0 - smoothstep(2.0, 5.0, abs(length(local) - radius));
    let slash = 1.0 - smoothstep(2.5, 6.0, abs(local.x + local.y) * 0.70710678);
    let span = 1.0 - smoothstep(radius * 0.75, radius * 1.25, length(local));
    return premul(frame.color_agent_pink_light.xyz, max(ring, slash * span) * frame.ripple.w);
}

fn cursor_sample(pixel: vec2<f32>, cursor_pos: vec2<f32>) -> vec4<f32> {
    if frame.flags.x == 0u {
        return vec4<f32>(0.0);
    }
    let angle = -radians(cursor_rotation_deg());
    let s = sin(angle);
    let c = cos(angle);
    let local = (pixel - cursor_pos) / cursor_scale();
    let rotated = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    let uv = (rotated + frame.cursor_metrics.zw) / frame.cursor_metrics.xy;
    // Sample the signed distance field (R channel) and take its screen-space
    // derivative BEFORE the bounds branch so `fwidth` stays in uniform control
    // flow. ClampToEdge keeps out-of-bounds samples at the transparent margin.
    let d = textureSample(cursor_texture, cursor_sampler, uv).r - 0.5;
    let aa = max(fwidth(d), 1e-5);
    if uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 {
        return vec4<f32>(0.0);
    }
    // Reconstruct the glyph from the distance field with fwidth-based AA so the
    // edge is crisp at the framebuffer resolution rather than a minified
    // pre-rasterized stroke. Coverage spans the fill plus the white ring out to
    // CURSOR_STROKE_EDGE; luminance ramps black fill (d<0) -> white outline (d>0).
    let alpha = clamp(0.5 - (d - CURSOR_STROKE_EDGE) / aa, 0.0, 1.0);
    if alpha <= 0.0 {
        return vec4<f32>(0.0);
    }
    let lum = clamp(0.5 + d / aa, 0.0, 1.0);
    // Tint the glyph toward the agent palette: a deep plum fill rising to a soft
    // pink-white outline, instead of pure black/white. The fill is kept very dark
    // (it brightens through the sRGB surface) so it reads near-black with a purple
    // cast and holds strong contrast against the outline.
    let fill = vec3<f32>(0.022, 0.006, 0.038);
    let edge = mix(vec3<f32>(1.0, 1.0, 1.0), frame.color_agent_pink_light.xyz, 0.5);
    let color = mix(fill, edge, lum);
    return premul(color, alpha);
}

// Soft grounding shadow, rendered as its own pass UNDER the smoke and arrow (see
// `render_pixel`) so it darkens the background, not its own pink aura. The
// silhouette is sampled from the distance field at a blurred mip (explicit LOD ->
// no derivatives needed) with a wide falloff that fades before the SDF saturates.
fn cursor_shadow(pixel: vec2<f32>, cursor_pos: vec2<f32>) -> vec4<f32> {
    if frame.flags.x == 0u {
        return vec4<f32>(0.0);
    }
    let angle = -radians(cursor_rotation_deg());
    let s = sin(angle);
    let c = cos(angle);
    let local = (pixel - cursor_pos) / cursor_scale();
    let rotated = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    let uv = (rotated + frame.cursor_metrics.zw) / frame.cursor_metrics.xy;
    let shadow_uv = uv - vec2<f32>(CURSOR_SHADOW_OFFSET_X, CURSOR_SHADOW_OFFSET_Y);
    if shadow_uv.x < 0.0 || shadow_uv.y < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y > 1.0 {
        return vec4<f32>(0.0);
    }
    let ds = textureSampleLevel(cursor_texture, cursor_sampler, shadow_uv, CURSOR_SHADOW_LOD).r - 0.5;
    let shadow_cov = clamp((CURSOR_SHADOW_REACH - ds) / CURSOR_SHADOW_FALLOFF, 0.0, 1.0);
    return premul(vec3<f32>(0.0, 0.0, 0.0), shadow_cov * CURSOR_SHADOW_STRENGTH);
}

fn render_pixel(pixel: vec2<f32>) -> vec4<f32> {
    let cursor_pos = animated_cursor_position();
    var color = vec4<f32>(0.0);
    color = over(edge_glow(pixel), color);
    color = over(inward_waves(pixel), color);
    color = over(trail(pixel, cursor_pos), color);
    // Shadow first so the smoke and arrow sit on top of it — it grounds the
    // cursor against the background instead of darkening its own aura.
    color = over(cursor_shadow(pixel, cursor_pos), color);
    color = over(cursor_smoke(pixel, cursor_pos), color);
    color = over(ripple(pixel), color);
    color = over(no_no_mark(pixel, cursor_pos), color);
    color = over(cursor_sample(pixel, cursor_pos), color);
    return color;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    var out: VertexOut;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return render_pixel(position.xy);
}

@compute @workgroup_size(1)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x == 4294967295u {
        return;
    }
    let progress = effect_progress();
    let ripple_progress = saturate(safe_div(frame.effect.x, frame.effect.z));
    let ripple_radius = mix(frame.ripple.x, frame.ripple.y, ease_in_out(ripple_progress));
    let ripple_alpha = 1.0 - ripple_progress;
    let cursor_pos = animated_cursor_position();
    let rotation = cursor_rotation_deg();
    let trail_head_alpha = frame.trail.y;
    conformance_out.values[0] = vec4<f32>(progress, ripple_radius, ripple_alpha, 0.0);
    conformance_out.values[1] = vec4<f32>(cursor_pos, rotation, cursor_scale());
    conformance_out.values[2] = vec4<f32>(no_no_rotation_offset(progress), trail_head_alpha, 0.0, 0.0);
    conformance_out.values[3] = edge_glow(vec2<f32>(1.0, frame.surface_size_px.y * 0.5));
    conformance_out.values[4] = ripple(first_effect_point_or_cursor() + vec2<f32>(ripple_radius, 0.0));
    conformance_out.values[5] = vec4<f32>(0.0);
}
"#;

/// Create the premultiplied-alpha render pipeline for `format`.
pub fn create_effect_pipeline(
    device: &::wgpu::Device,
    bind_group_layout: &::wgpu::BindGroupLayout,
    format: ::wgpu::TextureFormat,
) -> ::wgpu::RenderPipeline {
    let shader = device.create_shader_module(::wgpu::ShaderModuleDescriptor {
        label: Some("sky-cua overlay effect shader"),
        source: ::wgpu::ShaderSource::Wgsl(EFFECT_SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&::wgpu::PipelineLayoutDescriptor {
        label: Some("sky-cua overlay effect pipeline layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&::wgpu::RenderPipelineDescriptor {
        label: Some("sky-cua overlay effect render pipeline"),
        layout: Some(&pipeline_layout),
        vertex: ::wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: ::wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(::wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: ::wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(::wgpu::ColorTargetState {
                format,
                blend: Some(::wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: ::wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: ::wgpu::PrimitiveState {
            topology: ::wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: ::wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: ::wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: ::wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub fn create_effect_bind_group_layout(device: &::wgpu::Device) -> ::wgpu::BindGroupLayout {
    device.create_bind_group_layout(&::wgpu::BindGroupLayoutDescriptor {
        label: Some("sky-cua overlay effect bind group layout"),
        entries: &[
            ::wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: ::wgpu::ShaderStages::VERTEX_FRAGMENT | ::wgpu::ShaderStages::COMPUTE,
                ty: ::wgpu::BindingType::Buffer {
                    ty: ::wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            ::wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: ::wgpu::ShaderStages::FRAGMENT | ::wgpu::ShaderStages::COMPUTE,
                ty: ::wgpu::BindingType::Buffer {
                    ty: ::wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            ::wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: ::wgpu::ShaderStages::FRAGMENT,
                ty: ::wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: ::wgpu::TextureViewDimension::D2,
                    sample_type: ::wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            ::wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: ::wgpu::ShaderStages::FRAGMENT,
                ty: ::wgpu::BindingType::Sampler(::wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::{EFFECT_SHADER, create_effect_bind_group_layout, create_effect_pipeline};
    use crate::renderer::{
        CursorImage,
        buffers::{
            EffectUniformInput, build_effect_uniform, effect_points_as_bytes,
            effect_uniform_as_bytes,
        },
        scene::{CursorPoint, EffectScene},
    };
    use sky_cua_platform::model::AgentOverlayGestureKind;
    use std::sync::mpsc;

    #[test]
    fn shader_contains_phase_four_effect_entry_points() {
        for needle in [
            "fn edge_glow",
            "fn inward_waves",
            "fn cursor_smoke",
            "fn ripple",
            "fn trail",
            "fn no_no_mark",
            "fn cursor_rotation_deg",
            "fn animated_cursor_position",
            "@compute",
        ] {
            assert!(
                EFFECT_SHADER.contains(needle),
                "missing WGSL symbol {needle}"
            );
        }
    }

    #[test]
    fn wgsl_compute_conformance_matches_fixture_values() {
        let Some((device, queue)) = test_device() else {
            eprintln!("skipping WGPU conformance test: no adapter available");
            return;
        };
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../resources/overlay/wgsl_animation_fixtures.json"
        ))
        .expect("parse WGSL animation fixtures");
        let layout = create_effect_bind_group_layout(&device);

        // tap_midpoint: Tap at progress 0.5, agent in control. Pins the lit edge
        // glow and the cursor-scale bounce overshoot at the midpoint.
        let expected = &fixture["fixtures"]["tap_midpoint"]["expected"];
        let (ub, pb, bind_group) = test_effect_bind_group(
            &device,
            &queue,
            &layout,
            Some((AgentOverlayGestureKind::Tap, 190, 380)),
            true,
        );
        let _keep_alive = (ub, pb);
        let values = run_conformance(&device, &queue, &layout, &bind_group);
        assert!((values[0] - expected["progress"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!((values[1] - expected["ripple_radius"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!((values[2] - expected["ripple_alpha"].as_f64().unwrap() as f32).abs() < 0.01);
        assert_eq!(values[4], expected["cursor"]["x"].as_f64().unwrap() as f32);
        assert_eq!(values[5], expected["cursor"]["y"].as_f64().unwrap() as f32);
        assert_eq!(
            values[6], 0.0,
            "tap/rest cursor rotation should match Android"
        );
        assert!(
            (values[7] - expected["cursor_scale"].as_f64().unwrap() as f32).abs() < 0.01,
            "cursor scale follows the Android bounce at the tap midpoint"
        );
        assert!(
            (values[12 + 3] - expected["edge_glow_alpha"].as_f64().unwrap() as f32).abs() < 0.02,
            "edge glow rim lights at the screen edge when the agent is in control"
        );
        assert!(
            (values[20 + 3] - expected["outside_alpha"].as_f64().unwrap() as f32).abs() < 0.001,
            "outside sample stays transparent"
        );

        // tap_offquarter: progress 0.3 exposes the eased ripple radius and the
        // damped-cosine cursor-scale bounce that both coincide with the old
        // linear math at the 0.5 midpoint.
        let off = &fixture["fixtures"]["tap_offquarter"]["expected"];
        let (ub, pb, bind_group) = test_effect_bind_group(
            &device,
            &queue,
            &layout,
            Some((AgentOverlayGestureKind::Tap, 114, 380)),
            true,
        );
        let _keep_alive = (ub, pb);
        let values = run_conformance(&device, &queue, &layout, &bind_group);
        assert!((values[0] - off["progress"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!(
            (values[1] - off["ripple_radius"].as_f64().unwrap() as f32).abs() < 0.01,
            "ripple radius eases to match Android off the midpoint"
        );
        assert!((values[2] - off["ripple_alpha"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!(
            (values[7] - off["cursor_scale"].as_f64().unwrap() as f32).abs() < 0.01,
            "cursor scale follows the Android bounce off the midpoint"
        );

        // no_no_midshake: progress 0.5 is where the previous WGSL gave the wrong
        // mid-shake angle; Android holds full amplitude through the hold window.
        let nn = &fixture["fixtures"]["no_no_midshake"]["expected"];
        let (ub, pb, bind_group) = test_effect_bind_group(
            &device,
            &queue,
            &layout,
            Some((AgentOverlayGestureKind::NoNo, 380, 760)),
            true,
        );
        let _keep_alive = (ub, pb);
        let values = run_conformance(&device, &queue, &layout, &bind_group);
        assert!((values[0] - nn["progress"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!(
            (values[8] - nn["rotation_offset_deg"].as_f64().unwrap() as f32).abs() < 0.01,
            "no_no rotation offset matches Android at mid-shake"
        );

        // no_no_tail: progress 0.78 is in the eased tail the previous WGSL
        // hard-zeroed; Android plays the shake out through this window.
        let tail = &fixture["fixtures"]["no_no_tail"]["expected"];
        let (ub, pb, bind_group) = test_effect_bind_group(
            &device,
            &queue,
            &layout,
            Some((AgentOverlayGestureKind::NoNo, 780, 1000)),
            true,
        );
        let _keep_alive = (ub, pb);
        let values = run_conformance(&device, &queue, &layout, &bind_group);
        assert!((values[0] - tail["progress"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!(
            (values[8] - tail["rotation_offset_deg"].as_f64().unwrap() as f32).abs() < 0.01,
            "no_no rotation plays through the eased tail to match Android"
        );
    }

    fn run_conformance(
        device: &::wgpu::Device,
        queue: &::wgpu::Queue,
        layout: &::wgpu::BindGroupLayout,
        bind_group: &::wgpu::BindGroup,
    ) -> Vec<f32> {
        let output_layout = device.create_bind_group_layout(&::wgpu::BindGroupLayoutDescriptor {
            label: Some("sky-cua conformance output layout"),
            entries: &[::wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: ::wgpu::ShaderStages::COMPUTE,
                ty: ::wgpu::BindingType::Buffer {
                    ty: ::wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let output = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua conformance output"),
            size: 6 * 16,
            usage: ::wgpu::BufferUsages::STORAGE | ::wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua conformance readback"),
            size: 6 * 16,
            usage: ::wgpu::BufferUsages::MAP_READ | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let output_group = device.create_bind_group(&::wgpu::BindGroupDescriptor {
            label: Some("sky-cua conformance output group"),
            layout: &output_layout,
            entries: &[::wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(::wgpu::ShaderModuleDescriptor {
            label: Some("sky-cua conformance shader"),
            source: ::wgpu::ShaderSource::Wgsl(EFFECT_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&::wgpu::PipelineLayoutDescriptor {
            label: Some("sky-cua conformance pipeline layout"),
            bind_group_layouts: &[Some(layout), Some(&output_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&::wgpu::ComputePipelineDescriptor {
            label: Some("sky-cua conformance compute pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_main"),
            compilation_options: ::wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&::wgpu::CommandEncoderDescriptor {
            label: Some("sky-cua conformance encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&::wgpu::ComputePassDescriptor {
                label: Some("sky-cua conformance pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.set_bind_group(1, &output_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 6 * 16);
        queue.submit(Some(encoder.finish()));
        read_f32_buffer(device, &readback, 6 * 4)
    }

    #[test]
    fn offscreen_render_is_deterministic_and_hidden_is_transparent() {
        let Some((device, queue)) = test_device() else {
            eprintln!("skipping WGPU offscreen test: no adapter available");
            return;
        };
        let hidden = render_offscreen_alpha(&device, &queue, false);
        assert!(hidden.iter().all(|alpha| *alpha == 0));

        let visible = render_offscreen_alpha(&device, &queue, true);
        assert!(visible.iter().any(|alpha| *alpha > 0));
        assert_eq!(visible, render_offscreen_alpha(&device, &queue, true));
    }

    fn test_device() -> Option<(::wgpu::Device, ::wgpu::Queue)> {
        let instance = ::wgpu::Instance::new(::wgpu::InstanceDescriptor {
            backends: ::wgpu::Backends::VULKAN | ::wgpu::Backends::GL,
            ..::wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            pollster::block_on(instance.request_adapter(&::wgpu::RequestAdapterOptions {
                power_preference: ::wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
            }))
            .ok()?;
        pollster::block_on(adapter.request_device(&::wgpu::DeviceDescriptor {
            label: Some("sky-cua overlay shader test device"),
            required_features: ::wgpu::Features::empty(),
            required_limits: ::wgpu::Limits::default(),
            experimental_features: ::wgpu::ExperimentalFeatures::disabled(),
            memory_hints: ::wgpu::MemoryHints::Performance,
            trace: ::wgpu::Trace::Off,
        }))
        .ok()
    }

    fn test_effect_bind_group(
        device: &::wgpu::Device,
        queue: &::wgpu::Queue,
        layout: &::wgpu::BindGroupLayout,
        scene: Option<(AgentOverlayGestureKind, u64, u64)>,
        glow_active: bool,
    ) -> (::wgpu::Buffer, ::wgpu::Buffer, ::wgpu::BindGroup) {
        let effect = scene.map(|(kind, _now_ms, duration_ms)| EffectScene {
            kind,
            started_at_ms: 0,
            duration_ms,
            points: vec![CursorPoint { x: 50.0, y: 60.0 }],
        });
        let (uniform, points) = build_effect_uniform(EffectUniformInput {
            width: 96,
            height: 96,
            now_ms: scene.map_or(0, |(_, now_ms, _)| now_ms),
            cursor: None,
            effect: effect.as_ref(),
            glow_active,
            // Representative logical density (~120 logical DPI) so the WGSL
            // edge glow sizes its physical-unit rim/containment in tests.
            px_per_mm: 4.7,
            render_scale: 1.0,
        });
        let uniform_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua test uniform"),
            size: std::mem::size_of_val(&uniform) as u64,
            usage: ::wgpu::BufferUsages::UNIFORM | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, effect_uniform_as_bytes(&uniform));
        let point_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua test point buffer"),
            size: std::mem::size_of_val(&points) as u64,
            usage: ::wgpu::BufferUsages::STORAGE | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&point_buffer, 0, effect_points_as_bytes(&points));
        let texture = device.create_texture(&::wgpu::TextureDescriptor {
            label: Some("sky-cua test cursor texture"),
            size: ::wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: ::wgpu::TextureDimension::D2,
            format: ::wgpu::TextureFormat::Rgba8Unorm,
            usage: ::wgpu::TextureUsages::TEXTURE_BINDING | ::wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            ::wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: ::wgpu::Origin3d::ZERO,
                aspect: ::wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            ::wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            ::wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&::wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&::wgpu::SamplerDescriptor::default());
        let bind_group = device.create_bind_group(&::wgpu::BindGroupDescriptor {
            label: Some("sky-cua test effect bind group"),
            layout,
            entries: &[
                ::wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                ::wgpu::BindGroupEntry {
                    binding: 1,
                    resource: point_buffer.as_entire_binding(),
                },
                ::wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ::wgpu::BindingResource::TextureView(&view),
                },
                ::wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ::wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        (uniform_buffer, point_buffer, bind_group)
    }

    fn render_offscreen_alpha(
        device: &::wgpu::Device,
        queue: &::wgpu::Queue,
        visible: bool,
    ) -> Vec<u8> {
        let layout = create_effect_bind_group_layout(device);
        let (uniform_buffer, point_buffer, bind_group) = test_effect_bind_group(
            device,
            queue,
            &layout,
            visible.then_some((AgentOverlayGestureKind::Tap, 190, 380)),
            visible,
        );
        let _keep_alive = (uniform_buffer, point_buffer);
        let pipeline = create_effect_pipeline(device, &layout, ::wgpu::TextureFormat::Rgba8Unorm);
        let texture = device.create_texture(&::wgpu::TextureDescriptor {
            label: Some("sky-cua offscreen texture"),
            size: ::wgpu::Extent3d {
                width: 96,
                height: 96,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: ::wgpu::TextureDimension::D2,
            format: ::wgpu::TextureFormat::Rgba8Unorm,
            usage: ::wgpu::TextureUsages::RENDER_ATTACHMENT | ::wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua offscreen readback"),
            size: 512 * 96,
            usage: ::wgpu::BufferUsages::MAP_READ | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&::wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&::wgpu::CommandEncoderDescriptor {
            label: Some("sky-cua offscreen encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&::wgpu::RenderPassDescriptor {
                label: Some("sky-cua offscreen pass"),
                color_attachments: &[Some(::wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: ::wgpu::Operations {
                        load: ::wgpu::LoadOp::Clear(::wgpu::Color::TRANSPARENT),
                        store: ::wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            ::wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: ::wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: Some(96),
                },
            },
            ::wgpu::Extent3d {
                width: 96,
                height: 96,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let bytes = read_bytes(device, &readback, 512 * 96);
        bytes
            .chunks(512)
            .flat_map(|row| row[..96 * 4].chunks(4).map(|rgba| rgba[3]))
            .collect()
    }

    fn read_f32_buffer(
        device: &::wgpu::Device,
        buffer: &::wgpu::Buffer,
        f32_count: usize,
    ) -> Vec<f32> {
        read_bytes(device, buffer, f32_count * 4)
            .chunks_exact(4)
            .map(|chunk| f32::from_ne_bytes(chunk.try_into().expect("four bytes")))
            .collect()
    }

    fn read_bytes(device: &::wgpu::Device, buffer: &::wgpu::Buffer, byte_count: usize) -> Vec<u8> {
        let slice = buffer.slice(..byte_count as u64);
        let (tx, rx) = mpsc::channel();
        slice.map_async(::wgpu::MapMode::Read, move |result| {
            tx.send(result).expect("send map result");
        });
        device
            .poll(::wgpu::PollType::wait_indefinitely())
            .expect("poll device");
        rx.recv()
            .expect("receive map result")
            .expect("map buffer for read");
        let data = slice.get_mapped_range().to_vec();
        buffer.unmap();
        data
    }

    /// Render the full effect pipeline (`fs_main`) for one gesture frame into a
    /// tightly-packed premultiplied RGBA8 buffer over a transparent backdrop,
    /// using the real cursor glyph. Drives nothing on the desktop; the consumer
    /// composites the frame over a chosen backdrop for visual gesture review.
    #[allow(clippy::too_many_arguments)]
    fn render_gesture_rgba(
        device: &::wgpu::Device,
        queue: &::wgpu::Queue,
        width: u32,
        height: u32,
        kind: AgentOverlayGestureKind,
        now_ms: u64,
        duration_ms: u64,
        points: &[CursorPoint],
    ) -> Vec<u8> {
        let layout = create_effect_bind_group_layout(device);
        let effect = EffectScene {
            kind,
            started_at_ms: 0,
            duration_ms,
            points: points.to_vec(),
        };
        let (uniform, point_data) = build_effect_uniform(EffectUniformInput {
            width,
            height,
            now_ms,
            cursor: points.first().copied(),
            effect: Some(&effect),
            glow_active: true,
            px_per_mm: 4.7,
            render_scale: 1.0,
        });
        let uniform_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua gesture uniform"),
            size: std::mem::size_of_val(&uniform) as u64,
            usage: ::wgpu::BufferUsages::UNIFORM | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, effect_uniform_as_bytes(&uniform));
        let point_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua gesture points"),
            size: std::mem::size_of_val(&point_data) as u64,
            usage: ::wgpu::BufferUsages::STORAGE | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&point_buffer, 0, effect_points_as_bytes(&point_data));

        let cursor = CursorImage::load().expect("load cursor asset");
        let cursor_texture = device.create_texture(&::wgpu::TextureDescriptor {
            label: Some("sky-cua gesture cursor texture"),
            size: ::wgpu::Extent3d {
                width: cursor.width,
                height: cursor.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: ::wgpu::TextureDimension::D2,
            format: ::wgpu::TextureFormat::Rgba8Unorm,
            usage: ::wgpu::TextureUsages::TEXTURE_BINDING | ::wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            ::wgpu::TexelCopyTextureInfo {
                texture: &cursor_texture,
                mip_level: 0,
                origin: ::wgpu::Origin3d::ZERO,
                aspect: ::wgpu::TextureAspect::All,
            },
            &cursor.rgba,
            ::wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(cursor.width * 4),
                rows_per_image: Some(cursor.height),
            },
            ::wgpu::Extent3d {
                width: cursor.width,
                height: cursor.height,
                depth_or_array_layers: 1,
            },
        );
        let cursor_view = cursor_texture.create_view(&::wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&::wgpu::SamplerDescriptor::default());
        let bind_group = device.create_bind_group(&::wgpu::BindGroupDescriptor {
            label: Some("sky-cua gesture bind group"),
            layout: &layout,
            entries: &[
                ::wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                ::wgpu::BindGroupEntry {
                    binding: 1,
                    resource: point_buffer.as_entire_binding(),
                },
                ::wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ::wgpu::BindingResource::TextureView(&cursor_view),
                },
                ::wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ::wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline = create_effect_pipeline(device, &layout, ::wgpu::TextureFormat::Rgba8Unorm);
        let target = device.create_texture(&::wgpu::TextureDescriptor {
            label: Some("sky-cua gesture target"),
            size: ::wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: ::wgpu::TextureDimension::D2,
            format: ::wgpu::TextureFormat::Rgba8Unorm,
            usage: ::wgpu::TextureUsages::RENDER_ATTACHMENT | ::wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let bytes_per_row = (width * 4).next_multiple_of(256);
        let readback = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua gesture readback"),
            size: u64::from(bytes_per_row * height),
            usage: ::wgpu::BufferUsages::MAP_READ | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let target_view = target.create_view(&::wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&::wgpu::CommandEncoderDescriptor {
            label: Some("sky-cua gesture encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&::wgpu::RenderPassDescriptor {
                label: Some("sky-cua gesture pass"),
                color_attachments: &[Some(::wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: ::wgpu::Operations {
                        load: ::wgpu::LoadOp::Clear(::wgpu::Color::TRANSPARENT),
                        store: ::wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            ::wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: ::wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            ::wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let _keep_alive = (uniform_buffer, point_buffer);
        queue.submit(Some(encoder.finish()));
        let padded = read_bytes(device, &readback, (bytes_per_row * height) as usize);
        let row_bytes = (width * 4) as usize;
        let mut tight = Vec::with_capacity(row_bytes * height as usize);
        for row in 0..height as usize {
            let start = row * bytes_per_row as usize;
            tight.extend_from_slice(&padded[start..start + row_bytes]);
        }
        tight
    }

    /// Gated visual capture: renders each gesture across its timeline to raw RGBA
    /// frames for offline review. No-op unless `SKY_CUA_CAPTURE_GESTURES` is set,
    /// so it never runs in normal CI; output dir defaults to
    /// `/tmp/overlay-demo/gestures` (override with `SKY_CUA_CAPTURE_DIR`).
    #[test]
    fn capture_gesture_frames_when_requested() {
        if std::env::var_os("SKY_CUA_CAPTURE_GESTURES").is_none() {
            return;
        }
        let Some((device, queue)) = test_device() else {
            eprintln!("skipping gesture capture: no adapter available");
            return;
        };
        let out = std::path::PathBuf::from(
            std::env::var("SKY_CUA_CAPTURE_DIR")
                .unwrap_or_else(|_| "/tmp/overlay-demo/gestures".to_string()),
        );
        std::fs::create_dir_all(&out).expect("create capture dir");
        let width = 768u32;
        let height = 448u32;
        let cx = f64::from(width) / 2.0;
        let cy = f64::from(height) / 2.0;
        let p = |fx: f64, fy: f64| CursorPoint {
            x: fx * f64::from(width),
            y: fy * f64::from(height),
        };
        let scenes: [(&str, AgentOverlayGestureKind, u64, Vec<CursorPoint>); 4] = [
            (
                "tap",
                AgentOverlayGestureKind::Tap,
                380,
                vec![CursorPoint { x: cx, y: cy }],
            ),
            (
                "drag",
                AgentOverlayGestureKind::Drag,
                950,
                vec![p(0.22, 0.66), p(0.78, 0.34)],
            ),
            (
                "swipe",
                AgentOverlayGestureKind::Swipe,
                950,
                vec![p(0.18, 0.5), p(0.82, 0.5)],
            ),
            (
                "no_no",
                AgentOverlayGestureKind::NoNo,
                760,
                vec![CursorPoint { x: cx, y: cy }],
            ),
        ];
        let progresses = [0.18f64, 0.42, 0.66, 0.9];
        for (name, kind, duration_ms, points) in &scenes {
            let duration_ms = *duration_ms;
            for (frame_index, prog) in progresses.iter().enumerate() {
                let now_ms = (prog * duration_ms as f64) as u64;
                let rgba = render_gesture_rgba(
                    &device,
                    &queue,
                    width,
                    height,
                    *kind,
                    now_ms,
                    duration_ms,
                    points,
                );
                std::fs::write(out.join(format!("{name}_{frame_index}.rgba")), &rgba)
                    .expect("write gesture frame");
            }
        }
        std::fs::write(out.join("dims.txt"), format!("{width} {height}\n")).expect("write dims");
        // Also dump the raw cursor texture so its glyph quality can be judged at
        // full resolution, separate from the on-screen footprint size.
        let cursor = CursorImage::load().expect("load cursor for capture");
        std::fs::write(out.join("cursor_texture.rgba"), &cursor.rgba).expect("write cursor tex");
        std::fs::write(
            out.join("cursor_dims.txt"),
            format!("{} {}\n", cursor.width, cursor.height),
        )
        .expect("write cursor dims");
        eprintln!("wrote gesture frames to {}", out.display());
    }
}
