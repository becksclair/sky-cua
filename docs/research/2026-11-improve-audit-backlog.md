# Deep audit backlog (2026-11-03)

## Context

On 2026-11-03 a deep codebase audit (improve skill) ran at commit `e928a40`.
Each finding was confirmed against source that day. The highest-leverage 17
findings became handoff plans (`plans/001`-`017`); every plan has since landed
on `main` and the plans were retired. This note keeps what the audit found
that is still open, the follow-ups raised while executing the plans, and the
findings deliberately rejected.

Open items are recorded as of the audit and were not re-verified when the
plans were retired. Confirm against current source before acting on one.

## Resolved by the audit plans

All 17 plans landed on `main` (plan commit + review-fix commit):

| Plan | Change | Commits |
|------|--------|---------|
| 001 | `just verify` and CI test the whole Rust workspace and the bun packages | `d9fd8d6` |
| 002 | Release direct-lane AppShot content artifacts | `727452b`, `32bb4dc` |
| 003 | Validate the `adb shell input keyevent` argument | `82e678d`, `bf41dcd` |
| 004 | Install the root input helper to a root-owned path | `9a56800`, `1544a66` |
| 005 | Bound the desktop and browser AppShot registries | `b00ed41`, `0da6a2d` |
| 006 | Answer malformed JSON-RPC frames with `-32700`; keep request id on panic | `0045067`, `af24752` |
| 007 | Scan only terminal descendants during window enrichment | `bef0934`, `3240f57` |
| 008 | Remove the browser control plane's blanket dead-code allows | `cf7ff0a`, `61a9ff4` |
| 009 | No ydotool re-send after the helper may have injected a chord | `a5eb0e2`, `31013e5` |
| 010 | `cosmic-protocols` past its MIT relicense, pinned toolchain, zbus major ignore | `aeb666c`, `e37cf37`, `71841a3`, `4a9155c` |
| 011 | Fix stale agent guides and guard their paths with a test | `a457d92`, `75e722f` |
| 012 | One timed-subprocess helper that drains child output | `c7fb397`, `a96b5e6`, `a87d706` |
| 013 | Test the standalone installer's Codex config rewriter; atomic write | `f3053e3`, `f363f4a` |
| 014 | Test the companion impostor and downgrade install gates | `c97540a`, `0e502c0` |
| 015 | Overlay host ticks slowly while the cursor is hidden | `71af336`, `201b3b7` |
| 016 | Keep `debug!` in release builds; document `RUST_LOG` and daemon logs | `1410792`, `a6f0aff` |
| 017 | Time out silent direct-LAN phone links | `43dd1eb`, `f61ddf8` |

Backlog ids these resolved, and so omitted below: `CORRECTNESS-06`,
`CORRECTNESS-09`/`DEBT-17`, `CORRECTNESS-10`, `TESTS-03`, `TESTS-06`,
`DEBT-01` (control-plane half), `DEPS-01`, `DEPS-06`, the ignore half of
`DEPS-02`, `DX-02`/`DEBT-23`, `DX-03` (feature flag and runbook half),
`PERF-16`, and the nextest profile half of `PERF-11`/`DX-04`. `DOCS-04`'s
duplicated `NOTES.md` bullet was removed later.

## Follow-ups raised while executing the plans

- The EIS fallback in `crates/sky-cua-linux/src/portal/eis_fallback.rs:602`
  can still double-press a chord after plan 009's post-injection error. Needs
  its own fix.
- Close Dependabot PR #22 (zbus 6); plan 010 only ignores semver-major.
- The prebuilt cosmic-helper binaries under `opencode/bin` still embed the old
  GPL `cosmic-protocols` rev until rebuilt.
- Plan 009's regression test needs XKB keymap data on the host.
- No live smokes were run on the plan batch.

## Open backlog

Effort: S small, M medium, L large.

Correctness
- `CORRECTNESS-03/04` Session-env re-hydration runs `systemctl`/`loginctl`
  with no timeout under a std `Mutex` on tokio workers every 10 s, and calls
  `env::set_var` at runtime in a multi-threaded process
  (`crates/sky-cua-linux/src/backend/mod.rs:217-224`, `session_env.rs`). M.
- `CORRECTNESS-07` The root input helper leaves keys/buttons held when a batch
  aborts mid-way; `Settle { millis }` is unbounded while holding the device
  lock (`crates/sky-cua-input-helper/src/server.rs:231-262`). S.
