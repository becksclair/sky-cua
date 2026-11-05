"""Pointer fixture with the on-screen debug oracle removed for model evaluation."""

from __future__ import annotations

import sys
from pathlib import Path

from gtk_pointer_smoke_fixture import Gtk, PointerSmokeWindow, install_dark_theme


class BlindPointerWindow(PointerSmokeWindow):
    def update_status(self) -> None:
        self.status.set_text("Desktop interaction test")


def main() -> None:
    install_dark_theme()
    window = BlindPointerWindow(Path(sys.argv[1]).resolve())
    window.show_all()
    Gtk.main()


if __name__ == "__main__":
    main()
