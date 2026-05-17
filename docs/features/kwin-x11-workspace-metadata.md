# KWin and X11 workspace metadata

## Status

Shipped. Last verified: 2026-05-14 via crate-level tests. A dedicated
real-session `list_windows` artifact capturing `workspace` values has not
been recorded yet; tracked as a follow-up sub-item in `ROADMAP.md`.

## Summary

`WindowInfo.workspace` carries the backend-native numeric workspace/desktop
value when the underlying window manager exposes one. Both KWin and X11
windowing backends populate this field; the unified registry passes it
through unchanged.

## Contract surface

- Public model: `WindowInfo.workspace: Option<i32>` in
  `crates/sky-cua-platform/src/model.rs`.
- Backend internals: `KWinWindowInfo.workspace: Option<i32>` in
  `crates/sky-cua-linux/src/kwin.rs` and
  `X11WindowInfo.workspace: Option<i32>` in
  `crates/sky-cua-linux/src/x11/windowing.rs`.
- Registry plumbing: `LinuxWindowInfo.workspace` in
  `crates/sky-cua-linux/src/windowing/types.rs`.

`workspace` is metadata only. It is not a `WindowTarget` selector and there
is no cross-desktop normalization. Future normalized workspace UX should use
a separate explicit field such as `workspace_display_index` or
`workspace_label`.

## Behavior

- KWin: parses workspace-like keys from qdbus / gdbus output, including
  scalar desktop/workspace values and list-like `desktops` output when a
  first numeric desktop can be parsed safely.
- X11: per-window `xprop` queries include `_NET_WM_DESKTOP`, parsed as a
  backend-native integer.
- Sticky / all-desktops values such as `0xFFFFFFFF` remain `None`.
- Non-numeric, missing, or ambiguous values remain `None`.

## Source paths

- `crates/sky-cua-linux/src/kwin.rs` (KWin parser)
- `crates/sky-cua-linux/src/x11/windowing.rs` (X11 `_NET_WM_DESKTOP` parsing)
- `crates/sky-cua-linux/src/windowing/registry.rs` (registry plumbing)
- `crates/sky-cua-linux/src/windowing/types.rs` (public conversion)
- `crates/sky-cua-platform/src/model.rs` (public field)

## Verification

```bash
cargo test -p sky-cua-linux kwin::tests
cargo test -p sky-cua-linux x11::windowing::tests
cargo test -p sky-cua-linux windowing::registry::tests
```

Live proof, not yet captured: run `./bin/sky-cua-client list-windows` (or
the MCP `list_windows` tool) on a real KWin and on a real X11 session with
`_NET_WM_DESKTOP` set, and confirm entries include `workspace` values.

## Known limitations

- No cross-desktop normalization. Numeric values are backend-native and not
  comparable across desktops.
- No dedicated `list_windows` workspace artifact has been recorded yet on
  real KWin or X11. Tracked in `ROADMAP.md`.
- Other backends (GNOME extension, COSMIC helper, Hyprland, i3) may or may
  not surface workspace values; this feature ships the KWin and X11 paths
  only.

## Related

- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Originating ExecPlan (retired into this feature doc; see git history for `plans/1778571910929-proud-mountain.md`).
