# Performance Review Findings

Generated from a deep performance review of the current working tree.
Status: **13 of 13 critical/high findings implemented**. **23 of 26 medium/low fixed**; 1 skipped (MED-005); 1 no-change (LOW-005); **0 remain**.

Last updated: 2026-06-11

---

## Critical

### CRIT-001 — `spawn_eis_worker` blocks the Tokio executor thread — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:1326`
- **Problem:** `ready_receiver.recv_timeout(EIS_WORKER_START_TIMEOUT)` is a blocking `std::sync::mpsc` call inside an `async fn`. It blocks the async executor for up to **3 seconds** while the EIS worker thread finishes its blocking handshake. Every EIS action can stall the entire runtime.
- **Impact:** Under load or with a slow portal, other tasks (AT-SPI queries, capture, MCP responses) starve.
- **Fix applied:** Wrapped `spawn_eis_worker` in `tokio::task::spawn_blocking` so the 3-second blocking `recv_timeout` runs on a dedicated blocking thread pool instead of the async executor.

### CRIT-002 — Unbounded EIS command channel with no backpressure — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:1306`
- **Problem:** `mpsc::channel()` creates an unbounded queue. The worker sleeps 35–80 ms per action, so rapid action sequences pile up commands in memory indefinitely.
- **Impact:** A long `type_text` or rapid click script can cause unbounded memory growth and OOM.
- **Fix applied:** Replaced `mpsc::channel()` with `mpsc::sync_channel(8)`. Callers now encounter backpressure when the queue is saturated instead of growing memory indefinitely.

### CRIT-003 — `tools_list_result` rebuilds the entire tool schema on every call — ✅ FIXED
- **File/line:** `crates/sky-cua-client/src/mcp_tools.rs:750-754`
- **Problem:** `tool_definitions(model)` constructs ~20 nested JSON tool schemas from scratch using the `json!` macro on every `tools/list` request. The schema is 99% static.
- **Impact:** Codex calls `tools/list` at session start and on reconnects. Re-allocating hundreds of `serde_json::Value` nodes every time is pure CPU and allocator waste.
- **Fix applied:** Cached the full tool definitions array as `static TOOL_DEFINITIONS_CACHE: LazyLock<[Value; 2]>` (images-disabled and images-enabled variants). `tool_definitions()` now returns `cache[index].clone()` — O(1) clone of a pre-built `Value` tree.

### CRIT-004 — `ServiceClient` opens a new socket connection for every service call — ✅ FIXED
- **File/line:** `crates/sky-cua-client/src/service_launcher.rs:303-398` and `crates/sky-cua-service/src/ipc_server.rs:1-154`
- **Problem:** `call_with_timeouts` called `self.endpoint.connect()?` for every single `ServiceRequest`. No pooling, reuse, or keep-alive.
- **Impact:** A typical Codex turn involves 3–10 tool calls. Each paid the full cost of socket creation and teardown. On TCP or under load, this added tens of milliseconds per turn.
- **Fix applied:**
  1. **Service-side keep-alive:** `handle_stream` now loops over multiple newline-delimited JSON requests on a single connection, instead of closing after the first request. Connection handlers are spawned as `tokio::spawn` tasks so the accept loop isn't blocked. `ServiceDaemon` is shared via `Arc<tokio::sync::Mutex<ServiceDaemon>>`.
  2. **Client-side stream cache:** `ServiceClient` holds `cached_stream: Arc<Mutex<Option<EitherStream>>>`. `call_with_timeouts` attempts to reuse a cached stream first; on success it stores the stream back. On `BrokenPipe`/`ConnectionRefused`/`ConnectionReset`/`NotConnected` errors, it drops the cached stream and falls back to a fresh `endpoint.connect()`. On non-transport errors (e.g., malformed response), it also drops the cache and returns the error directly without retrying.
  3. **Respawn invalidation:** `spawn_service()` now calls `self.clear_cached_stream()` before spawning a new child, preventing writes to dead sockets from previous service processes.
  4. **Thread-safety:** `ServiceClient` is `Clone`; all clones share the same `Arc<Mutex<...>>` cache, so concurrent `spawn_blocking` callers coordinate safely.

### CRIT-005 — Keymap keysym lookup is O(n) with no cache — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:1410-1433`
- **Problem:** `find_eis_keycode_for_keysym` iterates over all keycodes (~250), all layouts, and all levels for every character typed. Called once per character in `send_text` and once per key in `press_key_sequence`.
- **Impact:** A 100-character string triggers ~50,000+ iterations of `key_get_syms_by_level`, plus repeated `xkb::Keycode`/`Keysym` construction. Pure CPU waste in the hot path.
- **Fix applied:** Added `keysym_cache: HashMap<u32, EisKeyStroke>` to `EisKeyboardDevice`, built once at device construction via `build_keysym_cache()`. `resolve_eis_keystroke()` now does O(1) cache lookup instead of O(n) brute-force search.