- `CORRECTNESS-11` PipeWire remote fd duplicated with `dup` instead of
  `F_DUPFD_CLOEXEC`, leaking into spawned children (`portal/pipewire.rs:275`). S.
- `CORRECTNESS-13` The daemon heals `DISPLAY` but not `XAUTHORITY` after an
  Xwayland restart (`session_env.rs:206-213`). S, low-medium confidence.

Security (project policy favours performance over hardening; these have a
concrete defect)
- `SECURITY-02` The Windows service TCP listener `127.0.0.1:48931` has no
  handshake and keeps reading after malformed lines (`ipc_server.rs:420-447`). S-M.
- `SECURITY-04` The Chrome-host app-server proxy strips `Origin` and checks it
  on WebSocket upgrades only when an extension id is known; `serve` mode
  passes none (`chrome-host/src/app_server/proxy.rs`). S.
- `SECURITY-05` No production path revokes an enrolled direct-LAN phone;
  `FileStateStore::revoke` and `LinkRegistry::revoke` are dead code
  (`phone/direct/mod.rs`). S-M.
- `SECURITY-06` The release pipeline attaches the Gitea token to any absolute
  URL and follows redirects (`scripts/release_pipeline.py`). S.

Performance
- `PERF-01/05` Window discovery runs three times per desktop observe and once
  per action, each a fan of `gdbus`/`xprop` spawns; re-activation and a full
  doctor report run on every observe (`daemon/appshot.rs`, `desktop_backend.rs`,
  `kwin.rs`). M.
- `PERF-03` The Linux capture path PNG-encodes the full frame, decodes, crops,
  re-encodes PNG, then WebP-encodes (`portal/pipewire.rs`, `portal/screenshot.rs`). M.
- `PERF-04` The AT-SPI walk is strictly sequential, about 8 D-Bus round trips
  per node (`atspi/tree.rs`). M.
- `PERF-06` Browser screenshots travel as multi-MiB base64 PNG through three
  hops before a WebP re-encode (`browser/cdp.rs`, `model_image.rs`). M.
- `PERF-07` The Windows UIA walk makes about 20 cross-process calls per
  element with no `CacheRequest` (`windows/src/uia.rs`). M.
- `PERF-08` Targeted AppShots orphan the full-frame PNG on tmpfs;
  `capture_desktop` never prunes (`daemon/appshot.rs`, `desktop.rs`). S.
- `PERF-09/10/12` Blocking subprocess/socket work on tokio workers; the phone
  manager lock is held across `screencap`; per-call `zbus::Connection::session()`. S each.
- `PERF-11`/`DX-04` CI wipes `target/` every run (caching half). S.

Tests
- `TESTS-04` Direct-LAN auth rejections are tested only through a test-only
  entry point; the production `accept_socket` auth path is unexercised. S.
- `TESTS-05` Rust and Kotlin pin their own hashes instead of reading the shared
  `docs/runtime/fixtures/phone-control-v2/*` fixtures. S.
- `TESTS-07` No headless client-service-daemon test over a real socket; the
  one that exists is excluded from `verify-python`. M.
- `TESTS-08` Settlement protocol constants are duplicated between service and
  chrome-host; the cross-component regression is covered only by mirrored copies. M.
- `TESTS-09`-`TESTS-15`: phone mutating actions, the input-helper seam, the
  Claude Code `mcp add-json` flow, the Windows crate never compiled in CI,
  sleep-based sync in socket tests, 1.3 MB snapshot fixtures, and a misnamed
  `rejects_wildcard_binds`.

Tech debt
- `DEBT-01` (remaining) Blanket dead-code allows outside the browser control
  plane (`phone/protocol/mod.rs`, `phone/direct/lan.rs`, `browser/transport.rs`). S.
- `DEBT-02` About 27.6k lines of inline `mod tests` inflate the 800-line
  god-file signal. S-M, mechanical.
- `DEBT-03` The service depends on the overlay-host binary crate for its IPC
  contract types, pulling wgpu/Wayland into every service build. M.
- `DEBT-04/05/06/16/18/19` Duplicated frame codec and size limits,
  `DiagnosticEntry` struct literals, drifted path-resolution copies,
  process-identity helpers, PATH lookup, env parsers, four WebP encode sites. S-M each.
- `DEBT-07..12` Named seams for the production god files
  (`phone/direct/mod.rs`, `windows/backend.rs`, `browser/bridge.rs`,
  `linux/actions/mod.rs`, `mcp_tools.rs` dispatch, `daemon/mod.rs`,
  `virtual_input.rs`, `chrome-host/host.rs`, `service_launcher.rs`, `model.rs`). M-L.
