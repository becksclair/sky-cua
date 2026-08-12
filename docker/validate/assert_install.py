#!/usr/bin/env python3
"""Headless fixed-root assertions for an extracted standalone sky-cua archive."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import NoReturn

TARGET_DIR = Path(os.environ.get("SKY_CUA_TARGET_DIR", "/root/.local/share/sky-cua"))
PACKAGE_ROOT = Path(os.environ["SKY_CUA_PACKAGE_ROOT"])
CLIENT = TARGET_DIR / "bin/sky-cua-client"


def fail(message: str) -> NoReturn:
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)


def check_shape() -> None:
    required = (
        "RELEASE.json",
        "bin/sky-cua-client",
        "bin/node_repl",
        "browser/browser-client.mjs",
        "browser/extension/manifest.json",
        "browser/native-host/sky-cua-chrome-host",
        "codex/openai-bundled/.agents/plugins/marketplace.json",
        "codex/openai-bundled/plugins/computer-use/.mcp.json",
        "codex/openai-bundled/plugins/browser-use/.mcp.json",
        "skills/computer-use/SKILL.md",
        "skills/browser-use/SKILL.md",
        "skills/phone-use/SKILL.md",
    )
    missing = [relative for relative in required if not (TARGET_DIR / relative).is_file()]
    if missing:
        fail(f"fixed install tree is incomplete: {missing}")
    forbidden = ("current", "releases", "activation-receipt.json", "promotion-journal.json")
    present = [name for name in forbidden if (TARGET_DIR / name).exists()]
    if present:
        fail(f"generation state remains in fixed install tree: {present}")
    release = json.loads((TARGET_DIR / "RELEASE.json").read_text(encoding="utf-8"))
    if release.get("target") != "linux-x64-glibc":
        fail(f"unexpected release target: {release}")
    print("ok: one complete fixed install tree")


def check_projections() -> None:
    launcher = Path("/root/.local/bin/sky-cua-client")
    if not launcher.is_symlink() or launcher.resolve() != CLIENT.resolve():
        fail(f"stable client launcher is incorrect: {launcher}")
    manifest_path = Path(
        "/root/.config/google-chrome/NativeMessagingHosts/com.openai.codexextension.json"
    )
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    expected_host = str(Path("/root/.local/bin/sky-cua-chrome-host"))
    if manifest.get("path") != expected_host:
        fail(f"native host path is not stable: {manifest}")
    for skill in ("computer-use", "browser-use", "phone-use"):
        projection = Path("/root/.agents/skills") / skill
        expected_link = Path(f"../../.local/share/sky-cua/skills/{skill}")
        if (
            not projection.is_symlink()
            or projection.readlink() != expected_link
            or not projection.resolve().is_relative_to(TARGET_DIR.resolve())
        ):
            fail(f"skill projection is incorrect: {projection}")
    print("ok: stable launchers, native host, and skills")


def check_doctor() -> None:
    try:
        result = subprocess.run([str(CLIENT), "doctor"], text=True, check=False)
    except OSError as error:
        fail(f"could not execute {CLIENT}: {error}")
    print(f"ok: sky-cua-client runs (doctor exit {result.returncode}; headless is allowed)")


def main() -> int:
    if PACKAGE_ROOT.name != "sky-cua-linux-x64-glibc":
        fail(f"unexpected extracted archive root: {PACKAGE_ROOT}")
    check_shape()
    check_projections()
    check_doctor()
    print("All standalone install validations passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