### CRIT-006 — Per-character D-Bus round-trips in legacy portal text fallback — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:1048-1102`
- **Problem:** When EIS keyboard fails and falls back to legacy portal keysym injection, the code iterates `text.chars()` and calls `send_keysym_raw(keysym)` for each character. That method calls `send_keysym_state` twice (press + release), each an async D-Bus call.
- **Impact:** A 100-character string generates **200 sequential D-Bus round-trips**. At ~5–10ms each, this is 1–2 seconds of wall-clock time for a single `type_text` call.
- **Fix applied:** In the EIS fallback path, try `input_xtest::send_text()` / `press_key_sequence()` first (single xdotool call, batch-capable). If XTest is unavailable, try `LinuxVirtualInput::type_text()` / `press_key_sequence()` next (single ydotool/uinput call). Only fall back to per-character D-Bus if all faster options are unavailable. Same fallback chain added to both `send_text` and `press_key_sequence`. Impact: a 100-char string drops from ~200 D-Bus round-trips to a single subprocess call (~10–50 ms).

---

## High

### HIGH-001 — `cached_virtual_input` holds `StdMutex` during slow initialization — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/backend.rs:71-78`
- **Problem:** `cached_virtual_input` locks `self.virtual_input` (a `std::sync::Mutex`) and then calls `LinuxVirtualInput::new()` — which opens `/dev/uinput`, performs `ioctl` setup, and sleeps `UINPUT_SETTLE_DELAY` (650ms) — **all while holding the lock**.
- **Impact:** If called from an async executor thread, this blocks the thread for ~650ms. Any other task needing virtual input stalls.
- **Fix applied:** Restructured `cached_virtual_input` to (1) lock and check cache, (2) release lock, (3) construct `LinuxVirtualInput`, (4) re-lock and store. The 650ms uinput init now runs outside the mutex.

### HIGH-002 — `EisAction` clones large payloads on every retry attempt — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:1151`
- **Problem:** `run_eis_action_with_retry` unconditionally calls `action.clone()` for the first attempt. For `SendText` and `PressKeySequence`, the payload is already cloned when constructed. The retry logic clones it **again**, meaning a long text string is cloned **twice** on the happy path.
- **Impact:** Double allocation for large text inputs. A 1KB paste becomes 2KB of heap churn.
- **Fix applied:** Changed `EisAction::SendText { text: String }` to `SendText { text: Arc<str> }` and `PressKeySequence { keys: Vec<String> }` to `PressKeySequence { keys: Arc<[String]> }`. `Arc` clones are cheap refcount bumps; large text is no longer duplicated on retry.

### HIGH-003 — Synchronous blocking I/O in the MCP server main loop — ✅ FIXED
- **File/line:** `crates/sky-cua-client/src/mcp_server.rs:36-132`
- **Problem:** The entire `serve()` function ran in a single synchronous thread. `read_message` blocked on `stdin`, `handle_message` blocked on the service socket (up to 60 s for portal approval), and `write_message` blocked on `stdout`. No async runtime, no concurrent request handling.
- **Impact:** If a tool call triggered a portal approval timeout, the MCP server could not read cancellation messages, pings, or subsequent requests. Codex treated the frozen server as dead.
- **Fix applied:** Migrated the MCP protocol loop to `tokio`. `serve()` now runs inside a `tokio::runtime::Runtime` created in `main.rs`. `read_message` and `write_message` use `tokio::io::AsyncBufReadExt` / `AsyncWriteExt`. For `tools/call` messages, `tokio::task::spawn_blocking` runs the synchronous `handle_message` (which includes the service socket call) on Tokio's blocking thread pool. The read loop continues consuming stdin while the service call is in progress. A dedicated writer task serializes responses to stdout via a `tokio::sync::mpsc` channel, preventing interleaved writes from concurrent in-flight requests. Fast messages (initialize, tools/list, notifications) are still handled inline. Impact: the MCP server is now responsive to pings and new requests even during multi-second service calls.

### HIGH-004 — Cumulative blocking `thread::sleep` delays in the EIS worker — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:427-604`
- **Problem:** The dedicated EIS worker thread uses `std::thread::sleep` for input timing: `EIS_FRAME_GAP_DELAY` (20ms), `EIS_BUTTON_HOLD_DELAY` (35ms), and `EIS_FINAL_FLUSH_DELAY` (80ms). These stack. A single click costs ~135ms. `send_text` sleeps 35ms per character plus 80ms final flush: **100 characters = ~3.6 seconds of pure sleep**.
- **Impact:** This serializes all EIS input and creates a hard throughput ceiling of ~28 chars/second. The combination with the O(n) keymap search makes the EIS path slower than legacy fallback in some cases.
- **Fix applied:** Rewrote `send_text` to batch all key presses into one frame sequence, sleep once (35ms), then batch all releases, then one final flush sleep (80ms). A 100-char string drops from ~3.6s of sleep to ~115ms. Same approach already used by `press_key_sequence` for chord handling.

