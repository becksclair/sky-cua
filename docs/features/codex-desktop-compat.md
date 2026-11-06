# Codex Desktop compatibility

## Status

Shipped on Linux. The fixed-root navigation helper passed live Brave Nightly
navigation, callback-result preservation, and no-op rejection after an actual
ChatGPT restart on 2026-11-06.
The native ChatGPT app owns its Browser package and startup reconciliation.

## Summary

Sky CUA supplies `computer-use@openai-bundled` through its fixed-root payload.
The native Linux ChatGPT app supplies its in-app Browser client, service,
bootstrap instructions, and Node runtime. Sky layers a strict navigation helper
on that maintained baseline through a Sky-owned supplemental skill and tab
decorator. Its separate
Browser client continues to serve external Chrome-family consumers through
trusted native pipes.

## Contract surface

- `bin/sky-cua-client mcp` is the portable computer-use MCP entrypoint.
- `codex/openai-bundled/plugins/computer-use` is Sky's compatibility root.
- Native Browser comes from the complete installed plugin directory at
  `/usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/browser`.
- `browser/sky-enhancements.mjs` exports `decorateSkyTab(tab)` and
  `createSkyBrowserRuntime(nativeSetup)`. These add
  `tab.sky.expectNavigation(action, options)` while preserving native methods.
  `RELEASE.json` exposes the helper at `paths.browser_enhancements`.
- The Codex supplemental `sky-browser-enhancements` skill activates the helper
  explicitly. Ordinary native `setupBrowserRuntime()` remains unmodified.
- `browser@openai-bundled` is the current Browser identity. Existing retired
  `browser-use@openai-bundled` entries are disabled during reconciliation.
- Sky's `browser/browser-client.mjs` and bundled `cua-node` retain the external
  Browser/native-pipe contract. Native Browser uses the app's packaged runtime.

## Behavior

Standalone installation projects Sky's computer-use plugin into the local
bundled marketplace. When the native app's Browser plugin is present, both
standalone installation and preflight materialize its complete baseline as
regular files, retaining its original manifest version, client, service and
instructions. Native `plugin/install` omits symlinks, so the projection must be
self-contained. ChatGPT owns the desired contents of its reserved marketplace
and can restore packaged Browser content at startup.

Sky's helper stays in the fixed install root. Installation projects a focused
supplemental skill into `CODEX_HOME/skills/sky-browser-enhancements`; that skill
imports the fixed-root helper and applies `decorateSkyTab` to an existing native
tab. Native setup, selection, ownership and transport instructions remain
canonical. Browser disablement or native-package absence removes only the
installer-owned supplemental skill. Installation does not replace an unowned
skill, alter native service mappings, or fork the reserved Browser version.

`tab.sky.expectNavigation` requires the action to succeed and a fresh top-level
navigation event after its pre-action cursor, then optional URL and load
readiness. A single deadline covers the whole wait. No-op and child-frame
navigation do not satisfy it. It does not replace native
`tab.playwright.expectNavigation`, retry mutations, or cancel an expired action.
Use the advertised CDP capability and its required documentation; navigate a
fresh tab to its intended HTTP(S) page before CDP use. Same-origin navigation
is verified. Native origin/session changes or eviction can invalidate the event
stream; truncated history rejects the wait rather than dropping the guarantee.

Browser disablement bypasses native Browser discovery, validation, and
projection while retaining Computer Use. With Browser enabled, a missing native
plugin selects the complete fallback even when the app's other plugins exist.
A present but malformed native package still produces its specific error.
Discovery validates once and passes that result through materialization.

Without a native Browser plugin, standalone installation retains the bundled
Sky Browser fallback. Its skill and `host_provided_iab` transport field belong
to that fallback, not to the native app's Browser API. Malformed native plugin
inputs fail before replacing the Browser projection.

Config reconciliation enables the current Browser companion when the Browser
surface is enabled and disables an existing retired `browser-use` identity.
Surface disablement disables current Browser and Chrome companions. The direct
`sky-cua@local` carrier remains disabled when the computer-use compatibility
plugin supplies the active server.

Preflight and standalone installation converge existing Chrome companion flags
on both disablement and re-enablement. Browser-only preflight does not create a
Chrome stanza. When Browser is disabled and no legacy marketplace source is
available, preflight only disables existing companion stanzas: it preserves
marketplace and feature configuration, and a fresh home remains untouched.

Native preflight stages the marketplace projection; deployment uses
`python3 install.py install` and the native `plugin/install` API to refresh the
installed Browser cache. Marketplace staging alone is not cache activation.

The development Browser fallback carries the assembled runtime and resolves its
canonical `@heliasar/browser-use` package through the existing trusted module
path. It does not require a native app or standalone installation. The standalone
adapter instead validates its release metadata at the injected runtime root and
loads that same trusted package. Neither mode broadens trust to copied client
files. The standalone producer explicitly selects its staging mode to avoid
packing a second runtime. Native Browser retains precedence when present.
The retired `codex-desktop` rebuild repository is not an installation or update
dependency of the current native app.

## Source paths

