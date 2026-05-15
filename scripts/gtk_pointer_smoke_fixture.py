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
            "submitted_text": "",
            "points": {},
            "window_size": {},
            "last_event": "starting",
        }

        self.fixed = Gtk.Fixed()
        self.add(self.fixed)
        self.fixed.set_hexpand(True)
        self.fixed.set_vexpand(True)

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
        self.text_entry.connect("changed", self.on_entry_changed)
        self.text_entry.connect("activate", self.on_entry_activate)

        self.click_button = Gtk.Button(label="Physical click target")
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

        margin_x = max(48, width // 20)
        top = max(48, height // 18)
        section_gap = max(36, width // 30)
        region_width = max(360, int(width * 0.32))
        region_height = max(200, int(height * 0.22))
        button_width = max(260, int(width * 0.18))
        button_height = 84
        entry_width = max(340, int(width * 0.32))
        entry_height = 48

        self.fixed.move(self.header, margin_x, top)
        self.header.set_size_request(width - (margin_x * 2), -1)

        instructions_y = top + 56
        self.fixed.move(self.instructions, margin_x, instructions_y)
        self.instructions.set_size_request(width - (margin_x * 2), -1)

        entry_x = margin_x
        entry_y = instructions_y + 88
        self.fixed.move(self.text_entry, entry_x, entry_y)
        self.text_entry.set_size_request(entry_width, entry_height)

        button_x = margin_x
        button_y = entry_y + entry_height + 24
        self.fixed.move(self.click_button, button_x, button_y)
        self.click_button.set_size_request(button_width, button_height)

        region_top = button_y + button_height + 48
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

        status_y = scroll_y + region_height + 48
        self.fixed.move(self.status, margin_x, status_y)
        self.status.set_size_request(width - (margin_x * 2), -1)

        origin_x, origin_y = self.window_origin(width, height)
        self.state["window_origin"] = {"x": origin_x, "y": origin_y}
        self.state["window_size"] = {"width": width, "height": height}
        self.state["points"] = {
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
                region_width,
                region_height,
            ),
            "scroll": self.center(
                origin_x + scroll_x, origin_y + scroll_y, region_width, region_height
            ),
            "scroll_safe": self.center(
                origin_x + scroll_x,
                origin_y + scroll_y,
                region_width,
                min(96, region_height),
            ),
        }
        self.state["ready"] = True
        self.state["last_event"] = f"layout:{width}x{height}"
        self.update_status()
        self.write_state()

    def center(self, x: int, y: int, width: int, height: int) -> dict[str, float]:
        return {"x": x + (width / 2.0), "y": y + (height / 2.0)}

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
                x, y = origin
                origin_x = int(x)
                origin_y = int(y)

        display = Gdk.Display.get_default()
        if display is None:
            return (origin_x, origin_y)

        monitor = display.get_monitor_at_window(gdk_window)
        if monitor is None:
            return (origin_x, origin_y)

        if "gnome" not in os.environ.get("XDG_CURRENT_DESKTOP", "").lower():
            return (origin_x, origin_y)

        geometry = monitor.get_geometry()
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
            "entry_text={entry_text!r}  submitted_text={submitted_text!r}\n"
            "points={points}".format(
                clicked=self.state["clicked"],
                secondary_clicked=self.state["secondary_clicked"],
                drag_completed=self.state["drag_completed"],
                scroll_events=self.state["scroll_events"],
                entry_text=self.state["entry_text"],
                submitted_text=self.state["submitted_text"],
                points=json.dumps(points, sort_keys=True),
            )
        )

    def write_state(self) -> None:
        self.state_path.parent.mkdir(parents=True, exist_ok=True)
        tmp_path = self.state_path.with_suffix(".tmp")
        tmp_path.write_text(json.dumps(self.state, indent=2, sort_keys=True), encoding="utf-8")
        tmp_path.replace(self.state_path)

    def on_click_button(self, *_args: object) -> None:
        self.state["clicked"] = True
        self.state["last_event"] = "click-button"
        self.update_status()
        self.write_state()

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
            self.write_state()
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
            self.write_state()
        self.drag_origin = None
        self.drag_seen = False
        return False

    def on_scroll_adjustment(self, adjustment: Gtk.Adjustment) -> None:
        if adjustment.get_value() <= 0:
            return
        self.state["scroll_events"] += 1
        self.state["last_event"] = f"scroll-adjustment:{adjustment.get_value():.1f}"
        self.update_status()
        self.write_state()


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