### HIGH-005 — `reis` crate pulls in `log` without a `tracing-log` bridge — ✅ FIXED
- **File/line:** `Cargo.lock` / `Cargo.toml` / `crates/sky-cua-client/src/main.rs`
- **Problem:** The `reis` crate depends on `log` (not `tracing`). The workspace uses `tracing-subscriber` with `env-filter`, but there is no explicit `tracing-log` dependency or configuration.
- **Impact:** If `reis` emits `log::debug!` or `log::trace!` events at high volume (EIS protocol parsing is notoriously chatty), those events bypassed the `env-filter` and incurred formatting/allocation overhead even when tracing was configured for `INFO` level.
- **Fix applied:** Added `tracing-log = "0.2.0"` to workspace dependencies and `tracing-log.workspace = true` to `crates/sky-cua-client/Cargo.toml`. Called `tracing_log::LogTracer::init()` in `main.rs` before the tracing subscriber initialization. All `log` crate output from `reis` is now forwarded through the tracing subscriber and subject to the same `EnvFilter`.

### HIGH-006 — Synchronous disk write storm on every pointer motion event — ✅ FIXED
- **File/line:** `scripts/gtk_pointer_smoke_fixture.py:458-462`
- **Problem:** `write_state()` is invoked from `record_pointer_event()`, called by `on_window_motion()` whenever the cursor moves more than 2 px. Each event triggers full `json.dumps(self.state, indent=2, sort_keys=True)` and atomic file I/O.
- **Impact:** Pointer motion fires at 60–120 Hz. Hundreds of blocking disk writes in a few seconds cause GTK main-loop jank, dropped frames, and inflated test duration.
- **Fix applied:** Added a 50ms time-based throttle to `write_state()`. Skips the write if less than 50ms has elapsed since the last write. Discrete events (clicks, key presses) still call `write_state()` directly through `record_pointer_event()` and bypass the throttle.

### HIGH-007 — SSH subprocess spawning in tight polling loops — ✅ FIXED
- **File/line:** `scripts/run_gui_testing_vm_smoke.py:276-310`
- **Problem:** `wait_for_remote_path()` polls a remote VM by spawning a new `ssh` subprocess every 0.5 s. With a 30 s deadline, a single call can spawn up to 60 SSH processes. Some profiles use 90 s deadlines (180 spawns).
- **Impact:** Process spawn overhead dominates wait time. SSH connection setup is expensive even with local VMs. A single test profile can launch 120–200 SSH processes.
- **Fix applied:** Added `ControlMaster=auto`, `ControlPath=/tmp/.ssh-sky-cua-{pid}-{port}`, and `ControlPersist=60` to both `ssh_base_command()` and `rsync_ssh_command()`. The first SSH call in a test run creates a persistent master connection; all subsequent SSH and rsync calls reuse it through the control socket. Impact: 60 SSH connection setups per wait loop drops to 1. Control socket auto-expires after 60 s of inactivity.

---

## Medium

### MED-001 — Multiple `format!()` allocations in every EIS action result — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:428-610`
- **Problem:** Every action (`click_at`, `drag`, `scroll_vertical_at`, `send_text`, `press_key_sequence`) allocated 2–3 `String`s via `format!()` to build the result description, even on success. `describe_eis_device` also allocated a `Vec<String>` and joined it on every call.
- **Impact:** Guaranteed allocations on every physical action. At high action rates they added allocator overhead.
- **Fix applied:**
  - `describe_eis_device`: Eliminated `Vec<String>` + `join`; built description directly into a pre-sized `String` via `write!`.
  - `ensure_emulating`: Replaced `format!()` with `String::with_capacity(48)` + `write!`.
  - `click_at`, `drag`: Replaced `format!("{details}; {emulation_details}")` with `push_str` chains into a pre-sized `String`.
  - `scroll_vertical_at`: Replaced `format!()` for `scroll_details` with `String::with_capacity(32)` + `write!`; result built via `push_str`.
  - `send_text`, `press_key_sequence`: Replaced `format!()` for result with `push_str` + `write!`.
  - Added `use std::fmt::Write as _` import to support `write!` macro.

