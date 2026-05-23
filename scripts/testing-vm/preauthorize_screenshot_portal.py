#!/usr/bin/env python3
"""Preauthorize screenshot portal permissions for testing VM smokes.

This seeds the xdg-desktop-portal PermissionStore with a 'yes' entry for
the screenshot table, preventing the interactive approval dialog from blocking
automated tests.
"""

from __future__ import annotations

import sys

try:
    import gi

    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib
except ImportError:
    print("GObject introspection not available; cannot preauthorize portal", file=sys.stderr)
    sys.exit(1)

PERMISSION_STORE_BUS_NAME = "org.freedesktop.impl.portal.PermissionStore"
PERMISSION_STORE_OBJECT_PATH = "/org/freedesktop/impl/portal/PermissionStore"
PERMISSION_STORE_INTERFACE = "org.freedesktop.impl.portal.PermissionStore"
SCREENSHOT_TABLE = "screenshot"
SCREENSHOT_ID = "screenshot"


def dbus_proxy(bus_name: str, object_path: str, interface: str) -> Gio.DBusProxy:
    return Gio.DBusProxy.new_for_bus_sync(
        Gio.BusType.SESSION,
        Gio.DBusProxyFlags.NONE,
        None,
        bus_name,
        object_path,
        interface,
        None,
    )


def preauthorize_screenshot(*, app_id: str = "") -> None:
    proxy = dbus_proxy(
        PERMISSION_STORE_BUS_NAME,
        PERMISSION_STORE_OBJECT_PATH,
        PERMISSION_STORE_INTERFACE,
    )
    permissions = {app_id: ["yes"]}
    proxy.call_sync(
        "Set",
        GLib.Variant(
            "(sbsa{sas}v)",
            (SCREENSHOT_TABLE, True, SCREENSHOT_ID, permissions, GLib.Variant("s", "")),
        ),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    )
    proxy.call_sync(
        "Lookup",
        GLib.Variant("(ss)", (SCREENSHOT_TABLE, SCREENSHOT_ID)),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
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
