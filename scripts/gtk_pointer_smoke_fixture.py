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


def install_dark_theme() -> None:
    settings = Gtk.Settings.get_default()
    if settings is not None:
        settings.set_property("gtk-application-prefer-dark-theme", True)

    screen = Gdk.Screen.get_default()
    if screen is None:
        return
    provider = Gtk.CssProvider()
    provider.load_from_data(
        b"""
        window, .background {
            background-color: #111318;
            color: #f3f4f6;
        }
        label { color: #f3f4f6; }
        entry, button, combobox, spinbutton, frame, scrolledwindow {
            background-color: #20242c;
            color: #f3f4f6;
        }
        entry, button, combobox, spinbutton {
            border-color: #5b6472;
        }
        """
    )
    Gtk.StyleContext.add_provider_for_screen(
        screen,
        provider,
        Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
    )


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
            "horizontal_scroll_events": 0,
            "drag_completed": False,
            "entry_text": "",
            "entry_focused": False,
            "submitted_text": "",
            "checkbox_toggled": False,
            "expander_expanded": False,
            "slider_h_value": 0.0,
            "slider_v_value": 0.0,
            "spin_value": 0,
            "combo_index": 0,
            "combo_text": "",
            "switch_active": False,
            "xy_pad_x": 0.0,
            "xy_pad_y": 0.0,
            "xy_pad_dragged": False,
            "dnd_dropped": False,
            "dnd_payload": "",
            "fixture_controls_version": 2,
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
            "This fixture waits for explicit-coordinate MCP actions across a grid of "
            "controls: click, right-click, drag, scroll, text entry, sliders, a spin "
            "button, combo, switch, a 2D drag pad, and drag-and-drop."
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
        self.scroll_box.get_accessible().set_name("Scroll region")
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
        self.scroll_box.get_hadjustment().connect(
            "value-changed", self.on_horizontal_scroll_adjustment
        )

        # Semantic-action targets discovered through the accessibility tree (not by
        # coordinate): a check button (CHECKABLE state -> desktop_toggle) and an
        # expander (EXPANDABLE state -> desktop_semantic expand/collapse). Their
        # state is ground truth for the smoke.
        self.semantic_checkbox = Gtk.CheckButton(label="Enable smoke option")
        self.semantic_checkbox.connect("toggled", self.on_semantic_checkbox_toggled)
        self.semantic_expander = Gtk.Expander(label="Smoke details (expander)")
        self.semantic_expander.add(Gtk.Label(label="Expanded smoke detail content."))
        self.semantic_expander.connect("notify::expanded", self.on_semantic_expander_expanded)

        # Drag-input controls. Sliders and the 2D pad must track the pointer
        # through continuous motion under the button grab; a single teleport
        # leaves them unmoved. draw_value is off so the trough spans the whole
        # widget and the exported from/to points map cleanly to thumb travel.
        self.slider_h = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0.0, 100.0, 1.0)
        self.slider_h.set_draw_value(False)
        self.slider_h.set_value(0.0)
        self.slider_h.connect("value-changed", self.on_slider_h_changed)

        self.slider_v = Gtk.Scale.new_with_range(Gtk.Orientation.VERTICAL, 0.0, 100.0, 1.0)
        self.slider_v.set_draw_value(False)
        self.slider_v.set_value(0.0)
        self.slider_v.connect("value-changed", self.on_slider_v_changed)

        self.spin = Gtk.SpinButton.new_with_range(0.0, 10.0, 1.0)
        self.spin.set_value(0.0)
        self.spin.connect("value-changed", self.on_spin_changed)

        self.combo = Gtk.ComboBoxText()
        for index, label in enumerate(("Choose option", "Alpha", "Bravo", "Charlie")):
            self.combo.append(str(index), label)
        self.combo.set_active(0)
        self.combo.connect("changed", self.on_combo_changed)

        self.switch = Gtk.Switch()
        self.switch.set_active(False)
        self.switch.connect("notify::active", self.on_switch_toggled)

        self.xy_pad_pos: tuple[float, float] | None = None
        self.xy_pad = Gtk.EventBox()
        self.xy_pad_canvas = Gtk.DrawingArea()
        self.xy_pad_canvas.connect("draw", self.on_xy_pad_draw)
        self.xy_pad.add(self.xy_pad_canvas)
        # BUTTON1_MOTION only: motion fires while button 1 is held, so hovering
        # the pad cannot spuriously mark it dragged.
        self.xy_pad.add_events(
            Gdk.EventMask.BUTTON_PRESS_MASK
            | Gdk.EventMask.BUTTON_RELEASE_MASK
            | Gdk.EventMask.BUTTON1_MOTION_MASK
        )
        self.xy_pad.connect("button-press-event", self.on_xy_pad_press)
        self.xy_pad.connect("motion-notify-event", self.on_xy_pad_motion)
        self.xy_pad.connect("button-release-event", self.on_xy_pad_release)

        # Drag-and-drop: a source chip dragged onto a drop zone. GTK only arms a
        # drag gesture once motion crosses its threshold while the button is
        # held, so this only completes with the interpolated backend drag.
        self.dnd_targets = [
            Gtk.TargetEntry.new("application/x-sky-cua-chip", Gtk.TargetFlags.SAME_APP, 0)
        ]
        self.dnd_source = Gtk.EventBox()
        dnd_source_frame = Gtk.Frame(label="DnD source")
        dnd_source_frame.set_shadow_type(Gtk.ShadowType.IN)
        dnd_source_label = Gtk.Label(label="Drag this chip onto the drop zone")
        dnd_source_label.set_line_wrap(True)
        dnd_source_frame.add(dnd_source_label)
        self.dnd_source.add(dnd_source_frame)
        self.dnd_source.drag_source_set(
            Gdk.ModifierType.BUTTON1_MASK, self.dnd_targets, Gdk.DragAction.COPY
        )
        self.dnd_source.connect("drag-data-get", self.on_dnd_data_get)

        self.dnd_zone = Gtk.EventBox()
        dnd_zone_frame = Gtk.Frame(label="Drop zone")
        dnd_zone_frame.set_shadow_type(Gtk.ShadowType.IN)
        dnd_zone_label = Gtk.Label(label="Release the chip here")
        dnd_zone_label.set_line_wrap(True)
        dnd_zone_frame.add(dnd_zone_label)
        self.dnd_zone.add(dnd_zone_frame)
        self.dnd_zone.drag_dest_set(Gtk.DestDefaults.ALL, self.dnd_targets, Gdk.DragAction.COPY)
        self.dnd_zone.connect("drag-data-received", self.on_dnd_data_received)

        for widget in (
            self.header,
            self.instructions,
            self.status,
            self.text_entry,
            self.click_button,
            self.secondary_box,
            self.drag_box,
            self.scroll_box,
            self.semantic_checkbox,
            self.semantic_expander,
            self.slider_h,
            self.slider_v,
            self.spin,
            self.combo,
            self.switch,
            self.xy_pad,
            self.dnd_source,
            self.dnd_zone,
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

    def on_semantic_checkbox_toggled(self, button: Gtk.CheckButton) -> None:
        self.state["checkbox_toggled"] = bool(button.get_active())
        self.state["last_event"] = "checkbox-toggled"
        self.write_state()

    def on_semantic_expander_expanded(self, expander: Gtk.Expander, _param: object) -> None:
        self.state["expander_expanded"] = bool(expander.get_expanded())
        self.state["last_event"] = "expander-toggled"
        self.write_state()

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
        if getattr(self, "_size_allocate_timeout_id", 0):
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
        self._size_allocate_timeout_id = 0
        visible_width, visible_height = self.visible_monitor_size(width, height)
        layout_width = min(width, visible_width) if visible_width > 0 else width
        layout_height = min(height, visible_height) if visible_height > 0 else height
        compact = layout_height < 820
        margin_x = max(32, min(64, layout_width // 20))
        top = max(24, min(48, layout_height // 18))
        card_gap = 14 if compact else 24

        # Chrome: title and instructions pinned to the top, status to the bottom.
        self.fixed.move(self.grid_canvas, 0, 0)
        self.grid_canvas.set_size_request(layout_width, layout_height)
        self.fixed.move(self.header, margin_x, top)
        self.header.set_size_request(layout_width - (margin_x * 2), -1)
        instructions_y = top + (40 if compact else 50)
        self.fixed.move(self.instructions, margin_x, instructions_y)
        self.instructions.set_size_request(layout_width - (margin_x * 2), -1)

        status_height = 56 if compact else 64
        status_y = layout_height - status_height - (top // 2)
        self.fixed.move(self.status, margin_x, status_y)
        self.status.set_size_request(layout_width - (margin_x * 2), -1)

        # Every interactive target lives in a uniform 4x4 card grid so the
        # harness can reach all of them from one state snapshot, on both
        # fullscreen and windowed sizes, without overflowing the monitor.
        cols = 4
        rows = 4
        grid_top = instructions_y + (50 if compact else 64)
        grid_bottom = status_y - card_gap
        grid_width = layout_width - (margin_x * 2)
        grid_height = max(rows * 80, grid_bottom - grid_top)
        card_w = (grid_width - (cols - 1) * card_gap) / cols
        card_h = (grid_height - (rows - 1) * card_gap) / rows

        def cell(col: int, row: int) -> tuple[int, int, int, int]:
            x = margin_x + col * (card_w + card_gap)
            y = grid_top + row * (card_h + card_gap)
            return (round(x), round(y), round(card_w), round(card_h))

        rects: dict[str, tuple[int, int, int, int]] = {}

        def place(widget: Gtk.Widget, name: str, col: int, row: int) -> tuple[int, int, int, int]:
            rect = cell(col, row)
            self.fixed.move(widget, rect[0], rect[1])
            widget.set_size_request(rect[2], rect[3])
            rects[name] = rect
            return rect

        # Row 0: text entry, click button, drag region, secondary-click region.
        ex, ey, ew, eh = cell(0, 0)
        entry_h = 44 if compact else 48
        entry_y = ey + max(0, (eh - entry_h) // 2)
        self.fixed.move(self.text_entry, ex, entry_y)
        self.text_entry.set_size_request(ew, entry_h)
        rects["text_entry"] = (ex, entry_y, ew, entry_h)
        place(self.click_button, "click_button", 1, 0)
        place(self.drag_box, "drag", 2, 0)
        place(self.secondary_box, "secondary", 3, 0)

        # Row 1: scroll region, semantic controls, horizontal + vertical sliders.
        scroll_rect = place(self.scroll_box, "scroll", 0, 1)
        # Add horizontal overflow only after the outer fullscreen allocation is
        # known. Advertising it during window construction distorts XWayland's
        # initial natural-size negotiation before KWin applies fullscreen.
        if layout_height >= 700:
            self.scroll_box_frame.set_size_request(scroll_rect[2] * 2, -1)
        sem_x, sem_y, sem_w, sem_h = cell(1, 1)
        self.fixed.move(self.semantic_checkbox, sem_x, sem_y)
        self.semantic_checkbox.set_size_request(sem_w, 36)
        self.fixed.move(self.semantic_expander, sem_x, sem_y + 44)
        self.semantic_expander.set_size_request(sem_w, max(40, sem_h - 44))
        rects["semantic"] = (sem_x, sem_y, sem_w, sem_h)

        sh_x, sh_y, sh_w, sh_h = cell(2, 1)
        slider_h_h = 48
        slider_h_y = sh_y + max(0, (sh_h - slider_h_h) // 2)
        self.fixed.move(self.slider_h, sh_x, slider_h_y)
        self.slider_h.set_size_request(sh_w, slider_h_h)
        rects["slider_h"] = (sh_x, slider_h_y, sh_w, slider_h_h)

        sv_x, sv_y, sv_w, sv_h = cell(3, 1)
        slider_v_w = 56
        slider_v_x = sv_x + max(0, (sv_w - slider_v_w) // 2)
        self.fixed.move(self.slider_v, slider_v_x, sv_y)
        self.slider_v.set_size_request(slider_v_w, sv_h)
        rects["slider_v"] = (slider_v_x, sv_y, slider_v_w, sv_h)

        # Row 2: spin button, combo, switch, 2D drag pad.
        sp_x, sp_y, sp_w, sp_h = cell(0, 2)
        spin_w = min(sp_w, 220)
        spin_h = 40
        spin_y = sp_y + max(0, (sp_h - spin_h) // 2)
        self.fixed.move(self.spin, sp_x, spin_y)
        self.spin.set_size_request(spin_w, spin_h)
        rects["spin"] = (sp_x, spin_y, spin_w, spin_h)

        co_x, co_y, co_w, co_h = cell(1, 2)
        combo_h = 40
        combo_y = co_y + max(0, (co_h - combo_h) // 2)
        self.fixed.move(self.combo, co_x, combo_y)
        self.combo.set_size_request(co_w, combo_h)
        rects["combo"] = (co_x, combo_y, co_w, combo_h)

        sw_x, sw_y, _sw_w, sw_h = cell(2, 2)
        switch_w = 96
        switch_h = 40
        switch_y = sw_y + max(0, (sw_h - switch_h) // 2)
        self.fixed.move(self.switch, sw_x, switch_y)
        self.switch.set_size_request(switch_w, switch_h)
        rects["switch"] = (sw_x, switch_y, switch_w, switch_h)

        place(self.xy_pad, "xy_pad", 3, 2)

        # Row 3: drag-and-drop source chip and drop zone.
        place(self.dnd_source, "dnd_source", 0, 3)
        place(self.dnd_zone, "dnd_zone", 1, 3)

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
        self.state["points"] = self.points_from_rects(origin_x, origin_y, rects)
        self.state["ready"] = True
        self.state["last_event"] = f"layout:{width}x{height}"
        self.update_status()
        # Keep the requested layout points; GTK allocation translation can report local widget
        # coordinates after activation, which makes the harness click the wrong targets.
        self.write_state()
        return False

    def points_from_rects(
        self, origin_x: int, origin_y: int, rects: dict[str, tuple[int, int, int, int]]
    ) -> dict[str, dict[str, float]]:
        def point(name: str, fx: float = 0.5, fy: float = 0.5) -> dict[str, float]:
            x, y, w, h = rects[name]
            return {"x": float(origin_x + x + w * fx), "y": float(origin_y + y + h * fy)}

        def offset(name: str, dx: float, dy: float) -> dict[str, float]:
            x, y, _w, _h = rects[name]
            return {"x": float(origin_x + x + dx), "y": float(origin_y + y + dy)}

        points = {
            "text_entry": point("text_entry"),
            "click_button": point("click_button"),
            # Two points inside the drag region, far enough apart to clear the
            # 80px completion threshold.
            "drag_from": point("drag", fx=0.28),
            "drag_to": point("drag", fx=0.74),
            "secondary": point("secondary"),
            "scroll": point("scroll"),
            "scroll_safe": offset(
                "scroll", rects["scroll"][2] / 2.0, min(48.0, rects["scroll"][3] * 0.2)
            ),
            # Horizontal slider: thumb starts at the left (min); drag rightward.
            "slider_h_from": offset("slider_h", 12.0, rects["slider_h"][3] / 2.0),
            "slider_h_to": point("slider_h", fx=0.8),
            # Vertical slider: a normal GTK vertical scale puts its minimum at the
            # top, so the value-0 thumb sits at the top; drag downward to raise it.
            "slider_v_from": offset("slider_v", rects["slider_v"][2] / 2.0, 12.0),
            "slider_v_to": offset(
                "slider_v", rects["slider_v"][2] / 2.0, rects["slider_v"][3] - 12.0
            ),
            # Spin button up-stepper sits at the upper-right of the widget.
            "spin_up_button": offset("spin", rects["spin"][2] - 12.0, 12.0),
            "spin_field": point("spin", fx=0.3),
            "combo": point("combo"),
            "switch": point("switch"),
            "switch_off": offset("switch", 12.0, rects["switch"][3] / 2.0),
            "switch_on": offset("switch", rects["switch"][2] - 12.0, rects["switch"][3] / 2.0),
            "xy_pad_from": point("xy_pad", fx=0.25, fy=0.25),
            "xy_pad_to": point("xy_pad", fx=0.75, fy=0.75),
            "dnd_source": point("dnd_source"),
            "dnd_target": point("dnd_zone"),
        }
        return points

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
        cairo.set_source_rgb(0.067, 0.075, 0.094)
        cairo.paint()
        cairo.set_source_rgba(0.60, 0.67, 0.76, 0.18)
        cairo.set_line_width(1.0)
        for x in range(0, width + 1, cell):
            cairo.move_to(x + 0.5, 0)
            cairo.line_to(x + 0.5, height)
        for y in range(0, height + 1, cell):
            cairo.move_to(0, y + 0.5)
            cairo.line_to(width, y + 0.5)
        cairo.stroke()

        cairo.set_source_rgba(0.54, 0.68, 0.86, 0.58)
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
        cairo.set_source_rgba(0.82, 0.86, 0.92, 0.72)
        for x in range(0, width + 1, cell * 2):
            cairo.move_to(x + 6, 20)
            cairo.show_text(str(x))
        for y in range(cell * 2, height + 1, cell * 2):
            cairo.move_to(8, y - 6)
            cairo.show_text(str(y))
        return False

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

    def on_horizontal_scroll_adjustment(self, adjustment: Gtk.Adjustment) -> None:
        if adjustment.get_value() <= 0:
            return
        self.state["horizontal_scroll_events"] += 1
        self.state["last_event"] = f"horizontal-scroll-adjustment:{adjustment.get_value():.1f}"
        self.update_status()
        self.write_state(force=True)

    def on_slider_h_changed(self, scale: Gtk.Scale) -> None:
        self.state["slider_h_value"] = float(scale.get_value())
        self.state["last_event"] = f"slider-h:{self.state['slider_h_value']:.1f}"
        self.update_status()
        self.write_state(force=True)

    def on_slider_v_changed(self, scale: Gtk.Scale) -> None:
        self.state["slider_v_value"] = float(scale.get_value())
        self.state["last_event"] = f"slider-v:{self.state['slider_v_value']:.1f}"
        self.update_status()
        self.write_state(force=True)

    def on_spin_changed(self, spin: Gtk.SpinButton) -> None:
        self.state["spin_value"] = int(spin.get_value_as_int())
        self.state["last_event"] = f"spin:{self.state['spin_value']}"
        self.update_status()
        self.write_state(force=True)

    def on_combo_changed(self, combo: Gtk.ComboBoxText) -> None:
        self.state["combo_index"] = int(combo.get_active())
        self.state["combo_text"] = combo.get_active_text() or ""
        self.state["last_event"] = f"combo:{self.state['combo_index']}"
        self.update_status()
        self.write_state(force=True)

    def on_switch_toggled(self, switch: Gtk.Switch, _param: object) -> None:
        self.state["switch_active"] = bool(switch.get_active())
        self.state["last_event"] = f"switch:{self.state['switch_active']}"
        self.update_status()
        self.write_state(force=True)

    def _update_xy_pad(self, event: Gdk.EventButton | Gdk.EventMotion, *, dragged: bool) -> None:
        allocation = self.xy_pad_canvas.get_allocation()
        width = max(1, allocation.width)
        height = max(1, allocation.height)
        norm_x = min(1.0, max(0.0, float(event.x) / width))
        norm_y = min(1.0, max(0.0, float(event.y) / height))
        self.xy_pad_pos = (float(event.x), float(event.y))
        self.state["xy_pad_x"] = norm_x
        self.state["xy_pad_y"] = norm_y
        if dragged:
            self.state["xy_pad_dragged"] = True
        self.xy_pad_canvas.queue_draw()
        self.update_status()
        self.write_state()

    def on_xy_pad_press(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        if event.button == 1:
            self.state["last_event"] = "xy-pad-press"
            self._update_xy_pad(event, dragged=False)
        return False

    def on_xy_pad_motion(self, _widget: Gtk.Widget, event: Gdk.EventMotion) -> bool:
        # Only count motion while button 1 is held — a real drag, not a hover.
        if not (event.state & Gdk.ModifierType.BUTTON1_MASK):
            return False
        self._update_xy_pad(event, dragged=True)
        return False

    def on_xy_pad_release(self, _widget: Gtk.Widget, event: Gdk.EventButton) -> bool:
        if event.button == 1:
            self.state["last_event"] = "xy-pad-release"
            self._update_xy_pad(event, dragged=True)
            self.write_state(force=True)
        return False

    def on_xy_pad_draw(self, widget: Gtk.Widget, context: Any) -> bool:
        width = widget.get_allocated_width()
        height = widget.get_allocated_height()
        if width <= 0 or height <= 0:
            return False
        cairo = context
        cairo.set_source_rgb(0.10, 0.12, 0.16)
        cairo.paint()
        cairo.set_source_rgba(0.64, 0.70, 0.80, 0.55)
        cairo.set_line_width(1.0)
        cairo.rectangle(0.5, 0.5, width - 1, height - 1)
        cairo.stroke()
        if self.xy_pad_pos is not None:
            pos_x, pos_y = self.xy_pad_pos
            cairo.set_source_rgb(0.96, 0.34, 0.46)
            cairo.set_line_width(2.0)
            cairo.move_to(pos_x - 9, pos_y)
            cairo.line_to(pos_x + 9, pos_y)
            cairo.move_to(pos_x, pos_y - 9)
            cairo.line_to(pos_x, pos_y + 9)
            cairo.stroke()
            cairo.arc(pos_x, pos_y, 5.0, 0.0, 6.283185)
            cairo.stroke()
        return False

    def on_dnd_data_get(
        self,
        _widget: Gtk.Widget,
        _context: Gdk.DragContext,
        selection_data: Gtk.SelectionData,
        _info: int,
        _time: int,
    ) -> None:
        selection_data.set(selection_data.get_target(), 8, b"sky-cua-chip")

    def on_dnd_data_received(
        self,
        _widget: Gtk.Widget,
        _context: Gdk.DragContext,
        _x: int,
        _y: int,
        selection_data: Gtk.SelectionData,
        _info: int,
        _time: int,
    ) -> None:
        payload = selection_data.get_data()
        self.state["dnd_dropped"] = True
        self.state["dnd_payload"] = payload.decode("utf-8", "replace") if payload else ""
        self.state["last_event"] = "dnd-dropped"
        self.update_status()
        self.write_state(force=True)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} STATE_PATH", file=sys.stderr)
        return 2

    state_path = Path(argv[1]).expanduser().resolve()
    install_dark_theme()
    window = PointerSmokeWindow(state_path)
    window.show_all()
    Gtk.main()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
