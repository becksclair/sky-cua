use std::{
    ptr::NonNull,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPoint,
    AgentCursorPointerTrackingBackendKind, AgentCursorRendererBackendKind, AgentCursorState,
    CoordinateSpace, DiagnosticEntry,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_shm,
    output::{OutputHandler, OutputState},
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_region, wl_registry, wl_shm, wl_surface},
};

use crate::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    cursor_asset, diagnostic,
    pointer_tracking::{PointerTracker, PointerTrackingBounds},
    system_cursor::{SystemCursorAdapter, SystemPointerPosition},
};

const INITIAL_ROUNDTRIPS: usize = 4;
const DEBUG_FILL_ENV: &str = "SKY_CUA_LAYER_SHELL_DEBUG_FILL";
const LAYER_ENV: &str = "SKY_CUA_LAYER_SHELL_LAYER";
const RENDERER_ENV: &str = "SKY_CUA_LAYER_SHELL_RENDERER";
const BUFFER_SLOTS_PER_LAYER: usize = 2;

#[derive(Debug)]
pub struct LayerShellOverlayBackend {
    event_queue: wayland_client::EventQueue<LayerShellApp>,
    app: LayerShellApp,
    system_cursor: SystemCursorAdapter,
    pointer_tracker: PointerTracker,
    conn: Connection,
}

impl LayerShellOverlayBackend {
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let (globals, event_queue) =
            registry_queue_init(&conn).context("failed to initialize Wayland registry")?;
        let qh = event_queue.handle();
        let compositor =
            CompositorState::bind(&globals, &qh).context("wl_compositor is unavailable")?;
        let layer_shell =
            LayerShell::bind(&globals, &qh).context("zwlr_layer_shell_v1 is unavailable")?;
        let shm = Shm::bind(&globals, &qh).context("wl_shm is unavailable")?;
        let output_state = OutputState::new(&globals, &qh);
        let cursor = CursorImage::load()?;
        let outputs: Vec<Option<wl_output::WlOutput>> = {
            let advertised_outputs: Vec<_> = output_state.outputs().collect();
            if advertised_outputs.is_empty() {
                vec![None]
            } else {
                advertised_outputs.into_iter().map(Some).collect()
            }
        };
        let layers = outputs
            .into_iter()
            .map(|output| {
                let layer = create_cursor_layer(&compositor, &layer_shell, &qh, output.as_ref());
                LayerSurfaceEntry {
                    output,
                    layer,
                    configured: false,
                    closed: false,
                    width: 1,
                    height: 1,
                    buffer: None,
                }
            })
            .collect::<Vec<_>>();

