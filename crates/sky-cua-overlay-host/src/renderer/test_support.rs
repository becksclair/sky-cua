//! Shared offscreen GPU helpers for renderer tests: headless device setup,
//! a full-pipeline single-frame renderer, and buffer readback. `#[cfg(test)]`
//! only — nothing here touches a compositor; frames render into an offscreen
//! texture and are read back over a mapped buffer.

use std::sync::{OnceLock, mpsc};

use crate::renderer::CursorImage;
use crate::renderer::buffers::{
    EffectUniformInput, build_effect_uniform, effect_points_as_bytes, effect_uniform_as_bytes,
};
use crate::renderer::scene::{CursorPoint, EffectScene};
use crate::renderer::shaders::{create_effect_bind_group_layout, create_effect_pipeline};

/// Request a headless device on the backends the overlay host actually uses.
/// Returns `None` when the machine has no adapter (CI without a GPU): callers
/// skip with an eprintln instead of failing.
pub(crate) fn test_device() -> Option<(::wgpu::Device, ::wgpu::Queue)> {
    let instance = ::wgpu::Instance::new(::wgpu::InstanceDescriptor {
        backends: ::wgpu::Backends::VULKAN | ::wgpu::Backends::GL,
        ..::wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&::wgpu::RequestAdapterOptions {
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

/// One frame of the full effect pipeline (`fs_main`) with CPU-supplied motion
/// state. The gesture and motion capture dumps both build their uniforms
/// through here so the offline evidence exercises the production shader with
/// the production uniform packing.
pub(crate) struct FrameRenderInput<'a> {
    pub width: u32,
    pub height: u32,
    /// Epoch-style clock the effect timeline runs on (compared against the
    /// scene's `started_at_ms` inside the uniform build).
    pub now_ms: u64,
    /// Drawn cursor position; `None` falls back to the first effect point in
    /// WGSL, matching the production fallback for effect-only frames.
    pub cursor: Option<CursorPoint>,
    pub effect: Option<&'a EffectScene>,
    /// CPU-eased glyph rotation in degrees (motion driver output).
    pub cursor_rotation_deg: f32,
    /// Smoke-aura master alpha in `[0, 1]` (motion driver cloud bloom).
    pub cursor_cloud_alpha: f32,
}

/// The real cursor glyph texture, synthesized once per test process: the SDF
/// bake is not cheap and the motion capture renders a hundred-plus frames.
fn cursor_image() -> &'static CursorImage {
    static IMAGE: OnceLock<CursorImage> = OnceLock::new();
    IMAGE.get_or_init(|| CursorImage::load().expect("load cursor asset"))
}

/// Render the full effect pipeline for one frame into a tightly-packed
/// premultiplied RGBA8 buffer over a transparent backdrop, using the REAL
/// cursor glyph texture. Drives nothing on the desktop; consumers composite
/// the frame over a chosen backdrop for visual review. Glow is always active
/// and the frame renders at scale 1.0 with a representative logical density
/// (~120 logical DPI), matching the original gesture-dump conditions.
pub(crate) fn render_frame_rgba(
    device: &::wgpu::Device,
    queue: &::wgpu::Queue,
    input: FrameRenderInput<'_>,
) -> Vec<u8> {
    let FrameRenderInput {
        width,
        height,
        now_ms,
        cursor,
        effect,
        cursor_rotation_deg,
        cursor_cloud_alpha,
    } = input;
    let layout = create_effect_bind_group_layout(device);
    let (uniform, point_data) = build_effect_uniform(EffectUniformInput {
        width,
        height,
        now_ms,
        cursor,
        effect,
        glow_active: true,
        px_per_mm: 4.7,
        render_scale: 1.0,
        cursor_rotation_deg,
        cursor_cloud_alpha,
    });
    let uniform_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
        label: Some("sky-cua frame uniform"),
        size: std::mem::size_of_val(&uniform) as u64,
        usage: ::wgpu::BufferUsages::UNIFORM | ::wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform_buffer, 0, effect_uniform_as_bytes(&uniform));
    let point_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
        label: Some("sky-cua frame points"),
        size: std::mem::size_of_val(&point_data) as u64,
        usage: ::wgpu::BufferUsages::STORAGE | ::wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&point_buffer, 0, effect_points_as_bytes(&point_data));

    let cursor_asset = cursor_image();
    let cursor_texture = device.create_texture(&::wgpu::TextureDescriptor {
        label: Some("sky-cua frame cursor texture"),
        size: ::wgpu::Extent3d {
            width: cursor_asset.width,
            height: cursor_asset.height,
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
        &cursor_asset.rgba,
        ::wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(cursor_asset.width * 4),
            rows_per_image: Some(cursor_asset.height),
        },
        ::wgpu::Extent3d {
            width: cursor_asset.width,
            height: cursor_asset.height,
            depth_or_array_layers: 1,
        },
    );
    let cursor_view = cursor_texture.create_view(&::wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&::wgpu::SamplerDescriptor::default());
    let bind_group = device.create_bind_group(&::wgpu::BindGroupDescriptor {
        label: Some("sky-cua frame bind group"),
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
        label: Some("sky-cua frame target"),
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
        label: Some("sky-cua frame readback"),
        size: u64::from(bytes_per_row * height),
        usage: ::wgpu::BufferUsages::MAP_READ | ::wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let target_view = target.create_view(&::wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&::wgpu::CommandEncoderDescriptor {
        label: Some("sky-cua frame encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&::wgpu::RenderPassDescriptor {
            label: Some("sky-cua frame pass"),
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

pub(crate) fn read_f32_buffer(
    device: &::wgpu::Device,
    buffer: &::wgpu::Buffer,
    f32_count: usize,
) -> Vec<f32> {
    read_bytes(device, buffer, f32_count * 4)
        .chunks_exact(4)
        .map(|chunk| f32::from_ne_bytes(chunk.try_into().expect("four bytes")))
        .collect()
}

pub(crate) fn read_bytes(
    device: &::wgpu::Device,
    buffer: &::wgpu::Buffer,
    byte_count: usize,
) -> Vec<u8> {
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