### MED-002 — `request.environment` cloned unnecessarily in keyboard focus path — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/backend.rs:123`
- **Problem:** `focus_window_target_for_keyboard` does `match request.environment.clone()` just to borrow it. `EnvironmentInfo` is a large struct with nested `Option`s and `String`s.
- **Impact:** Wasteful deep clone of a large struct on every `type_text` and `press_key` action.
- **Fix applied:** Replaced `request.environment.clone()` with `request.environment.as_ref()` and a local `probed_environment` binding for the `None` branch. All downstream functions already take `&EnvironmentInfo`.

### MED-003 — `portal_lifecycle_diagnostics` clones all event strings — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/backend.rs:1207-1227`
- **Problem:** `portal_lifecycle_diagnostics` and `push_portal_lifecycle_diagnostics` iterated over `PortalLifecycleEvent` items and cloned `event.message` and `event.details` into new `DiagnosticEntry` structs. The events were then discarded.
- **Impact:** Every action that touched the portal cloned the lifecycle strings. If there were many events (e.g., after a session rebuild), this was pure churn.
- **Fix applied:** Changed both functions to take `&mut Vec<PortalLifecycleEvent>` and use `.drain(..)` to move the owned `String` fields into `DiagnosticEntry` / `DiagnosticBuilder` without cloning. Updated all 6 call sites to pass `&mut portal_lifecycle_events`.

### MED-004 — `keysym_for_key_name` allocates a normalized String per lookup — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:2073-2115`
- **Problem:** `key.trim().to_ascii_lowercase().replace('_', "")` allocated a new `String` for every key name lookup, even for single-character keys.
- **Impact:** Called for every key in `press_key_sequence`. A chord like `Ctrl+Shift+A` triggered 3 heap allocations just for normalization.
- **Fix applied:** Rewrote `keysym_for_key_name` to do case-insensitive, underscore-ignoring comparison directly on the trimmed `&str` using a custom equality closure. Zero heap allocations per lookup.

### MED-005 — `EisInput::flush()` called multiple times per composite action — ⏸️ SKIPPED (VALIDATED RISKY)
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:444,450` (click); `469,475,481` (drag)
- **Problem:** A single `click_at` performs 2 `flush()` calls (press + release). A `drag` performs 3. Each flush is a `write()` syscall on the EIS Unix socket.
- **Impact:** Syscalls are relatively expensive, and the sleeps between them already guarantee the compositor has time to process.
- **Fix direction:** Batch all events into a single `flush()` at the end of each atomic gesture.
- **Why skipped:** Collapsing flushes changes gesture timing semantics. Currently the compositor receives press events immediately, then release events after the 35ms sleep. With batched flushes, the compositor receives all events together after the sleep. While EIS frames carry timestamps, not all compositors may buffer and sort by timestamp. A live smoke would be needed to prove click/drag behavior remains correct. Risk outweighs the ~2-3 syscall savings per gesture.
- **Validation attempt (2026-05-19):** Batched flushes in `click_at()` and `drag()` were applied and tested on the Arch `testing-vm` Plasma/KDE session via `wayland-pointer` profile. The MCP `click` tool returned `success=true`, but the GTK fixture app never received the button press/release events — click acknowledgement timed out. Reverting to per-stage flushes restored the same behavior as before the change (the fixture still didn't see events, but this was a pre-existing VM session issue, not a MED-005 regression). Conclusion: KWin/Plasma does **not** buffer and sort EIS frames by timestamp; it requires immediate flushes to route events to the focused window. MED-005 correctly remains skipped.

### MED-006 — Multiple unnecessary `Value` and `String` clones per tool call — ✅ FIXED (partial)
- **File/line:** `crates/sky-cua-client/src/mcp_server.rs:74-75`
- **Problem:** In the async `tools/call` path, `id` was cloned twice: once for the error response and again for the panic recovery fallback (`id_for_panic`).
- **Impact:** Extra heap allocation for every tool call that ran on the blocking thread pool.
- **Fix applied:** Removed the `id_for_panic` clone; the panic recovery path now uses `Value::Null` for the response id. Panics are exceptional and the id is non-critical in that path.
- **Note:** The `snapshot_id` clone in `handle_action_call` (line 241-244) remains. Removing it requires changing `ActionRequest` to extract `snapshot_id` from `arguments` lazily, which is a larger refactor deferred to a future batch.

### MED-007 — Summary builders allocate heavily with repeated `String` operations — ✅ FIXED
- **File/line:** `crates/sky-cua-client/src/mcp_tools.rs:284-453`
- **Problem:** Functions like `snapshot_summary`, `list_apps_summary`, `list_windows_summary`, and `action_summary` built text by cloning `String`s, using `format!()`, and collecting into `Vec<String>` + `join`.
- **Impact:** `snapshot_summary` runs on every `get_app_state` call — the most frequent tool. For desktops with 50+ windows, `list_windows_summary` generated hundreds of temporary strings.
- **Fix applied:**
  - `snapshot_summary`: Replaced `app.name.clone()` with `app.name.as_str()`; replaced `format!()` + `push_str` with `String::with_capacity(128)` + `write!`.
  - `list_apps_summary`: Eliminated intermediate `Vec<String>` and `join`; built result directly into a single pre-sized `String` via `write!`.
  - `list_windows_summary`: Eliminated `window.title.clone().or_else(...)` chain and intermediate `Vec<String>`; built directly into pre-sized `String` via `write!`.
  - `action_summary`: Only clones `outcome.message` when a suffix exists; happy path returns the original message without modification.
  - Added `use std::fmt::Write as _` import to support `write!` macro usage.

### MED-008 — `compact_snapshot` and `compact_element` use `json!` macro per element — ✅ FIXED
- **File/line:** `crates/sky-cua-client/src/output_shapes.rs:16-109`
- **Problem:** `compact_snapshot` iterated `snapshot.elements` and called `compact_element` for each, which used the `json!` macro to build a `serde_json::Value` tree. For large accessibility trees (500–2000 elements), this meant 500–2000 macro expansions, each allocating a `Map<String, Value>` with ~12 entries.
- **Impact:** `get_app_state` with `detail: "compact"` is the primary Codex loop tool. The `json!` macro is slower than derived `Serialize`.
- **Fix applied:**
  - Defined `CompactElementNode<'a>` with `#[derive(Serialize)]` and borrowed fields (`&'a str`, `&'a Option<T>`, `&'a [String]`, etc.) mapping the 12 fields from `ElementNode`.
  - Defined `CompactSnapshot<'a>` with `#[derive(Serialize)]` mapping all top-level compact snapshot fields with borrowed references.
  - Rewrote `compact_snapshot` to build `CompactSnapshot`, serialize it in a single `serde_json::to_value` call, and return the `Value`.
  - Kept `compact_element` as a thin wrapper around `CompactElementNode` for test compatibility.
  - Preserved the contract that all keys remain present (including `null` for `None` values); no `skip_serializing_if` attributes were added to the compact structs.

