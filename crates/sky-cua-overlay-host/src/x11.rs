use std::{
    collections::BTreeMap,
    fmt,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorPoint,
    AgentCursorPointerTrackingBackendKind, AgentCursorRendererBackendKind, AgentCursorState,
    CoordinateSpace, DiagnosticEntry,
};
use x11rb::{
    COPY_DEPTH_FROM_PARENT, NONE,
    connection::Connection as X11Connection,
    protocol::{
        shape::{self, SK, SO},
        xproto::{
            self, ChangeGCAux, ConfigureWindowAux, ConnectionExt as XprotoConnectionExt,
            CreateGCAux, CreateWindowAux, EventMask, Gcontext, Rectangle, StackMode, Visualid,
            Visualtype, Window, WindowClass,
        },
    },
    rust_connection::RustConnection,
};

use crate::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    cursor_asset, diagnostic,
    system_cursor::{SystemCursorAdapter, SystemPointerPosition},
};

const ALPHA_VISIBLE_THRESHOLD: u8 = 8;

pub struct X11OverlayBackend {
    conn: Rc<RustConnection>,
    screen_num: usize,
    root: Window,
    window: Window,
    gc: Gcontext,
    cursor: CursorImage,
    visual: VisualFormat,
    system_cursor: SystemCursorAdapter,
    state: Option<AgentCursorState>,
}

