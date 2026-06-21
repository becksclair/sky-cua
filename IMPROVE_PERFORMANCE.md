# Performance-Affecting Security Inventory

Scope: source scan of the Rust workspace, Linux backend, phone manager, MCP
installer, and browser preflight paths. "Security measure" is treated broadly:
permission/consent gates, auth/integrity checks, owner-only isolation,
environment allowlists, bounded parsing/output, stale-state validation, and
fail-safe retries. Ratings estimate the gain from removing the measure entirely,
not from tuning or replacing it with a faster implementation.

## Summary

Highest potential speed gain:

1. Replace the Wayland portal capture/input path with a direct privileged
   backend where that simplifies implementation or reduces latency.
2. Shorten or bypass phone companion bootstrap identity/token/capability probes.
3. Cache or reduce launch-environment repair and daemon health checks.
4. Tune EIS/portal input pacing and fallback retries.

Lowest-value removals:

1. Owner-only socket/file modes.
2. Inherited FD cleanup.
3. Atomic config writes and JSON/TOML validation.
4. Snapshot/selector bounds checks.

## Privileged Fast Path Research

### `uinput` input backend

Reality: this is the cleanest privileged fast path. The Linux kernel `uinput`
API lets a userspace process create virtual input devices and emit events
through `/dev/uinput`; libinput then treats common devices, including keyboards,
mice, touchscreens, and virtual absolute pointing devices, as normal input for
compositors. On Wayland, libinput sits inside the compositor, so a virtual
kernel input device reaches Wayland apps as hardware-like input instead of
using a Wayland client injection protocol.

Project status: `crates/sky-cua-linux/src/virtual_input.rs` already has direct
`/dev/uinput` support for pointer actions. It creates an absolute pointer with
buttons and wheel support when `/dev/uinput` is writable and desktop bounds can
be detected. Keyboard/text still routes through `ydotool`; direct uinput only
supports pointer actions today. The current direct path also carries conservative
latency: 650 ms first-device settle, 180 ms pointer-action settle, and 120 ms
button hold.

What is really possible:

- A small privileged helper can own `/dev/uinput`, create persistent virtual
  pointer, keyboard, and possibly touch devices, and accept compact commands
  from `sky-cua-service` over an owner-only Unix socket.
- This should work across most libinput-based Wayland compositors and X11,
  because it enters below the display server rather than through portal/EIS.
- It can remove EIS worker startup, RemoteDesktop input D-Bus calls, ydotool
  process/daemon dependency for text, and most fixed sleeps.
- It still needs desktop bounds/seat mapping. Absolute pointer coordinates are
  only as good as the logical-to-device mapping, especially with mixed scale,
  rotated outputs, tablets, or multi-seat setups.
- It may not solve lock-screen or secure-input cases uniformly; those are
  compositor/session-policy decisions, not uinput mechanics.

Estimated speed gain:

- Cold input setup: high if the helper creates persistent devices at service
  start. Avoids the current 650 ms direct-device settle on first use and avoids
  EIS worker/session setup when portal input is selected.
- Pointer actions: medium to high. Current portal/EIS paths include 20-80 ms
  frame/flush sleeps plus D-Bus/fallback cost; current direct uinput pointer
  actions include 120-300 ms of conservative sleeps. A tuned persistent helper
  should be able to get dispatch close to event-write plus compositor processing,
  likely single-digit to low-tens of milliseconds per click/scroll/drag step.
- Text/key input: high. Replacing ydotool subprocess/daemon text and portal
  keyboard with direct virtual keyboard events can remove process spawn, socket,
  and 20 ms key delay/hold defaults. End-to-end wins depend on the app and model
  loop, but input-heavy workflows should see clear improvement.

Difficulty: medium. The kernel path is straightforward and already partially
implemented. The real work is packaging the privileged helper, defining the IPC
protocol, adding direct keyboard/text support, tuning sleeps without creating
flaky desktop behavior, and proving it across KDE, GNOME, COSMIC, Hyprland, and
X11.

