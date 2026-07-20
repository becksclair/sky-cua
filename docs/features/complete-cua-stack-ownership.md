# Complete CUA stack ownership

## Status

Shipped. Last verified: 2026-07-20 at producer commit
`e2d39d24801d8df96c15aee21914dfe1c7897a57`, immutable release
`f82b61b4962f318b5121464223ba5911d1f66adfed9511ecc42f909fa8b67c11`,
and Codex consumer commit `1f8d0cc1ad5fbee8e142e34f4b8f8851ab409127`.
The standalone, Codex Desktop, OpenClaw, and OpenCode installations are active
on that generation.

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

The final reviewed Codex package has SHA-256
`f012be97652e783725d37353dda8b0c28cb91f95e7aea5965eeb4d18232f3bd4`.
After installation and GUI relaunch, GUI PID `199267`, app-server PID `199584`,
direct client PID `200912`, `node_repl` PID `202950`, and the consumer task's
direct client PID `202960` all resolved from the immutable `f82b61b…`
generation with manifest SHA-256 `16fd1878…` and `codex_desktop` provenance.
The surviving pre-install app-server tree was terminated, and no old
`/opt/chatgpt-desktop/resources/cua_node` process remains.

Installed skill discovery followed the canonical f82 Browser routing and
loaded `browser.documentation()`. Persistent Browser JavaScript opened
`https://example.com/` through the logical Codex IAB entry and verified
`Example Domain`; the entry metadata records the physical
`extension_native_host` transport and real `codex_desktop` provenance. A
separate direct Browser tab `379563395` verified Brave Origin with
`navigator.brave=true`, `Example Domain`, and `CODEX_BROWSER_PROVIDER` unset.
The Web Store extension was not modified.

Persistent Phone JS acceptance used the Android 36 `Pixel_9a` emulator through
the ordinary daemon/service path. It proved discovery/connect, capability
inventory, WebP screenshots, local-file round trips, two emitted images, tap,
swipe, key, text, terminal disconnect, and structured post-disconnect rejection
with `direct_mcp` provenance. OpenCode independently discovered the installed
f82 Phone skill and invoked `node_repl` with `opencode` provenance.

## Known limitations

- v1 `node_repl` supports Linux x86-64 glibc. macOS is a placeholder; arm64,
  musl, Windows `node_repl`, and `@heliasar/sky-cua/advanced` are follow-ups.
- Physical-device coverage is not part of the v1 completion gate; the complete
  Phone JS lifecycle is accepted against an Android 36 emulator target.
- No npm packages are published; packages are release components only.

## Related

- [`ROADMAP.md`](../../ROADMAP.md)
- [`docs/features/unified-browser-bridge-control-plane.md`](unified-browser-bridge-control-plane.md)
- [`docs/features/codex-desktop-compat.md`](codex-desktop-compat.md)
- [`docs/research/2026-07-model-facing-cua-docs-phone-js.md`](../research/2026-07-model-facing-cua-docs-phone-js.md)
