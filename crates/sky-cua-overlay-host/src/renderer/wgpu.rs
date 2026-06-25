//! WGPU renderer implementation for the static agent cursor.

use crate::renderer::{
    CursorImage,
    buffers::{cursor_quad_vertices, f32_slice_as_bytes},
    scene::SurfaceDrawRequest,
    shaders::create_cursor_pipeline,
    surface::SurfaceGuard,
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
    pub driver: String,
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
    vertex_buffer: ::wgpu::Buffer,
    info: RendererInfo,
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
            if capabilities.present_modes.is_empty() {
                bail!("wgpu surface {index} has no compatible present modes");
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

        let (bind_group, bind_group_layout) = create_cursor_texture(&device, &queue, cursor);
        let vertex_buffer = device.create_buffer(&::wgpu::BufferDescriptor {
            label: Some("sky-cua cursor quad vertices"),
            size: (6 * std::mem::size_of::<super::buffers::CursorVertex>())
                as ::wgpu::BufferAddress,
            usage: ::wgpu::BufferUsages::VERTEX | ::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

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
            vertex_buffer,
            info,
        })
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
            let config = guard.ensure_configured(&self.device, &self.adapter, width, height);

            let frame = match guard.acquire_frame(&self.device) {
                super::surface::SurfaceAcquisitionResult::Success(frame) => frame,
                super::surface::SurfaceAcquisitionResult::Retry => continue,
                super::surface::SurfaceAcquisitionResult::Validation => {
                    bail!(
                        "wgpu validation error while acquiring overlay frame for surface {index}"
                    );
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
                    label: Some("sky-cua overlay cursor pass"),
                    color_attachments: &[Some(::wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: ::wgpu::Operations {
                            load: ::wgpu::LoadOp::Clear(::wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: ::wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                if let Some(point) = spec.cursor {
                    self.ensure_pipeline(config.format);
                    let vertices = cursor_quad_vertices(point.x, point.y, width, height);
                    self.queue
                        .write_buffer(&self.vertex_buffer, 0, f32_slice_as_bytes(&vertices));
                    pass.set_pipeline(self.pipeline.as_ref().expect("pipeline initialized"));
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.draw(0..6, 0..1);
                }
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
        self.pipeline = Some(create_cursor_pipeline(
            &self.device,
            &self.bind_group_layout,
            format,
        ));
        self.pipeline_format = Some(format);
    }
}

fn create_cursor_texture(
    device: &::wgpu::Device,
    queue: &::wgpu::Queue,
    cursor: &CursorImage,
) -> (::wgpu::BindGroup, ::wgpu::BindGroupLayout) {
    let texture = device.create_texture(&::wgpu::TextureDescriptor {
        label: Some("sky-cua agent cursor texture"),
        size: ::wgpu::Extent3d {
            width: cursor.width,
            height: cursor.height,
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
        cursor.rgba.as_slice(),
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
    let texture_view = texture.create_view(&::wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&::wgpu::SamplerDescriptor {
        label: Some("sky-cua cursor sampler"),
        address_mode_u: ::wgpu::AddressMode::ClampToEdge,
        address_mode_v: ::wgpu::AddressMode::ClampToEdge,
        address_mode_w: ::wgpu::AddressMode::ClampToEdge,
        mag_filter: ::wgpu::FilterMode::Linear,
        min_filter: ::wgpu::FilterMode::Linear,
        mipmap_filter: ::wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let bind_group_layout = device.create_bind_group_layout(&::wgpu::BindGroupLayoutDescriptor {
        label: Some("sky-cua cursor bind group layout"),
        entries: &[
            ::wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: ::wgpu::ShaderStages::FRAGMENT,
                ty: ::wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: ::wgpu::TextureViewDimension::D2,
                    sample_type: ::wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            ::wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: ::wgpu::ShaderStages::FRAGMENT,
                ty: ::wgpu::BindingType::Sampler(::wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&::wgpu::BindGroupDescriptor {
        label: Some("sky-cua cursor bind group"),
        layout: &bind_group_layout,
        entries: &[
            ::wgpu::BindGroupEntry {
                binding: 0,
                resource: ::wgpu::BindingResource::TextureView(&texture_view),
            },
            ::wgpu::BindGroupEntry {
                binding: 1,
                resource: ::wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (bind_group, bind_group_layout)
}
