"""Installer drop-in that exports ACCESSIBILITY_ENABLED through environment.d."""

from __future__ import annotations

import re
from pathlib import Path

import _session_accessibility_env

REPO_ROOT = Path(__file__).resolve().parents[1]
DESKTOP_BACKEND_RS = REPO_ROOT / "crates/sky-cua-linux/src/backend/desktop_backend.rs"


def _assignments(text: str) -> dict[str, str]:
    pairs: dict[str, str] = {}
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        key, separator, value = stripped.partition("=")
        assert separator, f"environment.d line is not KEY=VALUE: {line!r}"
        pairs[key] = value
    return pairs


def test_drop_in_sets_only_accessibility_enabled() -> None:
    assert _assignments(_session_accessibility_env.DROP_IN_TEXT) == {"ACCESSIBILITY_ENABLED": "1"}


def test_drop_in_name_sorts_as_a_conf_file() -> None:
    # systemd reads only *.conf from environment.d, in lexical order.
    assert re.fullmatch(r"\d\d-[a-z0-9-]+\.conf", _session_accessibility_env.DROP_IN_NAME)


def test_environment_d_dir_honours_xdg_config_home(tmp_path: Path) -> None:
    home = tmp_path / "home"
    assert _session_accessibility_env.environment_d_dir(home, {}) == home / ".config/environment.d"
    assert _session_accessibility_env.environment_d_dir(
        home, {"XDG_CONFIG_HOME": str(tmp_path / "config")}
    ) == (tmp_path / "config/environment.d")


def test_install_writes_the_default_drop_in(tmp_path: Path) -> None:
    home = tmp_path / "home"
    path = _session_accessibility_env.install_session_accessibility_env(home, {})
    assert path == home / ".config/environment.d/60-sky-cua-accessibility.conf"
    assert path.read_text(encoding="utf-8") == _session_accessibility_env.DROP_IN_TEXT
    assert _session_accessibility_env.install_session_accessibility_env(home, {}) == path
    assert path.read_text(encoding="utf-8") == _session_accessibility_env.DROP_IN_TEXT


def test_install_keeps_a_user_opt_out(tmp_path: Path) -> None:
    home = tmp_path / "home"
    path = _session_accessibility_env.install_session_accessibility_env(home, {})
    path.write_text("ACCESSIBILITY_ENABLED=0\n", encoding="utf-8")
    assert _session_accessibility_env.install_session_accessibility_env(home, {}) == path
    assert path.read_text(encoding="utf-8") == "ACCESSIBILITY_ENABLED=0\n"


def test_drop_in_matches_the_desktop_launch_app_environment() -> None:
    # desktop_launch_app children get the same key and value from the daemon.
    source = DESKTOP_BACKEND_RS.read_text(encoding="utf-8")
    assert '.env("ACCESSIBILITY_ENABLED", "1")' in source