impl fmt::Debug for X11OverlayBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("X11OverlayBackend")
            .field("screen_num", &self.screen_num)
            .field("root", &self.root)
            .field("window", &self.window)
            .field("gc", &self.gc)
            .field("cursor", &self.cursor)
            .field("visual", &self.visual)
            .field("system_cursor", &self.system_cursor)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl X11OverlayBackend {
    pub fn connect() -> Result<Self> {
        let (raw_conn, screen_num) =
            x11rb::connect(None).context("failed to connect to X11 display")?;
        let conn = Rc::new(raw_conn);
        let _shape_version = shape::query_version(conn.as_ref())
            .context("failed to query X Shape extension")?
            .reply()
            .context("X Shape extension query failed")?;
        let screen = conn
            .setup()
            .roots
            .get(screen_num)
            .context("X11 default screen is unavailable")?;
        let root = screen.root;
        let root_visual = screen.root_visual;
        let visual = VisualFormat::for_root_visual(screen.allowed_depths.as_slice(), root_visual)
            .with_context(|| format!("X11 root visual {root_visual} is not TrueColor"))?;
        let cursor = CursorImage::load(&visual)?;
        let window = conn
            .generate_id()
            .context("failed to allocate X11 window id")?;
        let gc = conn.generate_id().context("failed to allocate X11 GC id")?;

        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            cursor.width,
            cursor.height,
            0,
            WindowClass::INPUT_OUTPUT,
            root_visual,
            &CreateWindowAux::new()
                .background_pixmap(NONE)
                .border_pixel(0)
                .override_redirect(1)
                .save_under(1)
                .event_mask(EventMask::EXPOSURE),
        )
        .context("failed to create X11 cursor overlay window")?;
        conn.create_gc(gc, window, &CreateGCAux::new().foreground(0))
            .context("failed to create X11 cursor overlay graphics context")?;
        set_shape(conn.as_ref(), window, cursor.shape_rectangles.as_slice())
            .context("failed to shape X11 cursor overlay window")?;
        conn.flush()
            .context("failed to flush X11 cursor overlay startup requests")?;

        Ok(Self {
            conn: conn.clone(),
            screen_num,
            root,
            window,
            gc,
            cursor,
            visual,
            system_cursor: SystemCursorAdapter::x11(conn.clone(), root),
            state: None,
        })
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
            | OverlayHostMessageKind::Capabilities => self.reply(true, Vec::new()),
            OverlayHostMessageKind::Shutdown => {
                let _ = self.hide_visible_cursor();
                self.reply(true, Vec::new())
            }
            OverlayHostMessageKind::SetCursor => {
                self.state = message.state;
                self.render_reply()
            }
            OverlayHostMessageKind::Hide => {
                if let Some(state) = self.state.as_mut() {
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
                self.state = message.state;
                if let Some(state) = self.state.as_mut() {
                    state.visible = true;
                }
                self.render_reply()
            }
        }
    }

    pub fn tick(&mut self) {
        let _ = self.follow_system_pointer();
    }

    fn render_reply(&mut self) -> OverlayHostReply {
        match self.render_current() {
            Ok(()) => self.reply(true, Vec::new()),
            Err(error) => self.reply(
                false,
                vec![diagnostic(
                    "OverlayRenderFailed",
                    "X11 overlay failed to render the agent cursor.",
                    Some(error.to_string()),
                )],
            ),
        }
    }

    fn render_current(&mut self) -> Result<()> {
        let visible_point = self
            .state
            .as_ref()
            .filter(|state| state.visible)
            .and_then(cursor_point);

        let Some((x, y)) = visible_point else {
            self.system_cursor
                .restore()
                .context("failed to restore X11 system cursor")?;
            self.conn
                .unmap_window(self.window)
                .context("failed to hide X11 cursor overlay window")?;
            self.conn
                .flush()
                .context("failed to flush X11 cursor overlay hide")?;
            return Ok(());
        };

        let left = rounded_i16(x, cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X)?;
        let top = rounded_i16(y, cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y)?;
        self.conn
            .configure_window(
                self.window,
                &ConfigureWindowAux::new()
                    .x(i32::from(left))
                    .y(i32::from(top))
                    .stack_mode(StackMode::ABOVE),
            )
            .context("failed to position X11 cursor overlay window")?;
        self.conn
            .map_window(self.window)
            .context("failed to map X11 cursor overlay window")?;
        self.draw_cursor()?;
        self.system_cursor
            .set_hidden(true)
            .context("failed to hide X11 system cursor")?;
        self.conn
            .flush()
            .context("failed to flush X11 cursor overlay render")?;
        Ok(())
    }

    fn hide_visible_cursor(&mut self) -> Result<()> {
        if let Some(state) = self.state.as_mut() {
            state.visible = false;
        }
        self.render_current()
    }

    fn follow_system_pointer(&mut self) -> Result<()> {
        if !self.state.as_ref().is_some_and(|state| state.visible) {
            return Ok(());
        }
        let Some(position) = self.system_cursor.pointer_position()? else {
            return Ok(());
        };
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        if !state_needs_system_pointer_update(state, position) {
            return Ok(());
        }
        apply_system_pointer_position(state, position);
        self.render_current()
    }

    fn draw_cursor(&self) -> Result<()> {
        for (pixel, rectangles) in &self.cursor.pixel_rectangles {
            self.conn
                .change_gc(self.gc, &ChangeGCAux::new().foreground(*pixel))
                .context("failed to set X11 cursor drawing color")?;
            self.conn
                .poly_fill_rectangle(self.window, self.gc, rectangles.as_slice())
                .context("failed to draw X11 cursor pixels")?;
        }
        Ok(())
    }

    fn reply(&self, ok: bool, diagnostics: Vec<DiagnosticEntry>) -> OverlayHostReply {
        OverlayHostReply {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            ok,
            capabilities: Some(self.capabilities()),
            state: self.state.clone(),
            diagnostics,
        }
    }

    fn capabilities(&self) -> AgentCursorCapabilities {
        let mut reason = x11_backend_reason(std::env::var("XDG_SESSION_TYPE").ok().as_deref());
        if let Some(system_cursor_reason) = self.system_cursor.reason() {
            reason.push_str("; system cursor hide unsupported: ");
            reason.push_str(system_cursor_reason);
        }
        AgentCursorCapabilities {
            backend: AgentCursorBackendKind::X11ShapedWindow,
            renderer_backend: AgentCursorRendererBackendKind::None,
            visible_overlay: true,
            screenshot_synthetic_cursor: false,
            click_through: true,
            capture_exclusion: false,
            pointer_tracking_backend: AgentCursorPointerTrackingBackendKind::X11Query,
            pointer_tracking_exact: true,
            system_cursor_hide_supported: self.system_cursor.supported(),
            system_cursor_hidden: self.system_cursor.hidden(),
            system_cursor_backend: self.system_cursor.backend(),
            needs_user_install: false,
            reason: Some(reason),
        }
    }
}

