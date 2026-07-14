//! WGPU renderer implementation for the GPU-driven agent cursor/effect scene.

use crate::renderer::{
    CursorImage,
    animation::{AnimationClock, SystemClock},
    buffers::{
        AgentEffectPoint, AgentEffectUniform, MAX_EFFECT_POINTS, build_effect_uniform,
        effect_points_as_bytes, effect_uniform_as_bytes,
    },
    scene::SurfaceDrawRequest,
    shaders::{create_effect_bind_group_layout, create_effect_pipeline},
    surface::{SurfaceGuard, choose_surface_format, supports_transparent_alpha_mode},
};
use anyhow::{Context, Result, bail};

/// Lightweight wrapper around a [`wgpu::Instance`].
///
/// The host creates an instance first, uses it to build [`SurfaceGuard`]s from
/// raw handles, then keeps the instance alive until after those guards drop.
#[derive(Debug)]
pub struct WgpuOverlayInstance {
    instance: ::wgpu::Instance,
}

impl WgpuOverlayInstance {
    pub fn new() -> Self {
        let mut instance_descriptor = ::wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = ::wgpu::Backends::VULKAN | ::wgpu::Backends::GL;
        let instance = ::wgpu::Instance::new(instance_descriptor);
        Self { instance }
    }

    pub(crate) fn inner(&self) -> &::wgpu::Instance {
        &self.instance
    }
}

impl Default for WgpuOverlayInstance {
    fn default() -> Self {
        Self::new()
    }
}

/// Static information about the selected GPU adapter.
#[derive(Debug, Clone)]
pub struct RendererInfo {
    pub adapter_name: String,
    pub backend: String,
    // Captured from the real adapter for future diagnostics surfaces; no
    // current caller reads these two beyond `adapter_name`/`backend`.
    #[allow(dead_code)]
    pub driver: String,
    #[allow(dead_code)]
    pub driver_info: String,
}

/// Renderer state. Does not own the surfaces: the host holds the
/// [`SurfaceGuard`]s and passes them in each frame.
pub struct WgpuOverlayRenderer {
    adapter: ::wgpu::Adapter,
    device: ::wgpu::Device,
    queue: ::wgpu::Queue,
    pipeline: Option<::wgpu::RenderPipeline>,
    pipeline_format: Option<::wgpu::TextureFormat>,
    bind_group_layout: ::wgpu::BindGroupLayout,
    bind_group: ::wgpu::BindGroup,
    uniform_buffer: ::wgpu::Buffer,
    point_buffer: ::wgpu::Buffer,
    clock: Box<dyn AnimationClock>,
    info: RendererInfo,
    /// Render-pass clear color. Transparent for the production click-through
    /// overlay; the effects playground overrides it to paint a solid backdrop.
    clear_color: ::wgpu::Color,
}

impl std::fmt::Debug for WgpuOverlayRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuOverlayRenderer")
            .field("info", &self.info)
            .field("pipeline_format", &self.pipeline_format)
            .finish_non_exhaustive()
    }
}

impl WgpuOverlayRenderer {
    /// Two-stage initialization: instance has already been used to create the
    /// host-owned surface guards. Here we pick an adapter compatible with the
    /// first active surface, validate *every* active surface, and request the
    /// device and cursor texture.
    pub fn new(
        instance: &WgpuOverlayInstance,
        surfaces: &mut [Option<SurfaceGuard>],
        cursor: &CursorImage,
    ) -> Result<Self> {
        Self::new_with_clock(instance, surfaces, cursor, Box::new(SystemClock))
    }

