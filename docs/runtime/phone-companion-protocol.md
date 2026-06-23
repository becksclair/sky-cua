# Phone companion RPC protocol (v1)

This document is the authoritative wire contract between the sky-cua host
(`crates/sky-cua-service/src/phone/companion/`) and the Android phone companion
app. It is the source of truth for both the Rust host client and the Android app
implementation. The protocol version is **1**; the host sends
`protocol_version: 1` on every request and rejects any other version.

## Transport

The companion exposes a localhost-only HTTP/1.1 endpoint inside the app. The host
reaches it through a host-managed ADB port forward:

```
adb -s <serial> forward tcp:<port> tcp:<port>
```

After the forward is established, the host connects to `127.0.0.1:<port>` and
issues one request per connection:

- Method/path: `POST /rpc`
- `Content-Type: application/json`
- `Connection: close` (the companion serves one request per TCP connection; the
  host opens a fresh connection per call)
- Exactly one JSON request body, exactly one JSON response body

The host buffers the response until EOF, capped at 32 MiB so a malformed or
hostile companion cannot exhaust host memory. The largest legitimate payload is a
base64-encoded screenshot.

The endpoint is reachable only over the forwarded loopback socket. The companion
must bind loopback only and must never accept off-device connections.

## Envelopes

### Request

```json
{
  "protocol_version": 1,
  "token": "<session-token>",
  "id": 7,
  "method": "screenshot",
  "params": { "include_overlay": false }
}
```

- `protocol_version` — always `1`.
- `token` — the ephemeral session token (see Authentication). Sent on every call.
- `id` — a host-chosen monotonically increasing integer. The response must echo
  it. The host treats a non-matching `id` as a protocol violation and falls back.
- `method` — one of the methods below.
- `params` — a method-specific object. Methods with no parameters send `{}`.

### Response — success

```json
{
  "protocol_version": 1,
  "ok": true,
  "id": 7,
  "result": { "...": "method-specific" }
}
```

### Response — error

```json
{
  "protocol_version": 1,
  "ok": false,
  "id": 7,
  "error": { "code": "secure_window", "message": "human-readable detail" }
}
```

The host routes on `error.code`, never on `error.message`.

## Authentication

The companion validates the token and its expiry on **every** call, before method
dispatch. A missing, wrong, or expired token returns error code `unauthorized`.

The host generates an ephemeral token plus a TTL once per session
(`companion_rpc_token_ttl_ms`, default 900000 ms = 15 minutes) and delivers it to
the companion out of band of `/rpc`, through an ADB-launched setup intent that
carries the token directly as a string extra:

```
adb -s <serial> shell am start -n <package>/.SetupActivity \
  --es sky_cua_rpc_token <token> \
  --el sky_cua_rpc_token_expires_at_ms <epoch_ms>
```

- `--es sky_cua_rpc_token` — the bearer token as a string extra.
- `--el sky_cua_rpc_token_expires_at_ms` — the absolute expiry in epoch
  milliseconds, as a long extra.

The companion also still accepts a `sky_cua_rpc_token_file` path extra as a
legacy fallback, but the host no longer uses it: Android 11+ gives each app an
isolated storage mount namespace, so a file the host (`adb`/shell) writes into
`/sdcard/Android/data/<package>/` is not readable by the app process. The earlier
file handoff therefore failed silently — `SetupActivity` could not read the
token, so it never started the RPC server. Direct-extra delivery works across API
30–36.

The companion stores the token and expiry and validates them on each request. The
token is a localhost-only, ADB-gated bearer credential scoped to one session; it
is never written to logs, config, artifacts, or responses on the host side.

## Security and threat model

The RPC server binds loopback only and gates every privileged ability
(accessibility-tree reads, gesture dispatch, screenshots, notifications) on the
session token above. The token is the sole authentication boundary, so its
delivery path is the critical surface.

`SetupActivity` is exported so the host can target it by explicit component
through `adb shell am start`, but it requires `android.permission.DUMP`. The
shell UID has that platform permission; ordinary co-resident apps do not, so
they cannot `startActivity()` it to install their own RPC token.

The token is delivered as an intent string extra. This does place the token in
the `am start` argv, but `hidepid` hides `/proc/<pid>/cmdline` from other uids on
modern Android and the token is ephemeral (15-minute TTL), localhost-only, and
ADB-gated, so the exposure window is bounded. The pushed-file alternative is not
viable: per-app storage mount-namespace isolation (Android 11+) makes a
host-written file under `/sdcard/Android/data/<package>/` unreadable by the app,
which silently broke the bootstrap. This remains a trusted-local, ADB-gated
bootstrap rather than a remote authentication protocol.

