"""Contract between the installer's desktop entry and the daemon's portal app id."""

from __future__ import annotations

import re
from pathlib import Path

import _portal_desktop_entry

REPO_ROOT = Path(__file__).resolve().parents[1]
HOST_APP_RS = REPO_ROOT / "crates/sky-cua-linux/src/portal/host_app.rs"


def test_desktop_entry_id_matches_registered_portal_app_id() -> None:
    source = HOST_APP_RS.read_text(encoding="utf-8")
    match = re.search(r'pub\(crate\) const PORTAL_APP_ID: &str = "([^"]+)";', source)
    assert match is not None, "PORTAL_APP_ID constant not found in host_app.rs"
    assert match.group(1) == _portal_desktop_entry.PORTAL_APP_ID


def test_applications_dir_honours_xdg_data_home(tmp_path: Path) -> None:
    home = tmp_path / "home"
    assert _portal_desktop_entry.applications_dir(home, {}) == home / ".local/share/applications"
    assert _portal_desktop_entry.applications_dir(
        home, {"XDG_DATA_HOME": str(tmp_path / "data")}
    ) == (tmp_path / "data/applications")
