# Isolated Daemon Smokes

Use this runbook when a test must prove the current checkout, not whichever
`sky-cua-service` process happens to be live.

## Core Invariant

Live tests should use a freshly built repo-local daemon on a run-specific socket.

- Build the current checkout before the smoke.
- Pass an explicit `SKY_CUA_SERVICE_SOCKET_PATH` for the whole test command.
- Avoid the default service socket unless the task is specifically testing the installed/live plugin.
- Do not kill unrelated `sky-cua-service` processes by name. Isolation comes from the socket path.

## Build Commands

Normal daemon-backed smokes:

```bash
cargo build --release -p sky-cua-service -p sky-cua-client
```

Browser/native-host smokes:

```bash
cargo build --release -p sky-cua-service -p sky-cua-client -p sky-cua-chrome-host
```

Use debug binaries only when intentional, and say so in the report. Most repo
wrappers and bundle paths expect `target/release`.

## Socket Pattern

```bash
SOCKET="/tmp/sky-cua-live-desktop-$$.sock"
rm -f "$SOCKET"
SKY_CUA_SERVICE_SOCKET_PATH="$SOCKET" python3 scripts/live_desktop_smoke.py
```

Keep the same environment variable on every process in the test lane. If the
test starts a client, harness, native host, or child app that talks to the
service, it must inherit the same socket.

## Browser Smoke Example

```bash
cargo build --release -p sky-cua-service -p sky-cua-client -p sky-cua-chrome-host
SOCKET="/tmp/sky-cua-live-browser-$$.sock"
rm -f "$SOCKET"
SKY_CUA_SERVICE_SOCKET_PATH="$SOCKET" python3 scripts/live_chrome_host_client_smoke.py \
  --browser brave \
  --install-temp-native-manifest \
  --host-path target/release/sky-cua-chrome-host
```

## Sanity Checks

- Confirm the build command completed after the code change under test.
- Confirm the smoke command printed or logged the isolated socket path, or report the exact path passed.
- For browser smokes, pass `--host-path target/release/sky-cua-chrome-host` when available so the browser does not discover an older installed host.
- If a test unexpectedly connects to the default socket or an installed binary, discard that run as contaminated and rerun with a fresh isolated socket.

For VM desktop profiles, use `docs/operations/testing-vm-desktop-smokes.md` and
confirm the selected guest session plus `WAYLAND_DISPLAY` or `DISPLAY` before
treating a failure as backend evidence.
