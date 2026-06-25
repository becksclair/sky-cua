//! WGSL shader source and pipeline factory for the static cursor.

use crate::renderer::buffers::cursor_vertex_buffer_layout;

pub const CURSOR_SHADER: &str = r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var cursor_texture: texture_2d<f32>;
@group(0) @binding(1) var cursor_sampler: sampler;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(cursor_texture, cursor_sampler, in.uv);
    return vec4<f32>(color.rgb * color.a, color.a);
}
"#;

/// Create the premultiplied-alpha cursor render pipeline for `format`.
pub fn create_cursor_pipeline(
    device: &::wgpu::Device,
    bind_group_layout: &::wgpu::BindGroupLayout,
    format: ::wgpu::TextureFormat,
) -> ::wgpu::RenderPipeline {
    let shader = device.create_shader_module(::wgpu::ShaderModuleDescriptor {
        label: Some("sky-cua cursor shader"),
        source: ::wgpu::ShaderSource::Wgsl(CURSOR_SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&::wgpu::PipelineLayoutDescriptor {
        label: Some("sky-cua cursor pipeline layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&::wgpu::RenderPipelineDescriptor {
        label: Some("sky-cua cursor render pipeline"),
        layout: Some(&pipeline_layout),
        vertex: ::wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: ::wgpu::PipelineCompilationOptions::default(),
            buffers: &[cursor_vertex_buffer_layout()],
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