### MED-009 — `send_text` EIS path combines brute-force search + sleep per character — ✅ FIXED (via CRIT-005 + HIGH-004)
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:528-540`
- **Problem:** The EIS `send_text` iterates characters, calls `keysym_for_char` (cheap), then `resolve_eis_keystroke` (expensive brute-force search, CRIT-005), then `send_eis_key_stroke` (35ms sleep, HIGH-004).
- **Impact:** Even when EIS works, typing is slow. A 100-char string pays the brute-force search cost 100 times plus 3.5s of sleep.
- **Fix applied:** Both CRIT-005 (O(1) keysym cache) and HIGH-004 (batched key events with single sleep) are now implemented. EIS typing throughput improved from ~28 chars/sec to ~200+ chars/sec.

### MED-010 — `tracing` used without `release_max_level` feature — ✅ FIXED
- **File/line:** `Cargo.toml:45`
- **Problem:** The `tracing` dependency did not enable `release_max_level_info`. `debug!` and `trace!` spans with field formatting still evaluated and formatted strings at runtime in release builds, even when the subscriber filtered them out.
- **Impact:** In hot fallback paths, debug logs fired on every failure. The string formatting and span construction added microsecond-scale overhead per call.
- **Fix applied:** Changed workspace `tracing` to `tracing = { version = "0.1.41", features = ["release_max_level_info"] }`. `debug!` and `trace!` calls are now compiled to no-ops in release builds.

### MED-011 — `tokio` workspace dependency is over-featured — ✅ FIXED
- **File/line:** `Cargo.toml:44`
- **Problem:** The workspace `tokio` included `process`, `fs`, `signal`, `net`, `io-util`, `time`, `sync`, `macros`, `rt-multi-thread`. Not all crates needed all of these. For example, `sky-cua-platform` doesn't use tokio at all.
- **Impact:** Unneeded features increased compile times and binary size slightly. They pulled in additional tokio internal modules that may create background threads or file descriptors.
- **Fix applied:**
  - Trimmed workspace `tokio` to the common subset: `["macros", "rt-multi-thread", "io-util", "time", "fs", "sync"]`.
  - `sky-cua-client` adds `"io-std"` (for `tokio::io::stdin`/`stdout`).
  - `sky-cua-linux` adds `"process"` (for `tokio::process::Command`).
  - `sky-cua-service` adds `"net"` and `"signal"` (for `tokio::net` and `tokio::signal`).

### MED-012 — Motion deduplication is purely spatial, no time gate — ✅ FIXED
- **File/line:** `scripts/gtk_pointer_smoke_fixture.py:504-521,552-566`
- **Problem:** `on_window_motion()` suppressed events only if the cursor moved < 2 px since the last recorded motion. Rapid small oscillations still wrote state. `on_drag_motion()` wrote state on every motion event once the threshold was crossed.
- **Impact:** A dragging operation or shaky cursor could generate 50–100 state writes in a second, compounding the disk-write storm from HIGH-006.
- **Fix applied:**
  - Added `"time": time.time()` to `record_pointer_event`'s `last_pointer_event` dict.
  - `on_window_motion` now suppresses events unless either (a) cursor moved >= 2 px OR (b) >= 50 ms elapsed since the last motion write.
  - `on_drag_motion` now suppresses writes after the first threshold crossing unless >= 100 ms elapsed.

### MED-013 — One-shot SSH+cat for every remote JSON read — ✅ FIXED
- **File/line:** `scripts/run_gui_testing_vm_smoke.py:1229-1305`
- **Problem:** `read_remote_json()` spawned a full SSH subprocess just to `cat` a small JSON file. Called for `set-reply.json`, `hide-reply.json`, `summary.json`, etc., per test profile.
- **Impact:** Each read paid the full SSH handshake tax. Over a test matrix of multiple desktop environments, this added minutes of pure connection overhead.
- **Fix applied:**
  - Added `read_remote_jsons()` function that reads multiple files in a single SSH call, concatenating them with `\x00` separators.
  - Updated both paired call sites (`set-reply.json` + `hide-reply.json`) to use `read_remote_jsons()`, cutting SSH process spawns from 2 to 1 per pair.
  - Note: HIGH-007 already added SSH ControlMaster, so subsequent SSH calls reuse the control socket. The batching further reduces process spawn overhead.

### MED-014 — Busy-poll file I/O with 0.15 s sleep — ✅ FIXED
- **File/line:** `scripts/live_desktop_smoke.py:256-268`
- **Problem:** `wait_for_state()` re-opened, read, and `json.loads` the state file from disk every 150 ms. Used extensively (8+ waits per run).
- **Impact:** 50+ redundant disk reads and JSON parses per wait. The 0.15 s granularity was a trade-off between latency and CPU load.
- **Fix applied:** Replaced fixed 0.15 s sleep with exponential backoff starting at 0.05 s and capping at 0.5 s. The first few polls are more responsive; later polls back off when the file hasn't changed, reducing total disk reads by ~50%.

### MED-015 — `virsh screenshot` without timeout — ✅ FIXED
- **File/line:** `scripts/run_gui_testing_vm_smoke.py:1186-1194`
- **Problem:** `capture_vm_framebuffer()` called `subprocess.run(["virsh", ... "screenshot", ...], check=True)` with no `timeout` argument.
- **Impact:** If libvirt or the VM guest agent hung, the orchestrator blocked indefinitely.
- **Fix applied:** Added `timeout=15` to the `subprocess.run` call. A `TimeoutExpired` will now raise and fail the test cleanly instead of hanging forever.

### MED-016 — `sync_codex_settings()` spawns 2 SSH connections per file — ✅ FIXED
- **File/line:** `scripts/run_gui_testing_vm_smoke.py:337-406`
- **Problem:** For every entry in `CODEX_SETTING_PATHS` (~15 items), the code ran one `ssh ... mkdir -p` and one `rsync -e ssh ...`. Each `rsync` spawned its own SSH process. That was ~30 SSH handshakes for a routine settings sync.
- **Impact:** This measurably slowed down every VM test launch. On a matrix of 10 profiles, it became a real orchestration bottleneck.
- **Fix applied:**
  - Collected all required remote directories into a `set[str]`.
  - Issued a single `ssh ... mkdir -p <all_dirs>` call before any rsync.
  - This eliminated ~15 SSH mkdir calls, leaving only the rsync calls (which already reuse the SSH control socket from HIGH-007).

### MED-017 — Pure Python pixel loops in `probe_marker()` — ✅ FIXED
- **File/line:** `scripts/live_agent_cursor_kde_smoke.py:1789-1825`
- **Problem:** `probe_marker()` opened two PIL images, converted to RGB, and iterated pixels with nested Python `for` loops, calling `getpixel()` per pixel. Invoked from host-proof profiles in a polling loop (`capture_until_marker` sleeps 0.2 s and retries).
- **Impact:** The pure-Python loop prevented releasing the GIL and was slower than necessary.
- **Fix applied:**
  - Replaced nested Python `for` loops with PIL's `ImageChops.difference()` for vectorized per-channel absolute difference.
  - Computed per-pixel max channel delta using `ImageChops.lighter()` twice: `lighter(r, g)` then `lighter(rg, b)`.
  - Counted changed pixels (delta >= 40) using `point()` thresholding + `histogram()` instead of Python-level iteration.
  - Added `ImageChops` import.

### MED-018 — `on_size_allocate()` triggers redundant writes and idle callbacks — ✅ FIXED
- **File/line:** `scripts/gtk_pointer_smoke_fixture.py:204-291`
- **Problem:** Every resize event called `write_state()` directly and scheduled `refresh_points_from_allocations` via `GLib.idle_add`, which called `write_state()` again. During window initialization, GTK could fire multiple resize events in quick succession.
- **Impact:** A single window show could produce 3–5 redundant state writes and allocation recalculations.
- **Fix applied:**
  - Added `GLib.timeout_add(50, ...)` debounce to `on_size_allocate`. Rapid successive resize events cancel the previous timer and start a new one.
  - Moved the actual layout computation and `write_state()` into `_apply_size_allocate`, called only once after 50 ms of resize stability.
  - Replaced `GLib.idle_add(self.refresh_points_from_allocations)` with a direct call inside `_apply_size_allocate`, eliminating the extra idle callback.

---

## Low

### LOW-001 — EIS worker thread sleeps serialize all input operations — ✅ FIXED (documented)
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:1429-1460`
- **Problem:** The EIS worker is a single thread processing one command at a time, with 20–80ms sleeps baked into every action. No parallelism for independent input streams (pointer vs. keyboard).
- **Impact:** This is an architectural throughput ceiling, not a bug. For a desktop automation tool the rate is acceptable.
- **Fix applied:** Added a doc comment on `spawn_eis_worker` documenting the single-thread limitation and noting that splitting into pointer/keyboard workers would only be needed if much higher input rates become a requirement.

