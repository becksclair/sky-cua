# Compact MCP tool surface

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `plans/AGENTS.md`. It also embeds the repository-relevant ExecPlan requirements from `~/.agents/PLANS.md`: the plan must be self-contained, must define its terms, must provide independently verifiable milestones, and must end in observable behavior rather than only code changes.

## Purpose / Big Picture

After this change, an agent host using the existing `sky-cua-client mcp` server can opt into a compact MCP tool surface without losing the behavior of the current desktop, browser, and Android phone workflows. MCP means Model Context Protocol, the JSON-RPC protocol this project uses to advertise callable tools to hosts such as Codex, OpenCode, Pi, Claude Code, Claude Desktop, and OpenClaw. Today the server advertises 66 tools by default, or 67 when browser JavaScript evaluation is enabled. Many names repeat the same kind of operation across surfaces, but those repeats are not all safely mergeable because they have different approval, coordinate, target, and trust contracts.

The observable outcome for this plan is an opt-in production profile named `compact`. When `SKY_CUA_MCP_TOOL_PROFILE=compact` is pinned in the launched MCP process, `tools/list` advertises only compact names, and `tools/call` accepts only names present in that same active registry. A default or explicitly pinned `legacy` profile continues to advertise and accept the current granular names. The unset server fallback remains `legacy` for this plan and for future upgrades unless a later rollout plan explicitly changes installer behavior for genuinely new host registrations.

The goal is not only fewer names. The compact registry must also be materially smaller as serialized MCP JSON, preserve host approval fidelity, keep compact schemas usable by real hosts, and avoid making agents worse at choosing the right sky-cua action. The target shape is about 34 default compact tools, or 35 when `browser_eval` is explicitly enabled. A slightly larger compact surface with small, predictable schemas is preferable to a smaller surface with broad, confusing union schemas.

The central design rule is: deduplicate only inside the same approval and trust class. MCP tool annotations are static metadata on each tool definition. Hosts use annotations such as `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint` to decide approval behavior. Because annotations cannot vary by a tool's `surface` or `operation` argument, this refactor must not merge read-only observation with mutating input, must not merge local desktop or phone input with open-world browser page input, and must not hide broad diagnostics behind narrow status names.

## Progress

- [x] (2026-06-21 18:44Z) Created the initial ExecPlan from a source inventory, direct `tools/list` probe, and the first Oracle design review of the compact surface proposal.
- [x] (2026-06-21 20:48Z) Revised the plan after a second Oracle review found release-blocking gaps around advertised-versus-callable tools, immutable session policy, installer persistence, host schema proof, compact smoke proof, and over-broad grouped tools.
- [x] (2026-06-21 21:00Z) Tightened the revised plan with exact installer/OpenClaw/Codex materialization function seams, verified the 34/35 compact target count against the listed tools, and ran a local checklist against the second Oracle review findings.
- [x] (2026-06-22 10:29Z) Ran one browser Oracle rereview of the revised plan with the current ExecPlan, roadmap, MCP registry code, installer seams, and bundled skill files attached. The review accepted the 34/35 compact shape and the `legacy`/`compact` production policy, but rejected implementation readiness until the compact public contract, installer persistence, registry construction, schema spike order, response parity, and installed proof were pinned more tightly.
- [x] (2026-06-22 10:29Z) Revised this plan again to require a complete compact contract fixture before handler work, one ordered registry entry set as the source for both `tools/list` and dispatch, exact installer-state semantics including Codex materialization, truthful `doctor` annotations, response parity, schema-size gates, and agent-driven installed proof.
- [x] (2026-06-22 10:53Z) Ran an inline browser Oracle final-review after the large attachment upload path timed out before model review. The review again accepted the compact shape and production policy, but found remaining blockers around horizontal desktop scroll, false phone `backend` fields, fixture authority, `doctor` audit ordering, immutable eval gates, response/prose policy, installer transaction semantics, and fixed size/schema-spike budgets.
- [x] (2026-06-22 10:53Z) Revised this plan to pin horizontal scroll, remove non-existent phone backend controls, split contract and public surface artifacts, move `doctor` audit into Milestone 0, freeze browser eval on both client and service paths, add explicit response policy fields, define installer launch-policy transactions, and fix the schema-size and branch-level host-spike gates.
- [x] (2026-06-22 11:42Z) Ran another focused Oracle approval review. The only remaining blockers were ambiguous config-layer behavior: runtime fallback versus installer rejection, and missing launch-policy precedence rules across CLI, persisted state, environment, and defaults.
- [x] (2026-06-22 11:42Z) Revised this plan to make config handling layer-specific. Raw MCP runtime initialization is lenient and diagnostic-only; installer and other config-writing surfaces are strict, validate all inputs before mutation, and resolve launch policy per field with explicit CLI, persisted state, environment, then defaults.
- [x] (2026-06-22 11:45Z) Ran the final focused Oracle approval review. Oracle returned "APPROVED TO IMPLEMENT" with no release-blocking plan defects; the only minor implementation-friction note was to bound "recognized installer environment", which this plan now closes explicitly.
- [x] (2026-06-22 12:01Z) Started implementation on branch `bex/compact-mcp-tool-surface` with an explicit end-to-end goal, isolated OpenCode worker lanes, Android emulator QA, and VM smoke validation as required closeout gates.
- [x] (2026-06-22 12:18Z) Landed the first registry/config slice in the working tree: `initialize` now freezes a profile-aware registry, compact advertises 34 tools by default and 35 with eval enabled, legacy still advertises 66/67, invalid raw runtime env falls back with diagnostics, second initialize is rejected, and inactive-profile calls return before service dispatch. Verified with focused Rust tests plus direct stdio readback.
- [x] (2026-06-22 12:32Z) Added the first compact facade dispatcher so advertised compact names route to existing typed desktop, browser, and phone handler paths instead of returning `UnknownTool`. Pinned branch-name mapping tests, a compact desktop keyboard typed-request test, compact `doctor` truthful non-read-only annotation, full `cargo test -p sky-cua-client`, and real compact stdio `status(component="browser")` readback.
- [x] (2026-06-22 12:56Z) Added fixture-backed compact contract gates: exact public `tools/list` matrix for legacy/compact across image/eval combinations, compact branch contract metadata, minimal valid/invalid compact call cases, serialized-size and schema-size budget checks, and richer compact schemas for real host branch selection. Verified with fixture regeneration, fixture comparison tests, compact unit tests, and rebuilt compact stdio schema readback.
- [x] (2026-06-22 13:05Z) Added compact response envelopes for compact `tools/call`: successful compact calls now return `structuredContent.profile/tool/branch/legacy_tool/result`, compact invalid requests return compact error envelopes before service dispatch, and content text is rewritten with the compact branch identity. Verified with compact unit tests and rebuilt stdio success/error probes.
- [x] (2026-06-22 13:17Z) Froze browser eval policy across client and service request paths: MCP session dispatch now uses the registry snapshot instead of request-time env reads, and the service daemon snapshots eval permission at startup before calling the browser bridge. Verified with focused client and service tests that request-time env changes cannot disable or enable eval contrary to the frozen policy.
- [x] (2026-06-22 13:35Z) Added strict installer/deploy launch-policy persistence for `SKY_CUA_MCP_TOOL_PROFILE`, `SKY_CUA_BROWSER_EVAL`, and `SKY_CUA_MODEL_SUPPORTS_IMAGES`: per-field CLI, persisted state, recognized env, default resolution; generic/OpenCode/Claude/Pi/OpenClaw config propagation; state-write-last persistence; deploy forwarding; and Codex materialization env-var allowlist updates. Verified with install-flow tests, OpenClaw tests, Ruff, basedpyright, temp config materialization, and Codex allowlist readback.
- [x] (2026-06-22 13:45Z) Hardened the Arch VM smoke harness for TTY-launched Hyprland sessions. The runner and provisioner now install a VM-only user unit for `xdg-desktop-portal.service`; the runner imports Hyprland instance identity during portal refresh, while provisioned sessions add a post-start importer after Hyprland creates that identity. The runner also stops wlroots/Hyprland portal backends during refresh and lets the layer-shell overlay smoke preauthorize screenshots. Verified the previously failing `wayland-layer-shell-overlay` profile against the real Hyprland VM session with portal screenshot capture and visible overlay proof.
- [ ] Add a generated or validated Markdown ledger for the compact public contract.
- [x] Add immutable MCP process/session configuration and enforce that advertised tool names are exactly the callable tool names for the active profile.
- [ ] Add the host schema and approval compatibility smoke before relying on grouped compact schemas.
- [ ] Implement compact action tools split by approval class, coordinate contract, and schema complexity.
- [x] Persist profile selection through every installer and host-launch path while preserving legacy for existing installs.
- [ ] Migrate skills, docs, and live smoke drivers to be profile-aware without teaching hidden tool names.
- [ ] Prove direct, installed, and host-level desktop, browser, and phone workflows in both `legacy` and `compact`.
- [ ] Retire this ExecPlan into feature docs and `ROADMAP.md` only after opt-in compact is shipped and proven. A default switch is out of scope for this plan.

## Surprises & Discoveries

- Observation: The current advertised MCP surface is 66 tools by default and 67 with `SKY_CUA_BROWSER_EVAL=on`.
  Evidence: A direct `target/release/sky-cua-client mcp` `tools/list` probe returned 66 names without `browser_eval` and 67 names with it.

- Observation: The count jump is mostly from the phone family.
  Evidence: `crates/sky-cua-client/src/mcp_tools/phone/schema.rs` pushes 27 `phone_*` tool definitions. `docs/features/phone-use.md` says this static family lives inside the single `computer-use` MCP server.

- Observation: The internal service contract is already more compact than the public MCP surface.
  Evidence: `crates/sky-cua-platform/src/model/service.rs` wraps browser requests as `ServiceRequest::Browser { request: BrowserRequest }` and phone requests as `ServiceRequest::Phone { request: PhoneRequest }`; `crates/sky-cua-platform/src/model.rs` routes desktop actions through `ActionName`.

- Observation: Static MCP annotations are the main hidden risk in target deduplication.
  Evidence: `crates/sky-cua-client/src/mcp_tools/annotations.rs` documents that hosts rely on these hints, and `crates/sky-cua-client/src/mcp_tools/definitions.rs` pins expected annotations for every advertised tool.

- Observation: A universal `input(surface, operation)` tool would be over-broad.
  Evidence: Browser input is open-world because it reaches real signed-in web pages, while phone and desktop input are local operations. A single public tool would have to advertise the worst browser annotation for every branch.

- Observation: A universal `screenshot(surface)` tool would also be misleading in the current contract.
  Evidence: Browser and phone screenshots are read-only, but desktop `screenshot` can activate and focus a target window before capture, so it is currently annotated as a local navigation action.

- Observation: The initial plan allowed hidden direct legacy calls in compact mode, which would break the profile trust boundary.
  Evidence: Current `crates/sky-cua-client/src/mcp_tools.rs` dispatch accepts every known legacy desktop name and delegates all recognized browser and phone names. Hosts attach annotations to advertised tool definitions, not to unadvertised direct calls.

- Observation: The MCP registry must be a frozen session contract.
  Evidence: Current `ServerSession` stores only `ModelSessionInfo`, and current `TOOL_DEFINITIONS_CACHE` is keyed only by image capability. Browser eval enablement is read during registry construction, while the initial profile design read environment state from `tools_list_result`.

- Observation: The future "compact for fresh installs, legacy for upgrades" rollout cannot be implemented by changing a Rust fallback.
  Evidence: Host configs generated by `scripts/install_mcp_server.py` currently pin `SKY_CUA_REPO_ROOT`, and Pi uses a wrapper because Pi cannot express environment values in MCP JSON. Existing unpinned installed configs cannot be distinguished from fresh configs without persisted installer state.

- Observation: Existing live smokes cannot prove compact mode as written.
  Evidence: The current desktop and phone smokes assert and call legacy names directly, and the current Chrome host smoke proves only a narrow browser list-tabs path. Running those scripts under `SKY_CUA_MCP_TOOL_PROFILE=compact` would fail early or prove the wrong behavior.

- Observation: Reducing names from 66 to 28 is about a 2.36x name reduction, not a 3x reduction, and name count alone can be the wrong metric.
  Evidence: Large grouped schemas can serialize to more JSON than narrower legacy schemas. Compact acceptance must record serialized `tools/list` bytes, description bytes, largest schema bytes, and duplicate-name count.

- Observation: The direct phone smoke contains legacy-name bugs that can hide during facade migration if not fixed first.
  Evidence: `scripts/live_phone_use_smoke.py` sends `reply_text` to `phone_notification_reply`, while `crates/sky-cua-client/src/mcp_tools/phone.rs` requires `text`; the smoke searches notification actions for `is_reply`, while the model field is `supports_inline_reply`; wireless device selection checks `connection_kind == "wireless"`, while serialized enum values include `legacy_tcpip` and `wireless_debugging`.

