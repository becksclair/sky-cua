#!/usr/bin/env python3
"""Fullscreen GTK fixture for live pointer-input smoke tests.

This window is intentionally simple and operator-facing. It exposes large,
fixed-purpose regions so the MCP smoke harness can prove explicit-coordinate
pointer actions against a real Wayland desktop.
"""

from __future__ import annotations

import json
import os
import signal
import sys
import time
from pathlib import Path
from typing import Any

import gi

from _pointer_geometry import adjusted_origin_for_visible_monitor

gi.require_version("Gdk", "3.0")
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

WINDOW_TITLE = "sky-cua pointer smoke"
DEFAULT_WINDOW_WIDTH = 1400
DEFAULT_WINDOW_HEIGHT = 900


class PointerSmokeWindow(Gtk.Window):
    def __init__(self, state_path: Path) -> None:
        super().__init__(title=WINDOW_TITLE)
        self.state_path = state_path
        self.layout_signature: tuple[int, int] | None = None
        self.drag_origin: tuple[float, float] | None = None
        self.drag_seen = False
        self.state: dict[str, Any] = {
            "ready": False,
            "title": WINDOW_TITLE,
            "clicked": False,
            "secondary_clicked": False,
            "scroll_events": 0,
            "drag_completed": False,
            "entry_text": "",
            "entry_focused": False,
            "submitted_text": "",
            "button_press_seen": False,
            "button_release_seen": False,
            "last_pointer_event": {},
            "points": {},
            "grid_canvas": {},
            "monitor": {},
            "window_size": {},
            "last_event": "starting",
        }

        self.fixed = Gtk.Fixed()
        self.add(self.fixed)
        self.fixed.set_hexpand(True)
        self.fixed.set_vexpand(True)

        self.grid_canvas = Gtk.DrawingArea()
        self.grid_canvas.connect("draw", self.on_grid_draw)
        self.fixed.put(self.grid_canvas, 0, 0)

        self.header = Gtk.Label()
        self.header.set_xalign(0.0)
        self.header.set_markup(
            "<span size='xx-large' weight='bold'>sky-cua live pointer smoke</span>"
        )

        self.instructions = Gtk.Label()
        self.instructions.set_xalign(0.0)
        self.instructions.set_line_wrap(True)
        self.instructions.set_text(
            "This fixture waits for explicit-coordinate MCP actions. "
            "The harness should trigger physical click, right-click, drag, and scroll here."
        )

        self.status = Gtk.Label()
        self.status.set_xalign(0.0)
        self.status.set_line_wrap(True)

        self.text_entry = Gtk.Entry()
        self.text_entry.set_placeholder_text("type_text + press_key target")
        self.text_entry.connect("focus-in-event", self.on_entry_focus_in)
        self.text_entry.connect("focus-out-event", self.on_entry_focus_out)
        self.text_entry.connect("changed", self.on_entry_changed)
        self.text_entry.connect("activate", self.on_entry_activate)

        self.click_button = Gtk.Button(label="Physical click target")
        self.click_button.add_events(
            Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.BUTTON_RELEASE_MASK
        )
        self.click_button.connect("button-press-event", self.on_click_button_press)
        self.click_button.connect("button-release-event", self.on_click_button_release)
        self.click_button.connect("clicked", self.on_click_button)

        self.secondary_box = self.make_region(
            title="Secondary-click region",
            subtitle="perform_secondary_action should toggle this.",
        )
        self.secondary_box.add_events(Gdk.EventMask.BUTTON_PRESS_MASK)
        self.secondary_box.connect("button-press-event", self.on_secondary_press)

        self.drag_box = self.make_region(
            title="Drag region",
            subtitle="drag should press, move, and release inside this box.",
        )
        self.drag_box.add_events(
            Gdk.EventMask.BUTTON_PRESS_MASK
            | Gdk.EventMask.BUTTON_RELEASE_MASK
            | Gdk.EventMask.BUTTON1_MOTION_MASK
            | Gdk.EventMask.POINTER_MOTION_MASK
        )
        self.drag_box.connect("button-press-event", self.on_drag_press)
        self.drag_box.connect("motion-notify-event", self.on_drag_motion)
        self.drag_box.connect("button-release-event", self.on_drag_release)

        self.scroll_box = Gtk.ScrolledWindow()
        self.scroll_box.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.AUTOMATIC)
        self.scroll_box_frame = Gtk.Frame(label="Scroll region")
        scroll_content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        scroll_content.set_border_width(18)
        scroll_intro = Gtk.Label(label="The MCP scroll tool should move this scrolled window.")
        scroll_intro.set_xalign(0.0)
        scroll_intro.set_line_wrap(True)
        scroll_content.pack_start(scroll_intro, False, False, 0)
        for line in range(1, 61):
            row = Gtk.Label(label=f"scroll smoke line {line:02d}")
            row.set_xalign(0.0)
            scroll_content.pack_start(row, False, False, 0)
        self.scroll_box_frame.add(scroll_content)
        self.scroll_box.add(self.scroll_box_frame)
        self.scroll_box.get_vadjustment().connect("value-changed", self.on_scroll_adjustment)

        for widget in (
            self.header,
            self.instructions,
            self.status,
            self.text_entry,
            self.click_button,
            self.secondary_box,
            self.drag_box,
            self.scroll_box,
        ):
            self.fixed.put(widget, 0, 0)

        self.connect("delete-event", self.on_delete)
        self.connect("destroy", self.on_destroy)
        self.connect("size-allocate", self.on_size_allocate)
        self.add_events(
            Gdk.EventMask.BUTTON_PRESS_MASK
            | Gdk.EventMask.BUTTON_RELEASE_MASK
            | Gdk.EventMask.POINTER_MOTION_MASK
        )
        self.connect("button-press-event", self.on_window_button_press)
        self.connect("button-release-event", self.on_window_button_release)
        self.connect("motion-notify-event", self.on_window_motion)

        signal.signal(signal.SIGTERM, self.on_signal)
        signal.signal(signal.SIGINT, self.on_signal)

        if os.environ.get("SKY_CUA_POINTER_FULLSCREEN", "1") == "0":
            width = int(os.environ.get("SKY_CUA_POINTER_WIDTH", DEFAULT_WINDOW_WIDTH))
            height = int(os.environ.get("SKY_CUA_POINTER_HEIGHT", DEFAULT_WINDOW_HEIGHT))
            self.set_default_size(width, height)
            self.resize(width, height)
            self.move(0, 0)
        else:
            self.fullscreen()
        self.show_all()
        self.present()
        self.write_state()
        GLib.idle_add(self.on_geometry_probe)
        GLib.timeout_add(250, self.on_geometry_probe)

    def make_region(self, title: str, subtitle: str) -> Gtk.EventBox:
        box = Gtk.EventBox()
        frame = Gtk.Frame()
        frame.set_shadow_type(Gtk.ShadowType.IN)
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        content.set_border_width(18)

        title_label = Gtk.Label()
        title_label.set_xalign(0.0)
        title_label.set_markup(
            f"<span size='x-large' weight='bold'>{GLib.markup_escape_text(title)}</span>"
        )

        subtitle_label = Gtk.Label(label=subtitle)
        subtitle_label.set_xalign(0.0)
        subtitle_label.set_line_wrap(True)

        content.pack_start(title_label, False, False, 0)
        content.pack_start(subtitle_label, False, False, 0)
        frame.add(content)
        box.add(frame)
        return box

    def on_signal(self, _signum: int, _frame: object) -> None:
        self.destroy()

    def on_delete(self, *_args: object) -> bool:
        self.state["last_event"] = "window-closed"
        self.write_state()
        return False

    def on_destroy(self, *_args: object) -> None:
        self.write_state()
        Gtk.main_quit()

    def on_size_allocate(self, _widget: Gtk.Widget, allocation: Gdk.Rectangle) -> None:
        width = allocation.width
        height = allocation.height
        if width <= 0 or height <= 0:
            return

        signature = (width, height)
        if signature == self.layout_signature:
            return
        self.layout_signature = signature

        # Debounce rapid successive resize events during window initialization.
        if hasattr(self, "_size_allocate_timeout_id"):
            GLib.source_remove(self._size_allocate_timeout_id)
        self._size_allocate_timeout_id = GLib.timeout_add(
            50, self._apply_size_allocate, width, height
        )

    def on_geometry_probe(self) -> bool:
        if self.state.get("ready"):
            return False
        width = self.get_allocated_width()
        height = self.get_allocated_height()
        if width <= 1 or height <= 1:
            width, height = self.visible_monitor_size(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
            self.resize(width, height)
            self.present()
        self._apply_size_allocate(width, height)
        return not self.state.get("ready")

    def _apply_size_allocate(self, width: int, height: int) -> bool:
        visible_width, visible_height = self.visible_monitor_size(width, height)
        layout_width = min(width, visible_width) if visible_width > 0 else width
        layout_height = min(height, visible_height) if visible_height > 0 else height
        compact = layout_height < 820
        margin_x = max(32, min(64, layout_width // 20))
        top = max(24, min(48, layout_height // 18))
        section_gap = max(20, min(40, layout_width // 34))
        region_width = max(300, int(layout_width * 0.32))
        region_height = max(128, min(220, int(layout_height * (0.18 if compact else 0.22))))
        button_width = max(220, int(layout_width * 0.18))
        button_height = 64 if compact else 84
        entry_width = max(300, int(layout_width * 0.32))
        entry_height = 44 if compact else 48

        self.fixed.move(self.header, margin_x, top)
        self.header.set_size_request(layout_width - (margin_x * 2), -1)
        self.fixed.move(self.grid_canvas, 0, 0)
        self.grid_canvas.set_size_request(layout_width, layout_height)

        instructions_y = top + (42 if compact else 56)
        self.fixed.move(self.instructions, margin_x, instructions_y)
        self.instructions.set_size_request(layout_width - (margin_x * 2), -1)

        entry_x = margin_x
        entry_y = instructions_y + (62 if compact else 88)
        self.fixed.move(self.text_entry, entry_x, entry_y)
        self.text_entry.set_size_request(entry_width, entry_height)

        button_x = margin_x
        button_y = entry_y + entry_height + (16 if compact else 24)
        self.fixed.move(self.click_button, button_x, button_y)
        self.click_button.set_size_request(button_width, button_height)

        region_top = button_y + button_height + (28 if compact else 48)
        drag_x = margin_x
        drag_y = region_top
        self.fixed.move(self.drag_box, drag_x, drag_y)
        self.drag_box.set_size_request(region_width, region_height)

        secondary_x = margin_x + region_width + section_gap
        secondary_y = region_top
        self.fixed.move(self.secondary_box, secondary_x, secondary_y)
        self.secondary_box.set_size_request(region_width, region_height)

        scroll_x = margin_x
        scroll_y = region_top + region_height + section_gap
        self.fixed.move(self.scroll_box, scroll_x, scroll_y)
        self.scroll_box.set_size_request(region_width, region_height)

        status_y = scroll_y + region_height + (20 if compact else 48)
        self.fixed.move(self.status, margin_x, status_y)
        self.status.set_size_request(layout_width - (margin_x * 2), -1)

        origin_x, origin_y = self.window_origin(width, height)
        self.state["window_origin"] = {"x": origin_x, "y": origin_y}
        self.state["window_size"] = {"width": width, "height": height}
        self.state["grid_canvas"] = {
            "x": origin_x,
            "y": origin_y,
            "width": layout_width,
            "height": layout_height,
            "cell_size": self.grid_cell_size(layout_width, layout_height),
        }
        self.state["monitor"] = self.monitor_state(width, height)
        self.state["points"] = self.points_from_requested_layout(
            origin_x=origin_x,
            origin_y=origin_y,
            entry=(entry_x, entry_y, entry_width, entry_height),
            button=(button_x, button_y, button_width, button_height),
            drag=(drag_x, drag_y, region_width, region_height),
            secondary=(secondary_x, secondary_y, region_width, region_height),
            scroll=(scroll_x, scroll_y, region_width, region_height),
        )
        self.state["ready"] = True
        self.state["last_event"] = f"layout:{width}x{height}"
        self.update_status()
        # Keep the requested layout points; GTK allocation translation can report local widget
        # coordinates after activation, which makes the harness click the wrong targets.
        self.write_state()
        return False

    def grid_cell_size(self, width: int, height: int) -> int:
        short_side = min(width, height)
        if short_side < 800:
            return 50
        if short_side < 1400:
            return 80
        return 100

    def on_grid_draw(self, widget: Gtk.Widget, context: Any) -> bool:
        width = widget.get_allocated_width()
        height = widget.get_allocated_height()
        if width <= 0 or height <= 0:
            return False

        cell = self.grid_cell_size(width, height)
        # Cairo is dynamically typed through PyGObject here.
        cairo = context
        cairo.set_source_rgb(0.965, 0.97, 0.965)
        cairo.paint()
        cairo.set_source_rgba(0.18, 0.24, 0.30, 0.16)
        cairo.set_line_width(1.0)
        for x in range(0, width + 1, cell):
            cairo.move_to(x + 0.5, 0)
            cairo.line_to(x + 0.5, height)
        for y in range(0, height + 1, cell):
            cairo.move_to(0, y + 0.5)
            cairo.line_to(width, y + 0.5)
        cairo.stroke()

        cairo.set_source_rgba(0.02, 0.09, 0.16, 0.55)
        cairo.set_line_width(2.0)
        center_x = width / 2.0
        center_y = height / 2.0
        cairo.move_to(center_x, 0)
        cairo.line_to(center_x, height)
        cairo.move_to(0, center_y)
        cairo.line_to(width, center_y)
        cairo.stroke()

        cairo.select_font_face("Sans", 0, 0)
        cairo.set_font_size(13)
        cairo.set_source_rgba(0.02, 0.09, 0.16, 0.62)
        for x in range(0, width + 1, cell * 2):
            cairo.move_to(x + 6, 20)
            cairo.show_text(str(x))
        for y in range(cell * 2, height + 1, cell * 2):
            cairo.move_to(8, y - 6)
            cairo.show_text(str(y))
        return False

    def center(self, x: int, y: int, width: int, height: int) -> dict[str, float]:
        return {"x": x + (width / 2.0), "y": y + (height / 2.0)}

    def points_from_requested_layout(
        self,
        *,
        origin_x: int,
        origin_y: int,
        entry: tuple[int, int, int, int],
        button: tuple[int, int, int, int],
        drag: tuple[int, int, int, int],
        secondary: tuple[int, int, int, int],
        scroll: tuple[int, int, int, int],
    ) -> dict[str, dict[str, float]]:
        entry_x, entry_y, entry_width, entry_height = entry
        button_x, button_y, button_width, button_height = button
        drag_x, drag_y, region_width, region_height = drag
        secondary_x, secondary_y, secondary_width, secondary_height = secondary
        scroll_x, scroll_y, scroll_width, scroll_height = scroll
        return {
            "text_entry": self.center(
                origin_x + entry_x, origin_y + entry_y, entry_width, entry_height
            ),
            "click_button": self.center(
                origin_x + button_x, origin_y + button_y, button_width, button_height
            ),
            "drag_from": self.center(
                origin_x + drag_x + 32,
                origin_y + drag_y + 32,
                region_width - 160,
                region_height - 64,
            ),
            "drag_to": self.center(
                origin_x + drag_x + 160,
                origin_y + drag_y + 32,
                region_width - 96,
                region_height - 64,
            ),
            "secondary": self.center(
                origin_x + secondary_x,
                origin_y + secondary_y,
                secondary_width,
                secondary_height,
            ),
            "scroll": self.center(
                origin_x + scroll_x, origin_y + scroll_y, scroll_width, scroll_height
            ),
            "scroll_safe": self.center(
                origin_x + scroll_x,
                origin_y + scroll_y,
                scroll_width,
                min(96, scroll_height),
            ),
        }

    def refresh_points_from_allocations(self) -> bool:
        origin_x, origin_y = self.window_origin(
            self.get_allocated_width(), self.get_allocated_height()
        )
        points = {
            "text_entry": self.widget_point(self.text_entry, origin_x, origin_y),
            "click_button": self.widget_point(self.click_button, origin_x, origin_y),
            "drag_from": self.widget_point(self.drag_box, origin_x, origin_y, x_ratio=0.32),
            "drag_to": self.widget_point(self.drag_box, origin_x, origin_y, x_ratio=0.72),
            "secondary": self.widget_point(self.secondary_box, origin_x, origin_y),
            "scroll": self.widget_point(self.scroll_box, origin_x, origin_y),
            "scroll_safe": self.widget_point(
                self.scroll_box,
                origin_x,
                origin_y,
                y_pixels=48.0,
            ),
        }
        if all(point is not None for point in points.values()):
            self.state["window_origin"] = {"x": origin_x, "y": origin_y}
            self.state["points"] = points
            self.state["last_event"] = (
                f"layout:{self.get_allocated_width()}x{self.get_allocated_height()}:allocations"
            )
            self.update_status()
            self.write_state()
        return False

    def widget_point(
        self,
        widget: Gtk.Widget,
        origin_x: int,
        origin_y: int,
        *,
        x_ratio: float = 0.5,
        y_ratio: float = 0.5,
        y_pixels: float | None = None,
    ) -> dict[str, float] | None:
        allocation = widget.get_allocation()
        if allocation.width <= 0 or allocation.height <= 0:
            return None
        local_x = allocation.width * x_ratio
        local_y = y_pixels if y_pixels is not None else allocation.height * y_ratio
        translated = widget.translate_coordinates(self, int(local_x), int(local_y))
        if translated is None:
            return None
        x, y = translated
        return {"x": origin_x + float(x), "y": origin_y + float(y)}

    def visible_monitor_size(self, fallback_width: int, fallback_height: int) -> tuple[int, int]:
        gdk_window = self.get_window()
        display = Gdk.Display.get_default()
        if gdk_window is None or display is None:
            return (fallback_width, fallback_height)
        monitor = display.get_monitor_at_window(gdk_window)
        if monitor is None:
            return (fallback_width, fallback_height)
        geometry = monitor.get_geometry()
        return (geometry.width, geometry.height)

    def monitor_state(self, allocation_width: int, allocation_height: int) -> dict[str, Any]:
        gdk_window = self.get_window()
        display = Gdk.Display.get_default()
        if gdk_window is None or display is None:
            return {}
        monitor = display.get_monitor_at_window(gdk_window)
        if monitor is None:
            return {}
        geometry = monitor.get_geometry()
        origin_x, origin_y = self.window_origin(allocation_width, allocation_height)
        state: dict[str, Any] = {
            "geometry": {
                "x": int(geometry.x),
                "y": int(geometry.y),
                "width": int(geometry.width),
                "height": int(geometry.height),
            },
            "window_origin": {"x": origin_x, "y": origin_y},
            "scale_factor": int(monitor.get_scale_factor()),
        }
        for key, getter in (
            ("manufacturer", monitor.get_manufacturer),
            ("model", monitor.get_model),
        ):
            try:
                value = getter()
            except TypeError:
                value = None
            if value:
                state[key] = value
        return state

    def window_origin(self, allocation_width: int, allocation_height: int) -> tuple[int, int]:
        gdk_window = self.get_window()
        if gdk_window is None:
            return (0, 0)

        origin_x = 0
        origin_y = 0
        try:
            origin = gdk_window.get_origin()
        except TypeError:
            origin = None

        if isinstance(origin, tuple):
            if len(origin) == 3:
                success, x, y = origin
                if success:
                    origin_x = int(x)
                    origin_y = int(y)
            elif len(origin) == 2:
                origin_x = int(origin[0])
                origin_y = int(origin[1])

        display = Gdk.Display.get_default()
        if display is None:
            return (origin_x, origin_y)

        monitor = display.get_monitor_at_window(gdk_window)
        if monitor is None:
            return (origin_x, origin_y)

        geometry = monitor.get_geometry()
        if (
            os.environ.get("XDG_SESSION_TYPE", "").lower() == "wayland"
            and "gnome" not in os.environ.get("XDG_CURRENT_DESKTOP", "").lower()
            and origin_x == 0
            and origin_y == 0
            and (geometry.x != 0 or geometry.y != 0)
        ):
            return (int(geometry.x), int(geometry.y))

        if "gnome" not in os.environ.get("XDG_CURRENT_DESKTOP", "").lower():
            return (origin_x, origin_y)

        return adjusted_origin_for_visible_monitor(
            origin_x,
            origin_y,
            allocation_width,
            allocation_height,
            geometry.width,
            geometry.height,
        )

    def update_status(self) -> None:
        points = self.state.get("points", {})
        self.status.set_text(
            "clicked={clicked}  secondary_clicked={secondary_clicked}  "
            "drag_completed={drag_completed}  scroll_events={scroll_events}  "
            "entry_focused={entry_focused}  entry_text={entry_text!r}  submitted_text={submitted_text!r}\n"
            "points={points}".format(
                clicked=self.state["clicked"],
                secondary_clicked=self.state["secondary_clicked"],
                drag_completed=self.state["drag_completed"],
                scroll_events=self.state["scroll_events"],
                entry_focused=self.state["entry_focused"],
                entry_text=self.state["entry_text"],
                submitted_text=self.state["submitted_text"],
                points=json.dumps(points, sort_keys=True),
            )
        )

    def write_state(self, *, force: bool = False) -> None:
        now = time.time()
        if not force and hasattr(self, "_last_write_time") and now - self._last_write_time < 0.05:
            return
        self._last_write_time = now
        self.state_path.parent.mkdir(parents=True, exist_ok=True)
        tmp_path = self.state_path.with_suffix(".tmp")
        tmp_path.write_text(json.dumps(self.state, indent=2, sort_keys=True), encoding="utf-8")
        tmp_path.replace(self.state_path)

    def record_pointer_event(
        self,
        widget_name: str,
        kind: str,
        event: Gdk.EventButton | Gdk.EventMotion,
        *,
        force_write: bool = False,
    ) -> None:
        self.state["last_pointer_event"] = {
            "kind": kind,
            "widget": widget_name,
            "x": float(getattr(event, "x", 0.0)),
            "y": float(getattr(event, "y", 0.0)),
            "x_root": float(getattr(event, "x_root", 0.0)),
            "y_root": float(getattr(event, "y_root", 0.0)),
            "time": time.time(),
        }
        self.state["last_event"] = f"pointer-{kind}:{widget_name}"
        self.write_state(force=force_write)

    def on_click_button_press(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        self.state["button_press_seen"] = True
        self.record_pointer_event("click_button", "press", event, force_write=True)
        return True

    def on_click_button_release(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        self.state["button_release_seen"] = True
        self.record_pointer_event("click_button", "release", event, force_write=True)
        self.on_click_button()
        return True

    def on_window_button_press(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        self.record_pointer_event("window", "press", event)
        return False

    def on_window_button_release(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        self.record_pointer_event("window", "release", event)
        return False

    def on_window_motion(self, _widget: Gtk.Widget, event: Gdk.EventMotion) -> bool:
        last = self.state.get("last_pointer_event", {})
        last_x = last.get("x", 0.0)
        last_y = last.get("y", 0.0)
        last_time = last.get("time", 0.0)
        now = time.time()
        if (
            last.get("kind") == "motion"
            and last.get("widget") == "window"
            and abs(last_x - event.x) < 2.0
            and abs(last_y - event.y) < 2.0
            and now - last_time < 0.05
        ):
            return False
        self.record_pointer_event("window", "motion", event)
        return False

    def on_click_button(self, *_args: object) -> None:
        self.state["clicked"] = True
        self.state["last_event"] = "click-button"
        self.update_status()
        self.write_state(force=True)

    def on_entry_focus_in(self, _entry: Gtk.Entry, _event: Gdk.EventFocus) -> bool:
        self.state["entry_focused"] = True
        self.state["last_event"] = "entry-focus-in"
        self.update_status()
        self.write_state(force=True)
        return False

    def on_entry_focus_out(self, _entry: Gtk.Entry, _event: Gdk.EventFocus) -> bool:
        self.state["entry_focused"] = False
        self.state["last_event"] = "entry-focus-out"
        self.update_status()
        self.write_state(force=True)
        return False

    def on_entry_changed(self, entry: Gtk.Entry) -> None:
        self.state["entry_text"] = entry.get_text()
        self.state["last_event"] = "entry-changed"
        self.update_status()
        self.write_state()

    def on_entry_activate(self, entry: Gtk.Entry) -> None:
        self.state["submitted_text"] = entry.get_text()
        self.state["last_event"] = "entry-activate"
        self.update_status()
        self.write_state()

    def on_secondary_press(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        if event.button == 3:
            self.state["secondary_clicked"] = True
            self.state["last_event"] = "secondary-click"
            self.update_status()
            self.write_state(force=True)
        return False

    def on_drag_press(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        if event.button == 1:
            self.drag_origin = (event.x, event.y)
            self.drag_seen = False
            self.state["last_event"] = "drag-press"
            self.write_state()
        return False

    def on_drag_motion(self, _widget: Gtk.Widget, event: Gdk.EventMotion) -> bool:
        if self.drag_origin is None:
            return False
        dx = abs(event.x - self.drag_origin[0])
        dy = abs(event.y - self.drag_origin[1])
        if dx >= 80 or dy >= 40:
            self.drag_seen = True
            self.state["last_event"] = f"drag-motion:{dx:.1f},{dy:.1f}"
            self.write_state()
        return False

    def on_drag_release(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        if event.button == 1 and self.drag_origin is not None and self.drag_seen:
            self.state["drag_completed"] = True
            self.state["last_event"] = "drag-release"
            self.update_status()
            self.write_state(force=True)
        self.drag_origin = None
        self.drag_seen = False
        return False

    def on_scroll_adjustment(self, adjustment: Gtk.Adjustment) -> None:
        if adjustment.get_value() <= 0:
            return
        self.state["scroll_events"] += 1
        self.state["last_event"] = f"scroll-adjustment:{adjustment.get_value():.1f}"
        self.update_status()
        self.write_state(force=True)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} STATE_PATH", file=sys.stderr)
        return 2

    state_path = Path(argv[1]).expanduser().resolve()
    window = PointerSmokeWindow(state_path)
    window.show_all()
    Gtk.main()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