- `scripts/standalone_release.py` — fixed-root assembly and installation.
- `scripts/_standalone_codex.py` — Codex registration and marketplace projection.
- `scripts/_standalone_hosts.py` — Pi/OpenClaw registrations and shared config helpers.
- `resources/chrome_preflight.py` — companion cache/projection reconciliation
  and Chrome-family native-host preflight.
- `scripts/_plugin_bundle.py` — current and retired companion enablement.
- `resources/codex-compat/openai-bundled/` — Sky compatibility roots.
- `resources/_native_browser_projection.py` — validated native baseline
  materialization.
- `resources/sky-browser-enhancements/SKILL.md` — focused native helper
  activation, rendered for the fixed install root.
- `packages/browser-use/src/native-enhancements.ts` — additive native helper.
- `packages/browser-use/` and `runtime/cua-node/` — external Browser client
  and trusted native-pipe runtime; their existing external path is unchanged.

## Verification

- The restart-safe helper change passed 1,269 Python tests, the separate
  23-test module including official Codex 0.149 API acceptance, and 13 focused
  helper tests. The fixed-root helper passed live Nightly same-origin
  navigation, callback-result preservation, button readback and no-op
  rejection. Evidence: `artifacts/review-closeout/nightly-helper-live.json`.
  After canonical deployment, a fresh validation kernel followed the installed
  supplemental skill on an initially undecorated native tab and passed the
  same checks. Fresh native session discovery retained the enabled global
  skill through simulated reserved-source reconciliation; all 378 cached
  baseline files matched native bytes. Evidence:
  `artifacts/review-closeout/installed-skill-live.json` and
  `artifacts/review-closeout/restart-safe-discovery.json`.
  An actual ChatGPT restart then replaced the main process and app-server.
  Skill/helper hashes remained unchanged, fresh skill discovery passed, and
  live Nightly navigation, callback preservation, button interaction and no-op
  rejection passed without reinstalling or repairing the deployment. Evidence:
  `artifacts/review-closeout/actual-app-restart.json` and
  `artifacts/review-closeout/actual-restart-live.json`.

- The earlier review closeout passed 1,264 Python tests and the separate 18-test module
  including official Codex 0.149 API acceptance. Two helper-only fixture
  versions passed native installation and fresh-session cache selection,
  with 380 matching regular files and no symlinks in each cache. Canonical
  installation and an AppShot-bound button click passed in the original
  Nightly session. Evidence: `artifacts/review-closeout/`.

- Prioritized installer follow-ups passed 1,260 Python tests plus the separate
  opt-in Codex 0.149 API acceptance, and 51 Browser tests. Real app-server
  installs and MCP calls verified both development and installed standalone
  fallback paths after removing their original source. Live IAB navigation and
  changed-state readback passed; the original Nightly session remained ready.
  Evidence: `artifacts/install-followups/verification.json`.

- `uv run pytest scripts/test_standalone_release.py scripts/test_plugin_bundle.py
  scripts/test_deploy_plugin.py` covers native complete-directory selection,
  existing Sky-link replacement, unchanged native baseline projection,
  supplemental skill ownership/removal, retired identity disablement, and
  no-native fallbacks.
- `cd packages/browser-use && bun run build && bun test` covers the external
  Browser client, including the real `cua-node` fixed-path trust integration.
- `python3 scripts/build_plugin.py` stages the development bundle; inspect
  its included preflight and resources before rollout.
- `packages/browser-use/test/native-enhancements.test.ts` covers strict
  event evidence, main-frame filtering, callback failures, deadlines and native
  setup/decorator preservation.
- The former materialized wrapper connected in the existing native runtime. Isolated
  local fixture tabs passed cross-document and same-document waits; no-op waits
  were rejected. Evidence: `artifacts/native-browser-layer/`.

- The 2026-11-05 Claude review found that native `plugin/install` omitted
  symlinked overlay entries. The corrected cache contains 380 regular files,
  preserves all 378 native files, and imports independently of its source
  projection. That exact cache passed live IAB page navigation, a changed button
  readback, and no-op navigation rejection. Evidence:
  `artifacts/claude-code-review/primary-verification.json`.

## Known limitations

The old versioned overlay and appended native skill were superseded after the
2026-11-06 restart restored the app-owned baseline. Historical cache-selection
proof above does not prove restart persistence of that former deployment.
Current activation is explicit through the supplemental skill; it does not
attach helpers to every ordinary native bootstrap automatically. An existing
chat's initial skill catalog remains a snapshot of its old selection.
Native Browser checks use disposable local fixture tabs; no compositor smoke
or fresh model turn is implied.

Yesterday's working Browser state was reported by the user. The native package
was installed before that report; today's projection/cache timestamps do not
prove the exact change that selected the stale client. See the bounded
[regression investigation](../research/2026-11-native-browser-plugin-regression.md).

The prior bypass-only repair plan is superseded. The
[evidence-backed enhancement inventory](../research/2026-11-native-browser-enhancement-baseline.md)
distinguishes native additions from upstream parity and external-only features.

## Related

- [Compatibility payload contract](../runtime/compat-plugin-contract.md)
- [Browser MCP tools](browser-mcp-tools.md)
- [Unified browser bridge control plane](unified-browser-bridge-control-plane.md)
- [Roadmap](../../ROADMAP.md)
