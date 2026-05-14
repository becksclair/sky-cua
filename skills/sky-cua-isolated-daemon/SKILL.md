---
name: sky-cua-isolated-daemon
description: "Use when running or debugging sky-cua live tests, browser/X11/portal smokes, or daemon-backed checks that must exercise the current checkout's latest build through an isolated service socket instead of any older installed or already-running sky-cua daemon."
---

# Sky CUA Isolated Daemon

Use this skill when a test should prove the current checkout, not whichever `sky-cua-service` process happens to be live.

## Core invariant

Live tests must use a freshly built repo-local daemon on a run-specific socket.

- Build the current checkout before the smoke.
- Pass an explicit `SKY_CUA_SERVICE_SOCKET_PATH` for the whole test command.
- Avoid the default service socket unless the task is specifically testing the installed/live plugin.
- Do not kill unrelated `sky-cua-service` processes by name. Isolation comes from the socket path.

## Build the binary under test

For normal daemon-backed smokes:

```bash
cargo build --release -p sky-cua-service -p sky-cua-client
```

For browser/native-host smokes, include the host binary:

```bash
cargo build --release -p sky-cua-service -p sky-cua-client -p sky-cua-chrome-host
```

Use debug binaries only when that is intentional, and say so in the final report. Most repo wrappers and bundle paths expect `target/release`.

## Use an isolated socket

Create one socket path per run, remove only that path if it already exists, and pass it as command-scoped environment:

```bash
SOCKET="/tmp/sky-cua-live-desktop-$$.sock"
rm -f "$SOCKET"
SKY_CUA_SERVICE_SOCKET_PATH="$SOCKET" python3 scripts/live_desktop_smoke.py
```

Keep the same environment variable on every process in the test lane. If the test starts a client, harness, native host, or child app that talks to the service, it must inherit the same socket.

## Smoke examples

Desktop portal smoke:

```bash
cargo build --release -p sky-cua-service -p sky-cua-client
SOCKET="/tmp/sky-cua-live-desktop-$$.sock"
rm -f "$SOCKET"
SKY_CUA_SERVICE_SOCKET_PATH="$SOCKET" python3 scripts/live_desktop_smoke.py
```

Pure X11 smoke:

```bash
cargo build --release -p sky-cua-service -p sky-cua-client
SOCKET="/tmp/sky-cua-live-x11-$$.sock"
rm -f "$SOCKET"
SKY_CUA_SERVICE_SOCKET_PATH="$SOCKET" python3 scripts/live_x11_smoke.py
```

Chrome native-host smoke:

```bash
cargo build --release -p sky-cua-service -p sky-cua-client -p sky-cua-chrome-host
SOCKET="/tmp/sky-cua-live-browser-$$.sock"
rm -f "$SOCKET"
SKY_CUA_SERVICE_SOCKET_PATH="$SOCKET" python3 scripts/live_chrome_host_client_smoke.py \
  --browser brave \
  --install-temp-native-manifest \
  --host-path target/release/sky-cua-chrome-host
```

## Sanity checks

Before trusting the result:

- Confirm the build command completed after the code change under test.
- Confirm the smoke command printed or logged the isolated socket path, or report the exact path you passed.
- For browser smokes, pass `--host-path target/release/sky-cua-chrome-host` when available so the browser does not discover an older installed host.
- If a test unexpectedly connects to the default socket or an installed binary, discard that run as contaminated and rerun with a fresh isolated socket.

## Reporting

In the final verification note, include the build command, the smoke command, and the isolated socket path. If you skipped rebuilding or intentionally tested the installed daemon, say that directly.
