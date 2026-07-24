# Rare operations and freshness

Load only for KWin, AT-SPI, OpenClaw reload, stale-deploy checks, or live
smoke/device testing.

## Desktop integration and accessibility

`install.py` has no KWin or accessibility flags. It projects detected supported
host integrations as part of the canonical install.

To build, install, and reload the KDE agent-cursor effect, use the selected MCP
host configuration:

```bash
python3 scripts/install_mcp_server.py --host <host> --kwin-effect
```

This refreshes that host's standalone MCP installation before the KWin effect
step.

Use the installed client for an explicit accessibility repair:

```bash
~/.local/bin/sky-cua-client setup-accessibility
```

Restart the affected graphical service or applications only when live evidence
shows AT-SPI is wedged. Chromium can re-register lazily; GTK applications may
need relaunching.

## OpenClaw

After an OpenClaw MCP configuration change, reload with:

```bash
openclaw mcp reload
```

This is a reload operation, not a full plugin deploy.

## Freshness before live tests

Live tests must use binaries built from current Rust sources. Check the local
client with:

```bash
python3 scripts/deploy_freshness.py --client ~/.local/share/sky-cua/bin/sky-cua-client
```

If stale, run `python3 install.py install` again before the live test. Do not
use a generation path as freshness truth.

## Validation scope

The live desktop, portal, KDE, COSMIC, Hyprland, GNOME, VM, and device smoke
profiles are separate gates. State any that were not run; do not imply that a
local deploy or restart proved them.