- Observation: The latest Oracle rereview found the compact shape acceptable but the plan not yet implementation-ready before this revision.
  Evidence: The 2026-06-22 browser review approved the 34/35 compact target and the `legacy`/`compact` production policy, but flagged missing public contract details, incomplete installer materialization paths, a registry two-source-of-truth risk, an unsafe `doctor` annotation escape hatch, schema-spike ordering problems, and weak response/agent parity gates.

- Observation: Codex profile materialization is broader than `.mcp.json` and the install helpers originally named in this plan.
  Evidence: The latest rereview pointed at `resources/chrome_preflight.py::sync_computer_use_compat_plugin` as the owner that materializes the active compat plugin and at `scripts/deploy_plugin.py::fast_deploy` as another active deployment path. `scripts/_plugin_bundle.py::compat_plugin_targets_payload` checks selected commands, while `scripts/_plugin_bundle.py::update_codex_config` toggles plugin enablement; neither alone proves the launched MCP environment.

- Observation: `SKY_CUA_MODEL_SUPPORTS_IMAGES` is negotiated session capability by default, not installer policy.
  Evidence: The current source derives model image capability during `initialize`, while the environment override can force image behavior. The installer must forward only an explicit operator or host override; synthesizing this variable would pin the wrong schema for later models.

- Observation: Approval-spike evidence must be collected from isolated annotation-aware host configurations.
  Evidence: The latest rereview noted that OpenClaw can be configured with approval mode `approve` and Claude Code can receive wildcard allow rules, so their default install proof can skip annotation gates entirely. The spike must mark such cases not applicable or run isolated configurations that actually honor MCP annotations.

- Observation: The compact contract must not promise controls absent from the existing typed request model.
  Evidence: The 2026-06-22 final Oracle rereview found that `desktop_scroll` had lost legacy `left` and `right`, and that `backend` was incorrectly listed on phone refresh, pointer, keyboard, accessibility-tree, and notification branches even though current phone request construction accepts backend routing only for observe, screenshot, and connect.

- Observation: Browser eval immutability is a client and service problem, not only a registry problem.
  Evidence: Current browser dispatch calls `browser_eval_enabled()` at tool-call time, and the helper reads `SKY_CUA_BROWSER_EVAL` directly. Freezing only `tools/list` would leave a request-time environment side channel.

- Observation: Artifact authority needs three layers, not one overloaded fixture.
  Evidence: The final rereview flagged that a public `tools/list` fixture cannot also contain handler ids, mappings, defaults, response policies, and expected errors without ceasing to be an exact MCP public response.

## Decision Log

- Decision: Keep only `legacy` and `compact` as production profiles in this plan.
  Rationale: `core` is a separate product decision that multiplies registry, installer, docs, skill, and smoke matrices before compact usage exists. A `compat` profile with both old and new names creates duplicate-name and tool-selection risks. Parity tests may use an internal combined registry helper, but it is not advertised as a normal production profile.
  Date/Author: 2026-06-21 / Codex

- Decision: A tool name is callable if and only if it is present in the active session registry.
  Rationale: Host approval hints attach to advertised definitions. Hidden direct calls to unadvertised legacy names would bypass the compact profile's approval and schema contract and could let compact smokes pass by accidentally using legacy tools.
  Date/Author: 2026-06-21 / Codex

- Decision: Freeze profile, browser-eval policy, model image capability, and the registry at MCP initialization.
  Rationale: The server advertises `tools.listChanged=false`. Runtime environment changes must not alter the registry after initialization. A process restart and new MCP session are required to change profile or eval policy.
  Date/Author: 2026-06-21 / Codex

- Decision: The server's unset profile fallback remains `legacy`; installers preserve existing persisted profile state unless the operator explicitly overrides it.
  Rationale: Existing installs must not change behavior because a new binary changes its fallback. Missing install state is treated as `legacy`. A no-flag installer run preserves the existing persisted host profile. An explicit `--mcp-tool-profile legacy|compact` updates state only after the host launch materialization succeeds. This prevents accidental compact upgrades and avoids resetting a prior compact opt-in back to legacy.
  Date/Author: 2026-06-21 / Codex

- Decision: Build compact as a client-side MCP facade over the existing service IPC, not as a service wire-format rewrite.
  Rationale: `ServiceRequest::Browser`, `BrowserRequest`, `ServiceRequest::Phone`, `PhoneRequest`, and desktop `ActionName` already form the correct runtime boundaries. The problem is public MCP advertisement and argument shape, not daemon IPC.
  Date/Author: 2026-06-21 / Codex

- Decision: Keep `doctor` as its own compact tool and do not make `status(component="desktop")` an alias for it.
  Rationale: `doctor` is a broad cross-system diagnostic that may report session-env repair, browser integration, and presence state. A narrow status tool should not hide that breadth or risk an annotation lie.
  Date/Author: 2026-06-21 / Codex

- Decision: Use `status(component=...)`, not `status(surface=...)`.
  Rationale: Status branches are component status calls, not surface observations. Initial supported components are `browser`, `phone`, `phone_companion`, and `session_presence`.
  Date/Author: 2026-06-21 / Codex

- Decision: Restrict `list_resources` to bounded resource discovery and current-resource lookup, and keep accessibility trees separate.
  Rationale: Desktop apps/windows, browser tabs, phone devices, and phone apps are inventories. Focused desktop window and current phone app are singular current-resource lookups. A phone accessibility tree is bounded foreground observation state, not a resource list.
  Date/Author: 2026-06-21 / Codex

- Decision: Keep notification reads phone-specific until a second real surface implements notifications.
  Rationale: Known unsupported desktop/browser branches should not be valid compact schema choices. Unsupported manual branches should fail as invalid requests before service dispatch.
  Date/Author: 2026-06-21 / Codex

- Decision: Split desktop and phone input into smaller tools rather than one broad input union.
  Rationale: Desktop pointer, keyboard, and semantic action branches have different required fields and default-injection hazards. Phone pointer and keyboard input have different coordinate and text/key contracts. Smaller schemas are safer for hosts and agents.
  Date/Author: 2026-06-21 / Codex

- Decision: Keep `browser_eval` separate and opt-in.
  Rationale: `browser_eval` runs arbitrary JavaScript in real user browser tabs and is already hidden unless `SKY_CUA_BROWSER_EVAL` is enabled. Embedding it in another browser tool would weaken the explicit trust boundary.
  Date/Author: 2026-06-21 / Codex

- Decision: Machine-readable fixtures are authoritative for tool registries; Markdown is generated from or validated against them.
  Rationale: A Markdown ledger will drift from executable definitions if it is the source of truth. Canonical JSON fixtures can be tested for exact definitions, exact deterministic order, duplicate-name absence, and serialized-size budgets. Fixtures must cover every profile, image-capability, and browser-eval combination; probing must not sort tools to make a bad order look stable.
  Date/Author: 2026-06-21 / Codex

- Decision: The future default switch is outside this ExecPlan.
  Rationale: This plan ships opt-in compact and legacy default. Changing fresh-install defaults requires a separate rollout plan with measured compact usage, persisted installer state, and explicit upgrade semantics.
  Date/Author: 2026-06-21 / Codex

- Decision: The compact public contract is a checked-in machine fixture before handler implementation, not a design task inside handler milestones.
  Rationale: The compact facade is public API. Each tool branch needs a pinned discriminator, required fields, optional fields, forbidden fields, defaults, annotation tuple, legacy mapping, response identity, and error behavior before runtime code can safely dispatch it. Minimal valid and invalid call fixtures make the branch matrix executable.
  Date/Author: 2026-06-22 / Codex

- Decision: One ordered registry entry set is the source for both `tools/list` and dispatch.
  Rationale: A separate `Vec` of advertised definitions and `BTreeSet` of callable names can drift. The registry must store ordered entries that each contain name, definition, annotation, compact contract metadata, and handler id. Any lookup index is derived from those entries at construction time.
  Date/Author: 2026-06-22 / Codex

- Decision: A second MCP `initialize` on an already initialized session returns a stable error and leaves the first registry untouched.
  Rationale: The server declares `tools.listChanged=false`, so accepting a later initialize that could rebuild profile, eval, or image capability would violate the session contract. Rejecting the second initialize is simpler to test and safer than trying to prove it cannot matter.
  Date/Author: 2026-06-22 / Codex

- Decision: Compact `doctor` must use truthful annotations; no compact grandfather exception is allowed.
  Rationale: If the runtime audit proves that `doctor` mutates environment or service state, advertising compact `doctor` as read-only would lie to hosts. Legacy may keep an old annotation only as a documented compatibility exception, but compact must either use the truthful tuple or split the mutating branch into a separate tool.
  Date/Author: 2026-06-22 / Codex

- Decision: Installers do not synthesize `SKY_CUA_MODEL_SUPPORTS_IMAGES`.
  Rationale: Model image support is negotiated during MCP initialization. The environment variable is an override for explicit operator or host constraints; writing it by default into installed launchers can permanently force the wrong schema and image delivery for future sessions.
  Date/Author: 2026-06-22 / Codex

- Decision: Size budgets are release gates, not local preferences.
  Rationale: The compact registry must be smaller and easier for agents to use. Serialized `{"tools":[...]}` bytes must stay at or below 65% of comparable legacy for every image/eval pair, and each compact tool schema must be at or below 8192 serialized bytes. Relaxing either gate requires another plan review.
  Date/Author: 2026-06-22 / Codex

- Decision: Compact schemas must not add fields that current typed requests ignore or cannot honor.
  Rationale: A compact branch that accepts `backend` without passing it through would teach agents false control. Compact can regroup existing behavior, but adding platform request features is a separately reviewed scope expansion.
  Date/Author: 2026-06-22 / Codex

- Decision: The `doctor` mutation audit is a prerequisite for the compact contract fixture.
  Rationale: If `doctor` mutates state, its annotation affects both the fixture and host approval evidence. Running the audit after registry and schema proof would make those artifacts stale.
  Date/Author: 2026-06-22 / Codex

- Decision: Browser eval policy is immutable in both the MCP client and the service.
  Rationale: The current shared helper reads the environment. To uphold `tools.listChanged=false`, no request path may consult `SKY_CUA_BROWSER_EVAL`; both the MCP registry/dispatch layer and service defence-in-depth gate must use startup snapshots.
  Date/Author: 2026-06-22 / Codex

- Decision: Public surface, internal contract, and call cases are separate authoritative artifacts.
  Rationale: `tools/list` evidence must remain an exact public MCP response, while implementation needs branch metadata that must not appear in that response. Splitting artifacts prevents hidden drift and keeps each artifact testable.
  Date/Author: 2026-06-22 / Codex

- Decision: Installer profile and eval persistence follow an explicit launch-policy transaction.
  Rationale: Updating host configs, wrappers, plugin materialization, and symlink entries cannot be described as a single filesystem atomic write. A staged transaction with snapshots, verification, state-write-last semantics, and fault-injection tests is the only implementable recovery story across hosts.
  Date/Author: 2026-06-22 / Codex

- Decision: Invalid config handling is layer-specific.
  Rationale: A manually launched MCP server should not fail initialization because of a bad environment variable; it must start in the safest compatible profile and emit diagnostics. Installer and config-writing surfaces are different: they persist future launch behavior, so accepting or silently rewriting invalid operator input would create sticky, surprising state. Raw MCP runtime parsing may fall back; config-writing surfaces must reject.
  Date/Author: 2026-06-22 / Codex

- Decision: Launch policy resolution is per-field and deterministic: CLI, persisted host state, environment, then defaults.
  Rationale: Operators need narrow overrides such as changing only profile while preserving eval and image policy. Per-field resolution is simpler to use than all-or-nothing config tiers, but it must still be deterministic. Before resolving, config-writing surfaces validate every present CLI, persisted-state, and recognized environment value; any invalid value aborts before mutation. Then each field independently chooses the first valid value from explicit CLI flag, persisted host state, recognized installer environment, and finally defaults.
  Date/Author: 2026-06-22 / Codex

- Decision: The recognized installer environment is a closed set.
  Rationale: Ambient process environment should not become an accidental policy API. For launch-policy resolution, recognized installer environment means only `SKY_CUA_MCP_TOOL_PROFILE`, `SKY_CUA_BROWSER_EVAL`, and `SKY_CUA_MODEL_SUPPORTS_IMAGES`. Other environment variables may still affect unrelated installer mechanics, but they cannot fill or override persisted MCP launch policy fields.
  Date/Author: 2026-06-22 / Codex

## Outcomes & Retrospective

No implementation has started. The plan has been revised through several Oracle-backed review loops before implementation because the earlier drafts still left release-blocking ambiguity in public API, registry, installer, validation, and config-policy behavior. The current expected outcome is an opt-in compact MCP facade that is smaller, authorization-safe, installer-pinned, host-proven, and profile-aware while leaving legacy behavior as the default and as a complete compatibility path.

