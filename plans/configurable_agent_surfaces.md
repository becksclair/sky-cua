# Make desktop, browser, and phone surfaces independently configurable

## Outcome

Sky CUA exposes one canonical MCP API, but a machine can independently enable or disable the three agent-facing control surfaces: `desktop` (the `computer-use` skill), `browser`, and `phone`.

The primary durable policy lives in the existing machine config, `~/.config/sky-cua/sky-cua.toml` (or `SKY_CUA_CONFIG_PATH`):

```toml
[surfaces]
desktop = true
browser = false
phone = true
```

All three fields default to `true`, so an existing config with no `[surfaces]` table preserves today's behavior. A process-scoped `SKY_CUA_SURFACES` allowlist may override the `[surfaces]` table for temporary runtime use, for example `SKY_CUA_SURFACES=browser` or `SKY_CUA_SURFACES=desktop,phone`. Provisioning must ignore this transient environment override and use the durable machine config when deciding which skills to install or enable.

The existing phone master switch remains meaningful: `[phone].enabled = false` or `SKY_CUA_PHONE=0` also disables the phone MCP surface. Effective phone exposure therefore requires both the surface policy and the existing phone subsystem switch to be enabled. `[surfaces].phone = false` must also make the resolved phone subsystem disabled, so the durable surface policy does not leave phone-use running behind a hidden MCP facade.

A disabled surface disappears from the model-facing contract, not merely from dispatch:

- surface-specific tools are absent from `tools/list` and are not callable;
- shared tools (`status`, `list_resources`, `observe`, `capture_screen`) expose only enabled branches, enums, fields, constraints, and descriptions;
- `browser_eval` is present only when the browser surface and its existing eval gate are both enabled;
- direct calls to hidden surface-specific names return `UnknownTool`, while an invalid disabled branch of a still-advertised shared tool is rejected as `InvalidRequest` before service dispatch;
- the registry remains frozen at MCP `initialize`, so changing the config requires a fresh MCP process/session;
- `doctor` remains as the always-available cross-cutting diagnostic tool and should use a neutral description rather than claiming any particular enabled surface.

Examples that must work cleanly are `desktop+phone` with no browser surface, `browser` only, `desktop` only, `browser+phone`, and the current all-enabled default. If all three surfaces are disabled, `doctor` is the only advertised tool.

The distribution artifact continues to contain all three bundled skills. Host provisioning projects the durable surface policy onto the installed skill set:

- `desktop` -> `computer-use`
- `browser` -> `browser-use`
- `phone` -> `phone-use`

Re-provisioning must remove or disable stale Sky CUA-managed skills for surfaces that have been turned off, while preserving unrelated user skills. Codex should keep the complete plugin payload and enforce the projected skill set through its managed skill configuration; Claude Code, Pi, Hermes, and the shared `~/.agents/skills` projection can physically install/link only enabled skills and remove stale Sky CUA-owned disabled ones. The individual skill texts must no longer assume that either sibling surface is installed; cross-surface routing advice should be conditional on that surface being available.

This is not a return to the removed `legacy`/`compact` MCP profile system. There remains exactly one canonical tool API. Surface configuration only projects a subset of that canonical registry, preserving the same names, annotations, handlers, and semantics for every enabled branch.

## Work

1. Add the durable surface policy to `crates/sky-cua-platform/src/config.rs`.

   Add a typed `[surfaces]` table with optional `desktop`, `browser`, and `phone` booleans and a resolved `AgentSurfacePolicy`/equivalent whose defaults are all enabled. Add strict parsing for the optional `SKY_CUA_SURFACES` runtime allowlist. Unknown surface names and malformed values must be errors rather than silently enabling more capability. Keep unknown unrelated TOML keys forward-compatible as today.

   Resolve phone exposure together with the existing `[phone].enabled` / `SKY_CUA_PHONE` switch. The durable `[surfaces].phone = false` must force `ResolvedPhoneSelection.enabled = false`. A transient `SKY_CUA_PHONE=0` must also suppress phone tools for that MCP process; it may narrow the durable surface policy but cannot widen `[surfaces].phone = false` without an explicit `SKY_CUA_SURFACES` override that includes phone.

   Add `SKY_CUA_SURFACES` to the canonical Rust env-key inventory and the MCP forwarding allowlists (`.mcp.json`, installer/runtime allowlists) with the existing env-contract tests. Do not persist it in `mcp-launch-policy.json`; the machine config is the durable owner.

2. Make MCP registry construction surface-aware in `crates/sky-cua-client/src/mcp_tools/definitions/` and freeze the resolved policy during `initialize`.

   Extend `McpProcessConfig` with the resolved surface policy. Machine-config or surface-policy parse failure must not fall back to all-enabled: fail MCP initialization with a clear configuration error so a malformed disable policy cannot silently widen the advertised surface.

   Build the existing canonical definitions conditionally rather than introducing alternate profiles. Surface-specific families are straightforward: desktop tools only when desktop is enabled, `browser_*` only when browser is enabled, and `phone_*` only when phone is enabled. Keep `doctor` independent.

   Refactor the shared schema builders so they receive the enabled surface set and construct only valid branches and properties:

   - `status`: browser; phone + phone_companion; session_presence for desktop.
   - `list_resources`: desktop apps/windows/focused_window; browser tabs; phone devices/apps/current_app.
   - `observe`: desktop, browser, and/or phone branches with only the fields needed by enabled branches.
   - `capture_screen`: browser and/or phone only; omit the tool entirely when both are disabled.

   The flattened advertised schemas and the richer validation schemas must be projected from the same branch set. Update shared-tool descriptions from the same enabled branch set so disabled surfaces are not still taught through prose.

   Preserve existing approval annotations and handler mappings. `browser_eval` remains the existing independent security gate, but can only be added when browser itself is enabled.