Recommendation: promote this first. Build a persistent privileged
`sky-cua-input-helper` around the existing direct uinput code, add keyboard and
touch support, and make it the preferred physical input backend before portal
EIS/RemoteDesktop. Keep portal and XTest/ydotool as fallbacks.

### Privileged/direct capture backend

Reality: privilege helps, but it does not create one universal Wayland screen
capture API. Under Wayland, the compositor owns the scene. A privileged process
can bypass the portal by going below the compositor to DRM/KMS, by using
compositor-specific capture protocols, or by owning a persistent capture
pipeline, but each option has different coverage and failure modes.

What is really possible:

- Persistent non-portal PipeWire/GStreamer: likely the best first capture
  optimization if a stream can be acquired without repeated portal startup or
  per-frame pipeline construction. It does not fully bypass compositor consent
  unless the stream source is obtained another way, but it targets the current
  hot cost directly: `pipewiresrc ! videoconvert ! pngenc ! appsink` is built
  per capture today.
- DRM/KMS scanout capture: possible with DRM master or `CAP_SYS_ADMIN`, and
  FFmpeg `kmsgrab` documents this exact model. It captures KMS scanout
  framebuffers by CRTC/plane as DRM objects that can be mapped or passed to
  hardware functions. Kernel DRM UAPI also exposes framebuffer metadata and GEM
  handles to DRM master or `CAP_SYS_ADMIN` clients, with DMA-BUF export via
  `DRM_IOCTL_PRIME_HANDLE_TO_FD`.
- DRM/KMS limits: it is screen/plane capture, not semantic window capture.
  It may miss cursor planes or hardware overlays unless we capture and compose
  multiple planes. CPU readback only works cleanly for linear/mappable buffers;
  tiled/compressed modifiers may require GPU mapping/conversion. Sampling is not
  synchronized to page flips in the `kmsgrab` model. Multi-monitor geometry,
  scaling, HDR/color, direct scanout, and NVIDIA/driver differences all need
  explicit handling.
- DRM writeback connectors: real kernel feature for writing a CRTC output to
  memory, but availability depends on driver/hardware exposing writeback
  connectors. It is not a general desktop capture answer.
- Wayland capture protocols: `ext-image-copy-capture-v1` can ask the compositor
  to copy outputs/toplevels into client buffers, and `wlr-screencopy` can copy
  screen content to a client buffer on wlroots-style compositors. These are
  promising direct compositor paths, but they require compositor support and are
  not made universal by root. `ext-image-capture-source-v1` is still marked as a
  testing-phase protocol, and `wlr-screencopy` is deprecated in favor of
  `ext-image-copy-capture-v1`.
- Compositor-specific helpers: KWin, wlroots, COSMIC, GNOME/Mutter, and vendor
  GPU paths may each have better local hooks. These can be fast, but they split
  the backend matrix.

Estimated speed gain:

- Cold capture/session startup: high. A non-portal capture backend can avoid
  portal preauthorization/session waits and user-prompt paths entirely.
- Steady screenshots: medium to high if we remove per-capture GStreamer
  pipeline construction, PNG encode, portal fallback, and D-Bus request churn.
  The remaining model image resize/encode cost still exists unless the new path
  produces the model image directly.
- Continuous/video capture: high. KMS/DMABUF or compositor-copy paths can keep
  frames on GPU and feed hardware scaling/encoding. This matters more for future
  live streaming or high-frequency observation than for single screenshot loops.
- Current agentic workflows: bounded. The existing image-size performance doc
  found backend MCP time under 9 s in older screenshot-heavy TIDAL runs; most
  end-to-end gain came from reducing model image size/tokens. Direct capture
  still matters for cold-start latency, flaky portal environments, and high-rate
  capture, but model ingestion can remain the dominant cost.

Difficulty:

- Persistent current PipeWire pipeline: medium. Best incremental work, less
  permission architecture risk.
- Wayland ext/wlr capture protocols: medium to high. Cleaner than KMS where
  supported, but backend coverage is fragmented.