The 2026-06-22 Oracle rereviews accepted the compact tool shape and production profile policy, but rejected earlier drafts because too much API, registry, installer, proof behavior, and config-layer behavior remained delegated to implementers. This revision closes those gaps in the plan by requiring a complete compact contract before handler work, exact public-surface fixtures, exact installer launch-policy transactions, one registry source of truth, truthful compact annotations, schema-proof ordering, response parity, strict versus lenient config parsing boundaries, agent-driven proof, and installed readback across materially different launch paths. If another review finds new blockers, revise this section, the Progress list, and the Decision Log before implementation.

## Context and Orientation

The MCP server entrypoint is `crates/sky-cua-client/src/mcp_server.rs`. It handles `initialize`, `tools/list`, and `tools/call` JSON-RPC messages. A JSON-RPC message is a structured request or response sent over stdio. `tools/list` currently calls `crate::mcp_tools::tools_list_result(&session.model)`, and `tools/call` currently passes the chosen tool name and JSON arguments to `crate::mcp_tools::handle_tool_call`.

The public tool registry is built in `crates/sky-cua-client/src/mcp_tools/definitions.rs`. `build_tool_definitions(can_receive_images)` creates desktop and session-presence tools, then appends browser tools through `browser::push_tool_definitions` and phone tools through `phone::push_tool_definitions`. The current `TOOL_DEFINITIONS_CACHE` has only two entries, one for models that can receive image blocks and one for text-only models.

The dispatcher is `crates/sky-cua-client/src/mcp_tools.rs`. It contains the host-facing legacy tool handlers and delegates browser and phone names to `crates/sky-cua-client/src/mcp_tools/browser.rs` and `crates/sky-cua-client/src/mcp_tools/phone.rs`. This refactor must route compact calls through typed request construction, not recursive string redispatch to legacy tool names.

The browser public schema lives in `crates/sky-cua-client/src/mcp_tools/browser/schema.rs`. Browser tools control a real Chrome-family browser through the browser bridge. `browser_eval` is hidden unless `SKY_CUA_BROWSER_EVAL` is `on`, `1`, or `true`. The compact registry must use the same eval policy snapshot for both advertisement and dispatch.

The phone public schema lives in `crates/sky-cua-client/src/mcp_tools/phone/schema.rs`. Phone tools control Android devices through ADB, the optional companion app, and optional scrcpy acceleration. `phone_observe` is the main phone perception tool after connecting.

The platform-neutral service request and response types live in `crates/sky-cua-platform/src/model/service.rs`. The browser request enum is in `crates/sky-cua-platform/src/model/browser.rs`. The phone request enum is in `crates/sky-cua-platform/src/model/phone.rs`. Desktop action names and action requests are in `crates/sky-cua-platform/src/model.rs`. These are the internal contract spine and must not be renamed, removed, or flattened as part of this plan.

The bundled agent guidance lives in `skills/computer-use/SKILL.md`, `skills/browser-use/SKILL.md`, and `skills/phone-use/SKILL.md`. These files currently name the granular public tools directly. They must remain separate skills after compact lands, because desktop, browser, and phone still have different coordinate systems, setup assumptions, trust boundaries, and recovery advice.

Installer and host-launch code lives primarily in `scripts/install_mcp_server.py`, with OpenClaw-specific installation in `scripts/_openclaw_install.py` when present. The root `.mcp.json` is only one launch path. OpenCode, Claude Desktop, Claude Code, Pi, OpenClaw, Codex materialization templates, one-shot installers, bundle installers, and fast deploy all need explicit profile propagation. Codex is special because the launched compat plugin is materialized through `resources/chrome_preflight.py::sync_computer_use_compat_plugin`; `scripts/_plugin_bundle.py::compat_plugin_targets_payload` and `scripts/_plugin_bundle.py::update_codex_config` are not sufficient proof by themselves. `scripts/deploy_plugin.py::fast_deploy` is also an active deployment path and must receive the same profile treatment.

The current source-advertised legacy tool list is:

- Desktop observe, setup, and window tools: `doctor`, `setup_accessibility`, `setup_window_targeting`, `list_apps`, `list_windows`, `focused_window`, `activate_window`, `screenshot`, and `get_app_state`.
- Session presence tools: `hold_session`, `unlock_session`, `release_session`, and `session_presence_status`.
- Desktop action and semantic tools: `focus_element`, `activate_element`, `select_element`, `expand_element`, `collapse_element`, `toggle_element`, `click`, `perform_action`, `perform_secondary_action`, `scroll`, `drag`, `type_text`, `press_key`, and `set_value`.
- Browser tools: `browser_status`, `browser_list_tabs`, `browser_open`, `browser_claim_tab`, `browser_move_mouse`, `browser_navigate`, `browser_snapshot`, `browser_screenshot`, `browser_click`, `browser_type_text`, `browser_press_key`, `browser_scroll`, and optional `browser_eval`.
- Phone tools: `phone_observe`, `phone_status`, `phone_list_devices`, `phone_refresh_capabilities`, `phone_pair_wireless`, `phone_connect`, `phone_disconnect`, `phone_screenshot`, `phone_tap`, `phone_swipe`, `phone_type_text`, `phone_press_key`, `phone_install_companion`, `phone_companion_status`, `phone_accessibility_tree`, `phone_notifications`, `phone_notification_open`, `phone_notification_dismiss`, `phone_notification_action`, `phone_notification_reply`, `phone_app_current`, `phone_app_list`, `phone_app_launch`, `phone_app_open_intent`, `phone_app_force_stop`, `phone_app_install`, and `phone_open_settings`.

The compact production profile should advertise these tools by default, plus `browser_eval` only when the existing eval gate is enabled:

- Read-only tools: `doctor`, `status`, `list_resources`, `observe`, `capture_screen`, `phone_accessibility_tree`, and `phone_notifications`.
- Local navigation and setup tools: `capture_desktop`, `setup_desktop`, `session_presence`, `activate_window`, `desktop_semantic`, `browser_claim_tab`, `browser_move_mouse`, `phone_connection`, `phone_pair_wireless`, `phone_setup`, and `phone_app_force_stop`.
- Stateful non-idempotent tools: `desktop_toggle`, `desktop_scroll`, and `browser_scroll`. `browser_scroll` must preserve its current open-world annotation.
- Local or open-world destructive action tools: `desktop_pointer`, `desktop_keyboard`, `desktop_action`, `desktop_set_value`, `browser_input`, `phone_pointer`, `phone_keyboard`, `phone_notification_action`, `phone_notification_reply`, `phone_app_action`, and `phone_app_install`.
- Browser lifecycle tools: `browser_open`, `browser_navigate`, and optional `browser_eval`.

The compact profile intentionally does not include `get_notifications`, `desktop_input`, `phone_input`, `core`, or production `compat`.

## Compact Tool Contract

The compact public API is fixed by machine fixtures before any compact handler implementation starts. The first implementation milestone must create `crates/sky-cua-client/tests/fixtures/compact_tool_contract.json`, `crates/sky-cua-client/tests/fixtures/mcp_tool_surface_matrix.json`, and `crates/sky-cua-client/tests/fixtures/compact_call_cases.json`. `compact_tool_contract.json` is the internal authority for ordered compact entries, branches, handler ids, annotation tuples, mappings, defaults, errors, and response policies. `mcp_tool_surface_matrix.json` is the exact public `{"tools":[...]}` projection for all profile, image-capability, and eval combinations. `compact_call_cases.json` contains one minimal valid object and at least one invalid object for every compact branch. A branch is a discriminator choice inside a compact tool, such as `status` with `component="browser"` or `desktop_pointer` with `operation="drag"`.

Every compact contract entry must include its tool name, description, annotation tuple, input schema, branch discriminator, legacy source tool or tools, handler id, required fields, optional fields, forbidden fields, defaults, response identity, expected error codes, `content_policy`, `structured_policy`, and `normalization_json_pointers`. The response identity says whether `structuredContent.result` must deep-equal an existing legacy structured payload, be a documented normalized subset, or be a new compact-only error object. `content_policy` is either `exact` or `profile_rewrite`. `structured_policy` is either `exact` or `normalized`; normalized branches list the exact JSON pointers that may differ from the legacy structured payload. Unsupported manual branches are invalid requests before service dispatch; they must not reach the service and must not be represented as a successful result.

Common selectors keep their legacy meaning. Desktop window selectors are `window_id`, `pid`, `tty`, `terminal_pid`, `terminal_command`, `terminal_cwd`, `app_id`, `wm_class`, and `title`. Desktop semantic selectors are `element_index`, `element_identifier`, `role`, `name`, `text`, and `states`. Desktop pointer and semantic tools may also accept `snapshot_id` when they need coordinate translation from a prior `observe` or `capture_desktop`. Browser tools use `target` with only `user_chrome` and use `tab_id` from `browser_open`, `browser_claim_tab`, or `list_resources(surface="browser", resource="tabs")`. Phone device-bound tools accept optional `session_id` and optional `serial`, with `session_id` preferred when both are present. Only phone observe, phone screenshot through `capture_screen(surface="phone")`, and `phone_connection(operation="connect")` accept `backend` with `auto`, `adb`, `companion`, `scrcpy`, or `none`; other phone branches must not advertise backend routing unless platform request types are expanded in a separate review.

The initial branch matrix is:

- `doctor`: no required or optional input fields. It returns `component="system"`, `operation="doctor"`, and the legacy `doctor` structured payload under `result`. If the mutation audit proves it writes state, compact `doctor` must receive the truthful annotation or be split before release.
- `status`: required `component`. Supported components are `browser`, `phone`, `phone_companion`, and `session_presence`. `component="phone"` may accept `refresh_devices`. `component="phone_companion"` may accept phone session selectors. Other components accept no branch-specific fields. The operation in the response is `status`.
- `list_resources`: required `surface` and `resource`. Supported pairs are `desktop/apps`, `desktop/windows`, `desktop/focused_window`, `browser/tabs`, `phone/devices`, `phone/apps`, and `phone/current_app`. Browser tabs may accept `target`, `url_contains`, and `title_contains`. Phone devices may accept `include_mdns`. Phone apps may accept `include_system`, `limit`, and phone session selectors. Phone current app may accept phone session selectors. This tool is bounded resource discovery and current-resource lookup; it is not a generic observation tool.
- `observe`: required `surface`. `surface="desktop"` accepts app/window selectors, `detail` as `full` or `compact` with default `compact`, `element_query` up to `APP_STATE_MAX_ELEMENT_QUERY_CHARS`, `element_offset` minimum 0, `element_limit` from 0 through `APP_STATE_MAX_ELEMENT_LIMIT` with compact default `APP_STATE_DEFAULT_ELEMENT_LIMIT`, and image-capability-dependent `capture_screen` as `auto`, `if_changed`, `always`, or `never` plus `screenshot_delivery` as `path` or `inline`. `surface="browser"` requires `tab_id` and accepts `target`, `element_offset` minimum 0, `element_limit` from 0 through `BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT`, `element_query`, and `text_limit` from 0 through `BROWSER_SNAPSHOT_MAX_TEXT_LIMIT`. `surface="phone"` accepts phone session selectors, `backend`, `include_accessibility`, and `include_notifications`. The phone branch may include bounded accessibility and notification sections for parity with `phone_observe`, but it must not switch into the full `phone_accessibility_tree` response family.
- `capture_screen`: required `surface`. `surface="browser"` requires `tab_id` and may accept `target`. `surface="phone"` accepts phone session selectors and `backend`. Desktop screenshots are not in this tool; use `capture_desktop`.
- `phone_accessibility_tree`: accepts phone session selectors and `node_limit` minimum 0. It is the only compact tool for the full phone accessibility tree response family, and it must not advertise `backend` because the current request is companion-backed rather than caller-routable.
- `phone_notifications`: accepts phone session selectors and `limit` minimum 0. It is the dedicated focused notification reader; `observe(surface="phone", include_notifications=true)` may still include the bounded notification section from `phone_observe`. It must not advertise `backend`.
- `capture_desktop`: accepts desktop window selectors, `display_id`, `display_name`, `display_index`, `capture_all_displays`, and image-capability-dependent `screenshot_delivery`. It maps to legacy `screenshot`, not to browser or phone capture.
- `setup_desktop`: required `operation` with `accessibility` or `window_targeting`. It accepts no branch-specific fields and maps to `setup_accessibility` or `setup_window_targeting`.
- `session_presence`: required `operation` with `hold`, `unlock`, or `release`. `hold` accepts `unlock`, `inhibit_lock`, and `inhibit_suspend` with defaults `false`, `true`, and `true`. `unlock` accepts `inhibit_lock` and `inhibit_suspend` and forces unlock true. `release` accepts `relock`, default false. Status remains only `status(component="session_presence")`.
- `activate_window`: accepts desktop window selectors and maps to legacy `activate_window`.
- `desktop_semantic`: required `operation` with `focus`, `select`, `expand`, or `collapse`. It accepts desktop semantic selectors and maps to the matching semantic legacy tool.
- `desktop_toggle`: accepts desktop semantic selectors and maps to `toggle_element`.
- `desktop_scroll`: requires `direction` as `up`, `down`, `left`, or `right`; accepts `pages` minimum 1, `element_index` minimum 0, and `snapshot_id`; maps to legacy `scroll`.
- `desktop_pointer`: required `operation` with `click`, `secondary_click`, or `drag`. `click` accepts either `element_index` or both `x` and `y`; if both an element and coordinates are supplied, the parser rejects the request. `secondary_click` follows the same target rule and also accepts optional `action`. `drag` accepts one start selector, either `element_index`, both `x` and `y`, or both `from_x` and `from_y`; it accepts one end selector, either `to_element_index` or both `to_x` and `to_y`; incomplete coordinate pairs and multiple start or end selectors are invalid. All branches accept `snapshot_id`.
- `desktop_keyboard`: required `operation` with `type_text` or `press_key`. `type_text` requires `text`; `press_key` requires `key`. Both accept desktop window selectors and `snapshot_id` for target activation.
- `desktop_action`: required `operation` with `activate` or `perform_action`. `activate` accepts desktop semantic selectors and maps to `activate_element`. `perform_action` accepts desktop semantic selectors plus `action_index`, `action_name`, and `action` and maps to `perform_action`.
- `desktop_set_value`: requires `value`; accepts desktop semantic selectors and `snapshot_id`; maps to `set_value`.
- `browser_open`: accepts `target` and `url`; `url` may be `http://`, `https://`, or `about:blank`. It preserves the legacy open-world non-idempotent annotation.
- `browser_claim_tab`: requires `tab_id`; accepts `target`; maps to `browser_claim_tab`.
- `browser_move_mouse`: requires `tab_id`, `x`, and `y`; accepts `target` and `wait_for_arrival`; maps to `browser_move_mouse`.
- `browser_navigate`: requires `tab_id` and `url`; accepts `target`; preserves the legacy open-world idempotent annotation.
- `browser_input`: required `operation` with `click`, `type_text`, or `press_key`. All branches require `tab_id` and may accept `target`. `click` requires `x` and `y`; `type_text` requires `text`; `press_key` requires `key`. It must not accept eval expressions.
- `browser_scroll`: requires `tab_id`; accepts `target`, `delta_x`, `delta_y`, `x`, and `y`. At least one delta must be non-zero, and `x` and `y` must appear together. It preserves the legacy open-world stateful annotation.
- `browser_eval`: requires `tab_id` and `expression`; accepts `target`; is advertised only when browser eval is enabled and returns `FeatureDisabled` before parsing when the name is known but disabled.
- `phone_connection`: required `operation` with `connect`, `disconnect`, or `refresh`. `connect` accepts `serial`, `backend`, `install_companion`, and `start_scrcpy`. `disconnect` accepts phone session selectors and `keep_wireless`. `refresh` accepts phone session selectors only. Pairing codes are forbidden here.
- `phone_pair_wireless`: requires `host_port` and `pairing_code`. It accepts no session selectors and must never echo the pairing code in text, structured output, logs, or artifacts.
- `phone_setup`: required `operation` with `install_companion` or `open_settings`. `install_companion` accepts phone session selectors, `force_reinstall`, and `allow_downgrade`. `open_settings` requires `screen`, accepts `package_name` and phone session selectors, and supports exactly `accessibility`, `notification_access`, `overlay_permission`, `app_details`, `wireless_debugging`, and `battery_optimization`.
- `phone_pointer`: required `operation` with `tap` or `swipe`. `tap` requires `x` and `y`; `swipe` requires `start_x`, `start_y`, `end_x`, and `end_y`. Both accept phone session selectors, `phone_snapshot_id`, and `use_device_coordinates`; `swipe` also accepts `duration_ms`. `phone_snapshot_id` is required unless `use_device_coordinates=true`. If both are supplied, raw device coordinates win and the parser records that snapshot translation was intentionally bypassed; tests must pin this behavior. This tool must not advertise `backend`.
- `phone_keyboard`: required `operation` with `type_text` or `press_key`. `type_text` requires `text`; `press_key` requires `key`. Both accept phone session selectors and must not advertise `backend`.
- `phone_notification_action`: required `operation` with `open`, `dismiss`, or `action`. All branches require `event_id` and may accept phone session selectors; `action` also requires `action_id`.
- `phone_notification_reply`: requires `event_id`, `action_id`, and `text`; accepts phone session selectors. The `text` field is the legacy field name; `reply_text` is invalid.
- `phone_app_action`: required `operation` with `launch` or `open_intent`. `launch` requires `package_name`; `open_intent` requires `intent_uri` and may accept `package_name`. Both accept phone session selectors.
- `phone_app_force_stop`: requires `package_name`; accepts phone session selectors; maps to legacy `phone_app_force_stop`.
- `phone_app_install`: requires `apk_paths`; accepts phone session selectors, `mode` as `single`, `multiple`, or `multi_package`, `reinstall`, `allow_downgrade`, `allow_test_apk`, and `grant_runtime_permissions`.

## Milestones

Milestone 0 is baseline inventory, the `doctor` mutation audit, and the compact public contract. At the end of this milestone, the repository has executable fixtures that describe the current legacy registry, the intended compact contract, the exact public MCP projections, and the valid and invalid call objects for every compact branch before compact dispatch exists. Add `scripts/probe_mcp_tool_surface.py` to initialize a stdio MCP server, call `tools/list`, and write redacted registry artifacts without sorting the returned tools. Add fixtures such as `crates/sky-cua-client/tests/fixtures/compact_tool_contract.json`, `crates/sky-cua-client/tests/fixtures/mcp_tool_surface_matrix.json`, and `crates/sky-cua-client/tests/fixtures/compact_call_cases.json`. Add a generated or validated ledger at `docs/runtime/mcp-tool-surface.md`. Before accepting the compact contract, run `doctor` in an isolated integration environment with temporary `HOME`, XDG directories, runtime state, and before/after filesystem and service-state snapshots. A fake service may supplement this audit, but cannot replace it because it cannot reveal process-environment repair or filesystem side effects. The Milestone 0 exit condition is one of: `doctor` is proven read-only and its annotation is fixed; compact `doctor` receives a truthful mutating annotation; or mutation is split into a separately annotated compact tool. The proof is that the legacy fixture exactly matches current source output for the full profile, image-capability, and eval matrix; the compact contract has deterministic order and zero duplicate names; every compact branch has a minimal valid and invalid call case; the `doctor` annotation is resolved before host approval evidence; and the serialized-size report uses `serde_json::to_vec(&json!({"tools": tools}))?.len()` with no pretty printing and no probe-side sorting.

Milestone 1 is immutable process/session config and a schema-only compact registry. At the end of this milestone, `initialize` builds one active registry from `McpProcessConfig` and `ModelSessionInfo`, and `tools/list` and `tools/call` both consult the same registry entry set. Compact definitions are derived from the contract fixture and projected into `mcp_tool_surface_matrix.json`. Pre-handler compact names may return explicit not-yet-implemented errors only in local schema-spike builds; every advertised release entry must have a real handler before release. `legacy` accepts only legacy names, `compact` accepts only compact names, and non-advertised names fail before argument parsing or any `McpService` call. Browser eval policy is parsed before entering the stdio request loop, passed into legacy and compact browser dispatch, and snapshotted again in service startup config for defence in depth; no request path may call an environment-reading `browser_eval_enabled()` helper. The proof is Rust tests for parser behavior, registry snapshots, advertised-name equals callable-name invariants, zero-service-call rejection, concurrency with cloned sessions, eval policy consistency, image-capability consistency, second-initialize rejection, source-level or instrumentation proof that no request-time eval environment read remains, isolated subprocess tests for environment mutation after initialize, and `listChanged=false` restart semantics.

Milestone 2 is the host schema and approval spike. Before grouped compact handlers are treated as safe, create `scripts/live_mcp_schema_compat_smoke.py`. It must run against the first real compact registry schemas from Milestone 1, not a toy production tool. For Codex, OpenCode, Pi, Claude Code, Claude Desktop, and OpenClaw, record host name and version, raw `tools/list`, actual argument objects emitted by the host, injected blank/null/false/zero defaults, whether discriminators survive, whether irrelevant fields are sent, and approval behavior for all annotation tuples present in the current registry. The spike must transport one minimal valid call for every branch in `compact_call_cases.json` for every supported host and recorded version; argument fidelity is never not applicable for a supported host. Use isolated host configurations that actually honor annotations; if a host or mode always approves, record approval as not applicable rather than manufacturing a pass. Any annotation change after the `doctor` audit invalidates and reruns this evidence. If supported hosts flatten `oneOf`, lose `const`, or inject ambiguous defaults, split the affected compact tool before implementation continues.

Milestone 3 is compact read operations. Add `doctor`, `status`, `list_resources`, `observe`, `capture_screen`, `phone_accessibility_tree`, and `phone_notifications` to compact using the already resolved `doctor` annotation from Milestone 0. `doctor` remains broad and separate. `status` supports `component=browser`, `component=phone`, `component=phone_companion`, and `component=session_presence`. `list_resources` supports only `desktop/apps`, `desktop/windows`, `desktop/focused_window`, `browser/tabs`, `phone/devices`, `phone/apps`, and `phone/current_app`. `observe` maps only to desktop app state, browser snapshot, and phone observe; no option may switch it to the phone accessibility-tree response family. The proof is request parity tests, response-envelope tests, profile-aware prose linting, branch-field rejection tests, and direct stdio calls.

Milestone 4 is compact action operations. Add the smaller action tools by approval class and coordinate contract: `capture_desktop`, `setup_desktop`, `session_presence`, `activate_window`, `desktop_semantic`, `desktop_toggle`, `desktop_scroll`, `desktop_pointer`, `desktop_keyboard`, `desktop_action`, `desktop_set_value`, `browser_claim_tab`, `browser_move_mouse`, `browser_open`, `browser_navigate`, `browser_input`, `browser_scroll`, optional `browser_eval`, `phone_connection`, `phone_pair_wireless`, `phone_setup`, `phone_pointer`, `phone_keyboard`, `phone_notification_action`, `phone_notification_reply`, `phone_app_action`, `phone_app_force_stop`, and `phone_app_install`. Each tool must have an explicit required, optional, and forbidden field matrix. The proof is table-driven request mapping, annotation-set tests, parser rejection tests, no-secret-leak tests for pairing codes and notification replies, horizontal desktop scroll fixtures, phone snapshot missing/stale/mismatched negative fixtures, settings-screen fixtures, app-install mode fixtures, selector-conflict fixtures, and direct stdio action calls against safe targets.

Milestone 5 is installer, packaging, skills, and docs. Add explicit installer switches `--mcp-tool-profile legacy|compact`, `--browser-eval on|off`, and `--model-supports-images auto|true|false`. Persist host-specific launch policy with per-field precedence: explicit CLI flag, persisted per-host state, recognized installer environment, then defaults of legacy profile, eval off, and no image override. Recognized installer environment is closed to `SKY_CUA_MCP_TOOL_PROFILE`, `SKY_CUA_BROWSER_EVAL`, and `SKY_CUA_MODEL_SUPPORTS_IMAGES`. Partial overrides are allowed per field: for example, `--mcp-tool-profile compact` may override only the profile while eval and image policy come from persisted state, environment, or defaults. Before resolving or mutating anything, validate every present CLI value, persisted-state value, and recognized environment value for all launch-policy fields. Invalid CLI, invalid persisted state, unknown state version, malformed state, or invalid recognized environment fails before mutation and never falls back or writes repaired state. Generic installers write `<target-dir>/install-state.json`; Codex writes `<codex-home>/sky-cua-install-state.json` because the launched compat plugin can live in a plugin cache rather than the generic target directory. Missing state means defaults unless a valid CLI flag or recognized environment value supplies a field. `auto` for model image support omits `SKY_CUA_MODEL_SUPPORTS_IMAGES`; it must not serialize a guessed value. Pin `SKY_CUA_MCP_TOOL_PROFILE` in every generated launch path and forward `SKY_CUA_BROWSER_EVAL` according to the resolved launch policy. Implement a staged transaction: read and validate existing state; validate recognized environment; resolve complete per-host policy; snapshot affected config, wrapper, plugin materialization, and symlink entries; stage new outputs without touching live targets; atomically replace each owned output; verify the effective host-owned launched server and `tools/list`; write persisted state last; and restore every snapshot on failure. For symlinks, require `lstat`, never open a shared symlink target for in-place writing, and write a consumer-owned wrapper or atomically replace the symlink entry. Update `.mcp.json`, `scripts/install_mcp_server.py::generate_mcp_config`, `scripts/install_mcp_server.py::install_opencode`, `scripts/install_mcp_server.py::install_claude_desktop`, `scripts/install_mcp_server.py::install_claude_code`, `scripts/install_mcp_server.py::install_pi`, `scripts/install_mcp_server.py::install_local_mcp_server`, `scripts/_openclaw_install.py::install_openclaw`, `scripts/_openclaw_install.py::plan_openclaw_agent_codex_mcp_servers`, `scripts/_plugin_bundle.py::compat_plugin_targets_payload`, `scripts/_plugin_bundle.py::update_codex_config`, `scripts/install_plugin.py::install_bundle`, `scripts/installer.py::run_codex_phase`, `scripts/deploy_plugin.py::fast_deploy`, and `resources/chrome_preflight.py::sync_computer_use_compat_plugin`. Cover copied, channel-fallback, compat-plugin, and symlink installs without mutating a shared bundle through a symlink. Update bundled skills to be profile-aware: Use compact names when they are advertised; otherwise use the legacy section. The proof is Python installer tests for migration, no-flag preservation, per-field CLI override, persisted-over-env precedence, env-over-default precedence, invalid CLI values, invalid persisted values, invalid recognized environment values, malformed-state rejection, transaction fault injection after every boundary, rollback, unrelated-config preservation, symlink source-hash preservation, skill-reference linting, plugin build, staged-bundle inspection, and host-owned installed readback.