        let pool_size = layer_buffer_pool_size(cursor.width, cursor.height, layers.len())
            .context("failed to size initial layer-shell shared-memory pool")?;
        let pool = SlotPool::new(pool_size, &shm)
            .context("failed to create Wayland shared-memory pool")?;
        let app = LayerShellApp {
            renderer: LayerShellRenderer::Shm {
                reason: Some("layer-shell renderer has not been selected yet".to_string()),
            },
            shm,
            output_state,
            pool,
            layers,
            cursor,
            state: None,
        };
        let mut backend = Self {
            event_queue,
            app,
            system_cursor: SystemCursorAdapter::for_wayland_session(),
            pointer_tracker: PointerTracker::none("pointer tracker has not been selected yet"),
            conn,
        };
        backend.prime()?;
        backend.app.select_renderer(&backend.conn)?;
        backend.pointer_tracker =
            PointerTracker::for_wayland_session(backend.app.pointer_tracking_bounds());
        backend.render_current()?;
        Ok(backend)
    }

    pub fn handle_message(&mut self, message: OverlayHostMessage) -> OverlayHostReply {
        if message.version != OVERLAY_HOST_PROTOCOL_VERSION {
            return self.reply(
                false,
                vec![diagnostic(
                    "OverlayProtocolVersionMismatch",
                    "Overlay host protocol version mismatch.",
                    Some(format!(
                        "expected={} got={}",
                        OVERLAY_HOST_PROTOCOL_VERSION, message.version
                    )),
                )],
            );
        }

        match message.kind {
            OverlayHostMessageKind::Hello
            | OverlayHostMessageKind::Ping
            | OverlayHostMessageKind::Capabilities => {
                let _ = self.follow_tracked_pointer();
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::Shutdown => {
                let _ = self.hide_visible_cursor();
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::SetCursor => {
                self.app.state = message.state;
                self.render_reply()
            }
            OverlayHostMessageKind::Hide => {
                if let Some(state) = self.app.state.as_mut() {
                    state.visible = false;
                }
                let mut reply = self.render_reply();
                if let Some(reason) = message.reason.filter(|value| !value.trim().is_empty()) {
                    reply.diagnostics.push(diagnostic(
                        "OverlayCursorHidden",
                        "Overlay host hid the cursor.",
                        Some(reason),
                    ));
                }
                reply
            }
            OverlayHostMessageKind::Show => {
                self.app.state = message.state;
                if let Some(state) = self.app.state.as_mut() {
                    state.visible = true;
                }
                self.render_reply()
            }
        }
    }

    pub fn tick(&mut self) {
        let _ = self.follow_tracked_pointer();
    }

    fn prime(&mut self) -> Result<()> {
        for _ in 0..INITIAL_ROUNDTRIPS {
            self.event_queue
                .roundtrip(&mut self.app)
                .context("Wayland roundtrip failed while priming layer-shell overlay")?;
            if self.app.has_configured_layer() || !self.app.has_open_layer() {
                break;
            }
        }
        if !self.app.has_open_layer() {
            bail!("layer-shell compositor closed all overlay surfaces during startup");
        }
        if !self.app.has_configured_layer() {
            bail!("layer-shell overlay surfaces were not configured by the compositor");
        }
        Ok(())
    }

    fn render_reply(&mut self) -> OverlayHostReply {
        match self.render_current() {
            Ok(()) => self.reply(true, Vec::new()),
            Err(error) => self.reply(
                false,
                vec![diagnostic(
                    "OverlayRenderFailed",
                    "Layer-shell overlay failed to render the agent cursor.",
                    Some(error.to_string()),
                )],
            ),
        }
    }

    fn render_current(&mut self) -> Result<()> {
        if !self.app.has_open_layer() {
            bail!("layer-shell overlay surfaces are closed");
        }
        self.system_cursor
            .set_hidden(self.app.state.as_ref().is_some_and(|state| state.visible))
            .context("failed to update layer-shell system cursor adapter")?;
        let qh = self.event_queue.handle();
        self.app.draw(&qh)?;
        self.event_queue
            .roundtrip(&mut self.app)
            .context("Wayland roundtrip failed after drawing layer-shell overlay")?;
        Ok(())
    }

    fn hide_visible_cursor(&mut self) -> Result<()> {
        if let Some(state) = self.app.state.as_mut() {
            state.visible = false;
        }
        self.render_current()
    }

    fn follow_tracked_pointer(&mut self) -> Result<()> {
        if !self.app.state.as_ref().is_some_and(|state| state.visible) {
            return Ok(());
        }
        let Some(position) = self.pointer_tracker.latest_position() else {
            return Ok(());
        };
        let Some(state) = self.app.state.as_mut() else {
            return Ok(());
        };
        if !state_needs_system_pointer_update(state, position) {
            return Ok(());
        }
        apply_system_pointer_position(state, position);
        self.render_current()
    }

    fn reply(&self, ok: bool, diagnostics: Vec<DiagnosticEntry>) -> OverlayHostReply {
        OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok,
            capabilities: Some(self.capabilities()),
            state: self.app.state.clone(),
            diagnostics,
        }
    }

    fn capabilities(&self) -> AgentCursorCapabilities {
        layer_shell_capabilities(
            self.app.open_layer_count(),
            self.app.has_open_layer(),
            self.app.renderer_kind(),
            self.app.renderer_reason(),
            self.pointer_tracker.backend(),
            self.pointer_tracker.exact(),
            self.pointer_tracker.reason(),
            &self.system_cursor,
        )
    }
}

fn layer_shell_capabilities(
    open_layer_count: usize,
    has_open_layer: bool,
    renderer_backend: AgentCursorRendererBackendKind,
    renderer_reason: Option<&str>,
    pointer_tracking_backend: AgentCursorPointerTrackingBackendKind,
    pointer_tracking_exact: bool,
    pointer_tracking_reason: Option<&str>,
    system_cursor: &SystemCursorAdapter,
) -> AgentCursorCapabilities {
    let mut reason = format!(
        "zwlr_layer_shell_v1 visible overlay active on {} output surface(s)",
        open_layer_count
    );
    if let Some(system_cursor_reason) = system_cursor.reason() {
        if system_cursor.supported() {
            reason.push_str("; system cursor: ");
        } else {
            reason.push_str("; system cursor hide unsupported: ");
        }
        reason.push_str(system_cursor_reason);
    }
    if let Some(renderer_reason) = renderer_reason {
        reason.push_str("; renderer: ");
        reason.push_str(renderer_reason);
    }
    if let Some(pointer_tracking_reason) = pointer_tracking_reason {
        reason.push_str("; pointer tracking: ");
        reason.push_str(pointer_tracking_reason);
    }
    AgentCursorCapabilities {
        backend: AgentCursorBackendKind::WaylandLayerShell,
        renderer_backend,
        visible_overlay: has_open_layer,
        screenshot_synthetic_cursor: false,
        click_through: true,
        capture_exclusion: false,
        pointer_tracking_backend,
        pointer_tracking_exact,
        system_cursor_hide_supported: system_cursor.supported(),
        system_cursor_hidden: system_cursor.hidden(),
        system_cursor_backend: system_cursor.backend(),
        needs_user_install: false,
        reason: Some(reason),
    }
}

#[derive(Debug)]
struct LayerShellApp {
    renderer: LayerShellRenderer,
    shm: Shm,
    output_state: OutputState,
    pool: SlotPool,
    layers: Vec<LayerSurfaceEntry>,
    cursor: CursorImage,
    state: Option<AgentCursorState>,
}