impl Drop for X11OverlayBackend {
    fn drop(&mut self) {
        let _ = self.system_cursor.restore();
        let _ = self.conn.free_gc(self.gc);
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

fn set_shape(
    conn: &RustConnection,
    window: Window,
    visible_rectangles: &[Rectangle],
) -> Result<()> {
    shape::rectangles(
        conn,
        SO::SET,
        SK::BOUNDING,
        xproto::ClipOrdering::UNSORTED,
        window,
        0,
        0,
        visible_rectangles,
    )
    .context("failed to set X11 cursor overlay bounding shape")?;
    shape::rectangles(
        conn,
        SO::SET,
        SK::INPUT,
        xproto::ClipOrdering::UNSORTED,
        window,
        0,
        0,
        &[],
    )
    .context("failed to set empty X11 cursor overlay input shape")?;
    Ok(())
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

fn rounded_i16(value: f64, hotspot: i32) -> Result<i16> {
    let rounded = value.round() as i64 - i64::from(hotspot);
    i16::try_from(rounded)
        .with_context(|| format!("X11 overlay coordinate {rounded} is outside i16 range"))
}

#[derive(Debug)]
struct CursorImage {
    width: u16,
    height: u16,
    shape_rectangles: Vec<Rectangle>,
    pixel_rectangles: Vec<(u32, Vec<Rectangle>)>,
}

impl CursorImage {
    fn load(visual: &VisualFormat) -> Result<Self> {
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
        Ok(cursor_geometry(
            image.as_raw(),
            u16::try_from(width).expect("cursor width fits u16"),
            u16::try_from(height).expect("cursor height fits u16"),
            visual,
        ))
    }
}

fn cursor_geometry(rgba: &[u8], width: u16, height: u16, visual: &VisualFormat) -> CursorImage {
    let mut shape_rectangles = Vec::new();
    let mut by_color: BTreeMap<u32, Vec<Rectangle>> = BTreeMap::new();
    for y in 0..height {
        let mut shape_start: Option<u16> = None;
        let mut color_start: Option<(u16, u32)> = None;
        for x in 0..width {
            let offset = (usize::from(y) * usize::from(width) + usize::from(x)) * 4;
            let r = rgba[offset];
            let g = rgba[offset + 1];
            let b = rgba[offset + 2];
            let a = rgba[offset + 3];
            if a <= ALPHA_VISIBLE_THRESHOLD {
                flush_shape_span(&mut shape_rectangles, &mut shape_start, x, y);
                flush_color_span(&mut by_color, &mut color_start, x, y);
                continue;
            }

            if shape_start.is_none() {
                shape_start = Some(x);
            }
            let pixel = visual.rgb_to_pixel(r, g, b);
            match color_start {
                Some((_, current_pixel)) if current_pixel == pixel => {}
                Some(_) => {
                    flush_color_span(&mut by_color, &mut color_start, x, y);
                    color_start = Some((x, pixel));
                }
                None => color_start = Some((x, pixel)),
            }
        }
        flush_shape_span(&mut shape_rectangles, &mut shape_start, width, y);
        flush_color_span(&mut by_color, &mut color_start, width, y);
    }
    CursorImage {
        width,
        height,
        shape_rectangles,
        pixel_rectangles: by_color.into_iter().collect(),
    }
}

fn flush_shape_span(rectangles: &mut Vec<Rectangle>, start: &mut Option<u16>, end: u16, y: u16) {
    let Some(x) = start.take() else {
        return;
    };
    if end > x {
        rectangles.push(Rectangle {
            x: i16::try_from(x).expect("cursor x fits i16"),
            y: i16::try_from(y).expect("cursor y fits i16"),
            width: end - x,
            height: 1,
        });
    }
}

fn flush_color_span(
    by_color: &mut BTreeMap<u32, Vec<Rectangle>>,
    start: &mut Option<(u16, u32)>,
    end: u16,
    y: u16,
) {
    let Some((x, pixel)) = start.take() else {
        return;
    };
    if end > x {
        by_color.entry(pixel).or_default().push(Rectangle {
            x: i16::try_from(x).expect("cursor x fits i16"),
            y: i16::try_from(y).expect("cursor y fits i16"),
            width: end - x,
            height: 1,
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct VisualFormat {
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
}

impl VisualFormat {
    fn for_root_visual(depths: &[xproto::Depth], root_visual: Visualid) -> Option<Self> {
        depths
            .iter()
            .flat_map(|depth| depth.visuals.iter())
            .find(|visual| visual.visual_id == root_visual)
            .and_then(Self::from_visual)
    }

    fn from_visual(visual: &Visualtype) -> Option<Self> {
        if visual.class != xproto::VisualClass::TRUE_COLOR
            && visual.class != xproto::VisualClass::DIRECT_COLOR
        {
            return None;
        }
        Some(Self {
            red_mask: visual.red_mask,
            green_mask: visual.green_mask,
            blue_mask: visual.blue_mask,
        })
    }

    fn rgb_to_pixel(&self, r: u8, g: u8, b: u8) -> u32 {
        channel_to_mask(r, self.red_mask)
            | channel_to_mask(g, self.green_mask)
            | channel_to_mask(b, self.blue_mask)
    }
}

fn channel_to_mask(channel: u8, mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let max_value = (1_u32 << bits) - 1;
    let scaled = (u32::from(channel) * max_value + 127) / 255;
    (scaled << shift) & mask
}

fn x11_backend_reason(session_type: Option<&str>) -> String {
    if session_type.is_some_and(|value| value.trim().eq_ignore_ascii_case("wayland")) {
        return "X Shape visible overlay active on X11/XWayland display; native Wayland surfaces may cover it".to_string();
    }
    "X Shape visible overlay active".to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        VisualFormat, apply_system_pointer_position, cursor_geometry, cursor_point,
        state_needs_system_pointer_update, x11_backend_reason,
    };
    use crate::system_cursor::SystemPointerPosition;
    use sky_cua_platform::model::{AgentCursorPoint, AgentCursorState, CoordinateSpace};

    #[test]
    fn cursor_point_prefers_native_coordinates_for_x11_root_coordinates() {
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
    fn system_pointer_update_moves_x11_state_to_desktop_coordinates() {
        let mut state = AgentCursorState {
            visible: true,
            sequence: 3,
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
        let position = SystemPointerPosition { x: 640.0, y: 360.0 };

        assert!(state_needs_system_pointer_update(&state, position));
        apply_system_pointer_position(&mut state, position);

        assert_eq!(state.sequence, 4);
        assert_eq!(cursor_point(&state), Some((640.0, 360.0)));
        assert!(!state_needs_system_pointer_update(
            &state,
            SystemPointerPosition {
                x: 640.25,
                y: 360.25
            }
        ));
        assert!(state_needs_system_pointer_update(
            &state,
            SystemPointerPosition { x: 641.0, y: 360.0 }
        ));
    }

    #[test]
    fn cursor_geometry_shapes_only_nontransparent_spans() {
        let rgba = [
            0, 0, 0, 0, 10, 20, 30, 255, 11, 21, 31, 255, 0, 0, 0, 0, 40, 50, 60, 255, 0, 0, 0, 0,
        ];
        let visual = true_color_visual();

        let cursor = cursor_geometry(&rgba, 3, 2, &visual);

        assert_eq!(cursor.shape_rectangles.len(), 2);
        assert_eq!(cursor.shape_rectangles[0].x, 1);
        assert_eq!(cursor.shape_rectangles[0].y, 0);
        assert_eq!(cursor.shape_rectangles[0].width, 2);
        assert_eq!(cursor.shape_rectangles[1].x, 1);
        assert_eq!(cursor.shape_rectangles[1].y, 1);
        assert_eq!(cursor.shape_rectangles[1].width, 1);
    }

    #[test]
    fn cursor_geometry_uses_root_visual_color_masks() {
        let rgba = [255, 128, 0, 255];
        let visual = true_color_visual();

        let cursor = cursor_geometry(&rgba, 1, 1, &visual);

        assert_eq!(cursor.pixel_rectangles[0].0, 0xff8000);
    }

    #[test]
    fn x11_backend_reason_names_xwayland_limitation() {
        assert!(
            x11_backend_reason(Some("wayland")).contains("native Wayland surfaces may cover it")
        );
        assert_eq!(
            x11_backend_reason(Some("x11")),
            "X Shape visible overlay active"
        );
    }

    fn true_color_visual() -> VisualFormat {
        VisualFormat {
            red_mask: 0x00ff0000,
            green_mask: 0x0000ff00,
            blue_mask: 0x000000ff,
        }
    }
}
