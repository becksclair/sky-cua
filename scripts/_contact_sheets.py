"""Pagination-safe contact-sheet helpers shared by the overlay animation harnesses.

Both the phone harness (``overlay_pointer_animations.py``) and the desktop
harness (``overlay_motion_animations.py``) turn a recording into per-scenario
frame contact sheets with ImageMagick ``montage``. ``montage`` paginates into
``<stem>-0.png``, ``<stem>-1.png`` … when the frames exceed one tile, so sheet
discovery and stale-sheet cleanup must match exactly this stem's pages and
never another scenario whose name merely starts with the same stem.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path


def _run_quiet(cmd: list[str]) -> int:
    """Runs a command quietly, returning its exit code."""
    return subprocess.run(cmd, capture_output=True, check=False).returncode


def own_sheets(out_png: Path) -> list[Path]:
    """This stem's sheet files: the exact ``out_png`` plus its ``<stem>-<n>.png``
    pagination pages, and never another scenario whose name merely starts with this
    stem (e.g. ``corners`` must not match ``corners-redirect-swipes-0.png``)."""
    prefix = out_png.stem + "-"
    pages = [
        page
        for page in out_png.parent.glob(f"{prefix}*.png")
        if page.name[len(prefix) : -len(".png")].isdigit()
    ]
    if out_png.exists():
        pages.append(out_png)
    return sorted(pages)


def montage_frames(frames: list[Path], out_png: Path, *, tile: str = "8x6") -> list[Path]:
    """Tiles already-extracted frame images into contact sheet(s).

    Returns the produced sheet path(s) — ``montage`` paginates into
    ``<stem>-0.png``, ``<stem>-1.png`` … when the frames exceed one tile, so this
    may return several. Returns ``[]`` when ``montage`` is missing, no frames
    were given, or montage fails. Clears only this stem's own (possibly
    paginated) sheets from a prior run first, so the post-montage discovery is
    accurate — never another scenario's.
    """
    if not frames or shutil.which("montage") is None:
        return []
    out_png.parent.mkdir(parents=True, exist_ok=True)
    for stale in own_sheets(out_png):
        stale.unlink()
    code = _run_quiet(
        [
            "montage",
            *[str(frame) for frame in frames],
            "-tile",
            tile,
            "-geometry",
            "+2+2",
            "-background",
            "#303030",
            str(out_png),
        ],
    )
    if code != 0:
        return []
    return own_sheets(out_png)


def make_contact_sheet(
    mp4: Path, out_png: Path, *, fps: int, scale: int = 300, tile: str = "8x6"
) -> list[Path]:
    """Extracts frames from [mp4] and tiles them into contact sheet(s).

    Returns the produced sheet path(s) (see :func:`montage_frames` for the
    pagination behavior). Returns ``[]`` when ``ffmpeg``/``montage`` are missing
    or extraction fails. Cleans up the intermediate frame directory.
    """
    if shutil.which("ffmpeg") is None or shutil.which("montage") is None:
        return []
    frames_dir = out_png.parent / (out_png.stem + "_frames")
    if frames_dir.exists():
        shutil.rmtree(frames_dir)
    frames_dir.mkdir(parents=True, exist_ok=True)
    try:
        pattern = str(frames_dir / "f_%04d.png")
        if (
            _run_quiet(
                ["ffmpeg", "-y", "-i", str(mp4), "-vf", f"fps={fps},scale={scale}:-2", pattern]
            )
            != 0
        ):
            return []
        frames = sorted(frames_dir.glob("f_*.png"))
        if not frames:
            return []
        return montage_frames(frames, out_png, tile=tile)
    finally:
        shutil.rmtree(frames_dir, ignore_errors=True)
