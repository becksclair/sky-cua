# Desktop live-capture pitfalls and diagnostics

Read this only when a live capture is needed or fails. These rules apply to
the KDE/KWin desktop path; they do not make offline artifacts equivalent to a
compositor capture.

## Process and socket safety

- Never run `pkill -f sky-cua-overlay-host`: it can match the shell's own
  argv/environment and kill the script. `pkill -x sky-cua-overlay` is shell-safe
  but can kill the operator's live service-owned host, so it is not a cleanup
  strategy either.
- Use `_overlay_host.terminate_leftover_hosts(sock)` with the exact private
  host socket. It SIGTERMs only a host bound to that socket. `capture.py` uses
  an isolated service socket at `/tmp/agent-cursor-debug/svc.sock`; its host
  socket is `<artifact_dir>/agent-cursor.sock`, distinct from the operator's
  `$XDG_RUNTIME_DIR/sky-cua/agent-cursor.sock`.
- Do not blanket-kill `sky-cua-service`. `capture.py` records and terminates
  its isolated service by PID, then reaps the private host. Keep start, capture,
  and teardown in one process because the Bash caller waits on lingering
  background daemons.
- The shell profile uses `set -e`; an ad-hoc `pkill` that matches nothing can
  abort a whole command. Prefer the harness lifecycle.

## Why an overlay can appear to be missing

- `CursorImage::load` runs at host startup. The host has a 2-second
  `HOST_START_TIMEOUT`; if loading exceeds it, the service kills the host and
  only the standalone KWin edge glow may remain. The host's stderr is
  discarded, so use the offline gesture dump to separate renderer health from
  startup/composition failure and inspect the isolated `service.log`.
- The cursor state must contain both `snapshot_id` and a near-now
  `updated_at_ms`. Refresh both when setting state; a stale or zero timestamp
  is treated as decayed and draws nothing.
- The overlay surface is fullscreen per output. There is no per-cursor damage
  rectangle to widen when the glyph or aura grows.

## Capture tooling and evidence

- KWin has no `wlr-screencopy`; `grim` fails. Use
  `spectacle -b -n -f -o <path>` for the full virtual desktop.
- The first KDE ScreenCast-portal run may show a share dialog. Its restore
  token is stored under the gitignored artifacts directory. A portal failure
  means the MP4 is not live proof; use the offline motion dump and say so.
- Spectacle captures at 2× logical resolution. Downsample before judging the
  glyph and inspect `cursor_native7x.png`, not a nearest-zoomed raw capture.
- Live captures and recordings can contain sensitive desktop content. Keep
  them in `/tmp/agent-cursor-debug/` or
  `artifacts/overlay-motion-animations/`; never commit them.
