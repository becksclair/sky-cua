#!/usr/bin/env python3
"""Preauthorize screenshot portal permissions for testing VM smokes.

This seeds the xdg-desktop-portal PermissionStore with a 'yes' entry for
the screenshot table, preventing the interactive approval dialog from blocking
automated tests.
"""

from __future__ import annotations

import sys
from collections.abc import Mapping, Sequence
from typing import Any

PERMISSION_STORE_BUS_NAME = "org.freedesktop.impl.portal.PermissionStore"
PERMISSION_STORE_OBJECT_PATH = "/org/freedesktop/impl/portal/PermissionStore"
PERMISSION_STORE_INTERFACE = "org.freedesktop.impl.portal.PermissionStore"
SCREENSHOT_TABLE = "screenshot"
SCREENSHOT_ID = "screenshot"
ALLOW_PERMISSION = "yes"
GENERIC_APP_IDS = ("", "desktop")


def load_gi() -> tuple[Any, Any]:
    try:
        import gi  # type: ignore[import-not-found]

        gi.require_version("Gio", "2.0")
        from gi.repository import Gio, GLib  # type: ignore[import-not-found]
    except ImportError as error:
        raise RuntimeError(
            "GObject introspection not available; cannot preauthorize portal"
        ) from error
    return Gio, GLib


def screenshot_permissions(app_ids: Sequence[str] = GENERIC_APP_IDS) -> dict[str, list[str]]:
    return {app_id: [ALLOW_PERMISSION] for app_id in app_ids}


def missing_screenshot_permissions(
    permissions: Mapping[str, Sequence[str]],
    app_ids: Sequence[str] = GENERIC_APP_IDS,
) -> list[str]:
    return [app_id for app_id in app_ids if ALLOW_PERMISSION not in permissions.get(app_id, [])]


def dbus_proxy(bus_name: str, object_path: str, interface: str) -> Any:
    gio, _glib = load_gi()
    return gio.DBusProxy.new_for_bus_sync(
        gio.BusType.SESSION,
        gio.DBusProxyFlags.NONE,
        None,
        bus_name,
        object_path,
        interface,
        None,
    )


def preauthorize_screenshot(*, app_ids: Sequence[str] = GENERIC_APP_IDS) -> None:
    gio, glib = load_gi()
    proxy = dbus_proxy(
        PERMISSION_STORE_BUS_NAME,
        PERMISSION_STORE_OBJECT_PATH,
        PERMISSION_STORE_INTERFACE,
    )
    proxy.call_sync(
        "Set",
        glib.Variant(
            "(sbsa{sas}v)",
            (
                SCREENSHOT_TABLE,
                True,
                SCREENSHOT_ID,
                screenshot_permissions(app_ids),
                glib.Variant("s", ""),
            ),
        ),
        gio.DBusCallFlags.NONE,
        -1,
        None,
    )
    result = proxy.call_sync(
        "Lookup",
        glib.Variant("(ss)", (SCREENSHOT_TABLE, SCREENSHOT_ID)),
        gio.DBusCallFlags.NONE,
        -1,
        None,
    )
    permissions, _data = result.unpack()
    missing = missing_screenshot_permissions(permissions, app_ids)
    if missing:
        raise RuntimeError(
            f"screenshot portal permission round-trip missing app_ids={missing!r}: {permissions!r}"
        )


def main() -> int:
    try:
        preauthorize_screenshot()
        print("preauthorized screenshot portal permissions")
        return 0
    except Exception as error:
        print(f"failed to preauthorize screenshot portal: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