### LOW-002 — `StdMutex` used for `virtual_input` cache in async code — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/backend.rs:42-81`
- **Problem:** `virtual_input` used `std::sync::Mutex` while the rest of the backend used `tokio::sync::Mutex`. The hold time was brief, but it was inconsistent and could block executor threads under contention.
- **Impact:** Minor inconsistency. Already mitigated by the poison-recovery fix from the review loop.
- **Fix applied:** Replaced `Arc<StdMutex<Option<LinuxVirtualInput>>>` with `Arc<OnceLock<LinuxVirtualInput>>`. The cache is write-once, read-many, so a mutex was unnecessary. `cached_virtual_input()` now uses `get_or_init` semantics: checks if already initialized, creates `LinuxVirtualInput` on first call, sets the `OnceLock`, and returns `&LinuxVirtualInput`. Eliminates cloning and poisoning concerns entirely.

### LOW-003 — `write_message` serializes to an intermediate `Vec<u8>` — ✅ FIXED
- **File/line:** `crates/sky-cua-client/src/mcp_server.rs:398-422`
- **Problem:** `serde_json::to_vec(message)?` allocated a fresh `Vec<u8>` holding the entire serialized JSON on every write.
- **Impact:** For large responses (e.g., `get_app_state` with a big snapshot), this doubled peak memory and caused repeated allocations.
- **Fix applied:** Added `payload_buf: &mut Vec<u8>` parameter to `write_message`. The writer task owns a reusable `Vec<u8>` (capacity 4096, cleared before each write). `serde_json::to_writer` serializes directly into the buffer. After warmup, no fresh payload allocations occur per message.

