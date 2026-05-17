# PipeWire vs Screenshot portal: Wayland capture lane choice

## Context

The Wayland Computer Use backend needs reliable per-frame screen
capture. The XDG portals expose two relevant lanes:

- **Screenshot portal** (`org.freedesktop.portal.Screenshot`) returns
  a single still image per call.
- **ScreenCast portal** (`org.freedesktop.portal.ScreenCast`) sets up
  a PipeWire stream the client can pull frames from continuously.

Early versions of `sky-cua` used the Screenshot portal because it was
the simplest path to a model-facing image. This research records why
the primary lane moved to in-process PipeWire frame capture, with
Screenshot kept only as a fallback, and how the resulting downgrade
diagnostic is wired.

## Investigation

The Screenshot portal lane was straightforward but had two practical
costs in repeated agent loops:

- Each `Screenshot::Screenshot()` call triggered the portal flow,
  including (depending on the desktop) a brief permission state
  check and then a full single-frame capture-and-save.
- The returned `file://` URI had to be copied or symlinked into the
  service's per-user capture directory before being handed back.

Per-frame latency was higher than continuous capture from a live
ScreenCast session, and the repeated portal hand-shaking made
high-frequency observe-act loops noisy in latency profiling.

The ScreenCast lane required a different startup cost: opening a
RemoteDesktop or ScreenCast portal session, selecting monitors,
selecting cursor mode, and getting the human approval dialog past.
Once the session was alive, frames were available continuously.

The first PipeWire implementation lived in a helper subprocess. It
worked but added another moving part: a child process whose lifetime
had to track the service's, plus inter-process plumbing for frame
buffers. The current implementation pulled the PipeWire pipeline
in-process via GStreamer
(`pipewiresrc -> videoconvert -> pngenc -> appsink`). That made
frame timing predictable, removed the helper-process coordination,
and let the service own the PipeWire remote fd directly. The remote
fd is now reused across captures.

Live evidence on KDE Wayland (Asgard):

- Original Screenshot-only path: each frame-grab waited on a portal
  call.
- After moving to in-process PipeWire: the same agent loop ran
  noticeably faster, and the recorded artifact path moved into
  `/run/user/1000/sky-cua/captures/` directly without portal-side
  staging.

Failure modes were different per lane:

- Screenshot portal could fail with permission errors that needed a
  human prompt, or with portal-side errors after a stale session.
- PipeWire could fail when the underlying ScreenCast stream wedged
  (e.g. compositor restart), needed a session rebuild, or when the
  remote fd became invalid.

Either lane being unavailable should not silently produce an empty
or wrong image. The runtime needs to tell the agent which lane was
selected and which lane actually produced the pixels.

## Conclusion

Use **in-process PipeWire frame capture** as the Wayland primary lane,
with the **Screenshot portal as the fallback**. Keep the lanes
distinguishable in the public capture metadata:

- `capture.backend` — the lane the runtime selected for this
  snapshot.
- `capture.image_backend` — the lane that actually produced the
  image bytes.

When `capture.backend = portal_pipe_wire` falls back to Screenshot,
the runtime emits a `CaptureBackendDowngraded` diagnostic with both
lane values and the reason. An operator can prove the downgrade path
through the dedicated forced-downgrade smoke at
`scripts/live_portal_downgrade_smoke.py`, which uses an isolated
service socket so the failure injection does not contaminate the
normal-path validation story.

## Implications

- The runtime architecture
  ([`docs/runtime/linux-architecture.md`](../runtime/linux-architecture.md))
  documents the two-field capture metadata and the downgrade path.
- New capture lanes (e.g. compositor-specific Wayland protocols, or
  Windows WGC / DXGI) should follow the same split: distinguish the
  selected lane from the actual image-producing lane, and emit a
  structured diagnostic on downgrade.
- The Screenshot portal lane stays in the codebase as a tested
  fallback, not as legacy code waiting for removal. Compositors
  without a working ScreenCast path still need the fallback.
- Future investigations into capture latency should compare against
  the in-process PipeWire baseline rather than the original
  Screenshot-only baseline, which is no longer the production path.