#[derive(Debug)]
struct LayerSurfaceEntry {
    output: Option<wl_output::WlOutput>,
    layer: LayerSurface,
    configured: bool,
    closed: bool,
    width: u32,
    height: u32,
    buffer: Option<Buffer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedLayerShellRenderer {
    Auto,
    Wgpu,
    Shm,
}

fn requested_renderer() -> RequestedLayerShellRenderer {
    match std::env::var(RENDERER_ENV)
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "shm" | "wayland_shm" | "wayland-shm" => RequestedLayerShellRenderer::Shm,
        "wgpu" | "gpu" => RequestedLayerShellRenderer::Wgpu,
        "auto" | "" => RequestedLayerShellRenderer::Auto,
        _ => RequestedLayerShellRenderer::Auto,
    }
}

#[derive(Debug)]
enum LayerShellRenderer {
    Shm { reason: Option<String> },
    Wgpu(WgpuLayerRenderer),
}

impl LayerShellRenderer {
    fn kind(&self) -> AgentCursorRendererBackendKind {
        match self {
            Self::Shm { .. } => AgentCursorRendererBackendKind::WaylandShm,
            Self::Wgpu(_) => AgentCursorRendererBackendKind::Wgpu,
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Shm { reason } => reason.as_deref(),
            Self::Wgpu(renderer) => Some(renderer.reason()),
        }
    }
}

#[derive(Debug)]
struct WgpuLayerRenderer {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: Option<wgpu::RenderPipeline>,
    pipeline_format: Option<wgpu::TextureFormat>,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    surfaces: Vec<Option<WgpuSurfaceEntry>>,
    reason: String,
}

#[derive(Debug)]
struct WgpuSurfaceEntry {
    surface: wgpu::Surface<'static>,
    config: Option<wgpu::SurfaceConfiguration>,
}

const CURSOR_SHADER: &str = r#"
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

impl WgpuLayerRenderer {
    fn new(conn: &Connection, layers: &[LayerSurfaceEntry], cursor: &CursorImage) -> Result<Self> {
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let instance = wgpu::Instance::new(instance_descriptor);
        let mut surfaces = create_wgpu_surfaces(&instance, conn, layers)?;
        let first_surface = surfaces
            .iter()
            .filter_map(|entry| entry.as_ref())
            .map(|entry| &entry.surface)
            .next()
            .context("no layer-shell Wayland surfaces available for wgpu")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(first_surface),
        }))
        .context("failed to find a GPU adapter compatible with layer-shell surfaces")?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("sky-cua layer-shell overlay"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("failed to request wgpu device for layer-shell overlay")?;

        let (bind_group, bind_group_layout) = create_cursor_texture(&device, &queue, cursor);
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky-cua cursor quad vertices"),
            size: (6 * 4 * std::mem::size_of::<f32>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let adapter_info = adapter.get_info();
        let backend = format!("{:?}", adapter_info.backend).to_ascii_lowercase();
        let reason = format!(
            "wgpu renderer active on {} via {}",
            adapter_info.name, backend
        );

        // Keep empty slots aligned with layer indices even when a surface failed
        // construction. This should not happen for normal Wayland surfaces, but
        // preserving indices lets rendering continue for other outputs.
        surfaces.resize_with(layers.len(), || None);

        Ok(Self {
            adapter,
            device,
            queue,
            pipeline: None,
            pipeline_format: None,
            bind_group_layout,
            bind_group,
            vertex_buffer,
            surfaces,
            reason,
        })
    }

    fn reason(&self) -> &str {
        self.reason.as_str()
    }

    fn draw(
        &mut self,
        qh: &QueueHandle<LayerShellApp>,
        layers: &mut [LayerSurfaceEntry],
        visible_target: &Option<LayerCursorTarget>,
    ) -> Result<()> {
        for (index, entry) in layers.iter_mut().enumerate() {
            if entry.closed || !entry.configured {
                continue;
            }
            let visible_point = visible_target
                .as_ref()
                .filter(|target| target.layer_index == index)
                .map(|target| (target.x, target.y));
            let width = entry.width.max(1);
            let height = entry.height.max(1);
            entry.layer.set_size(width, height);
            entry.layer.set_margin(0, 0, 0, 0);
            entry
                .layer
                .wl_surface()
                .damage_buffer(0, 0, width as i32, height as i32);
            entry
                .layer
                .wl_surface()
                .frame(qh, entry.layer.wl_surface().clone());

            if index >= self.surfaces.len() {
                continue;
            };
            let Some(mut surface_entry) = self.surfaces[index].take() else {
                continue;
            };
            self.configure_surface(&mut surface_entry, width, height);
            let draw_result = self.draw_surface(&mut surface_entry, width, height, visible_point);
            self.surfaces[index] = Some(surface_entry);
            draw_result?;
            entry.layer.commit();
        }
        Ok(())
    }

    fn configure_surface(&self, surface_entry: &mut WgpuSurfaceEntry, width: u32, height: u32) {
        if surface_entry
            .config
            .as_ref()
            .is_some_and(|config| config.width == width && config.height == height)
        {
            return;
        }
        let capabilities = surface_entry.surface.get_capabilities(&self.adapter);
        let format = choose_surface_format(&capabilities.formats);
        let present_mode = choose_present_mode(&capabilities.present_modes);
        let alpha_mode = choose_alpha_mode(&capabilities.alpha_modes);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![format],
        };
        surface_entry.surface.configure(&self.device, &config);
        surface_entry.config = Some(config);
    }

    fn draw_surface(
        &mut self,
        surface_entry: &mut WgpuSurfaceEntry,
        width: u32,
        height: u32,
        visible_point: Option<(f64, f64)>,
    ) -> Result<()> {
        let frame = match surface_entry.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                if let Some(config) = surface_entry.config.as_ref() {
                    surface_entry.surface.configure(&self.device, config);
                }
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(anyhow::anyhow!(
                    "wgpu validation error while acquiring layer-shell frame"
                ));
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sky-cua layer-shell cursor encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky-cua layer-shell cursor pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some((x, y)) = visible_point {
                let Some(config) = surface_entry.config.as_ref() else {
                    return Ok(());
                };
                self.ensure_pipeline(config.format);
                let vertices = cursor_quad_vertices(x, y, width, height);
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
        Ok(())
    }

    fn ensure_pipeline(&mut self, format: wgpu::TextureFormat) {
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

fn create_wgpu_surfaces(
    instance: &wgpu::Instance,
    conn: &Connection,
    layers: &[LayerSurfaceEntry],
) -> Result<Vec<Option<WgpuSurfaceEntry>>> {
    let display = NonNull::new(conn.backend().display_ptr() as *mut _)
        .context("Wayland display pointer was null")?;
    let raw_display_handle =
        wgpu::rwh::RawDisplayHandle::Wayland(wgpu::rwh::WaylandDisplayHandle::new(display));
    layers
        .iter()
        .map(|entry| {
            let surface_ptr = NonNull::new(entry.layer.wl_surface().id().as_ptr() as *mut _)
                .context("Wayland surface pointer was null")?;
            let raw_window_handle = wgpu::rwh::RawWindowHandle::Wayland(
                wgpu::rwh::WaylandWindowHandle::new(surface_ptr),
            );
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_display_handle),
                    raw_window_handle,
                })
            }
            .context("failed to create wgpu surface for layer-shell wl_surface")?;
            Ok(Some(WgpuSurfaceEntry {
                surface,
                config: None,
            }))
        })
        .collect()
}