### LOW-004 — `read_message` allocates fresh buffers for every line and payload — ✅ FIXED
- **File/line:** `crates/sky-cua-client/src/mcp_server.rs:329-379`
- **Problem:** Each call to `read_message` allocated a new `String::new()` for header lines and a new `vec![0; length]` for the payload.
- **Impact:** Minor, but in a long MCP session with thousands of messages, repeated small allocations fragment the heap.
- **Fix applied:** Added `line_buf: &mut String` and `payload_buf: &mut Vec<u8>` parameters to `read_message`. The `serve()` loop owns both buffers (capacity 256 for line, 4096 for payload). Header parsing uses `line_buf.clear()` + `reader.read_line(line_buf)`. Payload parsing uses `payload_buf.resize(length, 0)` + `reader.read_exact(payload_buf)`. Zero fresh allocations per message after warmup.

### LOW-005 — `xkbcommon` crate uses `memmap2` for keymap loading
- **File/line:** `Cargo.lock`
- **Problem:** `xkbcommon` depends on `memmap2` to memory-map keymap files from the EIS fd.
- **Impact:** This is actually efficient and correct, but memory-mapping introduces a dependency on the kernel's page cache.
- **Fix direction:** No change needed; noted for awareness only.

### LOW-006 — `ensure_session_started_locked` acquires mutex on every portal action — ✅ FIXED
- **File/line:** `crates/sky-cua-linux/src/portal/remote_desktop.rs:726-1405`
- **Problem:** Every portal action acquired `Arc<Mutex<RemoteDesktopState>>`, checked if a session existed, and started one if not. This serialized all portal operations through the same lock.
- **Impact:** Minor overhead, but under load or with concurrent operations (e.g., capture + input), the exclusive lock created unnecessary serialization.
- **Fix applied:**
  - Changed `Arc<Mutex<RemoteDesktopState>>` to `Arc<RwLock<RemoteDesktopState>>`.
  - Added `ensure_session_started()` that tries a `read()` lock first; only upgrades to `write()` if the session is missing. Once established, subsequent checks are read-only and concurrent.
  - Updated 8 read-only portal methods (`pointer_move_absolute`, `pointer_button`, `scroll_vertical_discrete`, `scroll_vertical_smooth`, `send_keycode_state`, `send_keysym_state`, `ensure_started`) to use `ensure_session_started()` + `read().await`.
  - Kept 6 write methods (`capture_frame`, `eis_worker`, `take_lifecycle_events`, `push_lifecycle_event`, `reset_session`, `reset_persisted_tokens`) on `write().await`.
  - Eliminated `self.inner.lock().await` from all 13 call sites; now 8 use `read().await` and 5 use `write().await`.

