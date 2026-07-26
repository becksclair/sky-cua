# Checkout install details

Load this reference only for a checkout install or its projected integrations.

Run:

```bash
python3 install.py install
```

From a checkout, this command builds or refreshes the durable distribution
outputs before installing. Do not run `scripts/deploy_plugin.py`, manually
prebuild Browser Use, or run a separate skill-sync step.

The fixed install root is:

```text
${XDG_DATA_HOME:-~/.local/share}/sky-cua
```

Expected projections include:

- stable launchers under `~/.local/bin` for the sky-cua client, service,
  overlay host, `node_repl`, and Chrome native host;
- `computer-use`, `browser-use`, and `phone-use` skills in detected normal
  agent skill roots;
- Chrome, Chromium, and Brave native messaging manifests;
- the two-plugin `openai-bundled` Codex marketplace in the fixed tree;
- detected host configuration, including global OpenClaw `node_repl` using the
  private bundled Node runtime without replacing the user's global `node`, and
  both fixed-root MCP servers in an existing Hermes Agent configuration.

OpenClaw owns per-agent Codex plugin installation before thread start. A live
OpenClaw acceptance should show `computer-use.doctor`/`observe` and
`node_repl.js`, the requested model with no fallback, and Browser Use as
`extension_native_host` with `isIab=false`.

A live Hermes acceptance requires both `hermes mcp test` probes plus
`scripts/live_hermes_mcp_smoke.py`, whose model turn must invoke
`mcp__sky_cua__status` and `mcp__node_repl__js` and produce nonce-bound Node
REPL evidence.