The documented robust future fix (not scheduled) is a handshake redesign: the
companion mints its own token and emits it to its own logcat; the host reads it
back via `adb logcat`. Third-party apps cannot read another app's logcat
(`READ_LOGS` has been `signature|privileged` since Android 4.1) but `adb shell`
can, which closes the injection vector across API 30–36 without root or the
platform signing certificate.

## Version negotiation

The host always sends `protocol_version: 1`. If the companion speaks a different
version it returns error code `version_mismatch` (and/or replies with an envelope
whose `protocol_version` differs from `1`). Either signal causes the host to fall
back to ADB/scrcpy rather than proceeding against an unknown protocol.

## Host failure handling and fallback

The host classifies every failure and decides whether to fall back to the ADB or
scrcpy backends. Fallback means "this companion call cannot be trusted; route the
operation through another backend" — the host never fabricates success.

| Host condition | Diagnostic code | Falls back? |
| --- | --- | --- |
| TCP connect refused/failed | `CompanionConnectRefused` | yes |
| Call exceeded timeout | `CompanionTimeout` | yes |
| Socket read/write failure mid-exchange | `CompanionIo` | yes |
| HTTP status line missing or non-200 | `CompanionHttpStatus` | yes |
| Body not valid JSON / envelope shape wrong / `result` shape wrong | `CompanionMalformedResponse` | yes |
| `protocol_version` mismatch or `version_mismatch` code | `CompanionVersionMismatch` | yes |
| `unauthorized` code (missing/wrong/expired token) | `unauthorized` | yes |
| Response `id` mismatch or `ok`/`result`/`error` inconsistency | `CompanionProtocolViolation` | yes |
| Dispatch-level code: `unknown_method` or `internal` | `CompanionProtocolViolation` | yes |
| `bad_request` (overloaded — see below) | the method's error code | **no** |
| Well-formed per-method application error (e.g. `secure_window`) | the method's error code | **no** |

`unknown_method` and `internal` are *dispatch-level* codes: the request never
reached a working method handler (the companion could not resolve the method or
hit an unhandled server fault). They are not tied to any one method's semantics,
so the host treats them as a protocol violation and falls back to ADB/scrcpy
rather than misreading them as a per-method application error.

`bad_request` is deliberately excluded from fallback because it is *overloaded*:
the companion emits it both for dispatch-level envelope/parameter validation and
for genuine per-method application errors (for example `open_intent` rejecting an
unparseable intent URI). The two are indistinguishable on the wire, so promoting
all `bad_request` responses to a session-wide fallback would tear down the whole
companion session over one benign per-method rejection. The host therefore treats
`bad_request` as a non-fallback per-method error; method families with an
alternate lane may fall back on their own, while companion-only families fail
that action without invalidating the session. (A future companion revision could
split the per-method use into a method-scoped code so `bad_request` becomes
purely dispatch-level.)

A per-method application error is a *successful* RPC that reports the operation
could not be performed (a secure window, an expired notification, etc.). The host
surfaces it as a structured diagnostic on the companion backend and does **not**
silently retry the same operation through ADB pretending the companion succeeded.

## Methods

### `health`

Liveness plus permission/capability booleans.

- Params: none (`{}`)
- Result:

```json
{
  "version": "1.2.0",
  "version_code": 12,
  "package": "com.skycua.phonecompanion",
  "accessibility_enabled": true,
  "can_perform_gestures": true,
  "can_retrieve_window_content": true,
  "can_take_screenshot": true,
  "notification_listener_enabled": true,
  "native_overlay": true,
  "native_overlay_pass_through": true,
  "privileged_setup": "shizuku"
}
```

`privileged_setup` is optional.

### `capabilities`

The `health` fields plus screenshot/gesture support detail.

- Params: none (`{}`)
- Result: all `health` fields, plus:

```json
{
  "screenshot_api_level": 34,
  "screenshot_supported": true,
  "gesture_supported": true
}
```

The host derives end-to-end capability flags conservatively: gesture dispatch
requires `accessibility_enabled && can_perform_gestures && gesture_supported`;
screenshot requires `can_take_screenshot && screenshot_supported`; accessibility
tree requires `accessibility_enabled && can_retrieve_window_content`;
notifications require `notification_listener_enabled`.

### `accessibility_tree`

Bounded active-window node list.

- Params: `{ "max_nodes": 250 }`
- Result:

```json
{
  "package": "com.example",
  "activity": ".MainActivity",
  "nodes": [
    {
      "class": "android.widget.Button",
      "text": "OK",
      "content_desc": "Confirm",
      "bounds": [10, 20, 110, 70],
      "focusable": true,
      "enabled": true,
      "clickable": true
    }
  ],
  "truncated": false,
  "redacted": false
}
```

