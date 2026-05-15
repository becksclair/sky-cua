#!/usr/bin/env python3
"""Preauthorize GNOME RemoteDesktop portal state for testing VM smokes."""

from __future__ import annotations

import argparse
import json
import os
import sys
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

PERMISSION_STORE_BUS_NAME = "org.freedesktop.impl.portal.PermissionStore"
PERMISSION_STORE_OBJECT_PATH = "/org/freedesktop/impl/portal/PermissionStore"
PERMISSION_STORE_INTERFACE = "org.freedesktop.impl.portal.PermissionStore"
DESKTOP_PORTAL_BUS_NAME = "org.freedesktop.portal.Desktop"
DESKTOP_PORTAL_OBJECT_PATH = "/org/freedesktop/portal/desktop"
MUTTER_DISPLAY_CONFIG_BUS_NAME = "org.gnome.Mutter.DisplayConfig"
MUTTER_DISPLAY_CONFIG_OBJECT_PATH = "/org/gnome/Mutter/DisplayConfig"
MUTTER_DISPLAY_CONFIG_INTERFACE = "org.gnome.Mutter.DisplayConfig"
REMOTE_DESKTOP_TABLE = "remote-desktop"
DEFAULT_TOKEN_PATH = Path.home() / ".local/state/sky-cua/portal-tokens.json"


@dataclass(frozen=True)
class MonitorIdentity:
    connector: str
    vendor: str
    product: str
    serial: str

    @property
    def match_string(self) -> str:
        if self.vendor == "unknown" and self.product == "unknown" and self.serial == "unknown":
            return self.connector
        return f"{self.vendor}:{self.product}:{self.serial}"


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Seed the xdg PermissionStore with GNOME RemoteDesktop restore data and "
            "write sky-cua's matching portal token file."
        )
    )
    parser.add_argument(
        "--app-id", default="", help="Portal app id to grant. Host tools usually use ''."
    )
    parser.add_argument(
        "--token-path",
        type=Path,
        default=DEFAULT_TOKEN_PATH,
        help="sky-cua portal token JSON path to write.",
    )
    parser.add_argument("--token", default="", help="Existing restore token UUID to reuse.")
    parser.add_argument(
        "--monitor-match",
        default="",
        help="GNOME monitor match string. Defaults to the primary Mutter monitor.",
    )
    parser.add_argument(
        "--device-types",
        type=int,
        default=3,
        help="RemoteDesktop device mask. 1=keyboard, 2=pointer, 3=both.",
    )
    parser.add_argument(
        "--clipboard",
        action="store_true",
        help="Mark clipboard enabled in the seeded restore data.",
    )
    parser.add_argument(
        "--print-json",
        action="store_true",
        help="Print machine-readable details instead of a short human summary.",
    )
    args = parser.parse_args()

    try:
        token = args.token or load_existing_token(args.token_path) or str(uuid.uuid4())
        uuid.UUID(token)
        monitor_match = args.monitor_match or primary_monitor_identity().match_string
        remote_desktop_version = portal_version("org.freedesktop.portal.RemoteDesktop")
        screencast_version = portal_version("org.freedesktop.portal.ScreenCast")
        seed_permission_store(
            token=token,
            app_id=args.app_id,
            monitor_match=monitor_match,
            device_types=args.device_types,
            clipboard_enabled=args.clipboard,
        )
        write_token_file(
            args.token_path,
            token=token,
            remote_desktop_version=remote_desktop_version,
            screencast_version=screencast_version,
        )
    except Exception as error:
        print(f"failed to preauthorize GNOME RemoteDesktop portal state: {error}", file=sys.stderr)
        return 1

    details = {
        "app_id": args.app_id,
        "monitor_match": monitor_match,
        "remote_desktop_version": remote_desktop_version,
        "screencast_version": screencast_version,
        "table": REMOTE_DESKTOP_TABLE,
        "token": token,
        "token_path": str(args.token_path),
    }
    if args.print_json:
        print(json.dumps(details, sort_keys=True))
    else:
        print(f"preauthorized GNOME RemoteDesktop portal token {token} for monitor {monitor_match}")
    return 0


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


