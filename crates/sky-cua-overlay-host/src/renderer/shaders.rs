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
    return 0.5 - 0.5 * cos(phase * PI * 2.0);
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
    let hold_fraction = frame.no_no.z;
    if progress <= 0.0 || progress >= 1.0 || progress > hold_fraction {
        return 0.0;
    }
    let held = saturate(safe_div(progress, max(hold_fraction, 0.0001)));
    let envelope = sin(held * PI);
    return frame.no_no.x * envelope * sin(held * frame.no_no.y * PI * 2.0);
}

fn cursor_rotation_deg() -> f32 {
    let kind = frame.flags.y;
    var rotation = frame.trail.z;
    if (kind == KIND_DRAG || kind == KIND_SWIPE) && active_point_count() >= 2u {
        let direction = last_effect_point_or_cursor() - first_effect_point_or_cursor();
        if length(direction) > 0.001 {
            rotation = degrees(atan2(direction.y, direction.x)) + frame.trail.z;
        }
    }
    if kind == KIND_NO_NO {
        rotation = rotation + no_no_rotation_offset(effect_progress());
    }
    return rotation;
}

fn cursor_scale() -> f32 {
    let kind = frame.flags.y;
    if kind != KIND_TAP {
        return 1.0;
    }
    let press_fraction = frame.no_no.w;
    let progress = effect_progress();
    if progress <= press_fraction {
        return mix(1.0, frame.trail.w, safe_div(progress, press_fraction));
    }
    return mix(frame.trail.w, 1.0, safe_div(progress - press_fraction, 1.0 - press_fraction));
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

fn edge_glow(pixel: vec2<f32>) -> vec4<f32> {
    if frame.flags.x == 0u {
        return vec4<f32>(0.0);
    }
    let distance = edge_distance(pixel);
    let breathe = breathing(frame.timing.x, frame.timing.y);
    let base_alpha = mix(frame.color_agent_pink.a, frame.color_agent_pink_light.a, breathe);
    let base = (1.0 - smoothstep(frame.glow.x, frame.glow.x + frame.glow.y, distance)) * base_alpha;
    let core = (1.0 - smoothstep(frame.glow.z, frame.glow.z + frame.glow.w, distance)) * 0.86;
    return premul(frame.color_agent_pink.xyz, max(base * 0.36, core));
}

fn inward_waves(pixel: vec2<f32>) -> vec4<f32> {
    if frame.flags.x == 0u {
        return vec4<f32>(0.0);
    }
    let distance = edge_distance(pixel);
    let min_dim = min(frame.surface_size_px.x, frame.surface_size_px.y);
    let travel_px = min_dim * frame.wave.z;
    let phase = fract(safe_div(frame.timing.x, frame.timing.z));
    var alpha = 0.0;
    for (var i = 0u; i < 3u; i = i + 1u) {
        let wave_phase = fract(phase + f32(i) / 3.0);
        let center = frame.glow.x + wave_phase * travel_px;
        let band = 1.0 - smoothstep(frame.wave.x, frame.wave.x + frame.wave.y, abs(distance - center));
        alpha = max(alpha, band * (1.0 - wave_phase) * frame.wave.w);
    }
    return premul(frame.color_agent_pink_light.xyz, alpha);
}

fn halo(pixel: vec2<f32>, cursor_pos: vec2<f32>) -> vec4<f32> {
    if frame.flags.x == 0u {
        return vec4<f32>(0.0);
    }
    let breathe = breathing(frame.timing.x, frame.timing.w);
    let radius = frame.halo.x * mix(frame.halo.y, frame.halo.z, breathe);
    let distance = length(pixel - cursor_pos);
    let alpha = (1.0 - smoothstep(radius * 0.28, radius, distance)) * frame.halo.w * frame.color_halo_inner.a;
    return premul(frame.color_halo_inner.xyz, alpha);
}

fn ripple(pixel: vec2<f32>) -> vec4<f32> {
    let kind = frame.flags.y;
    if kind == KIND_NONE || active_point_count() == 0u {
        return vec4<f32>(0.0);
    }
    let progress = saturate(safe_div(frame.effect.x, frame.effect.z));
    let center = first_effect_point_or_cursor();
    let radius = mix(frame.ripple.x, frame.ripple.y, progress);
    let distance = abs(length(pixel - center) - radius);
    let band = 1.0 - smoothstep(frame.ripple.z * 0.35, frame.ripple.z, distance);
    return premul(frame.color_agent_pink_light.xyz, band * (1.0 - progress) * frame.ripple.w);
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
    let band = 1.0 - smoothstep(frame.trail.x * 0.45, frame.trail.x, segment.x);
    let alpha = band * segment.y * frame.trail.y;
    return premul(frame.color_agent_pink.xyz, alpha);
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
    if uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 {
        return vec4<f32>(0.0);
    }
    let color = textureSample(cursor_texture, cursor_sampler, uv);
    return premul(color.rgb, color.a);
}

fn render_pixel(pixel: vec2<f32>) -> vec4<f32> {
    let cursor_pos = animated_cursor_position();
    var color = vec4<f32>(0.0);
    color = over(edge_glow(pixel), color);
    color = over(inward_waves(pixel), color);
    color = over(trail(pixel, cursor_pos), color);
    color = over(halo(pixel, cursor_pos), color);
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
    let ripple_radius = mix(frame.ripple.x, frame.ripple.y, ripple_progress);
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
            "fn halo",
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
        let expected = &fixture["fixtures"]["tap_midpoint"]["expected"];
        let layout = create_effect_bind_group_layout(&device);
        let (uniform_buffer, point_buffer, bind_group) =
            test_effect_bind_group(&device, &queue, &layout, true);
        let _keep_alive = (uniform_buffer, point_buffer);
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
            bind_group_layouts: &[Some(&layout), Some(&output_layout)],
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_bind_group(1, &output_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 6 * 16);
        queue.submit(Some(encoder.finish()));
        let values = read_f32_buffer(&device, &readback, 6 * 4);

        assert!((values[0] - expected["progress"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!((values[1] - expected["ripple_radius"].as_f64().unwrap() as f32).abs() < 0.01);
        assert!((values[2] - expected["ripple_alpha"].as_f64().unwrap() as f32).abs() < 0.01);
        assert_eq!(values[4], expected["cursor"]["x"].as_f64().unwrap() as f32);
        assert_eq!(values[5], expected["cursor"]["y"].as_f64().unwrap() as f32);
        assert!(values[12 + 3] > 0.0, "edge glow alpha should be visible");
        assert!(
            (values[20 + 3] - expected["outside_alpha"].as_f64().unwrap() as f32).abs() < 0.001,
            "outside sample stays transparent"
        );
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
        visible: bool,
    ) -> (::wgpu::Buffer, ::wgpu::Buffer, ::wgpu::BindGroup) {
        let effect = visible.then(|| EffectScene {
            kind: AgentOverlayGestureKind::Tap,
            started_at_ms: 0,
            duration_ms: 380,
            points: vec![CursorPoint { x: 50.0, y: 60.0 }],
        });
        let (uniform, points) = build_effect_uniform(EffectUniformInput {
            width: 96,
            height: 96,
            now_ms: if visible { 190 } else { 0 },
            cursor: None,
            effect: effect.as_ref(),
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
            format: ::wgpu::TextureFormat::Rgba8UnormSrgb,
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
        let (uniform_buffer, point_buffer, bind_group) =
            test_effect_bind_group(device, queue, &layout, visible);
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
}