fn create_cursor_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cursor: &CursorImage,
) -> (wgpu::BindGroup, wgpu::BindGroupLayout) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky-cua agent cursor texture"),
        size: wgpu::Extent3d {
            width: cursor.width,
            height: cursor.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        cursor.rgba.as_slice(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(cursor.width * 4),
            rows_per_image: Some(cursor.height),
        },
        wgpu::Extent3d {
            width: cursor.width,
            height: cursor.height,
            depth_or_array_layers: 1,
        },
    );
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("sky-cua cursor sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sky-cua cursor bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sky-cua cursor bind group"),
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (bind_group, bind_group_layout)
}

fn create_cursor_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky-cua cursor shader"),
        source: wgpu::ShaderSource::Wgsl(CURSOR_SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky-cua cursor pipeline layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky-cua cursor render pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: (4 * std::mem::size_of::<f32>()) as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: (2 * std::mem::size_of::<f32>()) as wgpu::BufferAddress,
                        shader_location: 1,
                    },
                ],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn choose_surface_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    formats
        .iter()
        .copied()
        .find(|format| *format == wgpu::TextureFormat::Bgra8UnormSrgb)
        .or_else(|| {
            formats
                .iter()
                .copied()
                .find(|format| *format == wgpu::TextureFormat::Rgba8UnormSrgb)
        })
        .or_else(|| formats.iter().copied().find(wgpu::TextureFormat::is_srgb))
        .or_else(|| formats.first().copied())
        .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb)
}

fn choose_present_mode(present_modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if present_modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    }
}

fn choose_alpha_mode(alpha_modes: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    if alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else if alpha_modes.contains(&wgpu::CompositeAlphaMode::Auto) {
        wgpu::CompositeAlphaMode::Auto
    } else {
        alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto)
    }
}

fn cursor_quad_vertices(x: f64, y: f64, surface_width: u32, surface_height: u32) -> [f32; 24] {
    let left = x - f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X);
    let top = y - f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y);
    let right = left + f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH);
    let bottom = top + f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT);
    let left = ndc_x(left, surface_width);
    let right = ndc_x(right, surface_width);
    let top = ndc_y(top, surface_height);
    let bottom = ndc_y(bottom, surface_height);
    [
        left, top, 0.0, 0.0, right, top, 1.0, 0.0, right, bottom, 1.0, 1.0, left, top, 0.0, 0.0,
        right, bottom, 1.0, 1.0, left, bottom, 0.0, 1.0,
    ]
}

fn ndc_x(x: f64, width: u32) -> f32 {
    ((x / f64::from(width.max(1))) * 2.0 - 1.0) as f32
}

fn ndc_y(y: f64, height: u32) -> f32 {
    (1.0 - (y / f64::from(height.max(1))) * 2.0) as f32
}

fn f32_slice_as_bytes(values: &[f32]) -> &[u8] {
    let byte_len = std::mem::size_of_val(values);
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), byte_len) }
}

