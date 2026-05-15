#!/usr/bin/env python3
"""Preauthorize KDE RemoteDesktop portal state for testing VM smokes."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from pathlib import Path

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

PERMISSION_STORE_BUS_NAME = "org.freedesktop.impl.portal.PermissionStore"
PERMISSION_STORE_OBJECT_PATH = "/org/freedesktop/impl/portal/PermissionStore"
PERMISSION_STORE_INTERFACE = "org.freedesktop.impl.portal.PermissionStore"
KDE_AUTHORIZED_TABLE = "kde-authorized"
REMOTE_DESKTOP_ID = "remote-desktop"
ALLOW_PERMISSION = "yes"
DEFAULT_TOKEN_PATH = Path.home() / ".local/state/sky-cua/portal-tokens.json"
DEFAULT_APP_IDS = ("", "desktop")


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Seed KDE's xdg PermissionStore RemoteDesktop authorization and clear "
            "any cached sky-cua restore token from another portal backend."
        )
    )
    parser.add_argument(
        "--app-id",
        action="append",
        dest="app_ids",
        default=None,
        help=(
            "Portal app id to grant. Repeat to grant multiple ids. Defaults to '' and 'desktop'."
        ),
    )
    parser.add_argument(
        "--token-path",
        type=Path,
        default=DEFAULT_TOKEN_PATH,
        help="sky-cua portal token JSON path to clear before relying on KDE authorization.",
    )
    parser.add_argument(
        "--keep-token",
        action="store_true",
        help="Keep the existing sky-cua portal token file.",
    )
    parser.add_argument(
        "--print-json",
        action="store_true",
        help="Print machine-readable details instead of a short human summary.",
    )
    args = parser.parse_args()

    try:
        app_ids = tuple(args.app_ids or DEFAULT_APP_IDS)
        token_removed = False if args.keep_token else clear_token_file(args.token_path)
        seed_permission_store(app_ids=app_ids)
        permissions = lookup_permissions()
        missing_app_ids = [
            app_id for app_id in app_ids if ALLOW_PERMISSION not in permissions.get(app_id, [])
        ]
        if missing_app_ids:
            raise RuntimeError(
                f"KDE authorization did not round-trip for app_ids={missing_app_ids!r}: "
                f"{permissions!r}"
            )
    except Exception as error:
        print(f"failed to preauthorize KDE RemoteDesktop portal state: {error}", file=sys.stderr)
        return 1

    details = {
        "app_ids": list(app_ids),
        "permission": ALLOW_PERMISSION,
        "remote_desktop_id": REMOTE_DESKTOP_ID,
        "table": KDE_AUTHORIZED_TABLE,
        "token_path": str(args.token_path),
        "token_removed": token_removed,
    }
    if args.print_json:
        print(json.dumps(details, sort_keys=True))
    else:
        print(f"preauthorized KDE RemoteDesktop portal permission for app_ids={app_ids!r}")
    return 0


def dbus_proxy() -> Gio.DBusProxy:
    return Gio.DBusProxy.new_for_bus_sync(
        Gio.BusType.SESSION,
        Gio.DBusProxyFlags.NONE,
        None,
        PERMISSION_STORE_BUS_NAME,
        PERMISSION_STORE_OBJECT_PATH,
        PERMISSION_STORE_INTERFACE,
        None,
    )


def clear_token_file(token_path: Path) -> bool:
    try:
        token_path.unlink()
    except FileNotFoundError:
        return False
    return True


def seed_permission_store(*, app_ids: Sequence[str]) -> None:
    proxy = dbus_proxy()
    for app_id in app_ids:
        proxy.call_sync(
            "SetPermission",
            GLib.Variant(
                "(sbssas)",
                (KDE_AUTHORIZED_TABLE, True, REMOTE_DESKTOP_ID, app_id, [ALLOW_PERMISSION]),
            ),
            Gio.DBusCallFlags.NONE,
            -1,
            None,
        )


def lookup_permissions() -> dict[str, list[str]]:
    result = dbus_proxy().call_sync(
        "Lookup",
        GLib.Variant("(ss)", (KDE_AUTHORIZED_TABLE, REMOTE_DESKTOP_ID)),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    )
    permissions, _data = result.unpack()
    return {str(app_id): [str(item) for item in values] for app_id, values in permissions.items()}


if __name__ == "__main__":
    raise SystemExit(main())
