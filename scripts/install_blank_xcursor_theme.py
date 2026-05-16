#!/usr/bin/env python3
"""Install a fully transparent Xcursor theme for dedicated COSMIC VM sessions."""

from __future__ import annotations

import argparse
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

CURSOR_NAMES = (
    "default",
    "left_ptr",
    "arrow",
    "hand1",
    "hand2",
    "pointer",
    "xterm",
    "text",
    "crosshair",
    "wait",
    "watch",
    "progress",
    "move",
    "grab",
    "grabbing",
    "not-allowed",
    "no-drop",
    "copy",
    "alias",
    "context-menu",
    "help",
    "cell",
    "vertical-text",
    "all-scroll",
    "col-resize",
    "row-resize",
    "e-resize",
    "w-resize",
    "n-resize",
    "s-resize",
    "ne-resize",
    "nw-resize",
    "se-resize",
    "sw-resize",
    "ew-resize",
    "ns-resize",
    "nesw-resize",
    "nwse-resize",
    "zoom-in",
    "zoom-out",
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--theme-name",
        default="sky-cua-blank",
        help="Xcursor theme name to create. Defaults to sky-cua-blank.",
    )
    parser.add_argument(
        "--destination",
        type=Path,
        default=Path.home() / ".local/share/icons",
        help="Icon theme root. Defaults to ~/.local/share/icons.",
    )
    parser.add_argument("--size", type=int, default=24, help="Transparent cursor size.")
    args = parser.parse_args()

    xcursorgen = shutil.which("xcursorgen")
    if xcursorgen is None:
        raise SystemExit("xcursorgen is required to build the blank Xcursor theme")

    theme_dir = args.destination / args.theme_name
    cursor_dir = theme_dir / "cursors"
    cursor_dir.mkdir(parents=True, exist_ok=True)
    (theme_dir / "index.theme").write_text(
        f"[Icon Theme]\nName={args.theme_name}\nComment=Transparent cursor theme for sky-cua COSMIC sessions\n",
        encoding="utf-8",
    )

    with tempfile.TemporaryDirectory(prefix="sky-cua-blank-xcursor-") as tmp:
        tmp_path = Path(tmp)
        png_path = tmp_path / "blank.png"
        config_path = tmp_path / "cursor.conf"
        Image.new("RGBA", (args.size, args.size), (0, 0, 0, 0)).save(png_path)
        config_path.write_text(f"{args.size} 0 0 {png_path}\n", encoding="utf-8")
        for name in CURSOR_NAMES:
            subprocess.run(
                [xcursorgen, str(config_path), str(cursor_dir / name)],
                check=True,
                stdout=subprocess.DEVNULL,
            )

    print(theme_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