- `DEBT-13/14` Five browser-control-mode enums; stringly-typed error codes. S-M.
- `DEBT-15` (remaining) Python installer write/merge helpers drifted: atomic
  write copies, regex TOML upserters, two Pi installers. M.
- `DEBT-20/21` Unused deps (`tracing-log`, `calloop-wayland-source`), orphan
  `scripts/_openclaw_cli_transaction.py`, deprecated `SKY_CUA_MCP_HOST` read,
  six copies of `terminate_process` across smokes. S.

Dependencies
- `DEPS-02` (remaining) Close Dependabot PR #22 (see follow-ups).
- `DEPS-03` Five cargo major PRs (sctk, reis, zbus, wgpu 30, if-addrs) saturate
  the default queue; if-addrs is trivial. S-L.
- `DEPS-05` Two RustCrypto stacks (digest 0.10 and 0.11). S.
- `DEPS-07` Vendored libwebp ~1.3.1 via `webp 0.3.1`. S-M.
- `DEPS-04`, `DEPS-08`, `DEPS-09` as reported by the audit.

DX and docs
- `DX-03` (remaining) No request span. S.
- `DX-05` No pre-commit hook; `resources/chrome_preflight.py` is outside
  ruff/basedpyright scope. S.
- `DX-06` Ten forwarded operator knobs are documented nowhere. S.
- `DX-07`, `DX-08` as reported by the audit.
- `DOCS-01/02` `phone-companion-protocol.md` describes a removed transport;
  `phone-control-v2.md` bind rules contradict code. S-M.
- `DOCS-03` Most unnumbered ExecPlans in `plans/` are stale or off-roadmap;
  `phone-use.md` and `standalone_release_distribution.md` cite many dead paths
  and should be retired per `plans/AGENTS.md`. M.
- `DOCS-04` (remaining) `NOTES.md` is far over its ~150-line budget and has
  stale companion RPC notes. S.
- `DOCS-05` `ROADMAP.md` linked a plan file that never existed. S.
- `DOCS-06` About 100 stale backticked source paths across docs after the
  2026-07-08 file splits. S.

Direction options (strategy is the maintainer's call)
- `DIRECTION-01` Expose extension capabilities the MCP browser tools lack:
  close tab, JS dialogs, file chooser, select/set_checked, double-click/drag. L.
- `DIRECTION-02` Add `click_count`, `mouse_button` and modifiers to
  `desktop_pointer`; the Linux executor already carries them. S.
- `DIRECTION-03` Prove and retire the configurable surfaces, cross-surface
  AppShots and companion-direct ExecPlans whose code has shipped. S-M.
- `DIRECTION-04` A Windows capture lane (WGC/DXGI) ahead of GDI. L.
- `DIRECTION-05` Helper absolute pointer beyond single-output COSMIC. M-L.
- `DIRECTION-06` A desktop clipboard tool to match `phone_clipboard`. M.
- `DIRECTION-07` A Windows visible overlay host. L.
- `DIRECTION-08` Phone permission re-probe (`selection.rs:446` TODO). M.

## Considered and rejected

- `cargo audit` quick-xml advisories: reached only through `zbus_xml` and the
  build-time `wayland-scanner`, both parsing trusted XML. Clears when those bump.
- `SECURITY-07` `transfer_id` in a temp path: fused onto a fixed prefix; not
  exploitable.
- Merging the three bun projects into one workspace (`DEBT-22`): separate
  lockfiles are deliberate because `runtime/cua-node` consumes the others as
  packed artifacts.
- Splitting the two largest Python smoke harnesses by size alone: little
  cross-file churn; `DEBT-21` helper extraction is the better lever.
- Removing the legacy browser-control mode: still the default when unset.
- Managed browser profile, macOS backend, arm64 release, consumer action
  broker, multi-sandbox fleet: no evidence of demand.
- `PERF-15` moving instead of cloning tool results: sub-millisecond; fold
  into any `mcp_tools.rs` dispatch refactor.
- `PERF-13/14/17` resize cost, persistent PipeWire pipeline, 500 ms XTest
  settle: low confidence; measure first.

## Implications

Pick follow-up work from this note by id and confirm it against current
source first. The EIS fallback double-press and the stale ExecPlans
(`DOCS-03`/`DIRECTION-03`) are the most concrete next items.