Milestone 6 is direct, agent-driven, and installed live proof. Update or add profile-aware live drivers for desktop, browser, phone, and OpenClaw. Each driver must accept `--mcp-tool-profile`, assert the exact advertised registry, call only names from that profile, verify consequential state with fresh observation, and fail on unknown names, schema errors, or hidden legacy calls. Add agent-driven tasks that discover tools from `tools/list` rather than hardcoding a hidden mapping: one desktop observe/action/observe loop, one browser claim/observe/input loop, and one phone connect/observe/action/observe loop. Installed proof must come from host-owned launch paths and cover at least one environment-config host, Pi's wrapper path, the active Codex compat or channel path, a live symlink launch lane, and a connected-phone compact workflow. The connected-phone proof must include a negative test where a missing or stale snapshot is rejected before dispatch. The proof is redacted artifacts under `artifacts/mcp-tool-surface/`, installed server readback under both profiles, and agent task transcripts showing compact tool choice without legacy calls.

Milestone 7 is retirement. Once opt-in compact has shipped with direct and installed proof while legacy remains default, create or update `docs/features/compact-mcp-tool-surface.md`, update relevant browser and phone feature docs, update `ROADMAP.md`, and delete this ExecPlan according to `plans/AGENTS.md`. A future fresh-install default switch must have a separate ROADMAP item and ExecPlan.

## Plan of Work

Start with fixtures and measurement, not prose. Build or add a direct stdio probe that initializes `sky-cua-client mcp`, calls `tools/list`, and writes JSON exactly in server order. Use it to create registry fixtures for all twelve combinations: two profiles, image support true/false/unknown, and eval on/off. The legacy half must match current source before compact code is added. The compact half begins as `compact_tool_contract.json`, not handler output, and must be reconciled with actual `tools/list` projections in `mcp_tool_surface_matrix.json` once Milestone 1 adds the schema-only registry. Add `compact_call_cases.json` with minimal valid and invalid requests for every branch in the Compact Tool Contract section. Add a serialized-size report that records tool count, full serialized `{"tools":[...]}` bytes, description bytes, largest `inputSchema` bytes, and duplicate-name count for each profile/image/eval combination. The byte calculation is exactly `serde_json::to_vec(&json!({"tools": tools}))?.len()`: no pretty printing, no probe-side sorting, and no reordering of the server's tool array. Compact must be no more than 65% of the comparable legacy serialized registry for each of the six matching image/eval pairs, and each compact `inputSchema` must be at or below 8192 serialized bytes. Relaxing either budget requires another plan review.

Add a profile module under `crates/sky-cua-client/src/mcp_tools/profile.rs`. Runtime MCP environment parsing accepts exact lowercase `legacy` and `compact` after trimming ASCII whitespace. Unknown or empty runtime values fall back to `legacy` for the server, with a stable tracing or stderr warning code `McpToolProfileInvalid`. Runtime eval parsing accepts only the current true spellings `on`, `1`, and `true`; false spellings `off`, `0`, `false`, and unset mean disabled; any other present value disables eval and emits a stable diagnostic such as `McpBrowserEvalInvalid`. Runtime image override parsing accepts only explicit true or false spellings; invalid present values mean no override and emit a stable diagnostic such as `McpModelSupportsImagesInvalid`. These fallback rules apply only to raw MCP process initialization. Installer CLI parsing, persisted-state parsing, and recognized installer environment parsing are strict and reject invalid values before writing configs. Do not alter the legacy `doctor` response just to report profile parsing diagnostics.

Replace ad hoc registry construction with an active session registry. Parse process-level policy once before entering the stdio request loop into `McpProcessConfig`. Parse model image capability once during `initialize`, build a `McpToolRegistry`, and store it in `ServerSession`. `tools/list` returns that registry. `tools/call` checks the same registry before parsing arguments. The registry is one ordered entry set containing the compact contract entry and a derived public definition; any lookup map is derived from that entry set at construction. Do not keep duplicate `name` or `annotations` values beside a free-form public definition that can disagree with them. A second `initialize` returns JSON-RPC `-32600` with `data.code="AlreadyInitialized"` and leaves the original session config and registry byte-for-byte unchanged. Registry builders, legacy browser dispatch, compact browser dispatch, and service request paths must not read `SKY_CUA_MCP_TOOL_PROFILE` or `SKY_CUA_BROWSER_EVAL` directly after startup.

Implement profile authorization before compact dispatch. Lookup precedence is: active registry entry; known policy-disabled feature such as eval-off `browser_eval`; known name from the other profile; then unknown name. A request for a known but inactive-profile tool returns a tool-level error such as `ToolNotInActiveProfile`, with `isError=true`, `active_profile`, and `tool_name`. A completely unknown tool remains `UnknownTool`. A policy-disabled tool returns `FeatureDisabled`. These errors happen before service calls. `AlreadyInitialized` and `NotInitialized` are protocol errors, not tool result objects.

Build compact modules as a directory, not a single god file. The expected layout is `crates/sky-cua-client/src/mcp_tools/compact/mod.rs`, `registry.rs`, `read.rs`, `desktop.rs`, `browser.rs`, `phone.rs`, `args.rs`, and `response.rs`. Keep shared profile authorization and registry policy outside surface-specific dispatch files.

Define compact response envelopes. Legacy direct tools keep their current `structuredContent` roots. Compact tools return stable wrappers so logs and future clients can know which branch produced a result. Every branch in `compact_tool_contract.json` declares `content_policy`, `structured_policy`, and `normalization_json_pointers`. If `content_policy` is `exact`, outer `content` ordering and block types are copied from the underlying legacy-shaped result. If it is `profile_rewrite`, only deterministic tool-name guidance may change; diagnostics, user data, paths, and observed state must not change. Outer `isError` is copied. If `structured_policy` is `exact`, `structuredContent.result` deep-equals the underlying structured payload. If it is `normalized`, only the listed JSON pointers may differ. Image-capable responses attach an image block at most once and must not duplicate base64 in `structuredContent`. For surface tools, use `structuredContent.surface`, `structuredContent.operation`, and `structuredContent.result`. For component tools, use `structuredContent.component`, `structuredContent.operation`, and `structuredContent.result`; `doctor` uses `component="system"` and `operation="doctor"`, and session-presence actions use `component="session_presence"`. Compact authorization and parser errors use a pinned object with `code`, `active_profile`, and `tool_name` when a tool name is involved, and they do not masquerade as successful results.

Implement compact reads first. Keep `doctor` separate. Add `status` with component branches for browser status, phone status, phone companion status, and session-presence status. Add `list_resources` with only the bounded discovery and current-resource branches in the Compact Tool Contract section. Add `observe` for desktop app state, browser snapshot, and phone observe. Add `capture_screen` for browser and phone visual capture. Add `phone_accessibility_tree` as its own read tool. Add `phone_notifications` as the dedicated focused notification read tool. Reject unsupported surfaces or irrelevant fields before service dispatch.

Implement compact actions by approval class. `session_presence` contains only mutating operations: `operation=hold`, `operation=unlock`, and `operation=release`. `operation=hold` allows `unlock`, `inhibit_lock`, and `inhibit_suspend` with defaults `false`, `true`, and `true`. `operation=unlock` allows `inhibit_lock` and `inhibit_suspend` and forces unlock true. `operation=release` allows `relock`, default false. Status is only `status(component="session_presence")`. `phone_connection` contains `connect`, `disconnect`, and `refresh`; `phone_pair_wireless` stays separate because it handles one-time pairing codes. `desktop_pointer` contains click, secondary click, and drag; `desktop_keyboard` contains type text and press key; `desktop_action` contains activate element and perform action; `desktop_set_value` stays separate. `phone_pointer` contains tap and swipe; `phone_keyboard` contains type text and press key.

Audit annotations without changing behavior casually. Add an annotation-audit column to the machine mapping or generated ledger with values `verified`, `legacy_grandfathered`, or `requires_runtime_proof`. For this refactor, preserve existing annotation tuples unless a test proves a mismatch. Compact tools cannot use a known false annotation; if a compact branch would combine mismatched or false tuples, split the tool or assign the truthful worst tuple. Legacy-only grandfathering is a compatibility note, not permission for compact to lie.

Make skills and docs profile-aware. During the opt-in release, skills must say: "Use compact names when they are advertised; otherwise use the legacy mapping." They must not unconditionally teach compact first while installers still default to legacy. Add a lint that checks unconditional tool references against the profile where they are used and checks compact prose for hidden legacy-only names.

Fix the phone smoke issues before compact phone parity counts. Update `scripts/live_phone_use_smoke.py` to send `text` for notification replies, detect `supports_inline_reply`, and accept or derive the actual wireless connection-kind values. Add unit tests around those helper decisions. Legacy installed phone `tools/list` and the full direct phone driver must pass before compact phone parity can close.

Finally, prove installed behavior. Direct source tests are not enough. Build and install through real host launch paths, read back the installed `tools/list`, and run profile-aware live workflows. Required installed lanes are one environment-config host, Pi's wrapper path, Codex compat or channel materialization, a live symlink launch lane, and a connected-phone compact path. Preserve redacted evidence under `artifacts/mcp-tool-surface/`, excluding pairing codes, notification bodies, reply text, private browser tab contents, and sensitive screenshots.

## Concrete Steps

Run commands from the repository root, the directory containing `Cargo.toml`.

First, capture current status and build the release client used by stdio probes:

    git status --short
    cargo build --release -p sky-cua-client

Create `scripts/probe_mcp_tool_surface.py`, then record the legacy baselines:

    uv run python scripts/probe_mcp_tool_surface.py \
      --client ./target/release/sky-cua-client \
      --profile legacy \
      --output artifacts/mcp-tool-surface/registry/legacy-default.json

    SKY_CUA_BROWSER_EVAL=on uv run python scripts/probe_mcp_tool_surface.py \
      --client ./target/release/sky-cua-client \
      --profile legacy \
      --output artifacts/mcp-tool-surface/registry/legacy-eval.json

Expected baseline behavior is 66 tools without eval and 67 tools with eval. The probe should also support image-capability cases:

    SKY_CUA_MODEL_SUPPORTS_IMAGES=false uv run python scripts/probe_mcp_tool_surface.py \
      --client ./target/release/sky-cua-client \
      --profile legacy \
      --output artifacts/mcp-tool-surface/registry/legacy-text-only.json

After fixtures and registry policy are added, run:

    cargo test -p sky-cua-client doctor_mutation_audit
    cargo test -p sky-cua-client compact_tool_contract_fixture
    cargo test -p sky-cua-client mcp_tool_surface_matrix_fixture
    cargo test -p sky-cua-client compact_call_case_fixtures
    cargo test -p sky-cua-client mcp_tool_profile
    cargo test -p sky-cua-client mcp_registry_snapshots
    cargo test -p sky-cua-client advertised_tools_are_callable_tools
    cargo test -p sky-cua-client mcp_session_config_is_immutable
    cargo test -p sky-cua-client second_initialize_is_rejected
    cargo test -p sky-cua-client mcp_registry_lookup_priority
    cargo test -p sky-cua-client no_request_time_browser_eval_env_reads