3. Make registry membership gate call-context preparation in `crates/sky-cua-client/src/mcp_server.rs`.

   Today `prepare_browser_call` runs before the registry rejects an unknown/disabled tool and registers an in-flight browser operation. With configurable surfaces, move or guard browser/phone context preparation so a disabled tool or disabled shared branch is rejected by the frozen registry/schema before any browser operation identity, cancellation registration, phone turn context, or service request can be created. This preserves the existing invariant that advertised/callable registry membership is the public authorization boundary.

4. Keep the canonical contract fixtures authoritative without multiplying the full giant fixture for every surface combination.

   Preserve the current all-enabled `mcp_tool_surface_matrix.json` as the exact full public definition fixture for image/eval variants. Add a small surface-policy matrix fixture/test covering all eight desktop/browser/phone combinations with expected tool names plus the branch/enumeration/property projection of the four shared tools. The all-enabled projection must remain behaviorally identical to the current canonical surface except for intentional documentation/description corrections.

   Add tests proving hidden names never dispatch, disabled shared branches fail validation, registry policy is immutable after `initialize`, `browser_eval` requires browser+eval, `capture_screen` disappears in desktop-only mode, and phone's legacy master switch removes the phone surface rather than leaving ghost tools.

   While touching the feature documentation, correct the stale 34/35 count in `docs/features/mcp-tool-surface.md`: the current source fixture is 40 tools by default and 41 with browser eval after the later phone content/clipboard/editor/camera/storage additions. Tests and probes must derive expectations from the registry/fixture instead of encoding the obsolete count.

5. Project the durable machine policy onto skills during provisioning.

   In `scripts/_install_shared.py`, add one surface-to-skill mapping and make `install_sky_cua_skills` accept the enabled durable surface set. Reconciliation must install enabled Sky CUA skills and remove stale disabled Sky CUA-owned destinations, but never touch unrelated skill directories.

   Update `scripts/sync_agent_skills.py` to read the durable machine config (or accept an explicit durable surface set supplied by provisioning), link only enabled skills, and delete stale managed links for disabled surfaces. It must not use `SKY_CUA_SURFACES`, because a transient runtime override must not mutate permanent skill installation.

   Thread the same durable selection through `scripts/install_mcp_server.py` for Claude Code, Pi, and Hermes. This is especially relevant to Saga/Hermes: a deployment can write the target machine's `[surfaces]` policy first, then the normal Hermes installer reconciles both MCP registration and the exact skill subset.

   For Codex, keep the plugin/cache payload complete so bundle validation and portability remain intact. Extend the existing managed `skills.config` block in `_plugin_bundle.py`/deployment provisioning: enabled surfaces retain the current shared-copy deduplication behavior; disabled surfaces additionally disable the active plugin/compat skill copy. Re-running install/deploy must converge stale rules when a surface is re-enabled. `resources/chrome_preflight.py` may continue materializing the complete compat skill tree; visibility is owned by the managed Codex skill policy rather than by mutating the release payload.

6. Make bundled skills valid in any subset.

   Update `skills/computer-use/SKILL.md`, `skills/browser-use/SKILL.md`, and `skills/phone-use/SKILL.md` so each describes its own surface without assuming sibling skills exist. Replace unconditional instructions such as "use computer-use" or "use phone-use" with conditional routing such as "if that surface is enabled/available, use it; otherwise report that the task needs a disabled surface." Do not teach hidden tool names from a disabled surface through the remaining skill.

7. Update operator docs and the roadmap when implementation lands.

   Document `[surfaces]`, defaults, restart semantics, the `SKY_CUA_SURFACES` temporary override, the interaction with `[phone].enabled`, and provisioning convergence in `docs/runtime/mcp-boundary.md` and the relevant feature/host docs. Add a shipped feature doc when proven, update `ROADMAP.md`, then retire this ExecPlan per `plans/AGENTS.md`.

## Validation

Run the focused config/registry tests first, then the normal gates because this changes the public MCP contract and every installer path:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets
cargo nextest run
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
python3 scripts/build_plugin.py
```

Add/extend `scripts/probe_mcp_tool_surface.py` so it can probe a surface policy and assert both tool names and shared schema projection. At minimum prove through real stdio MCP:

```text
all enabled       -> current canonical surface
browser only      -> doctor + browser/shared-browser tools; no desktop/phone branches
desktop + phone   -> no browser_* tools, no browser branches in shared tools
desktop only      -> no capture_screen; no browser/phone branches
```

For each disabled surface, manually calling a hidden surface-specific tool must return `UnknownTool` with zero service dispatch, and manually selecting its branch on a remaining shared tool must return `InvalidRequest` with zero service dispatch.

Provisioning tests must cover enable -> disable -> re-enable convergence for all three skill names on the shared agents root and the Claude/Pi/Hermes installers, plus Codex managed skill rules in both compat-first and local-channel fallback modes. Unrelated user skills/config must survive unchanged.

Finish with installed-host proof, not source-only tests: deploy one all-enabled regression install, one browser-only install, and one mixed `desktop+phone` install. For the mixed and browser-only installs, inspect the real host's `tools/list` and installed/enabled skills, then execute at least one live action on every enabled surface and confirm no disabled surface is discoverable. A Saga/Hermes deployment is a suitable proof for the provisioning path once its desired surface subset is chosen; Asgard/Codex is the proof for managed plugin-skill disabling.
