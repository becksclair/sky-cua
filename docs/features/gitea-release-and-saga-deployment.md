# Gitea release and Saga deployment

## Status

Shipped for Linux x86-64 glibc. Last verified on 2026-07-24 with
`standalone-v0.1.0` and Gitea deployment run 41.

## Summary

Gitea Actions builds, verifies, publishes, and reads back the standalone Sky
CUA archive on Asgard. A separate retryable workflow sends the published tag
and SHA-256 to Saga, where one direct deploy hook replaces the fixed install
root and restarts its consumers in dependency order.

## Contract surface

Release tags have the form:

```text
standalone-v<VERSION>
```

The normal producer entrypoint is `python3 install.py release`, optionally with
`--patch`, `--minor`, `--major`, or `--version X.Y.Z`. It creates an annotated
tag and atomically pushes that tag with its single-file standalone version
commit. Local success reports pushed refs only; workflow completion owns the
published and deployed result.

Each Gitea Release exposes exactly:

```text
sky-cua-linux-x64-glibc.tar.gz
sky-cua-linux-x64-glibc.tar.gz.sha256
```

`.gitea/workflows/release-standalone.yml` runs for matching tag pushes and
retains a manual `tag` input for retries. Publication dispatches
`.gitea/workflows/deploy-saga.yml` from `main` with:

```text
tag=<standalone tag>
archive_sha256=<64 lowercase hexadecimal characters>
```

Saga's operator entrypoint is:

```text
/opt/homelab/scripts/deploy-sky-cua <standalone tag> <archive sha256>
```

The digest identifies the published transport bytes. It is not an installer
argument, trust store, or selectable installed generation.

## Behavior

`asgard-build-1` runs at capacity one and owns Sky CUA verification and release
jobs. The release job checks out the requested tag, verifies its source and
version identity, runs the Rust and Python gates, builds the archive, validates
its paths and links, and installs it under a disposable home. It then creates
or reuses the release without overwriting different bytes, downloads both
attachments, verifies the archive digest, and dispatches Saga deployment.

Before that handoff, the local release command requires a clean synchronized
`main`, rejects reused tags and non-increasing or unstable versions, runs
`just verify`, and pushes without force. The atomic branch/tag push prevents a
remote tag from being created without its release commit.

Saga downloads, hashes, validates, and extracts the archive before stopping
services. It stops `openclaw-gateway.service` before `brave-origin.service`,
runs the extracted fixed-root installer as `ubuntu`, starts Brave and waits for
the Xpra session and native-host runtime, then starts OpenClaw and waits for a
new gateway process with `rpc.ok=true`.

Final health checks the installed release identity, stable launchers, native
messaging manifests, canonical Codex plugins, canonical OpenClaw Browser
plugin, service processes, and runtime sockets. A nonblocking Saga-local
`flock` serializes deployments. Failure is forward-only: correct the cause and
redispatch the same published tag and digest, or publish a new tag for changed
bytes. There are no retained generations, backups, activation selectors, or
rollback controller.

## Source paths

- `.gitea/workflows/verify.yml` — main and pull-request verification.
- `scripts/_standalone_release_command.py` — guarded version selection, commit,
  annotated tag, and atomic push.
- `.gitea/workflows/release-standalone.yml` — tag/manual release publication.
- `.gitea/workflows/deploy-saga.yml` — published-identity deployment dispatch.
- `scripts/release_pipeline.py` — archive safety, isolated install, immutable
  release reuse, attachment readback, and deployment dispatch.
- `scripts/test_release_pipeline.py` — focused producer and workflow tests.
- `/opt/homelab/scripts/deploy-sky-cua` on Saga — stable operator wrapper.
- `/opt/homelab/ops/src/saga_ops/sky_cua.py` on Saga — direct deployment and
  deterministic health implementation.

## Verification

The 2026-07-24 release canary proved authenticated draft readback, public
readback, byte-for-byte digest agreement, and cleanup without changing the
existing product tag.

Gitea release run 38 published release ID 458 for
`standalone-v0.1.0` from commit
`ef9f3065d9a58ed3a0e874066fa40582ac2e58a8`. Public readback returned
148,583,263 bytes whose SHA-256,
`5eed578042a3624dcc5b186f5af30d92961eac4837eb9e8e6b3688eb5a4e8ef4`,
matched the sidecar.

Deployment run 41 installed that exact identity on Saga. Its deterministic
health reported Sky CUA `0.1.0` at the fixed root, active Brave/Xpra and
OpenClaw services with new processes, OpenClaw RPC readiness, and the canonical
Browser plugin loaded.

Focused producer verification:

```bash
uv run pytest scripts/test_release_pipeline.py
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run
```

## Known limitations

- The published target is currently Linux x86-64 glibc.
- Gitea 1.25.4 does not enforce workflow `concurrency`; Asgard runner capacity
  one and Saga's deployment lock provide the required serialization.
- Full model-backed Computer Use and Browser Use acceptance is an intentional
  first-cutover/manual-release gate, not part of routine automatic deployment.

## Related

- [`release-package.md`](release-package.md)
- [`one-shot-installer.md`](one-shot-installer.md)
- [`../operations/plugin-release.md`](../operations/plugin-release.md)
