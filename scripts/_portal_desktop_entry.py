"""Hidden desktop entry that gives the sky-cua daemon a stable portal app id.

The daemon registers its portal connection as ``PORTAL_APP_ID`` through
``org.freedesktop.host.portal.Registry`` (crates/sky-cua-linux/src/portal/
host_app.rs). xdg-desktop-portal only accepts an id it can resolve to a
``<id>.desktop`` entry, so the installer projects one. ``NoDisplay`` keeps it
out of menus; it must not be ``Hidden``, which makes the entry unresolvable.
"""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

PORTAL_APP_ID = "com.heliasar.SkyCua"


def applications_dir(home: Path, env: Mapping[str, str]) -> Path:
    data_home = env.get("XDG_DATA_HOME")
    root = Path(data_home) if data_home else home / ".local/share"
    return root / "applications"


def desktop_entry_text(service_launcher: Path) -> str:
    return (
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=sky-cua\n"
        "Comment=sky-cua desktop automation service (portal identity)\n"
        f"Exec={service_launcher} daemon\n"
        "NoDisplay=true\n"
        "Terminal=false\n"
    )


def install_portal_desktop_entry(
    home: Path, env: Mapping[str, str], service_launcher: Path
) -> Path:
    path = applications_dir(home, env) / f"{PORTAL_APP_ID}.desktop"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(desktop_entry_text(service_launcher), encoding="utf-8")
    return path