Expected behavior at this stage:

    unset profile -> legacy registry
    invalid profile -> legacy registry plus McpToolProfileInvalid warning
    SKY_CUA_MCP_TOOL_PROFILE=legacy -> legacy names only
    SKY_CUA_MCP_TOOL_PROFILE=compact -> compact names only
    browser_eval absent when eval is off and present only when eval is on
    a legacy-only tool called in compact returns ToolNotInActiveProfile and records zero service calls
    a compact-only tool called in legacy returns ToolNotInActiveProfile and records zero service calls
    browser_eval with eval off returns FeatureDisabled before compact argument parsing
    a truly unknown name returns UnknownTool
    mutating profile, eval, or image-capability environment after initialize does not change tools/list or call behavior
    a second initialize returns JSON-RPC -32600 with data.code AlreadyInitialized and leaves the original registry byte-for-byte unchanged
    no notifications/tools/list_changed event is emitted

Before relying on grouped schemas, run the schema compatibility spike:

    uv run python scripts/live_mcp_schema_compat_smoke.py --host codex --profile compact --isolated-config --approval-matrix all
    uv run python scripts/live_mcp_schema_compat_smoke.py --host opencode --profile compact --isolated-config --approval-matrix all
    uv run python scripts/live_mcp_schema_compat_smoke.py --host pi --profile compact --isolated-config --approval-matrix all
    uv run python scripts/live_mcp_schema_compat_smoke.py --host claude-code --profile compact --isolated-config --approval-matrix all
    uv run python scripts/live_mcp_schema_compat_smoke.py --host claude-desktop --profile compact --isolated-config --approval-matrix all
    uv run python scripts/live_mcp_schema_compat_smoke.py --host openclaw --profile compact --isolated-config --approval-matrix all

The spike must write artifacts under `artifacts/mcp-tool-surface/schema/<host>/` containing the raw registry, the emitted argument objects for every branch in `compact_call_cases.json`, and approval observations for every annotation tuple in `crates/sky-cua-client/src/mcp_tools/annotations.rs` plus any custom tuple in the registry. If a host cannot preserve discriminators, emits irrelevant branch fields, injects ambiguous defaults, or cannot produce annotation evidence because the tested mode auto-approves everything, split the affected compact tool or mark approval not applicable before continuing. Argument fidelity cannot be marked not applicable for a supported host.

After compact read tools are added, run focused tests and direct stdio calls:

    cargo test -p sky-cua-client compact_read_tools
    cargo test -p sky-cua-client compact_response_envelopes
    cargo test -p sky-cua-client compact_prose_does_not_reference_hidden_legacy_tools
    cargo test -p sky-cua-client compact_capture_surface_mapping

Direct examples:

    uv run python scripts/probe_mcp_tool_surface.py call --client ./target/release/sky-cua-client --profile compact --tool doctor --arguments '{}'
    uv run python scripts/probe_mcp_tool_surface.py call --client ./target/release/sky-cua-client --profile compact --tool status --arguments '{"component":"browser"}'
    uv run python scripts/probe_mcp_tool_surface.py call --client ./target/release/sky-cua-client --profile compact --tool status --arguments '{"component":"session_presence"}'
    uv run python scripts/probe_mcp_tool_surface.py call --client ./target/release/sky-cua-client --profile compact --tool list_resources --arguments '{"surface":"browser","resource":"tabs"}'
    uv run python scripts/probe_mcp_tool_surface.py call --client ./target/release/sky-cua-client --profile compact --tool observe --arguments '{"surface":"phone","include_notifications":true}'
    uv run python scripts/probe_mcp_tool_surface.py call --client ./target/release/sky-cua-client --profile compact --tool phone_notifications --arguments '{"limit":5}'

After compact action tools are added, run focused parity and parser tests:

    cargo test -p sky-cua-client compact_action_tools
    cargo test -p sky-cua-client compact_operation_field_matrix
    cargo test -p sky-cua-client every_compact_tool_has_one_annotation_class
    cargo test -p sky-cua-client compact_secrets_are_not_echoed
    cargo test -p sky-cua-client compact_response_parity
    cargo test -p sky-cua-client legacy_dispatch_regression
    cargo test -p sky-cua-client compact_horizontal_scroll_parity
    cargo test -p sky-cua-client compact_phone_snapshot_rejection

After installer and script changes, run Python checks:

    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest
    cargo test -p sky-cua-client mcp_runtime_config_invalid_values_fallback
    uv run pytest scripts/test_mcp_tool_profile_install_state.py
    uv run pytest scripts/test_mcp_launch_policy_precedence.py
    uv run pytest scripts/test_mcp_launch_policy_invalid_values.py
    uv run pytest scripts/test_mcp_launch_policy_transaction.py

After docs, skills, and packaging metadata change, run:

    python3 scripts/build_plugin.py

Inspect the staged bundle and generated install outputs for profile-bearing configs, Pi wrappers, skills, docs, and manifests. The inspection must include `.mcp.json`, generated host config snippets, Pi wrapper contents, bundled skills, any plugin manifest that controls MCP environment forwarding, the Codex materialized compat or channel plugin output produced by `resources/chrome_preflight.py::sync_computer_use_compat_plugin`, the path exercised by `scripts/deploy_plugin.py::fast_deploy`, and any symlink install mode. It must prove `SKY_CUA_MCP_TOOL_PROFILE` is present, `SKY_CUA_BROWSER_EVAL` is forwarded according to persisted launch policy, and `SKY_CUA_MODEL_SUPPORTS_IMAGES` is absent when `--model-supports-images auto` is used.

Then run installed proof for at least OpenClaw as the environment-config host:

    python3 scripts/install_mcp_server.py \
      --target-dir ~/.local/share/sky-cua \
      --host openclaw \
      --bin-dir ~/.local/bin \
      --mcp-tool-profile compact \
      --restart-runtime

Read back `tools/list` from the installed server and confirm compact names are advertised and legacy names are rejected as inactive-profile calls. Repeat an installed legacy readback after reinstalling or repinning `--mcp-tool-profile legacy`.
Also read back from Pi's wrapper path, from Codex compat or channel materialization, and from a symlink launch lane. At least one installed lane must prove eval off and eval on, with `browser_eval` absent in the former and present plus callable in the latter. This evidence must come from the host-owned launch path, not from manually invoking the installed binary with a convenient environment.

For live-smoke acceptance, run profile-aware drivers rather than the current legacy-only commands:

    uv run python scripts/live_desktop_smoke.py --mcp-tool-profile legacy
    uv run python scripts/live_desktop_smoke.py --mcp-tool-profile compact
    uv run python scripts/live_browser_mcp_smoke.py --mcp-tool-profile legacy
    uv run python scripts/live_browser_mcp_smoke.py --mcp-tool-profile compact
    uv run python scripts/live_phone_use_smoke.py --mcp-tool-profile legacy
    uv run python scripts/live_phone_use_smoke.py --mcp-tool-profile compact
    uv run python scripts/live_openclaw_mcp_smoke.py --mcp-tool-profile compact
    uv run python scripts/live_agent_tool_choice_smoke.py --task desktop --mcp-tool-profile compact
    uv run python scripts/live_agent_tool_choice_smoke.py --task browser --mcp-tool-profile compact
    uv run python scripts/live_agent_tool_choice_smoke.py --task phone --mcp-tool-profile compact
    uv run python scripts/live_phone_use_smoke.py --mcp-tool-profile compact --negative stale-snapshot

If no phone is connected, record that as an environment limitation and do not count compact phone parity as proven. If a compact call is rejected because of an unknown tool, hidden legacy call, or schema mismatch, that is a hard failure.

Final checks before retiring the plan:

    cargo fmt --check
    cargo test
    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest
    python3 scripts/build_plugin.py

If the user asks to run all smoke tests after implementation, run the full all profile exactly as the root guide specifies:

    python3 scripts/run_gui_testing_vm_smoke.py --profile all

## Validation and Acceptance

The first acceptance gate is exact registry and contract behavior. `legacy` preserves current public names, annotations, image-capability schema differences, and browser eval hiding behavior. `compact` advertises only compact names, with every tool annotated according to the worst accepted branch. There are no duplicate tool names. `compact_tool_contract.json` pins ordered compact entries and branch metadata. `mcp_tool_surface_matrix.json` pins each public `{"tools":[...]}` projection, in exact server order, for all profile, image-capability, and eval combinations. The serialized-size report shows compact is materially smaller than legacy using `serde_json::to_vec(&json!({"tools": tools}))?.len()` with no pretty printing and no probe-side sorting.

The second acceptance gate is authorization. A tool name is callable if and only if it is present in the active session registry. Inactive-profile names fail with `ToolNotInActiveProfile` before argument parsing or service calls. Unknown names remain `UnknownTool`. Disabled eval returns `FeatureDisabled`. Tests must prove rejected calls record zero service requests.

The third acceptance gate is immutable session policy. Profile, browser eval enablement, model image capability, and the active registry are frozen at initialization. The server continues to advertise `listChanged=false`. Changing policy requires process restart and a new MCP session. Tests cover cloned concurrent sessions, environment mutation after initialize in isolated subprocesses, no request-time eval environment reads, service-side eval startup snapshots, and second-initialize rejection with JSON-RPC `-32600` and `data.code="AlreadyInitialized"` that leaves the original registry byte-for-byte unchanged and emits no `notifications/tools/list_changed`.

The fourth acceptance gate is request and response parity. For every compact operation that replaces a legacy tool, at least one table-driven test proves that the compact call builds the same `ServiceRequest`, nested `BrowserRequest`, nested `PhoneRequest`, or desktop `ActionName` as the predecessor. A paired response test proves that `isError` is preserved, content block ordering and block types are preserved unless `content_policy=profile_rewrite`, `structuredContent.result` deep-equals the legacy structured payload unless `structured_policy=normalized`, only listed JSON pointers differ for normalized branches, and image blocks appear exactly once. Profile rewrites may change only deterministic tool-name guidance, never diagnostics or user data. Actual returned text, including dynamic failure and text-only-image paths, contains no inactive-profile tool names. Legacy direct calls retain old structured response roots. Compact calls use the compact envelope with nested result data.

The fifth acceptance gate is approval safety. The `doctor` mutation audit has run before compact contract acceptance. A compact public tool may contain multiple operations only when every operation shares the same annotation tuple. The annotation test fails if a future branch mixes read-only, local navigation, local stateful, local destructive, or open-world behavior incorrectly. The annotation audit records whether each inherited tuple is verified, legacy-grandfathered, or requires runtime proof. Compact tools may not use a known false read-only annotation.

The sixth acceptance gate is schema and parser safety. Branch-specific wrong fields are rejected before service dispatch. Examples include a browser `tab_id` passed to `phone_pointer`, a phone `session_id` passed to `browser_input`, a `phone_snapshot_id` passed to desktop pointer input, an eval expression attempted through `browser_input`, pairing-code fields passed to ordinary `phone_connection`, backend fields passed to phone branches that cannot honor them, missing or stale phone snapshots when `use_device_coordinates` is false, and status fields passed to mutating `session_presence`.

The capture mapping gate is explicit because desktop, browser, and phone captures have different trust and focus behavior. Legacy `browser_screenshot` maps only to `capture_screen(surface="browser")`, legacy `phone_screenshot` maps only to `capture_screen(surface="phone")`, and legacy desktop `screenshot` maps only to `capture_desktop`. Tests fail if a browser or phone capture branch is routed through `capture_desktop`.

The seventh acceptance gate is image-capability safety. `supports_images=None` continues to mean image-capable in this refactor. Image-capable, text-only, and unknown-capability sessions are all tested. Capture tools remain advertised in text-only sessions and return paths plus metadata rather than inline image blocks. Image-capable requests attach at most one image block and do not duplicate base64 in `structuredContent`.

The eighth acceptance gate is installer persistence. Every generated host launch path pins `SKY_CUA_MCP_TOOL_PROFILE` explicitly. Launch policy precedence is per field: explicit CLI flag, persisted per-host state, recognized installer environment, then defaults of legacy profile, eval off, and no image override. Partial CLI overrides are valid and affect only the supplied fields. Persisted state wins over environment for the same field; environment is used only when CLI and persisted state omit that field. Before any mutation, installers validate every present CLI, persisted-state, and recognized environment value for profile, browser eval, and image override. Invalid values fail hard and do not fall back, do not repair state, and do not partially write outputs. Existing unpinned installs are treated as missing state, therefore defaults unless valid CLI or environment supplies a field. A no-flag installer run preserves existing state; an explicit flag updates state only after successful materialization. Re-running installers is idempotent, staged, rollback-safe, and preserves unrelated host config. Pi wrappers export the selected profile because Pi MCP JSON cannot express env fields. Codex stores profile state outside the generic target directory and proves the materialized compat or channel plugin launch environment. `--model-supports-images auto` omits `SKY_CUA_MODEL_SUPPORTS_IMAGES`; only explicit true/false forwards it. Malformed or unknown-version state fails before mutation, and fault-injection tests prove rollback after every transaction boundary.