impl LayerShellApp {
    fn select_renderer(&mut self, conn: &Connection) -> Result<()> {
        match requested_renderer() {
            RequestedLayerShellRenderer::Shm => {
                self.renderer = LayerShellRenderer::Shm {
                    reason: Some(format!("{RENDERER_ENV}=shm")),
                };
                Ok(())
            }
            RequestedLayerShellRenderer::Wgpu => {
                self.renderer = LayerShellRenderer::Wgpu(
                    WgpuLayerRenderer::new(conn, &self.layers, &self.cursor)
                        .context("explicit wgpu layer-shell renderer failed to initialize")?,
                );
                Ok(())
            }
            RequestedLayerShellRenderer::Auto => {
                if debug_fill_enabled() {
                    self.renderer = LayerShellRenderer::Shm {
                        reason: Some(format!(
                            "{DEBUG_FILL_ENV} is set, using shm renderer for debug fill support"
                        )),
                    };
                    return Ok(());
                }
                match WgpuLayerRenderer::new(conn, &self.layers, &self.cursor) {
                    Ok(renderer) => {
                        self.renderer = LayerShellRenderer::Wgpu(renderer);
                    }
                    Err(error) => {
                        self.renderer = LayerShellRenderer::Shm {
                            reason: Some(format!("wgpu unavailable, using shm fallback: {error}")),
                        };
                    }
                }
                Ok(())
            }
        }
    }

    fn renderer_kind(&self) -> AgentCursorRendererBackendKind {
        self.renderer.kind()
    }

    fn renderer_reason(&self) -> Option<&str> {
        self.renderer.reason()
    }

    fn pointer_tracking_bounds(&self) -> Option<PointerTrackingBounds> {
        let mut left = i32::MAX;
        let mut top = i32::MAX;
        let mut right = i32::MIN;
        let mut bottom = i32::MIN;
        for entry in self
            .layers
            .iter()
            .filter(|entry| !entry.closed && entry.configured)
        {
            let Some(output) = entry.output.as_ref() else {
                left = left.min(0);
                top = top.min(0);
                right = right.max(i32::try_from(entry.width).unwrap_or(i32::MAX));
                bottom = bottom.max(i32::try_from(entry.height).unwrap_or(i32::MAX));
                continue;
            };
            let Some(info) = self.output_state.info(output) else {
                continue;
            };
            let position = info.logical_position.unwrap_or(info.location);
            let Some(size) = info.logical_size else {
                continue;
            };
            left = left.min(position.0);
            top = top.min(position.1);
            right = right.max(position.0.saturating_add(size.0));
            bottom = bottom.max(position.1.saturating_add(size.1));
        }
        if right <= left || bottom <= top {
            return None;
        }
        Some(PointerTrackingBounds {
            x: left,
            y: top,
            width: u32::try_from(right - left).ok()?,
            height: u32::try_from(bottom - top).ok()?,
            scale_milli: 1000,
        })
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let visible_target = self
            .state
            .as_ref()
            .filter(|state| state.visible)
            .and_then(|state| self.cursor_target(state));
        if let LayerShellRenderer::Wgpu(renderer) = &mut self.renderer {
            return renderer.draw(qh, &mut self.layers, &visible_target);
        }
        self.draw_shm(qh, visible_target)
    }