- DRM/KMS capture helper: high. Real privileged path, but plane composition,
  modifiers, cursor, GPU readback, monitor geometry, and driver differences make
  it a substantial backend.
- DRM writeback: high and opportunistic because hardware support is not
  guaranteed.

Recommendation: do input first, then capture in two tracks. Track A should make
the current PipeWire path persistent and avoid per-frame PNG/pipeline work.
Track B should prototype a privileged KMS/DMABUF capture helper for whole-output
capture, with explicit diagnostics for unsupported modifiers, missing cursor
planes, multi-plane composition, and driver limitations. Add ext-image-copy and
wlr-screencopy backends only where compositor support is detected, not as a
replacement for portal.

### Research sources

- Linux kernel uinput docs: https://docs.kernel.org/input/uinput.html
- libinput Wayland/input stack docs: https://wayland.freedesktop.org/libinput/doc/latest/what-is-libinput.html
- libinput uinput test behavior: https://wayland.freedesktop.org/libinput/doc/latest/test-suite.html
- XDG Desktop Portal ScreenCast docs: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
- XDG Desktop Portal RemoteDesktop docs: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html
- Wayland `ext-image-copy-capture-v1`: https://wayland.app/protocols/ext-image-copy-capture-v1
- Wayland `ext-image-capture-source-v1`: https://wayland.app/protocols/ext-image-capture-source-v1
- Wayland `wlr-screencopy-unstable-v1`: https://wayland.app/protocols/wlr-screencopy-unstable-v1
- Linux DRM/KMS UAPI docs: https://docs.kernel.org/gpu/drm-uapi.html
- Linux DRM/KMS writeback docs: https://docs.kernel.org/gpu/drm-kms.html
- FFmpeg `kmsgrab` docs: https://ffmpeg.org/ffmpeg-devices.html#kmsgrab

## Inventory