The ninth acceptance gate is skills and docs. Bundled skills are profile-aware and never unconditionally recommend a tool absent from the active profile. Compact descriptions and representative compact result text do not teach hidden legacy-only names. Feature docs describe opt-in compact, legacy fallback, installer profile selection, and the fact that future default switching is separate.

The tenth acceptance gate is installed behavior. A built and installed MCP server advertises compact tools under compact and legacy tools under legacy through materially different launch mechanisms: one environment-config host, Pi's wrapper path, Codex compat or channel materialization, a live symlink lane, and a connected-phone compact path. Direct source tests are not enough. Installed readback must prove both advertisement and inactive-profile rejection, and at least one installed lane must prove eval-off and eval-on behavior. Evidence must come from host-owned launch paths.

The eleventh acceptance gate is schema and size budget. Compact total serialized registry bytes must be at or below 65% of comparable legacy for every matching image/eval pair, and every compact `inputSchema` must be at or below 8192 serialized bytes. The fixture probe must fail on duplicate names, order drift, missing matrix cases, or schema-size drift. Relaxing either budget requires another plan review.

The final acceptance gate is live workflows and agent usability. A human or agent can operate at least one desktop app, one real browser tab, and one connected phone through compact names, then verify the result with fresh compact observation calls. At least one task per surface must choose tools from the advertised compact registry rather than from a hidden hardcoded mapping. Legacy workflows still work through legacy names. Compact live proof must not use hidden legacy tools.

## Idempotence and Recovery

All work should be additive until compact is proven. Re-running tests, probes, builds, and installs should be safe. If a compact profile test fails, pin or reinstall `SKY_CUA_MCP_TOOL_PROFILE=legacy` to recover the old advertised and callable surface without reverting source.

Do not delete legacy tool implementations in this plan. The reliable compatibility mechanism is the `legacy` profile, not hidden direct-call aliases. If a host caches MCP tool lists, restart or reload that host before judging profile behavior. For OpenClaw, run `openclaw mcp reload` after changing the installed MCP profile, then start a fresh agent session because reused sessions can retain stale tool state. For Codex Desktop and other hosts, use the existing install and deploy scripts rather than editing host config by hand.

If a host cannot handle a compact schema shape such as discriminated unions, do not force that schema. Split the compact tool more finely or move exact branch enforcement into Rust parsing while keeping host-visible schemas simple. Record the evidence under `artifacts/mcp-tool-surface/schema/<host>/` and update the Decision Log.

If the phone lane skips because no device is connected, record it as an environment limitation, not feature proof. If pairing or notification tests run, redact one-time codes, notification bodies, reply text, private browser tab content, and sensitive screenshots from stored artifacts.

Installer recovery is launch-policy pinning. Existing installs with no persisted policy use the per-field resolver: CLI flag, persisted host state, recognized installer environment, then defaults of legacy profile, browser eval off, and no image-capability override. Generic hosts read and write `<target-dir>/install-state.json`; Codex reads and writes `<codex-home>/sky-cua-install-state.json`. Config-writing surfaces are strict: invalid CLI values, invalid persisted values, invalid recognized environment values, malformed state, and unknown state versions fail before mutation. Raw MCP runtime startup is the only layer allowed to fall back from invalid environment values to legacy/eval-off/no-image-override diagnostics. The minimum state shape is:

    {
      "version": 1,
      "hosts": {
        "codex": {
          "mcp_tool_profile": "compact",
          "browser_eval_enabled": false,
          "model_supports_images_override": null
        },
        "pi": {
          "mcp_tool_profile": "legacy",
          "browser_eval_enabled": false,
          "model_supports_images_override": null
        }
      }
    }

Explicit operator changes through `--mcp-tool-profile`, `--browser-eval`, and `--model-supports-images` update persisted state only after host config materialization and host-owned readback succeed. Valid recognized installer environment values can fill omitted fields, but do not override CLI or persisted host state. `model_supports_images_override: null` means auto and omits `SKY_CUA_MODEL_SUPPORTS_IMAGES`. Config-writing tests must cover partial CLI override, persisted-over-env conflict, env-over-default fill, invalid persisted state, and invalid recognized environment. Invalid environment values inside a raw server launch warn and fall back to legacy for profile, eval off for browser eval, and negotiated model capability for images.

## Artifacts and Notes

Initial inventory evidence:

    default tools/list count: 66
    eval-on tools/list count: 67
    phone family count: 27
    browser family count: 12 default, 13 with eval
    desktop observe/setup/window count: 9
    session presence count: 4
    desktop action/semantic count: 14

Expected generated artifacts:

    artifacts/mcp-tool-surface/registry/legacy-default.json
    artifacts/mcp-tool-surface/registry/legacy-eval.json
    artifacts/mcp-tool-surface/registry/legacy-text-only.json
    artifacts/mcp-tool-surface/registry/legacy-text-only-eval.json
    artifacts/mcp-tool-surface/registry/legacy-unknown-image.json
    artifacts/mcp-tool-surface/registry/legacy-unknown-image-eval.json
    artifacts/mcp-tool-surface/registry/compact-default.json
    artifacts/mcp-tool-surface/registry/compact-eval.json
    artifacts/mcp-tool-surface/registry/compact-text-only.json
    artifacts/mcp-tool-surface/registry/compact-text-only-eval.json
    artifacts/mcp-tool-surface/registry/compact-unknown-image.json
    artifacts/mcp-tool-surface/registry/compact-unknown-image-eval.json
    artifacts/mcp-tool-surface/registry/serialized-size-report.json
    artifacts/mcp-tool-surface/schema/<host>/
    artifacts/mcp-tool-surface/install/environment-config-host/
    artifacts/mcp-tool-surface/install/pi-wrapper/
    artifacts/mcp-tool-surface/install/codex-materialized/
    artifacts/mcp-tool-surface/install/symlink-launch/
    artifacts/mcp-tool-surface/install/phone-compact/
    artifacts/mcp-tool-surface/live/desktop/
    artifacts/mcp-tool-surface/live/browser/
    artifacts/mcp-tool-surface/live/phone/
    artifacts/mcp-tool-surface/live/agent-tool-choice/

Authoritative fixtures and docs:

    crates/sky-cua-client/tests/fixtures/compact_tool_contract.json
    crates/sky-cua-client/tests/fixtures/mcp_tool_surface_matrix.json
    crates/sky-cua-client/tests/fixtures/compact_call_cases.json
    docs/runtime/mcp-tool-surface.md
    docs/features/compact-mcp-tool-surface.md

External Oracle review was used as advisory design input, not as reproducible acceptance evidence. Acceptance evidence must come from repository fixtures, tests, generated artifacts, installed readback, and live smokes.

## Interfaces and Dependencies

Add profile and registry policy under `crates/sky-cua-client/src/mcp_tools/`.

The expected profile types are:

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    enum McpToolProfile {
        Legacy,
        Compact,
    }

    struct McpProfileParseOutcome {
        profile: McpToolProfile,
        diagnostic: Option<McpConfigDiagnostic>,
    }

    impl McpToolProfile {
        fn as_str(self) -> &'static str;
    }

    fn parse_mcp_tool_profile_runtime(value: Option<&str>) -> McpProfileParseOutcome;
    fn parse_mcp_tool_profile_strict(value: &str) -> Result<McpToolProfile, McpConfigError>;
    fn parse_mcp_process_config_from_env(env: &dyn McpConfigEnv) -> McpProcessConfig;

The runtime profile parser trims ASCII whitespace and accepts exact lowercase `legacy` and `compact`. Invalid runtime values return `Legacy` plus `McpToolProfileInvalid`. Strict profile parsing is used by installers, persisted state, and recognized installer environment; it rejects invalid values before writing configs. Eval and image-override parsers need the same split: raw MCP process environment parsing is lenient and diagnostic, while config-writing parsing is strict.

The expected session configuration types are:

    #[derive(Debug, Clone)]
    struct McpProcessConfig {
        profile: McpToolProfile,
        browser_eval_enabled: bool,
        model_supports_images_override: Option<bool>,
        diagnostics: Vec<McpConfigDiagnostic>,
    }

    #[derive(Debug, Clone)]
    struct ServiceProcessConfig {
        browser_eval_enabled: bool,
    }

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    enum LaunchPolicyField {
        McpToolProfile,
        BrowserEval,
        ModelSupportsImages,
    }

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    enum LaunchPolicySource {
        Cli,
        PersistedHostState,
        InstallerEnvironment,
        Defaults,
    }

    #[derive(Debug, Clone)]
    struct McpSessionConfig {
        process: std::sync::Arc<McpProcessConfig>,
        model: ModelSessionInfo,
        registry: std::sync::Arc<McpToolRegistry>,
    }

    #[derive(Debug, Default)]
    struct ServerSession {
        config: Option<std::sync::Arc<McpSessionConfig>>,
    }

    #[derive(Debug, Clone)]
    struct McpToolRegistry {
        profile: McpToolProfile,
        entries: Vec<McpToolEntry>,
        name_index: std::collections::BTreeMap<String, usize>,
        inactive_names: std::collections::BTreeMap<String, InactiveToolReason>,
    }

    #[derive(Debug, Clone)]
    struct McpToolEntry {
        contract: McpToolContract,
        handler: McpToolHandlerId,
    }

    impl McpToolEntry {
        fn name(&self) -> &str;
        fn public_definition(&self) -> serde_json::Value;
    }

The exact type names may change if a better local pattern exists, but the behavior must not change: one registry is built during initialize and passed to both `tools/list` and `tools/call`. `entries` is the only source of truth for advertised and callable tools. Public `tools/list` definitions are derived from the contract by `public_definition()`; the entry must not independently own both `contract.name` and `definition.name`, or both `contract.annotations` and `definition.annotations`. `name_index` is derived from `entries` during registry construction and exists only for efficient lookup. `inactive_names` is a separate catalogue for better `ToolNotInActiveProfile` and `FeatureDisabled` diagnostics; it is policy-derived and not a callable-name source. Registry builders receive explicit profile, eval, and image-capability arguments and do not read environment variables.