def primary_monitor_identity() -> MonitorIdentity:
    proxy = dbus_proxy(
        MUTTER_DISPLAY_CONFIG_BUS_NAME,
        MUTTER_DISPLAY_CONFIG_OBJECT_PATH,
        MUTTER_DISPLAY_CONFIG_INTERFACE,
    )
    state = proxy.call_sync("GetCurrentState", None, Gio.DBusCallFlags.NONE, -1, None)
    _serial, monitors, logical_monitors, _properties = state.unpack()
    monitor_by_connector = {
        identity.connector: identity
        for identity in (monitor_identity(monitor) for monitor in monitors)
    }

    for logical_monitor in logical_monitors:
        _x, _y, _scale, _transform, primary, logical_monitor_specs, _logical_properties = (
            logical_monitor
        )
        if primary and logical_monitor_specs:
            connector = logical_monitor_specs[0][0]
            if connector in monitor_by_connector:
                return monitor_by_connector[connector]

    if monitors:
        return monitor_identity(monitors[0])

    raise RuntimeError("Mutter DisplayConfig did not report any monitors")


def monitor_identity(monitor: tuple[Any, ...]) -> MonitorIdentity:
    connector, vendor, product, serial = monitor[0]
    return MonitorIdentity(
        connector=str(connector),
        vendor=str(vendor),
        product=str(product),
        serial=str(serial),
    )


def portal_version(interface: str) -> int | None:
    try:
        proxy = dbus_proxy(
            DESKTOP_PORTAL_BUS_NAME,
            DESKTOP_PORTAL_OBJECT_PATH,
            "org.freedesktop.DBus.Properties",
        )
        value = proxy.call_sync(
            "Get",
            GLib.Variant("(ss)", (interface, "version")),
            Gio.DBusCallFlags.NONE,
            -1,
            None,
        )
        version = value.unpack()[0]
        if isinstance(version, GLib.Variant):
            version = version.unpack()
        return int(version)
    except Exception:
        return None


def load_existing_token(token_path: Path) -> str | None:
    try:
        token = json.loads(token_path.read_text()).get("restore_token")
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(token, str):
        return None
    try:
        uuid.UUID(token)
    except ValueError:
        return None
    return token


def seed_permission_store(
    *,
    token: str,
    app_id: str,
    monitor_match: str,
    device_types: int,
    clipboard_enabled: bool,
) -> None:
    now = GLib.get_real_time()
    impl_data = GLib.Variant(
        "(xxuba(uuv))",
        (
            now,
            now,
            device_types,
            clipboard_enabled,
            [(0, 1, GLib.Variant("s", monitor_match))],
        ),
    )
    restore_data = GLib.Variant("(suv)", ("GNOME", 1, impl_data))
    permissions = {app_id: ["yes"]}
    proxy = dbus_proxy(
        PERMISSION_STORE_BUS_NAME,
        PERMISSION_STORE_OBJECT_PATH,
        PERMISSION_STORE_INTERFACE,
    )
    proxy.call_sync(
        "Set",
        GLib.Variant(
            "(sbsa{sas}v)",
            (REMOTE_DESKTOP_TABLE, True, token, permissions, restore_data),
        ),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    )
    proxy.call_sync(
        "Lookup",
        GLib.Variant("(ss)", (REMOTE_DESKTOP_TABLE, token)),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    )


def write_token_file(
    token_path: Path,
    *,
    token: str,
    remote_desktop_version: int | None,
    screencast_version: int | None,
) -> None:
    token_path.parent.mkdir(parents=True, exist_ok=True)
    token_path.parent.chmod(0o700)
    record = {
        "restore_token": token,
        "updated_at": datetime.now(UTC).isoformat(timespec="microseconds").replace("+00:00", "Z"),
        "xdg_session_type": os.environ.get("XDG_SESSION_TYPE"),
        "compositor": os.environ.get("XDG_CURRENT_DESKTOP"),
        "remote_desktop_version": remote_desktop_version,
        "screencast_version": screencast_version,
    }
    token_path.write_text(json.dumps(record, indent=2) + "\n")
    token_path.chmod(0o600)


if __name__ == "__main__":
    raise SystemExit(main())
