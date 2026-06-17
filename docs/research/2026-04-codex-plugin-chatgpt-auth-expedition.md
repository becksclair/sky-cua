# Installed-plugin ChatGPT-auth E2E expedition

This document records the investigation that got the installed `sky-cua`
plugin working under real **ChatGPT-auth Codex** runs on Asgard.

It exists so we do **not** have to rediscover the same ugly seams again with
`strace`, Ghostty detours, and increasingly rude language.

## Scope

This is specifically about **installed-plugin E2E through Codex**:

- local plugin bundle installed as `sky-cua@debug`
- ChatGPT-auth Codex runtime
- proving that Codex actually called the installed `computer-use` MCP server
- proving that desktop interaction succeeded through the plugin

This is **not** the same thing as backend proof. Backend proof can be done with:

- direct MCP stdio against `sky-cua-client mcp`
- the live desktop smoke harnesses
- the historical X11/XWayland smoke evidence
- the Kate / Krita workflow smokes

Those remain important, but they do not prove the installed-plugin Codex lane.

## Final working recipe

For local-plugin E2E on this machine, the working setup is:

1. **ChatGPT auth only**
   - copy `~/.codex/auth.json` into a dedicated test `CODEX_HOME`
2. **plugins enabled**
   - keep `[features] plugins = true`
3. **apps disabled**
   - force `[features] apps = false`
4. **run `codex exec` with the explicit bypass flag**
   - `--dangerously-bypass-approvals-and-sandbox`

That exact combination is what the current harnesses now use:

- `scripts/live_codex_exec_smoke.py`

The lightweight installed-plugin smoke is now live-proven with that recipe:

- `artifacts/codex-e2e/plugin-smoke/20260423T161421Z/last-message.json`

That said, `codex exec` is no longer the acceptance harness after this expedition. It stays useful as a cheap diagnostic probe. The 2026-04 installed-plugin acceptance lane used `scripts/live_app_server_smoke.py`; current acceptance has since moved to `scripts/live_agentic_loop_smoke.py`, which drives the installed MCP server through an external agent CLI.

## What we had to prove

### 1) The plugin itself was not dead

The first useful thing was separating plugin health from Codex runtime weirdness.

These all proved the plugin bundle and MCP server were alive:

- `.mcp.json` named the server `computer-use`
- direct stdio probe against the installed bundle advertised the expected tools
- explicit `codex app-server` could bring `computer-use` from `starting` to `ready`

Useful artifacts:

- `artifacts/app-server-probe/direct-desktop-v13/summary.json`
- direct stdio probe of:
  - `~/.codex/plugins/cache/debug/sky-cua/local/bin/sky-cua-client mcp`

That killed the old “plugin startup timeout” story.

### 2) Tool visibility was getting masked by the hosted `apps` lane

Under ChatGPT auth, a live detached run with `features.apps = true` kept surfacing
the host harness tool world instead of the local plugin tools.

The bad signatures were things like:

- `web.run`
- `functions.exec_command`
- `tool_search.tool_search_tool`
- `multi_tool_use.parallel`

When that happened, no remote-control prompt appeared because the turn never got
the local `computer-use` namespace at all.

The important proof was that this was **not** just prompt wording or shell-env dirt.

With a dedicated ChatGPT-auth test `CODEX_HOME` and:

- `plugins = true`
- `apps = false`

the live tool list exposed the correct local plugin tools again:

- `mcp__computer_use__.list_apps`
- `mcp__computer_use__.get_app_state`
- `mcp__computer_use__.click`
- and the rest

Useful artifact:

- `artifacts/codex-exec-probe/live-chatgpt-noapps.last.json`

### 3) Plugin mention syntax was not the baseline problem

We also proved that explicit plugin mention was **not** the main missing seam.

Under a sane non-`/tmp` `CODEX_HOME`, both mock-backed `codex exec` and
`codex app-server` request assembly already included the `mcp__computer_use__`
namespace once the plugin was loaded.

Useful artifacts:

- `artifacts/app-server-probe/direct-desktop-v24-home-probe-plugin-mention/responses-request.json`
- `artifacts/codex-exec-probe/without_plugin_mention/responses-request.json`
- `artifacts/codex-exec-probe/with_plugin_mention/responses-request.json`

That killed the “maybe we just need to mention the plugin harder” theory.

### 4) `/tmp` `CODEX_HOME` was a lying little trap

