# Complete CUA stack ownership

## Status

Partial. Last verified: 2026-07-20 at producer commit
`e2d39d24801d8df96c15aee21914dfe1c7897a57`, immutable release
`f82b61b4962f318b5121464223ba5911d1f66adfed9511ecc42f909fa8b67c11`.
The standalone, Codex Desktop, OpenClaw, and OpenCode installations are active
on that generation. Physical-phone acceptance remains open because no Android
device is connected.

## Summary

sky-cua owns the complete Linux x86-64 glibc CUA release: the native direct
MCP server, persistent Node 24 `node_repl`, Browser/Computer/Phone JavaScript
facades, model-facing documentation, compliance records, and installation.
Codex Desktop consumes a verified compatibility projection; OpenClaw and
OpenCode install both MCP servers directly from the same immutable generation.

## Contract surface

Each release contains `RELEASE.json`, `SHA256SUMS`, provenance, licenses,
SBOMs, and seven hash-bound components: `core-linux-x64`, `browser-js`,
`cua-node-linux-x64-glibc`, `codex-compat`, `documentation`, and the release
compliance/provenance payloads. Complete generations install under
`~/.local/share/sky-cua/releases/<release-id>` and `current` selects one whole
generation atomically.

`sky_cua` is the direct native MCP server. `node_repl` preserves
`js`, `js_reset`, and `js_add_node_module_dir` with a persistent Node 24 VM.
The installed JavaScript packages expose `@heliasar/browser-use` and
`@heliasar/sky-cua`, including the persistent `@heliasar/sky-cua/phone`
export. Browser compatibility clients are exact projections of the canonical
Browser bytes; documentation projections are pointers into the immutable
documentation component.

The resolver and launch environment bind one verified generation through
`SKY_CUA_RELEASE_ROOT`, `CODEX_NODE_REPL_PATH`, `NODE_REPL_NODE_PATH`,
`NODE_REPL_NODE_MODULE_DIRS`, `NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S`,
`SKY_CUA_CODEX_BROWSER_SOCKET_PATH`, and
`SKY_CUA_MCP_CALLER_PROVENANCE`. Explicit caller provenance includes
`codex_desktop`, `openclaw`, `opencode`, and `direct_mcp`.

## Behavior

Both MCP servers share the existing daemon and daemon-owned Browser scheduler;
direct `sky_cua` has no Node hop. Same-user owner-only sockets are trusted.
The Browser bridge preserves caller provenance and owns separate tab groups for
concurrent hosts. Codex IAB remains host-provided, while Chrome-family browsers
use the one extension bridge and the already-installed Web Store extension.

The `node_repl` runtime provides persistent JavaScript, Browser API, Computer
Use JS, Phone JS, Sharp, Canvas, pixelmatch, PDF.js, Tesseract, standalone
Playwright with system Chrome-family browsers, and local file/buffer/URL
workflows. WebP is the default screenshot format. Compact Browser, Computer,
and Phone skills route by progressive disclosure into the installed canonical
references and runnable recipes.

## Source paths

- `scripts/build_complete_release.py`, `scripts/release_generation.py`,
  `scripts/install_complete_release.py`, and `scripts/complete_release_cli.py`
- `runtime/cua-node/`
- `packages/browser-use/`
- `packages/sky-cua-js/`
- `scripts/build_model_documentation.py`
- `skills/browser-use/`, `skills/computer-use/`, and `skills/phone-use/`
- `docs/research/2026-07-model-facing-cua-docs-phone-js.md`

## Verification

The final release verifies all seven component trees and has `RELEASE.json`
SHA-256 `16fd1878175a6031e0ebc19d77074e1224cc709cf5e83ba7e6b4363315dbd83f`.
Installed bundled-Node execution passes all ten documentation examples,
including persistent REPL state, Browser, Computer screenshot/image emission,
binary local files, Sharp, Canvas/pixelmatch, PDF.js, Tesseract, truthful Phone
no-device behavior, and standalone Playwright. Exact written binary and WebP
outputs are asserted.

Source verification includes Rust format/clippy and `cargo nextest run`
(1,339 tests), Python formatting/type checking and 878 pytest cases, 193
`cua_node` runtime cases with seven expected skips, the full generated Browser
surface, Phone contract tests, trust-hash negatives before socket connection,
and installed performance within the locked median and p95 budgets. The focused
KDE Wayland pointer VM acceptance passes at
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260720T111641Z`.

After the final Codex GUI relaunch, app-server PID `128057` and every spawned
Codex `node_repl` child resolved from the immutable `f82b61b…` generation; no
old `/opt/chatgpt-desktop/resources/cua_node` process remained. Installed IAB
acceptance loaded `browser.documentation()` and opened `https://example.com/`
through the Codex-provided socket. A separate Brave Origin task ran with
`CODEX_BROWSER_PROVIDER` unset and used the daemon-owned extension bridge; the
Web Store extension was not modified. Installed Computer and Phone skills
routed into the canonical release documentation, and persistent JavaScript
captured and emitted a live WebP screenshot while the Phone facade reported the
truthful no-device result. OpenCode independently discovered the installed f82
Phone skill and invoked `node_repl` with `opencode` provenance; its final prose
also mentioned a retained older package path that it had searched manually, so
that statement is not release-identity evidence.

## Known limitations

- v1 `node_repl` supports Linux x86-64 glibc. macOS is a placeholder; arm64,
  musl, Windows `node_repl`, and `@heliasar/sky-cua/advanced` are follow-ups.
- A physical Android phone was not connected for the final release, so the
  persistent Phone JS no-device path is proven but device screenshot/action
  parity remains pending.
- No npm packages are published; packages are release components only.

## Related

- [`ROADMAP.md`](../../ROADMAP.md)
- [`docs/features/unified-browser-bridge-control-plane.md`](unified-browser-bridge-control-plane.md)
- [`docs/features/codex-desktop-compat.md`](codex-desktop-compat.md)
- [`docs/research/2026-07-model-facing-cua-docs-phone-js.md`](../research/2026-07-model-facing-cua-docs-phone-js.md)
- [`plans/complete-cua-stack-ownership.md`](../../plans/complete-cua-stack-ownership.md)