`bounds` is a raw `[left, top, right, bottom]` device-pixel rectangle. `package`,
`activity`, and the per-node text fields are optional.

### `screenshot`

On-device capture.

- Params: `{ "include_overlay": false }`
- Result:

```json
{
  "mime_type": "image/png",
  "data_base64": "<base64>",
  "width": 1080,
  "height": 2400,
  "contains_native_overlay": false
}
```

- Error codes: `secure_window`, `unsupported_api`, `disabled_service`,
  `oem_policy`, `throttled`, `transient`.

`oem_policy` is reserved on the screenshot route: the companion captures the
full display via the accessibility-service screenshot API and has no current OEM
code path that returns it, so the host accepts it as a structured error but does
not expect to observe it. (The overlay-free `takeScreenshotOfWindow` capture that
might surface an OEM window-capture restriction is not implemented; see the
feature doc.)

`include_overlay` controls whether the phone-native agent overlay (cursor, ripple,
trail, and edge glow) appears in the capture:

- `include_overlay: false` (the default for model-facing screenshots): the
  companion MUST synchronously hide every overlay pixel for the duration of the
  capture and restore the prior state afterward, so the frame the model sees is
  clean. The hide cancels any in-flight gesture animation and stops the breathing
  loop before capture; the restore re-arms whatever was showing before. The
  result reports `contains_native_overlay: false`.
- `include_overlay: true`: the overlay is left as-is and the captured frame may
  contain it; `contains_native_overlay` then reflects whether the overlay was
  actually visible at capture time.

`contains_native_overlay` tells the host whether the captured pixels already
include the phone-native agent overlay, so the host avoids double-compositing a
screenshot-synthetic cursor.

### `gesture`

Dispatch a tap or swipe.

- Params: `{ "kind": "tap" | "swipe", "points": [{ "x": 5, "y": 6 }], "duration_ms": 50 }`
  - `tap` uses one point; `swipe` uses two (start, end). Coordinates are
    device pixels.
- Result: `{ "dispatched": true }`

### `cursor_overlay`

Show, move, or hide the phone-native cursor overlay.

- Params: `{ "visible": true, "x": 100, "y": 200 }` (device pixels)
- Result: `{ "shown": true, "pass_through": true }`

The overlay must be non-focusable and non-touchable; `pass_through` reports
whether taps pass through it to the underlying app.

`cursor_overlay` sets a single static cursor position. The animated agent
overlay — the persistent "agent in control" glow and per-action cursor
animations — is driven by `overlay_active` and `overlay_gesture` below. All three
are backed by the same single full-screen `TYPE_ACCESSIBILITY_OVERLAY` view.

### `overlay_active`

Toggle the persistent "agent in control" breathing screen-edge glow. The host
calls this with `active: true` when a phone session establishes with a reachable
companion, and `active: false` on disconnect/release.

- Params: `{ "active": true }`
- Result: `{ "active": true, "glow_supported": true }`

`active` in the result reports the glow state after the call. `glow_supported` is
`false` only when the accessibility service is unavailable (the overlay cannot be
drawn); a session is still usable without the glow, so the host treats a
`glow_supported: false` result as a swallowed per-method failure rather than a
fallback trigger.

### `overlay_gesture`

Animate the agent cursor for one action. **Visual only — it must not dispatch any
real input**; the real tap/swipe is dispatched separately by the companion
`gesture` method. The host calls this after each successful coordinate action
when the companion is reachable.

- Params:

```json
{
  "kind": "tap",
  "points": [{ "x": 100, "y": 200 }],
  "duration_ms": 220
}
```

- `kind` — `"tap"`, `"swipe"`, or `"drag"`. Unlike `gesture`, the overlay path
  supports `drag` in addition to `tap`/`swipe`, so `kind` is a free-form wire
  string here rather than the `gesture` enum.
- `points` — the device-pixel path. `tap` uses one point; `swipe`/`drag` use two
  or more (start..end). Coordinates are device pixels, the same space `gesture`
  uses.
- `duration_ms` — an animation duration hint; the companion clamps it to a sane
  minimum.
- Result: `{ "animated": true }`

Behavior: the companion moves the cursor to `points[0]`; for `tap` it shows an
expanding, fading ripple at the point; for `swipe`/`drag` it traverses the path
drawing a fading trail. The edge glow pulses brighter for the duration, then
returns to the breathing baseline. The animation is on-device and asynchronous
(fire-and-forget); `animated` reports only that the overlay view was available to
animate, not that the animation has finished.

### `notifications`

Bounded recent notification events.

- Params: `{ "max": 20 }`
- Result:

```json
{
  "listener_enabled": true,
  "events": [
    {
      "event_id": "evt-123",
      "package": "com.example.chat",
      "channel": "messages",
      "title": "Alice",
      "body": "see you at 5",
      "redaction": "none",
      "ranking": 3,
      "when_ms": 1718600000000,
      "can_open": true,
      "can_dismiss": true,
      "ongoing": false,
      "actions": [
        { "action_id": "reply", "title": "Reply", "is_reply": true }
      ]
    }
  ],
  "truncated": false
}
```

`redaction` is `none | partial | full`. `channel`, `title`, `body`, and `ranking`
are optional. The redaction level controls how much content is emitted: `none`
keeps both `title` and `body`; `partial` (the mapping for an Android
`VISIBILITY_PRIVATE` notification) keeps the `title` but omits the `body`,
mirroring Android's private-lockscreen behavior where the sender/app label
survives but the sensitive content is withheld on untrusted surfaces; `full`
omits both. An omitted field is absent from the JSON, not emptied.

`can_open`, `can_dismiss`, `ongoing`, and `ranking` are producer-populated by the
companion from the live `StatusBarNotification`/`RankingMap`: `can_open` reflects
a non-null content `PendingIntent`, `can_dismiss` reflects whether the
notification is clearable, and `ongoing` reflects the ongoing/foreground flag.
A `full`-redacted event reports `can_open=false` because its content intent is
not exposed.

### `notification_op`

Open, dismiss, action, or inline-reply on an explicit event id.

- Params: `{ "event_id": "evt-123", "op": "open" | "dismiss" | "action" | "reply", "action_id": "reply", "reply_text": "on my way" }`
  - `action_id` is required for `action`/`reply`; `reply_text` is required for
    `reply`.
- Result: `{ "ok": true }`
- Error codes: `gone`, `redacted`, `pending_intent_missing`, `canceled`,
  `expired`, `immutable`, `reply_unavailable`, `oem_filtered`.

The companion emits `immutable` when an operation needs a fill-in intent, such as
inline reply, but the target `PendingIntent` is immutable (API 31+ surfaces the
immutable flag). Plain open/action sends that do not attach fill-in data may use
immutable PendingIntents. `expired` and `oem_filtered` remain platform-limited:
the companion reports them when the platform exposes the condition, but neither
is reliably observable across OEM builds.

### `current_app`

Foreground app.

- Params: none (`{}`)
- Result: `{ "package": "com.android.chrome", "activity": ".Main", "label": "Chrome" }`
  (`activity` and `label` optional)

`activity` is best-effort: the companion populates it from the resolved
foreground component when available and omits it otherwise, so callers must not
treat a missing `activity` as an error.

### `app_list`

Installed app inventory.

- Params: `{ "launchable_only": true }`
- Result:

```json
{
  "apps": [
    { "package": "com.example", "label": "Example", "launchable": true }
  ],
  "truncated": false
}
```

### `app_op`

Launch, open an intent URI, or force-stop.

- Params: `{ "op": "launch" | "open_intent" | "force_stop", "package": "com.example", "intent_uri": "https://..." }`
  - `package` is used by `launch`/`force_stop`; `intent_uri` by `open_intent`.
- Result: `{ "ok": true }`

## Host implementation

The Rust host side lives under
`crates/sky-cua-service/src/phone/companion/`:

- `protocol.rs` — typed serde DTOs for every envelope, method param, and result,
  plus the `methods` and `error_codes` constant tables.
- `client.rs` — `CompanionClient`, a hand-rolled HTTP/1.1 JSON-RPC client over
  `tokio::net::TcpStream` (no extra HTTP dependency), with the failure
  classification and fallback signaling described above.
- `identity.rs` — install/update/refuse decisioning from package version +
  signing cert SHA-256 + APK SHA-256, ephemeral token generation, and the ADB
  setup-intent and `adb install -r` argv builders (which return argv for the
  command runner and do not themselves run adb).

## Companion identity and install policy

`phone_connect` must never trust an installed APK just because the package name
matches. The host compares the installed package against expected packaged
metadata (version code, signing certificate SHA-256, APK SHA-256) and decides:

- not installed → install the packaged APK (`adb install -r`)
- installed certificate differs from expected → **refuse** to silently replace;
  recovery requires an explicit operator uninstall/reinstall
- installed version newer than expected and downgrade not allowed → **refuse**
  the downgrade (override only with `companion_allow_downgrade=true`, which adds
  `-d` to the install argv)
- installed version older than expected → update
- otherwise → up to date

The certificate refusal is evaluated before the version comparison so a malicious
same-named package can never be "updated" over a trusted one.
