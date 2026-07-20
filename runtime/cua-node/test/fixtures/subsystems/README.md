# Offline subsystem fixtures

These text fixtures are intentionally small and deterministic. They are inputs for
the media harness adapters, not evidence that the final production packages are
available. The adapter reports a locked-artifact blocker whenever a pinned package,
browser revision, native addon, language file, or notice is absent.

The local Sharp and FFmpeg proofs are opt-in through `CUA_NODE_SHARP_PATH` and
`CUA_NODE_FFMPEG_PATH`; they report `available-local-proof` and never claim bundled
runtime parity.