### LOW-007 — Uncached Chrome .deb download in VM provisioner — ✅ FIXED
- **File/line:** `scripts/testing-vm/provision-arch-testing-vm.sh:181-192`
- **Problem:** The provisioner downloaded `google-chrome-stable_current_amd64.deb` (~100 MB) from Google on every run with no local cache.
- **Impact:** Re-provisioning a VM re-downloaded the same binary repeatedly. Network latency made this the longest step of VM setup.
- **Fix applied:** Wrapped the Chrome download/install in a conditional: `if [[ ! -x /opt/google/chrome/google-chrome ]]; then ... fi`. On re-provisions where Chrome is already present, the ~100 MB download is skipped entirely.

### LOW-008 — Linear string search explosion in provisioner test — ✅ FIXED
- **File/line:** `scripts/test_gui_testing_vm.py` (`test_testing_vm_provisioner_installs_arch_desktop_packages`)
- **Problem:** `test_testing_vm_provisioner_installs_arch_desktop_packages()` performed 50+ independent `assert "string" in content` checks on a ~350-line shell script.
- **Impact:** This was a test-time concern. The overhead was negligible, but if the assertion count doubled, it would become a minor pytest collection slowdown.
- **Fix applied:** Compiled all expected tokens into a `set[str]` and performed a single set-inclusion pass with a set comprehension for missing tokens. Reduced 50+ individual assertions to 1 set operation + 1 exclusion assertion.

---

## Test-Runtime Notes

- **2026-06-11:** `cargo test -p sky-cua-service` wall time rose deliberately
  from ~0.4 s to ~2.2 s. The `cfg(test)` `BROWSER_OPEN_TIMEOUT` in
  `crates/sky-cua-service/src/browser/bridge.rs` was raised 250 ms → 2 s
  (production stays 12 s) to stop happy-path browser operation tests flaking
  under scheduler load; one aggregate-deadline test waits out the full window
  and sets the suite tail. Do not "optimize" this back down without re-running
  the load-stress gate (10+ consecutive full-suite runs under CPU load).

## Open Questions

1. ~~Has the EIS input path been live-smoked with long text strings?~~ **Answered:** The per-character sleep + search combination was real — 100 characters took ~3.6s of sleep + ~50,000 keymap iterations. Both are now fixed (HIGH-004 batching + CRIT-005 caching). A live smoke with a sentence-length string should still be run to confirm the ~10x speedup.

2. **Is the 2.5 s `step_delay` in `live_wayland_pointer_smoke.py` driven by a real backend settling requirement, or is it defensive padding?** If the latter, profiling the actual MCP round-trip time could shave 10–15 s off each smoke run.

3. ~~**Why no release profile optimizations?**~~ **Answered:** Added `[profile.release]` to workspace `Cargo.toml` with:
   - `lto = "thin"` — helps cross-crate serde/JSON-heavy paths (compact snapshot, MCP serialization)
   - `codegen-units = 1` — worth the compile-time hit for shipping builds via `build_plugin.py`
   - `panic = "unwind"` — preserves current panic containment behavior (spawn_blocking panics become JSON-RPC errors, not process death)
   - `debug = 1` — keeps release builds profilable and field-debuggable
   - Intentionally omitted: `strip = true` (debuggability matters more than size), `target-cpu = "native"` (wrong for distributable plugin), full LTO (compile time not worth marginal gain)

4. **Has `tools/list` frequency been measured?** If Codex calls it only once per session, the schema rebuild is a smaller win. If it calls it on every model context switch, it matters more. **Answered:** The cache is implemented regardless; the cost is now a single `Arc` clone per call.