    pub fn new_with_clock(
        instance: &WgpuOverlayInstance,
        surfaces: &mut [Option<SurfaceGuard>],
        cursor: &CursorImage,
        clock: Box<dyn AnimationClock>,
    ) -> Result<Self> {
        let active_surfaces: Vec<&mut SurfaceGuard> = surfaces
            .iter_mut()
            .filter_map(|surface| surface.as_mut())
            .collect();
        if active_surfaces.is_empty() {
            bail!("no active wgpu surfaces available for renderer initialization");
        }

        let first_surface = active_surfaces[0].surface();
        let adapter = pollster::block_on(instance.inner().request_adapter(
            &::wgpu::RequestAdapterOptions {
                power_preference: ::wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: Some(first_surface),
            },
        ))
        .context("failed to find a GPU adapter compatible with layer-shell surfaces")?;

        // Fail closed if any active output cannot be rendered with this adapter.
        for (index, guard) in active_surfaces.iter().enumerate() {
            let capabilities = guard.capabilities(&adapter);
            if capabilities.formats.is_empty() {
                bail!("wgpu surface {index} has no compatible texture formats");
            }
            if choose_surface_format(&capabilities.formats).is_none() {
                bail!(
                    "wgpu surface {index} has no compatible premultiplied display-space format; expected Bgra8Unorm or Rgba8Unorm, advertised {:?}",
                    capabilities.formats
                );
            }
            if capabilities.present_modes.is_empty() {
                bail!("wgpu surface {index} has no compatible present modes");
            }
            if !supports_transparent_alpha_mode(&capabilities.alpha_modes) {
                bail!("wgpu surface {index} has no transparent alpha compositing mode");
            }
        }

        let (device, queue) =
            pollster::block_on(adapter.request_device(&::wgpu::DeviceDescriptor {
                label: Some("sky-cua overlay renderer"),
                required_features: ::wgpu::Features::empty(),
                required_limits: ::wgpu::Limits::default(),
                experimental_features: ::wgpu::ExperimentalFeatures::disabled(),
                memory_hints: ::wgpu::MemoryHints::Performance,
                trace: ::wgpu::Trace::Off,
            }))
            .context("failed to request wgpu device for overlay renderer")?;

        let bind_group_layout = create_effect_bind_group_layout(&device);
        let uniform_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua overlay effect uniform"),
            size: std::mem::size_of::<AgentEffectUniform>() as ::wgpu::BufferAddress,
            usage: ::wgpu::BufferUsages::UNIFORM | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let point_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua overlay effect points"),
            size: (std::mem::size_of::<AgentEffectPoint>() * MAX_EFFECT_POINTS)
                as ::wgpu::BufferAddress,
            usage: ::wgpu::BufferUsages::STORAGE | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = create_effect_bind_group(
            &device,
            &queue,
            &bind_group_layout,
            &uniform_buffer,
            &point_buffer,
            cursor,
        );

        let adapter_info = adapter.get_info();
        let info = RendererInfo {
            adapter_name: adapter_info.name,
            backend: format!("{:?}", adapter_info.backend).to_ascii_lowercase(),
            driver: adapter_info.driver,
            driver_info: adapter_info.driver_info,
        };

        Ok(Self {
            adapter,
            device,
            queue,
            pipeline: None,
            pipeline_format: None,
            bind_group_layout,
            bind_group,
            uniform_buffer,
            point_buffer,
            clock,
            info,
            clear_color: ::wgpu::Color::TRANSPARENT,
        })
    }

    /// Override the render-pass clear color (defaults to transparent). Used by
    /// the effects playground to paint a solid backdrop behind the overlay.
    pub fn set_clear_color(&mut self, color: ::wgpu::Color) {
        self.clear_color = color;
    }

    /// Information about the adapter selected during initialization.
    #[must_use]
    pub fn info(&self) -> &RendererInfo {
        &self.info
    }

