# Portal Review Fixes

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document follows `plans/AGENTS.md` and `/home/bex/.agents/PLANS.md`.

## Purpose / Big Picture

The portal and Hyprland review pass found several correctness problems after the RemoteDesktop decomposition. After this work, EIS input should recover from device pause/removal and worker startup edge cases, keyboard injection should correctly handle non-Shift modifiers such as AltGr, Hyprland targeting should choose the intended compositor instance, and portal startup timeouts should explicitly clean up D-Bus-side sessions. The behavior is verified through `cargo clippy -p sky-cua-linux --all-targets` and `cargo test -p sky-cua-linux`; live desktop smokes remain the final environment proof.

## Progress

- [x] (2026-05-23) Fixed public/internal module visibility from the decomposition.
- [x] (2026-05-23) Fixed stale EIS worker handle recovery and keyboard `InvalidRequest` fallback behavior.
- [x] (2026-05-23) Refactored capture setup so blocking PipeWire/GStreamer work no longer holds the RemoteDesktop write lock.
- [x] (2026-05-23) Replaced session `expect` panics in action paths with structured `BackendError` failures.
- [x] (2026-05-23) Fixed EIS device pause/remove lifecycle handling, Hyprland instance choice, stale Hyprland cache invalidation, mapped-window filtering, cursor mode preference, Shift de-duplication, and raw XKB keycode underflow.
- [x] (2026-05-23) Finished XKB modifier modeling for Shift and AltGr/Level3 with xkbcommon level masks, explicit chord modifier de-duplication, and Unicode keysym conversion through xkbcommon.
- [x] (2026-05-23) Refactored EIS worker startup to use an async Tokio oneshot readiness wait instead of wrapping the dedicated worker startup in `spawn_blocking`.
- [x] (2026-05-23) Moved the interactive portal startup timeout inside `start_session` after session creation and added `Session::close()` cleanup on setup timeout/error.
- [x] (2026-05-23) Added focused regression tests for XKB Unicode/modifier helpers and Hyprland target cache mutation; final Rust validation passed with 196 tests.
- [x] (2026-05-23) Fixed follow-up review concurrency findings: stale EIS-worker retry deadlock, capture retry lock scope, EIS worker startup panic reporting, and worker readiness outside the state lock.
- [x] (2026-05-23) Ran live VM proof across Plasma, GNOME, Hyprland, COSMIC, and i3/X11; all pointer/input/overlay smokes passed.
- [x] (2026-05-23) Fixed deep-review `prepare_capture` write-lock hold: restructured to use read lock for the async D-Bus call and only briefly write-lock to store the cached fd.
- [x] (2026-05-23) Fixed deep-review session leak on clear/take: `capture_frame` retry, `reset_session`, and `reset_persisted_tokens` now explicitly close the old portal session before dropping it.
- [x] (2026-05-23) Fixed deep-review Hyprland stale cache after compositor restart: validate cached instance with `hyprctl -i <instance> version` before use, and clear on failure.
- [x] (2026-05-23) Fixed deep-review poisoned mutex handling and stale env inheritance in Hyprland discovery commands.
- [x] (2026-05-23) Fixed deep-review capture retry dropping lifecycle events for unused concurrent sessions: events are now pushed before the unused session is closed.
- [x] (2026-05-23) Fixed deep-review low items: replaced blocking `std::sync::mpsc` with `tokio::sync::mpsc` in EIS worker, caught `run` panics, added EIS write timeout, replaced fixed 16-mask array with dynamic `Vec`, documented modifier keycode "first wins" policy, and added evdev offset comment for keycodes 1–7.
- [x] (2026-05-23) Fixed performance-review critical findings: `ensure_session_started` no longer holds write lock across portal startup; `eis_worker` no longer holds write lock across `ensure_session_started`; per-character text input sleep reduced from 55 ms to 15 ms (5 ms key hold + 10 ms inter-char); Hyprland cache validation now uses socket existence check instead of process spawn.
- [x] (2026-05-23) Fixed performance-review high/medium items: cached EIS device descriptions, `find_eis_keycodes_for_keysyms` now uses pre-built cache, `build_keysym_cache` uses `with_capacity(1024)`, Hyprland `sort_by_key` clone eliminated, probe uses lighter `monitors -j`, `send_text` uses `text.len()` as O(1) capacity bound.
- [x] (2026-05-23) Fixed remaining low/medium items from performance review: `prepare_capture` fd duplication moved outside write lock; fallback subprocess spawning wrapped in `spawn_blocking`; legacy `send_keysym_raw` pipelines press+release concurrently; EIS retry reset has 50 ms backoff; click/drag legacy sleeps reduced to 15 ms/20 ms; `UinputPointerDevice` cached inside `LinuxVirtualInput` so direct uinput pointer actions reuse the device instead of recreating it per action (eliminating the 650 ms settle delay).