    fn draw_shm(
        &mut self,
        qh: &QueueHandle<Self>,
        visible_target: Option<LayerCursorTarget>,
    ) -> Result<()> {
        let layer_count = self.layers.len();

        for (index, entry) in self.layers.iter_mut().enumerate() {
            if entry.closed || !entry.configured {
                continue;
            }
            let visible_point = visible_target
                .as_ref()
                .filter(|target| target.layer_index == index)
                .map(|target| (target.x, target.y));
            let width = entry.width.max(1);
            let height = entry.height.max(1);
            entry.layer.set_size(width, height);
            entry.layer.set_margin(0, 0, 0, 0);

            let stride = width as i32 * 4;
            ensure_layer_pool_capacity(&mut self.pool, width, height, layer_count)
                .context("failed to resize layer-shell cursor shared-memory pool")?;
            let (buffer, canvas) = self
                .pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                )
                .context("failed to create layer-shell cursor buffer")?;
            canvas.fill(0);
            if debug_fill_enabled() {
                draw_debug_fill(canvas, width, height, visible_point);
            }
            if let Some((x, y)) = visible_point {
                let left = x.round() as i32 - cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X;
                let top = y.round() as i32 - cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y;
                draw_cursor_asset(canvas, width, height, &self.cursor, left, top);
            }

            entry
                .layer
                .wl_surface()
                .damage_buffer(0, 0, width as i32, height as i32);
            entry
                .layer
                .wl_surface()
                .frame(qh, entry.layer.wl_surface().clone());
            buffer
                .attach_to(entry.layer.wl_surface())
                .context("failed to attach layer-shell cursor buffer")?;
            entry.layer.commit();
            entry.buffer = Some(buffer);
        }
        Ok(())
    }

    fn cursor_target(&self, state: &AgentCursorState) -> Option<LayerCursorTarget> {
        let point = state.native_point.as_ref().or(state.model_point.as_ref())?;
        let (x, y) = cursor_point(state)?;
        if point.coordinate_space == CoordinateSpace::DesktopLogical
            && let Some(target) = self.desktop_logical_target(x, y)
        {
            return Some(target);
        }
        self.first_open_layer_index()
            .map(|layer_index| LayerCursorTarget { layer_index, x, y })
    }

    fn desktop_logical_target(&self, x: f64, y: f64) -> Option<LayerCursorTarget> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.closed)
            .filter(|(_, entry)| entry.configured)
            .filter_map(|(layer_index, entry)| {
                let output = entry.output.as_ref()?;
                let info = self.output_state.info(output)?;
                let position = info.logical_position.unwrap_or(info.location);
                let size = info.logical_size?;
                output_local_point((x, y), position, size).map(|(x, y)| LayerCursorTarget {
                    layer_index,
                    x,
                    y,
                })
            })
            .next()
    }

    fn first_open_layer_index(&self) -> Option<usize> {
        self.layers
            .iter()
            .enumerate()
            .find_map(|(index, entry)| (!entry.closed && entry.configured).then_some(index))
    }

    fn open_layer_count(&self) -> usize {
        self.layers.iter().filter(|entry| !entry.closed).count()
    }

    fn has_open_layer(&self) -> bool {
        self.open_layer_count() > 0
    }

    fn has_configured_layer(&self) -> bool {
        self.layers
            .iter()
            .any(|entry| !entry.closed && entry.configured)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LayerCursorTarget {
    layer_index: usize,
    x: f64,
    y: f64,
}

fn create_cursor_layer(
    compositor: &CompositorState,
    layer_shell: &LayerShell,
    qh: &QueueHandle<LayerShellApp>,
    output: Option<&wl_output::WlOutput>,
) -> LayerSurface {
    let surface = compositor.create_surface(qh);
    let layer = layer_shell.create_layer_surface(
        qh,
        surface,
        requested_layer(),
        Some("sky-cua-agent-cursor"),
        output,
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // The production cursor overlay must cover compositor-global logical
    // coordinates, not the panel-constrained work area. KWin otherwise offsets
    // click-through layer surfaces away from exclusive panel edges.
    layer.set_exclusive_zone(-1);
    layer.set_size(0, 0);
    set_empty_input_region(compositor, &layer, qh);
    layer.commit();
    layer
}

fn requested_layer() -> Layer {
    match std::env::var(LAYER_ENV)
        .unwrap_or_else(|_| "overlay".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "background" => Layer::Background,
        "bottom" => Layer::Bottom,
        "top" => Layer::Top,
        "overlay" => Layer::Overlay,
        _ => Layer::Overlay,
    }
}

fn set_empty_input_region(
    compositor: &CompositorState,
    layer: &LayerSurface,
    qh: &QueueHandle<LayerShellApp>,
) {
    // An empty Wayland input region makes the overlay click-through.
    let region = compositor.wl_compositor().create_region(qh, ());
    layer.set_input_region(Some(&region));
    region.destroy();
}

fn cursor_point(state: &AgentCursorState) -> Option<(f64, f64)> {
    state
        .native_point
        .as_ref()
        .or(state.model_point.as_ref())
        .and_then(point_to_overlay_coordinates)
}

fn point_to_overlay_coordinates(point: &AgentCursorPoint) -> Option<(f64, f64)> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    match point.coordinate_space {
        CoordinateSpace::DesktopLogical
        | CoordinateSpace::StreamLogical
        | CoordinateSpace::StreamPixels => Some((point.x, point.y)),
    }
}

fn state_needs_system_pointer_update(
    state: &AgentCursorState,
    position: SystemPointerPosition,
) -> bool {
    let Some(point) = state.native_point.as_ref() else {
        return true;
    };
    if point.coordinate_space != CoordinateSpace::DesktopLogical {
        return true;
    }
    (point.x - position.x).abs() >= 0.5 || (point.y - position.y).abs() >= 0.5
}

fn apply_system_pointer_position(state: &mut AgentCursorState, position: SystemPointerPosition) {
    state.native_point = Some(AgentCursorPoint {
        x: position.x,
        y: position.y,
        coordinate_space: CoordinateSpace::DesktopLogical,
        mapping_id: None,
    });
    state.sequence = state.sequence.saturating_add(1);
    state.updated_at_ms = current_epoch_ms();
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn output_local_point(
    point: (f64, f64),
    position: (i32, i32),
    size: (i32, i32),
) -> Option<(f64, f64)> {
    if size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    let x = point.0 - f64::from(position.0);
    let y = point.1 - f64::from(position.1);
    (x >= 0.0 && y >= 0.0 && x < f64::from(size.0) && y < f64::from(size.1)).then_some((x, y))
}

pub(crate) fn ensure_layer_pool_capacity(
    pool: &mut SlotPool,
    width: u32,
    height: u32,
    layer_count: usize,
) -> Result<()> {
    let required = layer_buffer_pool_size(width, height, layer_count)?;
    if pool.len() < required {
        pool.resize(required)?;
    }
    Ok(())
}

pub(crate) fn layer_buffer_pool_size(width: u32, height: u32, layer_count: usize) -> Result<usize> {
    let layer_count = layer_count.max(1);
    let bytes_per_buffer = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("layer-shell cursor buffer dimensions overflowed usize")?;
    bytes_per_buffer
        .checked_mul(layer_count)
        .and_then(|bytes| bytes.checked_mul(BUFFER_SLOTS_PER_LAYER))
        .map(|bytes| bytes.max(4096))
        .context("layer-shell cursor buffer pool size overflowed usize")
}

pub(crate) fn draw_cursor_asset(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    cursor: &CursorImage,
    left: i32,
    top: i32,
) {
    for source_y in 0..cursor.height {
        let dest_y = top + i32::try_from(source_y).expect("cursor source y fits i32");
        if dest_y < 0 || dest_y >= i32::try_from(height).expect("surface height fits i32") {
            continue;
        }
        for source_x in 0..cursor.width {
            let dest_x = left + i32::try_from(source_x).expect("cursor source x fits i32");
            if dest_x < 0 || dest_x >= i32::try_from(width).expect("surface width fits i32") {
                continue;
            }
            let source_offset = ((source_y * cursor.width + source_x) * 4) as usize;
            let r = cursor.rgba[source_offset];
            let g = cursor.rgba[source_offset + 1];
            let b = cursor.rgba[source_offset + 2];
            let a = cursor.rgba[source_offset + 3];
            if a == 0 {
                continue;
            };
            let dest_x = u32::try_from(dest_x).expect("nonnegative destination x");
            let dest_y = u32::try_from(dest_y).expect("nonnegative destination y");
            let offset = ((dest_y * width + dest_x) * 4) as usize;
            let r = premultiply(r, a);
            let g = premultiply(g, a);
            let b = premultiply(b, a);
            let color = ((u32::from(a)) << 24)
                | ((u32::from(r)) << 16)
                | ((u32::from(g)) << 8)
                | u32::from(b);
            canvas[offset..offset + 4].copy_from_slice(&color.to_le_bytes());
        }
    }
}

fn debug_fill_enabled() -> bool {
    std::env::var(DEBUG_FILL_ENV)
        .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "full" | "rect"))
}