    /// Render a frame to every active surface.
    ///
    /// `surfaces` and `requests` must have the same length; indices correspond
    /// to the host's layer ordering.
    pub fn draw(
        &mut self,
        surfaces: &mut [Option<SurfaceGuard>],
        requests: &[SurfaceDrawRequest],
    ) -> Result<()> {
        if requests.len() != surfaces.len() {
            bail!(
                "draw request count ({}) does not match surface count ({})",
                requests.len(),
                surfaces.len()
            );
        }

        for (index, request) in requests.iter().enumerate() {
            let Some(spec) = request else {
                continue;
            };
            let Some(guard) = surfaces[index].as_mut() else {
                continue;
            };

            let width = spec.width.max(1);
            let height = spec.height.max(1);
            // Render at physical buffer resolution (logical * integer buffer
            // scale) so the compositor downsamples a sharp buffer instead of
            // upscaling a soft logical one on hidpi / fractionally-scaled outputs.
            let render_scale = spec.render_scale.max(1.0);
            let phys_width = ((width as f32) * render_scale).round().max(1.0) as u32;
            let phys_height = ((height as f32) * render_scale).round().max(1.0) as u32;
            let config =
                guard.ensure_configured(&self.device, &self.adapter, phys_width, phys_height);

            // Transient surface states (lost/outdated after a resize or output
            // change, a momentary timeout/occlusion) are recoverable: the first
            // `acquire_frame` reconfigures the surface, so retry once before
            // failing. Bailing on the first transient would let a routine,
            // self-healing event permanently fail the renderer closed when the
            // draw runs on the message path (`render_reply_with_diagnostics`
            // marks the renderer `Unsupported` on any `Err`).
            let mut attempt = 0;
            let frame = loop {
                match guard.acquire_frame(&self.device) {
                    super::surface::SurfaceAcquisitionResult::Success(frame) => break frame,
                    super::surface::SurfaceAcquisitionResult::Validation => {
                        bail!(
                            "wgpu validation error while acquiring overlay frame for surface {index}"
                        );
                    }
                    super::surface::SurfaceAcquisitionResult::Retry(reason) => {
                        attempt += 1;
                        if attempt >= 2 {
                            bail!("wgpu surface {index} frame unavailable: {reason}");
                        }
                    }
                }
            };

            let view = frame
                .texture
                .create_view(&::wgpu::TextureViewDescriptor::default());
            let mut encoder =
                self.device
                    .create_command_encoder(&::wgpu::CommandEncoderDescriptor {
                        label: Some("sky-cua overlay cursor encoder"),
                    });
            {
                let mut pass = encoder.begin_render_pass(&::wgpu::RenderPassDescriptor {
                    label: Some("sky-cua overlay effect pass"),
                    color_attachments: &[Some(::wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: ::wgpu::Operations {
                            load: ::wgpu::LoadOp::Clear(self.clear_color),
                            store: ::wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                self.ensure_pipeline(config.format);
                let (uniform, points) = build_effect_uniform(super::buffers::EffectUniformInput {
                    width,
                    height,
                    now_ms: self.clock.now_ms(),
                    cursor: spec.cursor,
                    effect: spec.effect.as_ref(),
                    glow_active: spec.glow_active,
                    px_per_mm: spec.px_per_mm,
                    render_scale,
                    cursor_rotation_deg: spec.cursor_rotation_deg,
                    cursor_cloud_alpha: spec.cursor_cloud_alpha,
                });
                self.queue
                    .write_buffer(&self.uniform_buffer, 0, effect_uniform_as_bytes(&uniform));
                self.queue
                    .write_buffer(&self.point_buffer, 0, effect_points_as_bytes(&points));
                pass.set_pipeline(self.pipeline.as_ref().expect("pipeline initialized"));
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            self.queue.submit(Some(encoder.finish()));
            frame.present();
        }

        Ok(())
    }

    fn ensure_pipeline(&mut self, format: ::wgpu::TextureFormat) {
        if self.pipeline_format == Some(format) {
            return;
        }
        self.pipeline = Some(create_effect_pipeline(
            &self.device,
            &self.bind_group_layout,
            format,
        ));
        self.pipeline_format = Some(format);
    }
}

fn create_effect_bind_group(
    device: &::wgpu::Device,
    queue: &::wgpu::Queue,
    bind_group_layout: &::wgpu::BindGroupLayout,
    uniform_buffer: &::wgpu::Buffer,
    point_buffer: &::wgpu::Buffer,
    cursor: &CursorImage,
) -> ::wgpu::BindGroup {
    // Full mip chain: the texture is rendered at CURSOR_TEXTURE_SCALE x its
    // on-screen footprint, so a single bilinear tap minifying that 4x reduction
    // stair-steps the glyph edges. Trilinear sampling of box-filtered mips fixes
    // it across every output scale.
    let mip_level_count = 1 + cursor.mips.len() as u32;
    let texture = device.create_texture(&::wgpu::TextureDescriptor {
        label: Some("sky-cua agent cursor texture"),
        size: ::wgpu::Extent3d {
            width: cursor.width,
            height: cursor.height,
            depth_or_array_layers: 1,
        },
        mip_level_count,
        sample_count: 1,
        dimension: ::wgpu::TextureDimension::D2,
        format: ::wgpu::TextureFormat::Rgba8Unorm,
        usage: ::wgpu::TextureUsages::TEXTURE_BINDING | ::wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let write_level = |level: u32, w: u32, h: u32, rgba: &[u8]| {
        queue.write_texture(
            ::wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: ::wgpu::Origin3d::ZERO,
                aspect: ::wgpu::TextureAspect::All,
            },
            rgba,
            ::wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            ::wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    };
    write_level(0, cursor.width, cursor.height, cursor.rgba.as_slice());
    for (index, mip) in cursor.mips.iter().enumerate() {
        write_level(index as u32 + 1, mip.width, mip.height, mip.rgba.as_slice());
    }
    let texture_view = texture.create_view(&::wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&::wgpu::SamplerDescriptor {
        label: Some("sky-cua cursor sampler"),
        address_mode_u: ::wgpu::AddressMode::ClampToEdge,
        address_mode_v: ::wgpu::AddressMode::ClampToEdge,
        address_mode_w: ::wgpu::AddressMode::ClampToEdge,
        mag_filter: ::wgpu::FilterMode::Linear,
        min_filter: ::wgpu::FilterMode::Linear,
        mipmap_filter: ::wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&::wgpu::BindGroupDescriptor {
        label: Some("sky-cua overlay effect bind group"),
        layout: bind_group_layout,
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
                resource: ::wgpu::BindingResource::TextureView(&texture_view),
            },
            ::wgpu::BindGroupEntry {
                binding: 3,
                resource: ::wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}