## Surprises & Discoveries

- Observation: `ashpd` high-level `select_devices`, `select_sources`, and `start` calls await the response before returning a `Request`; a timed-out future cannot access `Request::close()`.
  Evidence: Librarian research against `ashpd` 0.13.10 source found `Request::close`, `Session::close`, and no `Drop` cleanup.
- Observation: `reis::event::EiEvent` lifecycle variants use distinct payload types, so Rust cannot combine `DevicePaused`, `DeviceStopEmulating`, and `DeviceRemoved` in a single OR-pattern with one binding.
  Evidence: `cargo check -p sky-cua-linux` rejected the combined match arm with mismatched variant payload types.
- Observation: EIS text injection must use `xkb::utf32_to_keysym` for Unicode characters; hand-encoding all non-ASCII as `0x01000000 | codepoint` breaks existing named keysyms such as `adiaeresis` and `EuroSign`.
  Evidence: Focused `portal::eis_keymap` tests cover `ä`, `€`, Shift, and German AltGr/EuroSign resolution.
- Observation: `tokio::sync::RwLock` is not recursive; retry branches must drop write guards before calling helpers that can re-enter `RemoteDesktopSessionManager` state.
  Evidence: Follow-up review found the stale EIS-worker retry branch would deadlock until the write guard was scoped before retrying.
- Observation: `OnceLock` is the wrong primitive for Hyprland target caching because `WAYLAND_DISPLAY` can change after the first lookup.
  Evidence: The cache is now `Mutex<Option<(String, String)>>` and `windowing::hyprland` has a regression test for display-keyed replacement.

## Decision Log

- Decision: Treat all EIS failures as eligible for fallback, while keeping session reset suppressed for `InvalidRequest`.
  Rationale: Keyboard `InvalidRequest` often means EIS cannot produce a character, while X11/XTest or ydotool may still succeed.
  Date/Author: 2026-05-23 / Sky
- Decision: Use explicit `Session::close()` on portal setup timeout or setup error instead of trying to close individual `Request` objects.
  Rationale: `ashpd` request handles are not available until the high-level futures complete, but the session handle exists after `create_session` and closing it cancels pending requests.
  Date/Author: 2026-05-23 / Sky
- Decision: Prefer xkbcommon `key_get_mods_for_level` masks for inverse key transformation, falling back to conventional level mapping only when no supported mask is exposed.
  Rationale: EIS emits keycodes, not characters. The keymap is the source of truth for whether Shift, AltGr/Level3, or both are needed for a keysym.
  Date/Author: 2026-05-23 / Sky
- Decision: Track a `session_generation` in `RemoteDesktopState` when moving EIS worker startup outside the readiness wait lock.
  Rationale: If the portal session changes while the worker thread is starting, the generation check prevents installing a worker for an obsolete session.
  Date/Author: 2026-05-23 / Sky

## Outcomes & Retrospective

The code fixes from this review pass are complete and live-proven. Validation passed with `cargo fmt -p sky-cua-linux --check`, `cargo clippy -p sky-cua-linux --all-targets`, and `cargo test -p sky-cua-linux` reporting 196 tests passed.