fn draw_debug_fill(canvas: &mut [u8], width: u32, height: u32, visible_point: Option<(f64, f64)>) {
    let fill_full_surface =
        std::env::var(DEBUG_FILL_ENV).is_ok_and(|value| value.trim().eq_ignore_ascii_case("full"));
    let (left, top, right, bottom) = if fill_full_surface {
        (0, 0, width as i32, height as i32)
    } else if let Some((x, y)) = visible_point {
        let left = x.round() as i32 - 48;
        let top = y.round() as i32 - 48;
        (left, top, left + 96, top + 96)
    } else {
        (0, 0, 96, 96)
    };
    let color = ((u32::from(224_u8)) << 24)
        | ((u32::from(224_u8)) << 16)
        | ((u32::from(24_u8)) << 8)
        | u32::from(128_u8);
    let bytes = color.to_le_bytes();
    for y in top.max(0)..bottom.min(height as i32) {
        for x in left.max(0)..right.min(width as i32) {
            let x = u32::try_from(x).expect("nonnegative x");
            let y = u32::try_from(y).expect("nonnegative y");
            let offset = ((y * width + x) * 4) as usize;
            canvas[offset..offset + 4].copy_from_slice(&bytes);
        }
    }
}

fn premultiply(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

#[derive(Debug)]
pub(crate) struct CursorImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

impl CursorImage {
    pub(crate) fn load() -> Result<Self> {
        let image = image::load_from_memory(cursor_asset::AGENT_CURSOR_PNG)
            .context("failed to decode bundled agent cursor image")?
            .to_rgba8();
        let (width, height) = image.dimensions();
        if width != cursor_asset::AGENT_CURSOR_SOURCE_WIDTH
            || height != cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT
        {
            bail!(
                "bundled agent cursor image changed size: expected {}x{} got {}x{}",
                cursor_asset::AGENT_CURSOR_SOURCE_WIDTH,
                cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT,
                width,
                height
            );
        }
        let image = image::imageops::resize(
            &image,
            cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH,
            cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT,
            FilterType::Lanczos3,
        );
        let (width, height) = image.dimensions();
        Ok(Self {
            width,
            height,
            rgba: image.into_raw(),
        })
    }
}

impl CompositorHandler for LayerShellApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for LayerShellApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(entry) = self.layers.iter_mut().find(|entry| &entry.layer == layer) {
            entry.closed = true;
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some(entry) = self.layers.iter_mut().find(|entry| &entry.layer == layer) {
            entry.configured = true;
            entry.width = if configure.new_size.0 == 0 {
                cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH
            } else {
                configure.new_size.0
            };
            entry.height = if configure.new_size.1 == 0 {
                cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT
            } else {
                configure.new_size.1
            };
        }
    }
}

