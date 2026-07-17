# `@heliasar/sky-cua`

Private pure-ESM model-facing Computer Use compatibility facade for the
already-running sky-cua service. The package is bundled into `cua_node` and is
never published.

The root export is exactly the lazy named export `sky`. Linux exposes
`click`, `drag`, `get_screenshot`, `move`, `press_key`, `scroll`, and
`type_text`. The Darwin-shaped API is present as a lazy placeholder and throws
`SKY_CUA_TARGET_UNAVAILABLE` because v1 has no macOS backend.

Configuration is read only on first enumeration or use. `OAI_SKY_CONFIG_PATH`
takes precedence over `SKY_CUA_JS_CONFIG_PATH`; both accept the upstream-shaped
JSON object with `target`, `post_action_sleep_ms`, `mouse_size_px`, and the
first-party optional `service_socket_path`.

The facade connects directly to the owner-only sky-cua service socket. It never
launches, restarts, or invokes the daemon or its MCP server. Mutating calls
require `session_id` and `turn_id` in `nodeRepl.requestMeta`, use a separate
CancelTurn connection for cancellation, and never retry a mutation after an
ambiguous disconnect. The optional `requestMeta.deadline_ms` is the service
contract's integer `1..30000` action deadline; no deadline alias is supported.
Screenshots retain the upstream array shape but use the service's WebP bytes and
do not call `nodeRepl.emitImage` implicitly.

The future `@heliasar/sky-cua/advanced` subpath is intentionally documented but
not implemented in v1.

## Screenshot performance benchmark

Run the durable Node 24 benchmark against an already-running service:

```sh
bun run benchmark:screenshot -- --socket /run/user/1000/sky-cua/service.sock --iterations 100
```

The socket path may also be the first positional argument and iterations the
second. Iterations default to `100`; the socket follows the facade's normal
environment/runtime/cache fallback. The benchmark never starts, stops, or
restarts the service and adds no authentication or sandbox layer.

The benchmark holds one raw NDJSON connection and one public-facade connection,
sends exactly one health request per connection, warms each once, then
interleaves measured `get_screenshot` reads. It reports raw and facade p50/p95
latency and fails above 10% facade p95 overhead. Every measured pair must have
the same screenshot count, but raw and facade captures are validated
independently because they observe the desktop at different times. Each lane
must produce existing absolute `.webp` paths, valid WebP dimensions, and
internally consistent bytes and data URLs. Aggregate facade binary payload size
may exceed the aggregate raw payload by at most 5%; the compatibility `data_url`
character count is reported separately because it is an explained
representation, not another wire payload.

The package command supplies `--expose-gc`. Post-GC heap samples fail only when
growth exceeds both the bounded allowance and per-iteration slope while at
least 70% of samples continue rising. Screenshots are reads: requests contain
no `post_action_sleep_ms`, while the facade retains its default 100ms pacing for
mutating actions.

For a fast calculation, parity, and connection-semantics check that does not
contact the real service, including exact path, bytes, dimensions, and data-URL
adapter transformation parity against deterministic fake-daemon captures:

```sh
bun run benchmark:screenshot:self-test
```
