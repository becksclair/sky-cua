# Hermes Agent integration

## Status

Shipped on Linux. Last verified: 2026-08-01 with Hermes Agent 0.19.0.

## Summary

Sky CUA installs `sky_cua` and `node_repl` as native stdio MCP servers for
NousResearch Hermes Agent. Both the targeted MCP installer and the standalone
fixed-root installer preserve unrelated Hermes configuration and skills.

## Contract surface

Hermes configuration lives at `${HERMES_HOME:-~/.hermes}/config.yaml` under
the top-level `mcp_servers` map. Sky CUA manages exactly these entries:

- `sky_cua` launches `<install-root>/bin/sky-cua-client mcp`.
- `node_repl` launches `<install-root>/bin/node_repl`.

The installer also manages one marker-delimited Node REPL guidance block in
`${HERMES_HOME}/AGENTS.md`, adapted to Hermes's `mcp__node_repl__*` tool names
and runtime sandbox contract. Existing instructions outside that block are
preserved.

Setup also converges Hermes to no-prompt operation:

- `approvals.mode: "off"`;
- `approvals.mcp_reload_confirm: false` and
  `approvals.destructive_slash_confirm: false`;
- `memory.write_approval: false` and `skills.write_approval: false`;
- `delegation.subagent_auto_approve: true`;
- `hooks_auto_accept: true`.

Existing `approvals.deny` patterns are removed because Hermes evaluates them
before the no-prompt bypass. Leaving them in place would contradict the
installer's always-granted permission contract.

Hermes registers their tools as `mcp__sky_cua__<tool>` and
`mcp__node_repl__<tool>` in the currently supported runtime. That tool-name
spelling is Hermes-owned and may change independently of Sky CUA.

## Behavior

`python3 install.py install` detects an existing Hermes configuration and
projects both fixed-root servers. A targeted development deployment uses:

```bash
python3 scripts/install_mcp_server.py \
  --target-dir ~/.local/share/sky-cua \
  --host hermes \
  --restart-runtime
```

The adapter replaces only `sky_cua` and `node_repl`, preserves all unrelated
YAML text and MCP entries, converges the no-prompt policy above, writes
atomically, and creates a content-addressed
backup under `${HERMES_HOME}/.sky-cua-backups` before changing an existing
file. The same backup and marker discipline applies to `AGENTS.md`. Repeated
deployment is byte-idempotent. Sky CUA's three model-facing skills are
projected into `${HERMES_HOME}/skills` by the targeted installer.

## Source paths

- `scripts/_hermes_config.py`
- `scripts/install_mcp_server.py`
- `scripts/standalone_release.py`
- `scripts/live_hermes_mcp_smoke.py`
- `scripts/test_hermes_config.py`

## Verification

Deterministic config and install coverage includes idempotent no-prompt policy
convergence and removal of explicit denial patterns:

```bash
uv run pytest scripts/test_hermes_config.py \
  scripts/test_install_flows.py \
  scripts/test_standalone_release.py
```

Live acceptance artifacts are written to
`artifacts/live-hermes-mcp-smoke/`:

```bash
python3 scripts/live_hermes_mcp_smoke.py
```

The live gate requires both `hermes mcp test` probes and a real model turn
that invokes `mcp__sky_cua__status` plus `mcp__node_repl__js`. Node REPL must
write nonce-bound evidence, and Hermes must return the generated invocation ID.

## Known limitations

- Automatic standalone projection requires an existing Hermes `config.yaml`;
  explicit `--host hermes` deployment may create one.
- Live validation consumes one configured Hermes model turn.
- Hermes profiles are supported through `HERMES_HOME`; profile selection is
  owned by Hermes.
- Hermes 0.19's gateway requires an enabled messaging platform to receive a
  gateway-delivered model turn. The acceptance harness uses Hermes's supported
  direct `chat` path with the same profile and MCP configuration; gateway
  restart/status are validated separately when no messaging platform is enabled.

## Related

- [Complete CUA stack ownership](complete-cua-stack-ownership.md)
- `ROADMAP.md`
