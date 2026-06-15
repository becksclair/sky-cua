#!/usr/bin/env python3
"""Headless post-install assertions for the sky-cua release package.

Runs inside the validation container after `install.py`. Proves the install
wired sky-cua up correctly without a desktop session:

- the installed `sky-cua-client doctor` can be executed;
- the Codex config enables a computer-use plugin (compat id on Linux, else the
  `sky-cua@local` channel fallback);
- the OpenCode host config registers the sky-cua MCP server;
- when a desktop session is available, a real MCP stdio handshake
  (`initialize` + `tools/list`) lists the desktop tool surface.

The handshake is informational in a bare container because the runtime service
needs a desktop session. Live desktop control and hard MCP tool-list proof stay
on the GUI VM smokes.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import NoReturn

HANDSHAKE_TIMEOUT = 20.0


TARGET_DIR = Path(os.environ.get("SKY_CUA_TARGET_DIR", "/root/.local/share/sky-cua"))
CODEX_HOME = Path(os.environ.get("CODEX_HOME", "/root/.codex"))
CLIENT = TARGET_DIR / "bin" / "sky-cua-client"
# A stable desktop tool name the MCP server must advertise (see the compat
# contract's stable-names list).
EXPECTED_TOOL = "get_app_state"
PACKAGE_ROOT = Path(os.environ["SKY_CUA_PACKAGE_ROOT"])
PACKAGED_PLUGIN_ROOT = PACKAGE_ROOT / "plugin" / "sky-cua"
sys.path.insert(0, str(PACKAGE_ROOT / "scripts"))

from _mcp_stdio import McpClient  # noqa: E402


def fail(message: str) -> NoReturn:
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)


def check_doctor() -> None:
    # The binary must exist and be runnable. `doctor` deliberately exits non-zero
    # when the desktop backend is not ready, which is expected headlessly - so we
    # only require that it runs, and report (not gate on) its exit code. The MCP
    # handshake below is best-effort in headless containers.
    if not CLIENT.exists():
        fail(f"installed client not found at {CLIENT}")
    try:
        result = subprocess.run([str(CLIENT), "doctor"], text=True, check=False)
    except OSError as error:
        fail(f"could not execute {CLIENT}: {error}")
    print(
        f"ok: sky-cua-client runs (doctor exit {result.returncode}; non-zero is expected headless)"
    )


def check_codex_config() -> None:
    config_path = CODEX_HOME / "config.toml"
    if not config_path.exists():
        fail(f"codex config not written at {config_path}")
    plugins = tomllib.loads(config_path.read_text(encoding="utf-8")).get("plugins", {})
    compat = plugins.get("computer-use@openai-bundled", {}).get("enabled") is True
    local = plugins.get("sky-cua@local", {}).get("enabled") is True
    if not (compat or local):
        fail(f"no computer-use plugin enabled in {config_path}: {plugins}")
    print(f"ok: codex config ({'compat plugin' if compat else 'sky-cua@local fallback'})")


def check_opencode_config() -> None:
    config_path = TARGET_DIR / "opencode.json"
    if not config_path.exists():
        fail(f"opencode config not written at {config_path}")
    text = config_path.read_text(encoding="utf-8")
    config = json.loads(text)
    servers = config.get("mcp") or config.get("mcpServers") or {}
    if not any("sky" in key for key in servers) and "sky-cua-client" not in text:
        fail(f"opencode config does not register the sky-cua server: {text}")
    print("ok: opencode config registers sky-cua")


def check_mcp_handshake() -> None:
    client = McpClient(
        [str(CLIENT), "mcp"],
        cwd=PACKAGE_ROOT,
        extra_env={"SKY_CUA_REPO_ROOT": str(PACKAGED_PLUGIN_ROOT)},
        read_timeout=HANDSHAKE_TIMEOUT,
        client_name="sky-cua-validate",
        client_version="0",
    )
    try:
        client.initialize()
        tool_list = client.tools_list()
    except RuntimeError as exc:
        # The runtime spawns a desktop service that needs a session (Wayland/X11,
        # logind, system bus). That is absent in a bare container, so the live
        # handshake is informational here - the full live-MCP check belongs on
        # the GUI VM. Install/config is proven above.
        print(f"skip: MCP handshake needs a desktop session ({exc})")
        return
    finally:
        client.close()

    try:
        names = {tool.get("name") for tool in tool_list if isinstance(tool, dict)}
    except AttributeError as exc:
        fail(f"tools/list returned invalid tool entries: {exc}")
    if EXPECTED_TOOL not in names:
        fail(f"tools/list missing {EXPECTED_TOOL!r}; got {sorted(n for n in names if n)}")
    print(f"ok: MCP handshake ({len(names)} tools, including {EXPECTED_TOOL})")


def main() -> int:
    check_doctor()
    check_codex_config()
    check_opencode_config()
    check_mcp_handshake()
    print("\nAll install validations passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