| Area | Security measure | Performance cost | Impact of removal | Potential speed gain | Difficulty | Recommendation |
| --- | --- | --- | --- | --- | --- | --- |
| Linux portal permissions | Preauthorize and start combined RemoteDesktop + ScreenCast portal sessions. The startup path tries preauthorization, uses persisted restore tokens, requests keyboard/pointer and monitor sources, and waits after setup. Sources: `crates/sky-cua-linux/src/backend.rs:741`, `crates/sky-cua-linux/src/portal/portal_session.rs:22`, `crates/sky-cua-linux/src/portal/portal_session.rs:35`, `crates/sky-cua-linux/src/portal/portal_session.rs:72`, `crates/sky-cua-linux/src/portal/portal_session.rs:83`, `crates/sky-cua-linux/src/portal/portal_session.rs:190`, `crates/sky-cua-linux/src/portal/preauthorize.rs:68`, `crates/sky-cua-linux/src/portal/preauthorize.rs:152`. | First portal use can spend up to 5 s on preauthorization and 12 s waiting for session start, plus a fixed 120 ms input settle delay. Cached approvals reduce later cost, but D-Bus and token I/O remain on session setup. | Removes portal consent/restore behavior and probably breaks unprivileged Wayland automation on desktops where direct capture/input is unavailable or blocked. A privileged local helper is acceptable for this project if it gives a simpler, faster contract. | High on cold Wayland sessions; low to medium on warm sessions. | High. | Add a direct privileged backend and keep portal as fallback. Privileged input via `uinput` is the clearest first slice; capture may still need compositor/PipeWire-specific work. |
| Portal token persistence | Persist portal restore tokens in per-user state and chmod token directories/files to owner-only permissions. Source: `crates/sky-cua-linux/src/portal/token_store.rs`. | Small JSON read/write and chmod cost when tokens are loaded or refreshed. | More portal prompts, weaker restore behavior, possible token exposure if permissions are loosened. | Low, often negligible. | Low. | Keep. The cost is tiny compared with portal startup. |
| Portal PipeWire capture | Capture Wayland frames through a portal session, duplicate the PipeWire fd, construct a GStreamer `pipewiresrc ! videoconvert ! pngenc ! appsink` pipeline, and wait up to 8 s for a sample. Sources: `crates/sky-cua-linux/src/portal/remote_desktop.rs:130`, `crates/sky-cua-linux/src/portal/remote_desktop.rs:191`, `crates/sky-cua-linux/src/portal/pipewire.rs:104`, `crates/sky-cua-linux/src/portal/pipewire.rs:113`, `crates/sky-cua-linux/src/portal/pipewire.rs:157`. | Per-capture overhead from PipeWire fd handling, GStreamer pipeline setup, PNG encode, buffer mapping, and timeout handling. | Removes the least-privileged Wayland capture path. A privileged helper can own a faster capture path, but Wayland still lacks one universal compositor-agnostic framebuffer API. | High for capture-heavy runs. | High. | Best performance target. First try a persistent non-portal PipeWire/GStreamer path or compositor-specific capture helper; keep portal as fallback. |
| Screenshot portal fallback | Fall back from PipeWire to Screenshot portal when a live frame is missing or failed, while suppressing fallback during pending approvals. Sources: `crates/sky-cua-linux/src/capture_plan.rs:23`, `crates/sky-cua-linux/src/capture_plan.rs:111`, `crates/sky-cua-linux/src/capture_plan.rs:171`, `crates/sky-cua-linux/src/capture_plan.rs:256`. | Extra portal request, file copy, and diagnostics when PipeWire capture fails. | Faster failures but less successful capture on flaky portal/PipeWire setups. Agents would see more hard failures. | Medium in failing portal environments; none on the normal success path. | Medium. | Keep for reliability unless replacing the whole portal capture path. |
| Model screenshot bounding and encode validation | Decode, resize with Lanczos3, and encode model screenshots with bounded dimensions and quality parsing that rejects unsafe values. Sources: `crates/sky-cua-linux/src/portal/screenshot.rs:229`, `crates/sky-cua-linux/src/portal/screenshot.rs:273`, `crates/sky-cua-linux/src/portal/screenshot.rs:287`, `crates/sky-cua-linux/src/portal/screenshot.rs:307`, `crates/sky-cua-linux/src/portal/screenshot.rs:574`. | CPU cost for resize and JPEG/WebP encoding. | Removing bounds can make downstream model payloads much larger, increasing latency and memory even if local capture gets faster. Removing validation accepts pathological env values. | Negative to medium. Faster local encode only if raw images are sent, but end-to-end may slow down. | Low. | Keep bounds. Tune resize filter/quality if image prep becomes a measured hotspot. |
| EIS input validation and pacing | EIS worker validates advertised input regions, waits for device readiness, and inserts hold/frame/flush/key delays. Sources: `crates/sky-cua-linux/src/portal/eis_input.rs:178`, `crates/sky-cua-linux/src/portal/eis_input.rs:438`, `crates/sky-cua-linux/src/portal/eis_input.rs:805`, `crates/sky-cua-linux/src/portal/eis_input.rs:890`, `crates/sky-cua-linux/src/portal/eis_input.rs:892`, `crates/sky-cua-linux/src/portal/eis_input.rs:896`, `crates/sky-cua-linux/src/portal/eis_input.rs:901`. | Per-action delays: 35 ms pointer hold, 20 ms frame gaps, 80 ms final flush, 10 ms text inter-char delay, 140 ms keyboard emulation settle, and up to 3 s worker/device readiness waits. | Faster clicks/typing, but more dropped input, out-of-region pointer events, and flaky keyboard behavior. | Medium for input-heavy automation. | Medium. | Tune with measured desktop matrices. Do not remove validation; consider lowering fixed sleeps behind an env/profile. |
| EIS and legacy fallback retries | Try EIS, reset/rebuild stale sessions, sleep 50 ms before retry, then fall back to legacy D-Bus, XTest, or Linux virtual input paths. Sources: `crates/sky-cua-linux/src/portal/eis_fallback.rs:62`, `crates/sky-cua-linux/src/portal/eis_fallback.rs:80`, `crates/sky-cua-linux/src/portal/eis_fallback.rs:248`, `crates/sky-cua-linux/src/portal/eis_fallback.rs:275`, `crates/sky-cua-linux/src/portal/eis_fallback.rs:463`, `crates/sky-cua-linux/src/portal/eis_fallback.rs:480`, `crates/sky-cua-linux/src/portal/remote_desktop.rs:365`. | Extra latency only on EIS failures or legacy action paths; legacy click includes a 15 ms delay and legacy drag uses 20 ms sleeps. | More hard failures and less cross-desktop compatibility. | Low on healthy EIS; medium on fallback-heavy systems. | Medium. | Keep fallback chain. Add metrics first, then skip known-bad branches per desktop/backend. |
| IPC owner-only socket and daemon singleton | Service socket parent is chmod 0700, socket is chmod 0600, and a singleton flock prevents duplicate daemons. Sources: `crates/sky-cua-service/src/ipc_server.rs:19`, `crates/sky-cua-service/src/ipc_server.rs:61`, `crates/sky-cua-service/src/ipc_server.rs:70`, `crates/sky-cua-service/src/ipc_server.rs:90`, `crates/sky-cua-service/src/ipc_server.rs:215`, `crates/sky-cua-service/src/ipc_server.rs:337`, `crates/sky-cua-service/src/ipc_server.rs:345`. | One-time chmod/flock work at daemon startup; idle timeout/rebind checks add tiny background overhead. | Same-machine users or processes could reach the automation daemon if permissions are loosened; duplicate daemons could race on input/capture state. | Negligible. | Low. | Keep. Not a worthwhile performance target. |
| Client daemon health checks | Client probes an existing daemon, checks repaired desktop/browser env compatibility, and polls startup up to 160 times at 150 ms intervals with short health timeouts. Sources: `crates/sky-cua-client/src/service_launcher.rs:23`, `crates/sky-cua-client/src/service_launcher.rs:25`, `crates/sky-cua-client/src/service_launcher.rs:26`, `crates/sky-cua-client/src/service_launcher.rs:217`, `crates/sky-cua-client/src/launch_environment.rs:259`, `crates/sky-cua-client/src/launch_environment.rs:302`, `crates/sky-cua-client/src/launch_environment.rs:322`, `crates/sky-cua-client/src/launch_environment.rs:337`. | Startup cost when daemon is absent, stale, or unhealthy; cheap on a warm healthy socket. | Faster startup attempts but more wrong-session reuse, stale browser socket reuse, and confusing failures when desktop env differs. | Low on healthy runs; medium during stale daemon recovery. | Medium. | Keep checks, but cache successful environment fingerprints and reduce repeated subprocess probing. |
| Inherited FD cleanup | Each long-lived binary closes inherited file descriptors at startup via `close_range`. Sources: `crates/sky-cua-platform/src/fd_hygiene.rs:13`, `crates/sky-cua-platform/src/fd_hygiene.rs:27`, `crates/sky-cua-client/src/main.rs:18`, `crates/sky-cua-service/src/main.rs:29`. | One startup syscall. | Descriptor leaks; Electron DevTools/native-host listeners can stay bound unintentionally. | Negligible. | Low. | Keep. |
| MCP env allowlists and launch env repair | `.mcp.json` exposes selected env vars; install/preflight emit that allowlist; client and Linux backend repair desktop env from loginctl/systemd/proc and reject stale graphical/browser env. Sources: `.mcp.json:8`, `scripts/install_mcp_server.py:193`, `resources/chrome_preflight.py:154`, `resources/chrome_preflight.py:340`, `crates/sky-cua-platform/src/lib.rs:13`, `crates/sky-cua-platform/src/lib.rs:27`, `crates/sky-cua-platform/src/lib.rs:36`, `crates/sky-cua-client/src/launch_environment.rs:63`, `crates/sky-cua-client/src/launch_environment.rs:120`, `crates/sky-cua-client/src/launch_environment.rs:171`, `crates/sky-cua-linux/src/session_env.rs:21`, `crates/sky-cua-linux/src/session_env.rs:247`, `crates/sky-cua-linux/src/session_env.rs:280`. | Subprocesses and `/proc` reads during detached launch/env repair; allowlist itself can force extra config/deploy work for new overrides. | Removing allowlists can leak host env broadly. Removing repair/rejection can launch into the wrong desktop, bus, browser, or socket. | Low to medium at startup. | Medium. | Performance target is caching and narrower probes, not broad env inheritance. |
| Atomic installer writes and config validation | Installer writes JSON/TOML atomically, preserves modes, validates TOML before/after edits, and refuses malformed Claude/Pi configs. Sources: `scripts/_install_shared.py:40`, `scripts/_install_shared.py:63`, `scripts/_install_shared.py:142`, `scripts/_install_shared.py:156`, `scripts/_install_shared.py:180`, `scripts/install_mcp_server.py:407`, `scripts/install_mcp_server.py:430`, `scripts/install_mcp_server.py:473`, `scripts/install_mcp_server.py:523`, `scripts/install_mcp_server.py:536`. | Install/deploy-only file parsing and temp writes. | Faster but riskier installs: corrupted config, lost user settings, broken partial writes. | Low; runtime none. | Low. | Keep. Not runtime relevant. |
| Claude Code permission routing | Install time denies built-in `computer-use` MCP and allows `sky-cua`. Sources: `scripts/install_mcp_server.py:59`, `scripts/install_mcp_server.py:63`, `scripts/install_mcp_server.py:64`, `scripts/install_mcp_server.py:456`. | Install-time JSON merge only. Runtime effect is usually faster because it avoids approval/route ambiguity. | Removing can reintroduce prompts or accidental built-in tool routing. | Negative or none. | Low. | Keep. This is performance-positive in practice. |
| Browser/native-host preflight validation | Browser preflight reads plugin manifests, materializes a compat plugin, preserves approvals by skipping unchanged rewrites, and writes Chrome native messaging manifests with allowed extension origin. Sources: `resources/chrome_preflight.py:137`, `resources/chrome_preflight.py:288`, `resources/chrome_preflight.py:319`, `resources/chrome_preflight.py:345`, `resources/chrome_preflight.py:381`, `resources/chrome_preflight.py:675`. | Preflight/install-time JSON and file work. Native-host indirection has launch overhead for browser integration. | Removing breaks or weakens browser integration provenance and can force re-approval or wrong extension/native-host pairing. | Low; runtime low after launch. | Medium. | Keep. Skip-current behavior is already a performance optimization. |
| Phone companion identity gate | Companion bootstrap compares installed package/version/signing cert/APK metadata, refuses signature mismatches, optionally installs with `adb install -r`, and only then grants companion permissions. Sources: `crates/sky-cua-service/src/phone/companion/identity.rs:1`, `crates/sky-cua-service/src/phone/companion/identity.rs:121`, `crates/sky-cua-service/src/phone/companion/identity.rs:234`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:75`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:106`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:194`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:509`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:815`. | `pm path`, `dumpsys package`, sidecar parse, hash/metadata comparison, optional install, and permission checks during phone connect/refresh. | Could trust an arbitrary same-package app, run against stale/incompatible companion versions, or grant permissions to the wrong package. | Medium on phone connect; none on desktop-only flows. | Medium. | Keep signature/version gates. Cache identity by serial/package/version if connect latency becomes a hotspot. |
| Phone companion session token and RPC envelope validation | Host generates a per-session token from `/dev/urandom`, delivers it through setup intent, forwards ADB TCP, sends token with each RPC, checks protocol version/id/status, caps responses at 32 MiB, and opens each RPC with `Connection: close`. Sources: `crates/sky-cua-service/src/phone/companion/identity.rs:265`, `crates/sky-cua-service/src/phone/companion/identity.rs:293`, `crates/sky-cua-service/src/phone/companion/identity.rs:307`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:203`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:237`, `crates/sky-cua-service/src/phone/companion/client.rs:43`, `crates/sky-cua-service/src/phone/companion/client.rs:48`, `crates/sky-cua-service/src/phone/companion/client.rs:143`, `crates/sky-cua-service/src/phone/companion/client.rs:353`, `crates/sky-cua-service/src/phone/companion/client.rs:383`, `crates/sky-cua-service/src/phone/companion/client.rs:489`. | Token generation is tiny; setup intent and ADB forward are connect-time; per-RPC overhead includes new TCP connection, JSON envelope checks, timeout, and response cap accounting. | Tokenless local RPC and weak protocol compatibility checks. Removing `Connection: close` without adding keepalive support requires companion/server changes. | Medium on connect; low per RPC, with possible medium gain from connection reuse. | Medium. | Keep auth/envelope checks. Consider persistent HTTP connection or batched RPCs for speed. |
| Phone companion capability retries | After setup intent, capability probe retries transport/unauthorized failures with 400 ms sleeps while the companion starts and installs the token. Source: `crates/sky-cua-service/src/phone/manager/companion_lane.rs:274`, `crates/sky-cua-service/src/phone/manager/companion_lane.rs:292`. | Up to seconds of connect latency when the companion is racing startup/token install. | Faster failure, but installed companions may look unavailable and fall back to slower ADB baseline. | Medium during phone connect. | Low. | Tune retry count/delay if measured; removal can make connect less reliable and slower overall through fallback. |
| Phone ADB permission merging | Companion permission setup reads and merges existing accessibility/notification settings instead of clobbering user services. Source: `crates/sky-cua-service/src/phone/adb/permissions.rs`. | Extra ADB settings/cmd calls during connect/install. | Faster permission setup but can disable unrelated services or misreport permission state. | Low to medium only during setup. | Medium. | Keep. Consider caching known-good permission state per serial/package. |
| Phone command timeouts and bounded subprocess output | All ADB/scrcpy command execution goes through a runner with `kill_on_drop`, captured output, and a default 120 s timeout. Pairing-style sensitive input can go through stdin. Sources: `crates/sky-cua-service/src/phone/command.rs:33`, `crates/sky-cua-service/src/phone/command.rs:34`, `crates/sky-cua-service/src/phone/command.rs:181`, `crates/sky-cua-service/src/phone/command.rs:207`, `crates/sky-cua-service/src/phone/command.rs:236`. | Capturing stdout/stderr costs memory/copying; timeout adds no latency unless commands hang. | Hung `adb`/`scrcpy` can block manager work indefinitely; less diagnostic data; possible sensitive args if stdin path is removed. | Low normally; negative during hangs. | Low. | Keep. Reduce captured output size if a measured command emits huge output. |
| Phone snapshot freshness and coordinate validation | Phone actions require a fresh `phone_snapshot_id` unless explicitly using device coordinates; snapshot resolution validates session, serial, TTL, orientation, resolution, and bounds. Sources: `crates/sky-cua-service/src/phone/snapshot.rs:28`, `crates/sky-cua-service/src/phone/snapshot.rs:161`, `crates/sky-cua-service/src/phone/snapshot.rs:198`, `crates/sky-cua-service/src/phone/snapshot.rs:223`, `crates/sky-cua-service/src/phone/manager/routing.rs:380`, `crates/sky-cua-service/src/phone/manager/routing.rs:396`, `crates/sky-cua-service/src/phone/manager/routing.rs:410`, `crates/sky-cua-service/src/phone/manager/routing.rs:417`, `crates/sky-cua-service/src/phone/manager/routing.rs:636`. | Small per-action lookup/check cost, but the contract encourages observe-before-act loops. | Faster raw-coordinate actions, but more wrong taps after rotation, resize, stale session, or serial mismatch. | Low per action; medium if removing observe-before-act from workflows. | Medium. | Keep validation. Use `use_device_coordinates` only for intentionally raw high-speed flows. |
| Phone capability/profile staleness checks | Cached phone profiles become stale after TTL, drift, wireless disconnect, or permission mismatch; observe can run `adb devices` to mark wireless drops stale. Sources: `crates/sky-cua-service/src/phone/manager/mod.rs:897`, `crates/sky-cua-service/src/phone/manager/mod.rs:914`, `crates/sky-cua-service/src/phone/manager/mod.rs:937`, `crates/sky-cua-service/src/phone/manager/mod.rs:1137`, `crates/sky-cua-service/src/phone/manager/mod.rs:1152`, `crates/sky-cua-service/src/phone/manager/mod.rs:1164`. | Occasional ADB device listing and profile checks. | Stale companion/scrcpy capabilities remain advertised; actions route to broken backends longer. | Low to medium on observe/connect loops. | Medium. | Keep, but tune TTL/profile refresh frequency for high-throughput phone sessions. |
| Desktop snapshot and accessibility selector validation | Desktop actions resolve cached accessibility elements by index or semantic selector, reject out-of-range indexes, filter visible matches, and reject ambiguous semantic matches. Sources: `crates/sky-cua-service/src/snapshot_manager.rs:6`, `crates/sky-cua-service/src/snapshot_manager.rs:25`, `crates/sky-cua-service/src/element_resolver.rs:12`, `crates/sky-cua-service/src/element_resolver.rs:56`, `crates/sky-cua-service/src/element_resolver.rs:100`, `crates/sky-cua-service/src/element_resolver.rs:138`, `crates/sky-cua-service/src/element_resolver.rs:154`. | O(n) scan over cached nodes for semantic selectors; tiny for direct element indexes. | Faster but more ambiguous/wrong UI actions. | Negligible to low. | Low. | Keep. Prefer `element_index` in hot paths. |
| Session presence inhibitors and idle release | Optional presence mode acquires/release lock/suspend/session inhibitors around active requests and checks idle release once per second when enabled. Sources: `crates/sky-cua-service/src/daemon.rs:67`, `crates/sky-cua-service/src/daemon.rs:99`, `crates/sky-cua-service/src/daemon.rs:107`, `crates/sky-cua-service/src/daemon.rs:985`, `crates/sky-cua-service/src/daemon.rs:1016`, `crates/sky-cua-service/src/daemon.rs:1137`. | 1 Hz background task plus D-Bus/logind/screensaver calls when acquiring or releasing. Disabled by default unless env enables it. | Long-running automation may be interrupted by lock/suspend; explicit presence requests stop working. | Negligible unless presence calls are frequent. | Low. | Leave disabled for maximum throughput unless needed. No meaningful gain from deleting it. |
| Overlay and scrcpy cleanup watchdogs | Background watchdog hides idle overlay cursor every 500 ms and polls managed scrcpy liveness every 2 s. Sources: `crates/sky-cua-service/src/daemon.rs:78`, `crates/sky-cua-service/src/daemon.rs:81`, `crates/sky-cua-service/src/daemon.rs:127`, `crates/sky-cua-service/src/daemon.rs:132`, `crates/sky-cua-service/src/daemon.rs:138`. | Tiny periodic wakeups and mutex checks. | Stale visible cursor/overlay state; dead scrcpy mirror can stay advertised as active. | Negligible. | Low. | Keep. |

## Practical Removal Order If Optimizing Hard

1. Measure capture latency first. If portal capture dominates, prototype a
   privileged fast backend before touching smaller checks. Input can likely use
   a root/capability-backed `uinput` helper; capture should start with a
   persistent non-portal PipeWire/GStreamer path or compositor-specific helper.
2. Measure phone connect separately from phone action latency. If connect is the
   issue, cache companion identity/permission/capability state per serial.
3. Add a high-performance profile for EIS sleeps and retry counts, guarded by
   explicit env flags, then test across KDE, GNOME, COSMIC, Hyprland, and X11.
4. Cache launch-environment probes and daemon health fingerprints for detached
   MCP launches.
5. Leave chmod/flock/fd-cleanup/atomic-write/snapshot-bounds checks alone unless
   a profiler proves otherwise.
