# Rare operations and freshness

Load only for KWin, AT-SPI, OpenClaw reload, stale-deploy checks, or live
smoke/device testing.

## KWin and accessibility

- Add `--kwin-effect` to deploy or install commands only when the KWin
  agent-cursor effect must be rebuilt/reloaded.
- `--refresh-accessibility` resets the user AT-SPI registry. It is opt-in and
  should be used only when the registry is genuinely wedged: the reset wipes
  every running app’s accessibility registration. Chromium re-registers
  lazily; GTK apps register eagerly and remain semantically dark until
  relaunched. sky-cua self-heals a wedged connection on reconnect.

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
python3 scripts/deploy_freshness.py
```

For a specific client, add `--client bin/sky-cua-client`. If stale, deploy
again before running the live test. Set `SKY_CUA_ALLOW_STALE_DEPLOY=1` only
when intentionally bypassing this gate, and report that bypass.

## Validation scope

The live desktop, portal, KDE, COSMIC, Hyprland, GNOME, VM, and device smoke
profiles are separate gates. State any that were not run; do not imply that a
local deploy or restart proved them.