Using a temporary Codex home under `/tmp` produced misleading plugin-loading
results. Helper/bin setup could be refused there, and `codex mcp list --json`
could miss the local plugin even when the bundle files existed.

Practical rule:

- do **not** root Codex plugin probes under `/tmp`
- use a repo- or home-rooted `CODEX_HOME`

### 5) The real blocker after tool visibility was MCP approval handling in exec mode

Once the no-apps ChatGPT home exposed the real `computer-use` tools, the next
failure changed shape:

- Codex now issued real `computer-use` MCP calls
- but they came back as:
  - `user cancelled MCP tool call`

That turned out to be a **Codex exec runtime** issue, not a plugin issue.

Source review in `../sky/codex/codex-rs` showed:

- `computer-use` is a **generic plugin MCP server**, not a `codex_apps` connector
- so per-app approval knobs under `apps.*` do **not** apply to it
- `codex exec` cannot satisfy the interactive approval paths for MCP tool calls:
  - MCP elicitation gets auto-cancelled
  - `request_user_input` is not supported in exec mode

In other words: exec was cancelling its own approval workflow.

### 6) The bypass flag is the clean practical answer for exec-based desktop E2E

The working fix was:

- `codex exec --dangerously-bypass-approvals-and-sandbox`

Why that matters:

- `codex exec` already defaults to `approval_policy = never` in headless mode
- but generic MCP approval handling still does not fully stand down unless the
  turn is also in `DangerFullAccess`

So the bypass flag is what makes the generic MCP approval path stop prompting
and stop auto-cancelling the tool call.

This was proved live first with a one-off probe:

- `artifacts/codex-e2e/plugin-smoke-bypass-probe/20260423T161318Z/last-message.json`

and then with the actual packaged smoke harness:

- `artifacts/codex-e2e/plugin-smoke/20260423T161421Z/last-message.json`

That second artifact is the one that matters most. It proves the real harness:

- saw the zenity dialog
- read the dialog text through `computer-use`
- dismissed it through `computer-use`
- re-checked state and confirmed the dialog was gone

## Dead ends we killed

These were plausible, but wrong or incomplete:

- **“The plugin isn’t starting.”**
  - wrong once explicit app-server startup was fixed and `computer-use` hit `ready`
- **“Maybe `.mcp.json` server naming is wrong.”**
  - wrong; `computer-use` naming was already correct
- **“Maybe Ghostty or a scrubbed shell will detach enough.”**
  - wrong on its own; the host tool surface contamination survived that
- **“Maybe plugin mention syntax is the whole issue.”**
  - wrong; local request assembly already carried `mcp__computer_use__`
- **“Maybe ChatGPT auth just can’t do local-plugin E2E.”**
  - too broad and therefore wrong; the real issues were:
    - `features.apps = true` masking the local plugin tool surface
    - exec-mode MCP approval cancellation

## Practical rules going forward

If you need installed-plugin E2E here, use this checklist:

### Always

- use **ChatGPT-auth**
- use a dedicated test `CODEX_HOME`
- keep the plugin installed into that dedicated home
- force:
  - `[features] plugins = true`
  - `[features] apps = false`
- run the E2E harness with:
  - `--dangerously-bypass-approvals-and-sandbox`

### Never

- do not use `/tmp` as the root for `CODEX_HOME`
- do not trust the model’s final JSON alone
- do not accept a “successful” run unless the JSONL transcript proves real
  `computer-use` MCP calls happened
- do not blame plugin mention syntax first
- do not assume backend smokes prove installed-plugin Codex E2E

## Official docs vs what we had to discover ourselves

The official Codex docs were useful for plugin shape and install expectations:

- plugins are the installable bundle unit
- skills belong inside the plugin
- local plugins are documented as marketplace-installed bundles

But the docs did **not** spell out the machine-specific runtime seams we had to prove:

- ChatGPT-auth + `features.apps = true` masking local plugin tools
- generic plugin MCP approvals not using app connector approval config
- exec-mode MCP approval cancellation
- the need for `--dangerously-bypass-approvals-and-sandbox` for exec-based
  desktop E2E on this setup

Those had to be proved from:

- local source review in `codex-rs`
- runtime artifacts
- live app-server / exec probes
- transcript validation

## Current status after the expedition

What is now proven:

- installed plugin discovery works
- installed plugin stdio server is healthy
- explicit app-server startup works
- local request assembly includes `mcp__computer_use__`
- ChatGPT-auth live turns can expose the local plugin tools when `apps = false`
- exec-based installed-plugin desktop smoke now passes live when run with the
  dedicated no-apps home plus `--dangerously-bypass-approvals-and-sandbox`
- a minimal rich-client `codex app-server` harness was retired; agent-loop
  acceptance now uses `scripts/live_agentic_loop_smoke.py`.

## 2026-05-13 Codex Desktop Bundled-Plugin Addendum

The newer Codex Desktop compatibility lane is adjacent to this expedition but
not the same proof target. The installed-plugin E2E above proves `sky-cua` as a
local plugin bundle. The bundled-plugin lane makes the same runtime present as
Codex Desktop's expected `computer-use` plugin while keeping Browser Use and
Chrome visible as companions.

Current bundled-plugin truth:

- `scripts/build_plugin.py` can stage OpenAI-bundled `chrome` and `browser-use`
  resources and embeds `sky-cua-chrome-host` under the Chrome plugin's Linux
  extension-host path.
- `resources/chrome_preflight.py` syncs `chrome`, `browser-use`, and a
  `computer-use` compatibility plugin into the local Codex bundled marketplace
  cache.
- `resources/chrome_preflight.py` enables
  `chrome@openai-bundled` and `browser-use@openai-bundled`. It stages
  `computer-use@openai-bundled` disabled so it is available for marketplace
  completeness without colliding with the active `sky-cua` MCP server.
- The companion CodexDesktop-Rebuild patch is intentionally narrow: it enables
  Linux `computerUse`, removes external Browser Use build-flavor pruning, and
  keeps Browser Use/Chrome descriptors available when `computerUse` is true.

Proof artifacts:

- `artifacts/desktop-ui-proof/20260513T100515Z-browser-settings-patched-profile/renderer-settings-computer-use-patched.png`
- `artifacts/desktop-ui-proof/20260513T100515Z-browser-settings-patched-profile/renderer-settings-browser-patched.png`
- `artifacts/desktop-ui-proof/20260513T100515Z-browser-settings-patched-profile/codexdesktop-dev.log`

The Settings proof shows `Computer Use` and `Browser Use` in Codex Desktop with
the Chrome companion plugin present. This does not replace the ChatGPT-auth
installed-plugin E2E recipe above; it proves a different adapter path.

The former TIDAL rich-client workflow remains useful as historical proof for
fallback-only screenshot interaction, but its live harness has been retired.
Current installed-plugin acceptance should use `scripts/live_agentic_loop_smoke.py`
and other active `live_*_smoke.py` entrypoints.

## 2026-05-17 Detached Session-Env Addendum

Codex and other MCP hosts can launch `computer-use` without the full Linux
desktop session environment. The runtime now repairs that detached state in both
the MCP client and Linux service instead of relying only on host-side env
forwarding.

Current session-env truth:

- `sky-cua-client` normalizes `PATH` and forwards reconstructed desktop
  variables before spawning `sky-cua-service`.
- `sky-cua-service` rehydrates the Linux backend from the process tree,
  systemd user manager, `/run/user/<uid>`, and the runtime bus path before
  probing desktop backends.
- `doctor` exposes a structured `session_env` report, and MCP text/diagnostics
  surface `SessionEnvRepaired` when repair changed the runtime.
- The bundled workflow skill tells agents to treat `SessionEnvRepaired` as
  recovered context and continue normal desktop verification.

Proof artifacts:

- Direct MCP: `artifacts/session-env-smoke/20260517T080206Z`
- Rich app-server: `artifacts/codex-e2e/app-server-session-env-smoke/20260517T060242Z`
- Codex exec: `artifacts/codex-e2e/codex-session-env-smoke/20260517T060439Z`

The original `scripts/live_session_env_smoke.py` failure was not a runtime
failure. The harness killed the `zenity` dialog immediately after sending Enter
because it polled once and terminated if the process had not exited yet. The
shared helper now waits for dialog submission before cleanup. The live smokes
prove both the repair signal and the actual submitted value.

## Related files

- `scripts/_plugin_bundle.py`
- `scripts/_codex_exec.py`
- `scripts/live_codex_exec_smoke.py`
- `scripts/live_session_env_smoke.py`
- `scripts/live_codex_exec_session_env_smoke.py`
- `scripts/live_app_server_session_env_smoke.py`
- `NOTES.md`