The expected public functions are conceptually:

    fn build_tool_registry(
        process: &McpProcessConfig,
        model: &ModelSessionInfo,
    ) -> anyhow::Result<McpToolRegistry>;
    fn tools_list_result(registry: &McpToolRegistry) -> serde_json::Value;
    fn handle_tool_call(
        service: &impl McpService,
        heuristics: &HeuristicsRegistry,
        session: &McpSessionConfig,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value>;

Registry projection tests must assert:

    tools/list == registry.entries.map(McpToolEntry::public_definition)
    registry.name_index == index derived from registry.entries
    registry.inactive_names == policy-derived names only
    every advertised release entry has a real handler

The compact module layout should be:

    crates/sky-cua-client/src/mcp_tools/compact/mod.rs
    crates/sky-cua-client/src/mcp_tools/compact/registry.rs
    crates/sky-cua-client/src/mcp_tools/compact/read.rs
    crates/sky-cua-client/src/mcp_tools/compact/desktop.rs
    crates/sky-cua-client/src/mcp_tools/compact/browser.rs
    crates/sky-cua-client/src/mcp_tools/compact/phone.rs
    crates/sky-cua-client/src/mcp_tools/compact/args.rs
    crates/sky-cua-client/src/mcp_tools/compact/response.rs

The compact response envelope is:

    {
      "content": [{"type": "text", "text": "..."}],
      "structuredContent": {
        "surface": "browser",
        "operation": "snapshot",
        "result": {}
      },
      "isError": false
    }

For status:

    {
      "content": [{"type": "text", "text": "..."}],
      "structuredContent": {
        "component": "session_presence",
        "operation": "status",
        "result": {}
      },
      "isError": false
    }

The JSON-RPC protocol error taxonomy must include:

    AlreadyInitialized
    NotInitialized

The tool result error taxonomy must include:

    UnknownTool
    ToolNotInActiveProfile
    FeatureDisabled
    InvalidRequest

Keep existing service interfaces:

- `sky_cua_platform::model::ServiceRequest`
- `sky_cua_platform::model::browser::BrowserRequest`
- `sky_cua_platform::model::phone::PhoneRequest`
- `sky_cua_platform::model::ActionName`

Do not rename, remove, or flatten those platform model enums as part of this plan. Compact MCP is an adapter above them.

At completion, the mapping should include:

    legacy doctor -> compact doctor
    legacy browser_status -> compact status(component="browser")
    legacy phone_status -> compact status(component="phone")
    legacy phone_companion_status -> compact status(component="phone_companion")
    legacy session_presence_status -> compact status(component="session_presence")
    legacy list_apps -> compact list_resources(surface="desktop", resource="apps")
    legacy list_windows -> compact list_resources(surface="desktop", resource="windows")
    legacy focused_window -> compact list_resources(surface="desktop", resource="focused_window")
    legacy browser_list_tabs -> compact list_resources(surface="browser", resource="tabs")
    legacy phone_list_devices -> compact list_resources(surface="phone", resource="devices")
    legacy phone_app_list -> compact list_resources(surface="phone", resource="apps")
    legacy phone_app_current -> compact list_resources(surface="phone", resource="current_app")
    legacy get_app_state -> compact observe(surface="desktop")
    legacy browser_snapshot -> compact observe(surface="browser")
    legacy phone_observe -> compact observe(surface="phone")
    legacy browser_screenshot -> compact capture_screen(surface="browser")
    legacy phone_screenshot -> compact capture_screen(surface="phone")
    legacy screenshot -> compact capture_desktop
    legacy setup_accessibility/setup_window_targeting -> compact setup_desktop
    legacy hold_session/unlock_session/release_session -> compact session_presence
    legacy focus_element/select_element/expand_element/collapse_element -> compact desktop_semantic
    legacy toggle_element -> compact desktop_toggle
    legacy scroll -> compact desktop_scroll
    legacy click/perform_secondary_action/drag -> compact desktop_pointer
    legacy type_text/press_key -> compact desktop_keyboard
    legacy activate_element/perform_action -> compact desktop_action
    legacy set_value -> compact desktop_set_value
    legacy browser_open -> compact browser_open
    legacy browser_claim_tab -> compact browser_claim_tab
    legacy browser_move_mouse -> compact browser_move_mouse
    legacy browser_navigate -> compact browser_navigate
    legacy browser_scroll -> compact browser_scroll
    legacy browser_eval -> compact browser_eval, only when eval is enabled
    legacy browser_click/browser_type_text/browser_press_key -> compact browser_input
    legacy phone_connect/phone_disconnect/phone_refresh_capabilities -> compact phone_connection
    legacy phone_pair_wireless -> compact phone_pair_wireless
    legacy phone_install_companion/phone_open_settings -> compact phone_setup
    legacy phone_tap/phone_swipe -> compact phone_pointer
    legacy phone_type_text/phone_press_key -> compact phone_keyboard
    legacy phone_notification_open/phone_notification_dismiss/phone_notification_action -> compact phone_notification_action
    legacy phone_notification_reply -> compact phone_notification_reply
    legacy phone_app_launch/phone_app_open_intent -> compact phone_app_action
    legacy phone_app_force_stop -> compact phone_app_force_stop
    legacy phone_app_install -> compact phone_app_install
    legacy phone_notifications -> compact phone_notifications
    legacy phone_accessibility_tree -> compact phone_accessibility_tree

## Revision Notes

- 2026-06-21: Initial ExecPlan created to capture the compact MCP tool surface refactor. The plan incorporated the source inventory and first Oracle design review, chose an approval-domain-first facade, and recorded migration and validation gates.
- 2026-06-21: Revised the ExecPlan after a second Oracle review. The revision removes production `core` and `compat`, forbids hidden inactive-profile calls, freezes registry policy at session initialization, keeps legacy fallback and installer-pinned profiles, separates `doctor`, restricts resource and notification fan-in, splits broad desktop and phone input tools, makes machine fixtures authoritative, adds schema and live-proof milestones, and moves any future default switch to a separate plan.
- 2026-06-21: Added exact installer, OpenClaw, and Codex materialization function seams after inspecting current source; at that point the Oracle rereview was still pending because browser and API execution were intentionally constrained.
- 2026-06-22: Ran one browser Oracle rereview and revised the ExecPlan to address the remaining blockers. The revision adds a Compact Tool Contract section with branch-level fields, requires compact call-case fixtures before handler work, reorders schema-proof work behind a schema-only registry milestone, changes the registry sketch to one ordered entry set, defines second-initialize rejection, makes compact `doctor` annotations truthful, specifies Codex and generic installer state semantics, forbids synthesized image-capability environment pins, adds Codex materialization and fast-deploy seams, and strengthens response parity, schema-size, installed-host, connected-phone, and agent-tool-choice gates.
- 2026-06-22: Ran an inline browser Oracle final-review after the large attachment upload path failed before model review. The revision restores `desktop_scroll` horizontal directions, removes false phone `backend` fields, pins phone snapshot requirements, splits `compact_tool_contract.json` from public `mcp_tool_surface_matrix.json`, moves `doctor` mutation proof into Milestone 0, freezes browser eval in client and service process config, separates protocol and tool errors, adds response `content_policy` and `structured_policy`, defines installer launch-policy transactions with eval and image override state, adds symlink and stale-snapshot proof, and fixes the 65% plus 8192-byte size gates.
- 2026-06-22: Ran a focused browser Oracle approval review and revised the ExecPlan to address the remaining config-policy blockers. The revision makes raw MCP runtime parsing lenient and diagnostic, makes installer and config-writing parsing strict, defines per-field launch-policy precedence as CLI, persisted host state, recognized installer environment, then defaults, permits partial overrides, and adds targeted precedence and invalid-value tests. A final focused Oracle review approved the plan for implementation; the remaining non-blocking note was addressed by closing recognized installer environment to `SKY_CUA_MCP_TOOL_PROFILE`, `SKY_CUA_BROWSER_EVAL`, and `SKY_CUA_MODEL_SUPPORTS_IMAGES`.
- 2026-06-22: Implemented the opt-in compact registry, compact facade dispatch, authoritative matrix/contract/call-case fixtures, compact response envelopes, frozen browser-eval policy in both client and service daemon, strict installer launch-policy persistence, compact skill guidance, and the first feature doc. Added `scripts/probe_mcp_tool_surface.py` as a real stdio probe for compact/legacy profile isolation, compact response envelopes, inactive-tool rejection, and degraded desktop/browser/phone status branches. The staged-installed probe exposed a deploy-freshness blind spot for installed shell wrappers; `scripts/deploy_freshness.py` now follows wrapper-local `bin/runtimes/<platform>/sky-cua-client` before judging freshness. Current focused verification: `cargo test -p sky-cua-client`, `cargo test -p sky-cua-service`, `uv run pytest scripts/test_install_flows.py scripts/test_openclaw_install.py scripts/test_deploy_freshness.py scripts/test_probe_mcp_tool_surface.py`, `uv run ruff check`, `uv run basedpyright`, `cargo build --release -p sky-cua-client`, `python3 scripts/build_plugin.py`, `python3 scripts/probe_mcp_tool_surface.py --profile both`, and `python3 scripts/probe_mcp_tool_surface.py --profile both --installed`. Remaining gates are installed-host proof, agent-host tool choice, Android emulator/device proof, full VM smoke matrix, and final ultra-review.
- 2026-06-22: After a live OpenCode attempt demonstrated the weak-agent failure mode (vague compact desktop action calls can click ambient focus or open the launcher), tightened the model-facing compact action schemas before further agent runs. `desktop_pointer`, `desktop_action`, `desktop_keyboard`, `desktop_set_value`, and `browser_input` now advertise selector/coordinate/text/key constraints and descriptions that reject operation-only calls; `doctor` is read-only again. Added Rust and installed-probe assertions for this shape, redeployed the compact OpenCode payload and synced skills, exported the installed tool definitions, and proved a real Zenity OK button dismissal deterministically via `observe(surface="desktop")` followed by `desktop_action(operation="activate", snapshot_id, element_index=13, name="OK")`.
- 2026-06-22: Validated the Android emulator path against the compact installed surface. The legacy phone live-smoke harness was still hard-coded to `phone_*` tool names, so it now detects the active MCP profile, verifies compact phone branch tools, maps its existing behavioral steps onto compact tool names, and unwraps compact response envelopes before checking backend payloads. Focused verification: `uv run pytest scripts/test_live_phone_use_smoke.py`, `uv run ruff check scripts/live_phone_use_smoke.py scripts/test_live_phone_use_smoke.py`, `python3 scripts/deploy_plugin.py --mcp-tool-profile compact --local-install-host opencode`, `python3 scripts/probe_mcp_tool_surface.py --profile both --installed`, and `SKY_CUA_MCP_TOOL_PROFILE=compact python3 scripts/live_phone_use_smoke.py --profile adb-usb --serial emulator-5554 --installed`, which passed observe, screenshot, tap, app-current/list, and disconnect on `emulator-5554` without touching the attached Samsung.
- 2026-06-22: The VM smoke matrix exposed two non-compact runtime/provisioning issues that affected real pointer validation. The Arch testing VM provisioner now installs `python-cairo` so the GTK pointer fixture can render without repeated Cairo callback failures. The VM runner now stamps the release `sky-cua-client` after its host build, satisfying the deploy-freshness gate in synced VM checkouts. Linux Wayland input auto-selection now prefers the RemoteDesktop portal when available and falls back to `ydotool` only when the portal is absent or explicitly overridden; the previous default chose `ydotool` first, reported success, and moved the pointer to origin on Plasma. Focused verification: `cargo test -p sky-cua-linux env_probe`, `cargo test -p sky-cua-linux actions::`, `uv run pytest scripts/test_gui_testing_vm.py::test_testing_vm_runner_builds_runtimes_on_host`, `uv run ruff check scripts/run_gui_testing_vm_smoke.py scripts/test_gui_testing_vm.py`, `uv run basedpyright`, and `uv run python scripts/run_gui_testing_vm_smoke.py ... --profile wayland-pointer --desktop-env KDE --wayland-display wayland-0`, which passed click, secondary click, drag, scroll, and text entry with `PortalEisInputUsed` diagnostics in `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260622T132634Z`.

- 2026-06-22: The Hyprland VM layer-shell overlay lane failed after the pointer fixture became ready because screenshot portal activation was blocked by the testing VM session lifecycle, not by compact MCP code. The TTY-launched Hyprland session leaves `graphical-session.target` inactive, while Arch's packaged `xdg-desktop-portal.service` has `Requisite=graphical-session.target`; direct D-Bus activation therefore failed before selecting `xdg-desktop-portal-hyprland`. The VM runner now writes the same full user-unit override that provisioning installs for synthetic testing sessions, imports `HYPRLAND_INSTANCE_SIGNATURE` during portal refresh, refreshes Hyprland/wlroots portal backends, and preauthorizes screenshot portal use for the layer-shell overlay profile. Autoreview caught that provisioning initially imported `HYPRLAND_INSTANCE_SIGNATURE` before Hyprland created it; provisioned sessions now start a post-launch importer that waits for the live `$XDG_RUNTIME_DIR/hypr` entry and imports the signature into dbus/systemd. Focused verification: `bash -n scripts/testing-vm/provision-arch-testing-vm.sh`, `uv run pytest scripts/test_gui_testing_vm.py`, `uv run ruff check scripts/run_gui_testing_vm_smoke.py scripts/live_agent_cursor_kde_smoke.py`, `uv run basedpyright`, and `SKY_CUA_POINTER_FIXTURE_READY_TIMEOUT_SECONDS=60 python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=/home/bex/projects/sky-cua/artifacts/testing-vm/known_hosts --profile wayland-layer-shell-overlay --desktop-env Hyprland --wayland-display wayland-1 --skip-host-build --skip-sync`, which passed with `xdg-desktop-portal-hyprland` screenshot capture, `target_display` `hyprland:Virtual-1`, and `visible_overlay_captured=true`.

- 2026-06-22: The COSMIC VM lanes exposed a Linux virtual input coordinate quirk independent of compact MCP: ydotool absolute pointer coordinates land at double the requested desktop-logical position under COSMIC, including unscaled displays, and at the same 2x client scale in the 1600x1200/1.25 scaled profile. The Linux action executor now adjusts Linux virtual input dispatch coordinates only for COSMIC Wayland while preserving public desktop-logical coordinates and emitting `LinuxVirtualInputCoordinateScale` diagnostics; the same conversion is used by click, secondary click, drag, targeted scroll, and the `set_value` physical fallback focus click. Keyboard delivery through COSMIC Linux virtual input still does not update the GTK fixture reliably, so `wayland-pointer-scaled` is now explicitly a pointer-coordinate proof and skips the duplicate keyboard section after click, secondary click, drag, and scroll pass. Focused verification: `cargo fmt --check`, `cargo test -p sky-cua-linux actions::`, `cargo test -p sky-cua-linux virtual_input`, `uv run ruff check scripts/live_wayland_pointer_smoke.py`, `bash -n scripts/testing-vm/profiles/wayland-pointer-scaled.sh`, `/home/bex/.agents/skills/autoreview/scripts/autoreview --mode local --engine codex --thinking medium ...`, `python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=/home/bex/projects/sky-cua/artifacts/testing-vm/known_hosts --profile cosmic-helper --desktop-env COSMIC --wayland-display wayland-1 --skip-host-build --skip-sync`, and `python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=/home/bex/projects/sky-cua/artifacts/testing-vm/known_hosts --profile wayland-pointer-scaled --desktop-env COSMIC --wayland-display wayland-1`, which passed the scaled pointer coordinate lane in `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260622T141812Z`.
