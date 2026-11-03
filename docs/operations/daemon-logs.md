# Daemon logs and log levels

Operator runbook for finding the sky-cua daemon's logs and raising the log
level on an installed (release) build.

## Where the logs are

When the MCP client spawns the daemon it captures stderr into a per-endpoint
file under the state directory and passes the same path to the daemon via
`SKY_CUA_DAEMON_LOG_PATH`, so tracing output goes through a self-rotating
writer:

- Linux: `~/.local/state/sky-cua/daemon-<socket-stem>.log` (the isolated
  desktop daemon gets its own file)
- Rotation: past 8 MiB, one `.log.old` generation is kept
- A daemon started by hand (`sky-cua-service daemon`) has no log path set and
  logs to stderr

See `docs/runtime/mcp-boundary.md` ("Daemon logs") for the contract.

## Raising the log level

Both binaries read `RUST_LOG` through `tracing_subscriber::EnvFilter`; the
default is `info`. Release builds compile `debug!` in and `trace!` out
(`tracing` feature `release_max_level_debug` in the root `Cargo.toml`), so:

- `RUST_LOG=debug` works on installed binaries.
- `RUST_LOG=trace` behaves like `debug` on release builds. For trace output
  use a dev build (`cargo build -p sky-cua-service`, which ignores the
  release cap) and point the client at it with `SKY_CUA_SERVICE_PATH`.
- Prefer target-scoped filters, e.g.
  `RUST_LOG=info,sky_cua_service=debug,sky_cua_linux=debug`. A bare
  `RUST_LOG=debug` also enables every dependency's debug output (zbus alone
  has a dozen sites), rotates the 8 MiB log faster, and only one `.log.old`
  is kept, so the forensic window shrinks. Restore the default after
  capturing.

How the variable reaches the daemon: the daemon inherits the MCP client's
full environment (the spawn in `crates/sky-cua-client/src/service_launcher.rs`
does not clear it), so `RUST_LOG` must reach the client. It is not in the
plugin's `.mcp.json` `env_vars`; hosts that filter MCP server environments
(Codex) need it set explicitly in that server's `env` configuration. The
daemon is long-lived and shared: a new value takes effect only when the
daemon restarts, so stop the targeted daemon (only that one) and let the
client respawn it. Alternatively run a daemon by hand on its own socket:
`RUST_LOG=debug SKY_CUA_SERVICE_SOCKET_PATH=<path> sky-cua-service daemon`;
it logs to stderr.

## Reading a log

Lines are `tracing` fmt output: timestamp, level, enclosing spans with their
fields, target, message, then event fields. Files contain ANSI colour codes
(view with `less -R`, or set `NO_COLOR=1` in the daemon's environment).
Desktop CUA actions and phone requests carry `session_id`/`turn_id` fields at
debug level; the browser lane tags `target`/`tab_id`. There is no per-request
span yet.
