# COSMIC Cursor Bridge

COSMIC does not currently expose public IPC for globally hiding the compositor cursor. sky-cua therefore uses a two-part bridge:

- `sky-cua-cosmic-helper cursor-bridge` listens on `$XDG_RUNTIME_DIR/sky-cua-cosmic-cursor.sock` and toggles `$XDG_RUNTIME_DIR/sky-cua-cosmic-cursor-hidden`.
- The COSMIC compositor patch in `cosmic-comp-sky-cua-cursor-bridge.patch` writes `$XDG_RUNTIME_DIR/sky-cua-cosmic-cursor-ready` from the compositor cursor path and makes `SeatExt::cursor_image_status()` return `CursorImageStatus::Hidden` while the hidden state file exists.

The patch is intentionally small and local to COSMIC's existing cursor state boundary. It should be replaced with an upstream compositor IPC hook if COSMIC adds one.

The bridge patch is a development prototype. It proves the compositor-owned path, but normal unpatched COSMIC should still be treated as unsupported for dynamic hide/show until COSMIC exposes an upstream cursor-visibility inhibitor/API. An upstreamable API should be generic, token/refcount based, clean up on client disconnect, and suppress all final cursor render sources.

## Transparent Xcursor Session Mode

For controlled COSMIC VMs that cannot run a patched compositor, sky-cua supports a dedicated no-patch session mode:

```bash
python3 scripts/install_blank_xcursor_theme.py --theme-name sky-cua-blank --size 24
export XCURSOR_THEME=sky-cua-blank
export XCURSOR_SIZE=24
exec cosmic-session
```

The Arch testing VM wrapper exposes this as `cosmic-blank` or `cosmic-transparent`.

In this mode COSMIC starts with a valid transparent Xcursor theme. The normal sky-cua layer-shell overlay still draws the agent cursor. `CosmicTransparentXcursorAdapter` reports `system_cursor_backend=cosmic_transparent_xcursor` only when COSMIC is launched with `XCURSOR_THEME=sky-cua-blank` and the theme files exist.

This preserves the one-visible-cursor invariant while the agent overlay is visible. It does not restore a normal native cursor when the overlay hides; the native cursor remains transparent for the whole session.

VM proof for unpatched transparent COSMIC uses:

```bash
python3 scripts/run_gui_testing_vm_smoke.py \
  --profile cosmic-transparent-xcursor-host-proof \
  --desktop-env COSMIC \
  --wayland-display wayland-1 \
  --vm-name testing-vm \
  --libvirt-uri qemu:///session
```

VM proof for patched COSMIC uses:

```bash
python3 scripts/run_gui_testing_vm_smoke.py \
  --profile cosmic-patched-cursor-host-proof \
  --desktop-env COSMIC \
  --wayland-display wayland-1 \
  --vm-name testing-vm \
  --libvirt-uri qemu:///session \
  --skip-host-build \
  --skip-sync
```

The profile assumes the VM is already running a `cosmic-comp` build with `cosmic-comp-sky-cua-cursor-bridge.patch` applied. It fails unless the compositor writes the ready sentinel.