Live desktop proof across the full VM matrix:
- **Plasma/KDE** `wayland-pointer` passed: `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260523T070159Z`
- **GNOME** `wayland-pointer` passed: `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260523T070911Z`
- **Hyprland** `wayland-pointer` passed: `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260523T071031Z`
- **COSMIC** `wayland-pointer` passed: `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260523T071155Z`
- **i3/X11** overlay cursor proof passed: `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260523T071309997556Z`
- **Plasma/KDE** `wayland-pointer` re-passed with reduced EIS delays (15 ms/char): `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260523T073405Z`
- **Plasma/KDE** `wayland-pointer` re-passed with uinput device caching and spawn_blocking fallbacks: `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260523T074336Z`

This plan is ready for retirement per `plans/AGENTS.md`.

## Context and Orientation

The Linux backend lives under `crates/sky-cua-linux/src`. Portal RemoteDesktop state is in `portal/remote_desktop.rs`, high-level EIS fallback policy is in `portal/eis_fallback.rs`, low-level EIS worker/device/key event handling is in `portal/eis_input.rs`, XKB key lookup is in `portal/eis_keymap.rs`, portal session startup and token persistence are in `portal/portal_session.rs`, and Hyprland window targeting is in `windowing/hyprland.rs`. EIS is the Wayland RemoteDesktop protocol path for virtual input. XKB is the keyboard layout library used to map characters and named keys to physical keycodes and modifier levels.

## Plan of Work

First, complete the EIS worker and keymap fixes. Extend `EisKeyStroke` so it records modifier requirements beyond Shift, discover modifier keycodes from XKB, and make key state emission press/release all required modifiers in a stable order. Refactor the worker startup handshake so `spawn_eis_worker` returns asynchronously through a Tokio oneshot without wrapping a synchronous readiness wait in `spawn_blocking`.

Second, complete portal timeout cleanup. Keep `RemoteDesktop` and `Screencast` proxy creation outside the interactive timeout, create the `Session`, then run `select_devices`, `select_sources`, and `start` inside `tokio::time::timeout`. If the timeout expires or setup returns an error after the session exists, call `session.close().await` and report a structured `BackendError`.

Third, add regression tests where existing seams permit it. Use unit tests for pure Hyprland parsing and target selection helpers, cursor mode selection, keymap modifier logic, and fallback policy. Validate the Rust crate with `cargo fmt -p sky-cua-linux`, `cargo clippy -p sky-cua-linux --all-targets`, and `cargo test -p sky-cua-linux`.

## Concrete Steps

Run commands from `/home/bex/projects/sky-cua`.

Use `cargo test -p sky-cua-linux portal::eis_keymap` for keymap-focused tests when editing XKB logic. Use `cargo test -p sky-cua-linux windowing::hyprland` after Hyprland test additions. End with `cargo fmt -p sky-cua-linux && cargo clippy -p sky-cua-linux --all-targets && cargo test -p sky-cua-linux`.

## Validation and Acceptance

The code is acceptable when `cargo clippy -p sky-cua-linux --all-targets` has no warnings and `cargo test -p sky-cua-linux` reports all tests passed. Live portal behavior still needs `python3 scripts/live_desktop_smoke.py` and `python3 scripts/live_portal_downgrade_smoke.py` on an appropriate desktop session.

## Idempotence and Recovery

All edits are source-only. If a refactor fails, rerun `cargo check -p sky-cua-linux` to get the narrowest compiler guidance. No destructive commands or stateful migrations are involved. Portal live smokes may open user-facing approval dialogs and should only be run when the desktop session can be observed.

## Artifacts and Notes

Previous validation after the first review-fix pass: `cargo fmt -p sky-cua-linux && cargo clippy -p sky-cua-linux --all-targets && cargo test -p sky-cua-linux` completed with 191 passed tests and no clippy warnings.

## Interfaces and Dependencies

`ashpd::desktop::Session::close()` is the cleanup API for pending portal sessions. `ashpd::desktop::Request::close()` exists but is unreachable on timeout when using high-level ashpd request methods. `tokio::sync::oneshot` is already available through Tokio and should replace the synchronous worker readiness channel for the async startup path.