impl ShmHandler for LayerShellApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for LayerShellApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for LayerShellApp {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_region::WlRegion, ()> for LayerShellApp {
    fn event(
        _state: &mut Self,
        _region: &wl_region::WlRegion,
        _event: wl_region::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_compositor!(LayerShellApp);
delegate_layer!(LayerShellApp);
delegate_output!(LayerShellApp);
delegate_shm!(LayerShellApp);

#[cfg(test)]
mod tests {
    use super::{
        BUFFER_SLOTS_PER_LAYER, CursorImage, RequestedLayerShellRenderer,
        apply_system_pointer_position, cursor_point, draw_cursor_asset, layer_buffer_pool_size,
        layer_shell_capabilities, output_local_point, requested_renderer,
        state_needs_system_pointer_update,
    };
    use crate::{
        cursor_asset,
        system_cursor::{SystemCursorAdapter, SystemPointerPosition},
    };
    use sky_cua_platform::model::{
        AgentCursorBackendKind, AgentCursorPoint, AgentCursorPointerTrackingBackendKind,
        AgentCursorRendererBackendKind, AgentCursorState, AgentCursorSystemCursorBackendKind,
        CoordinateSpace,
    };

    #[test]
    fn cursor_point_prefers_native_coordinates_for_visible_overlay() {
        let state = AgentCursorState {
            visible: true,
            sequence: 1,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: None,
            }),
            native_point: Some(AgentCursorPoint {
                x: 100.0,
                y: 200.0,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            }),
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 1,
        };

        assert_eq!(cursor_point(&state), Some((100.0, 200.0)));
    }

    #[test]
    fn system_pointer_update_moves_visible_state_to_desktop_coordinates() {
        let mut state = AgentCursorState {
            visible: true,
            sequence: 7,
            model_point: Some(AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: CoordinateSpace::StreamPixels,
                mapping_id: Some("stream".to_string()),
            }),
            native_point: None,
            snapshot_id: None,
            source_action: None,
            updated_at_ms: 0,
        };
        let position = SystemPointerPosition { x: 300.0, y: 400.0 };

        assert!(state_needs_system_pointer_update(&state, position));
        apply_system_pointer_position(&mut state, position);

        assert_eq!(state.sequence, 8);
        assert_eq!(
            state.native_point,
            Some(AgentCursorPoint {
                x: 300.0,
                y: 400.0,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            })
        );
        assert_eq!(cursor_point(&state), Some((300.0, 400.0)));
        assert!(!state_needs_system_pointer_update(
            &state,
            SystemPointerPosition {
                x: 300.25,
                y: 400.25
            }
        ));
        assert!(state_needs_system_pointer_update(
            &state,
            SystemPointerPosition { x: 301.0, y: 400.0 }
        ));
    }

    #[test]
    fn desktop_output_point_maps_to_output_local_coordinates() {
        assert_eq!(
            output_local_point((2020.0, 90.0), (1920, 0), (1280, 720)),
            Some((100.0, 90.0))
        );
        assert_eq!(
            output_local_point((1919.0, 90.0), (1920, 0), (1280, 720)),
            None
        );
        assert_eq!(
            output_local_point((3200.0, 90.0), (1920, 0), (1280, 720)),
            None
        );
    }

    #[test]
    fn layer_pool_size_tracks_full_surface_buffers() {
        assert_eq!(
            layer_buffer_pool_size(1920, 1080, 2).expect("pool size"),
            1920 * 1080 * 4 * 2 * BUFFER_SLOTS_PER_LAYER
        );
    }

    #[test]
    fn cursor_asset_draw_keeps_background_fully_transparent() {
        let cursor = CursorImage::load().expect("load cursor");
        let mut canvas = vec![0_u8; (cursor.width * cursor.height * 4) as usize];

        draw_cursor_asset(&mut canvas, cursor.width, cursor.height, &cursor, 0, 0);

        // Top-left corner stays transparent; the hotspot lands on the opaque body.
        let corner_alpha = canvas[3];
        let hotspot_offset = ((cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y as u32 * cursor.width
            + cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X as u32)
            * 4) as usize;
        assert_eq!(corner_alpha, 0);
        assert_eq!(canvas[hotspot_offset + 3], 255);
    }

    #[test]
    fn layer_shell_capabilities_report_kwin_system_cursor_split_path() {
        let capabilities = layer_shell_capabilities(
            1,
            true,
            AgentCursorRendererBackendKind::Wgpu,
            Some("wgpu renderer active"),
            AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
            true,
            Some("KWin signal tracker active"),
            &SystemCursorAdapter::test_kwin_effect(true),
        );

        assert_eq!(
            capabilities.backend,
            AgentCursorBackendKind::WaylandLayerShell
        );
        assert_eq!(
            capabilities.renderer_backend,
            AgentCursorRendererBackendKind::Wgpu
        );
        assert!(capabilities.visible_overlay);
        assert!(capabilities.click_through);
        assert_eq!(
            capabilities.pointer_tracking_backend,
            AgentCursorPointerTrackingBackendKind::KwinEffectSignal
        );
        assert!(capabilities.pointer_tracking_exact);
        assert!(capabilities.system_cursor_hide_supported);
        assert!(capabilities.system_cursor_hidden);
        assert_eq!(
            capabilities.system_cursor_backend,
            AgentCursorSystemCursorBackendKind::KwinEffect
        );
    }

    #[test]
    fn layer_shell_renderer_env_selects_wgpu_and_shm() {
        unsafe { std::env::set_var(super::RENDERER_ENV, "wgpu") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Wgpu);
        unsafe { std::env::set_var(super::RENDERER_ENV, "shm") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Shm);
        unsafe { std::env::set_var(super::RENDERER_ENV, "auto") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Auto);
        unsafe { std::env::remove_var(super::RENDERER_ENV) };
    }
}
