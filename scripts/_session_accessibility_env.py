"""systemd ``environment.d`` drop-in that turns on toolkit accessibility at login.

Chromium and Electron decide once, at startup, whether to register on the
AT-SPI bus (``ui/accessibility/platform/atk_util_auralinux.cc``,
``ShouldEnableAccessibility``): ``ACCESSIBILITY_ENABLED=1`` wins, otherwise
they read ``org.a11y.Status.IsEnabled``, which nothing sets after a KDE,
COSMIC, Hyprland or plain X11 login. The sky-cua daemon sets ``IsEnabled``
when it starts, but apps that autostart before the daemon never register.
Exporting the variable through the systemd user environment makes every app
the session launches register regardless of that ordering. Registration does
not build the tree; the daemon wakes a dormant Chromium tree on first observe
(crates/sky-cua-linux/src/atspi/wake.rs).

The drop-in applies from the next login, to sessions whose launcher inherits
the systemd user environment (KDE Plasma and GNOME under systemd; Hyprland
only through uwsm).
"""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

DROP_IN_NAME = "60-sky-cua-accessibility.conf"

DROP_IN_TEXT = (
    "# Installed by sky-cua. Makes Chromium/Electron (and other toolkits that\n"
    "# check it) register on the AT-SPI accessibility bus at startup, so the\n"
    "# sky-cua daemon can read their accessibility trees. Applies from the next\n"
    "# login. To opt out, set ACCESSIBILITY_ENABLED=0; sky-cua installs keep an\n"
    "# edited file and only rewrite this exact default.\n"
    "ACCESSIBILITY_ENABLED=1\n"
)


def environment_d_dir(home: Path, env: Mapping[str, str]) -> Path:
    config_home = env.get("XDG_CONFIG_HOME")
    root = Path(config_home) if config_home else home / ".config"
    return root / "environment.d"


def install_session_accessibility_env(home: Path, env: Mapping[str, str]) -> Path:
    """Write the drop-in unless the user has edited it; return its path."""
    path = environment_d_dir(home, env) / DROP_IN_NAME
    if path.is_file() and path.read_text(encoding="utf-8") != DROP_IN_TEXT:
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(DROP_IN_TEXT, encoding="utf-8")
    return path
