# Phone Control v2

`phone-control.v2` is the direct Android Companion protocol. It coexists with
the ADB-forwarded Companion RPC v1 documented in
`docs/runtime/phone-companion-protocol.md`; a direct session uses a stable
`device_id`, never an ADB serial.

## Transport and limits

The Companion initiates one persistent WebSocket to the configured Saga
MagicDNS endpoint over Tailscale. Saga enables the listener explicitly and
binds a concrete tailnet unicast address. Loopback is valid for tests;
unspecified/wildcard and public binds are rejected.

The built-in listener is a raw `ws://` endpoint carried inside Tailscale's
encrypted tunnel; it does not terminate TLS. The Companion accepts cleartext
WebSocket URLs only for loopback, Tailscale CGNAT/IPv6 addresses, or `*.ts.net`
MagicDNS names. A separately terminated `wss://` endpoint remains valid.

Text frames contain one UTF-8 JSON `PhoneDirectControlFrame` and are capped at
1 MiB. Binary frames contain one fixed, length-delimited chunk header followed
by bytes; the default chunk size is 256 KiB. Binary content is never base64 in
JSON. Control frames are interleaved ahead of subsequent bulk chunks. Camera
preview requests are latest-frame-wins.

V1 does not define a chunk acknowledgement or transport-drain window. Its
OkHttp sender therefore does not claim protocol-level backpressure for
arbitrarily large files; adding an acknowledged streaming window is a deferred
transport revision. V1 camera output is bounded at capture time as described
below, and every phone-to-host media transfer is a separate explicit content
export request.

## Enrollment

`phone_setup(operation="create_enrollment")` creates a random five-minute,
single-use enrollment record and returns a manual code plus a QR image. The QR
encodes only:

```json
{
  "protocol": "phone-control.v2",
  "endpoint": "ws://saga.example.ts.net:47684/phone/control",
  "enrollment_id": "<random id>",
  "bootstrap_credential": "<random one-time credential>",
  "expires_at_ms": 0
}
```

Successful redemption atomically consumes the bootstrap and persists a
`Pending` device containing a random stable `device_id`, random 256-bit
per-device secret, enrollment ID, and a separate bounded commit deadline. Only
then does Saga return `enrollment_ok`; this is the only frame that ever carries
the durable secret. A consumed bootstrap is never restored or replayed, even
when that response is lost.

Android atomically stores the complete credential and endpoint in
credential-protected storage, encrypted by an AES-GCM key held in Android
Keystore. After that durable commit, it sends an idempotent `enrollment_ack`.
Saga verifies the acknowledgement and atomically promotes `Pending` to
`Active`, then returns `enrollment_committed`. Pending devices cannot dispatch
control operations. An expired pending device cannot acknowledge or reconnect
and is removed lazily. A first-frame acknowledgement retry is idempotent and
returns the retained commit receipt when the original response was lost. If the
acknowledgement never reached Saga, a successful mutual-HMAC reconnect using the
pending credential performs the same atomic promotion before application
traffic is admitted. Neither recovery path retransmits the secret.

The acknowledgement proof uses the same LP32 encoding as authentication, with
the exact fields:

```text
protocol, "enrollment_ack", enrollment_id, device_id, client_nonce
```

The nonce is canonical unpadded base64url decoding to exactly 32 bytes. The
proof is lowercase hex HMAC-SHA256 under the newly stored device secret.

Revocation terminates the current link and removes the durable enrollment.
Reconnection then requires fresh enrollment.

## Mutual authentication

The WebSocket endpoint is static. After it opens, the Companion sends
`auth_hello` with protocol, canonical device ID, and a fresh 32-byte client
nonce encoded as unpadded base64url. Stable device identity is never placed in
the URL, upgrade headers, or WebSocket subprotocol.

Saga looks up the non-revoked device, generates a fresh 32-byte server nonce,
and reserves (without persisting) the next epoch. It replies with
`auth_challenge`, containing the server nonce, canonical decimal epoch string,
and its `saga`-role HMAC proof. The Companion validates the echoed handshake
state, requires an increasing epoch, verifies the proof in constant time, then
sends `auth_proof` with the epoch and its `companion`-role proof. Saga validates
that proof under the same per-device handshake lock.

The canonical HMAC input is length-prefixed UTF-8 fields in this exact order:

```text
protocol, device_id, server_nonce, client_nonce, decimal_link_epoch, role
```

Each field is encoded as a four-byte unsigned big-endian byte length followed
by the field bytes. Roles are the serialized strings `companion` and `saga`.
Proofs are lowercase hex HMAC-SHA256. UUIDs are lowercase hyphenated ASCII;
nonces are canonical unpadded base64url strings decoding to exactly 32 bytes;
epoch strings have no sign or leading zeros. Canonicalization is validated, not
silently normalized after receipt. Implementations compare proofs in constant
time and reject nonce replay, a non-increasing epoch, the wrong role, an unknown
device, or a revoked enrollment.

On valid client proof, Saga confirms the persisted predecessor, preconstructs
the new actor, durably records the epoch, atomically fences/cancels the prior
link, and sends `auth_ok`. Only after that matching `auth_ok` is written does
Saga make the new actor routable; the Companion likewise sends no application
or binary traffic before receiving it. Persistence failure leaves the prior
link active. Failure to deliver `auth_ok` after persistence leaves the new epoch
fenced and no active candidate; reconnect advances again rather than reviving an
old epoch.

Every request, response, event, transfer declaration, chunk, commit, and abort
carries the accepted link epoch; old-epoch traffic is rejected. A newer pending
hello may replace an older pending handshake, but never disturbs the active link
until proof verification and the durable activation commit succeed.

## Control semantics

Requests include `request_id`, `device_id`, `link_epoch`, `idempotent`, absolute
`expires_at_ms`, method, and params. Expired requests fail without dispatch.
Non-idempotent requests are never replayed after an ambiguous disconnect.
Responses and events must match the authenticated device and epoch.

The stable JSON enum is defined by
`crates/sky-cua-platform/src/model/phone_direct.rs`. Unknown frame types or
invalid shapes are protocol errors, not capability fallbacks.

## Finite content transfer

A sender first emits `content_declare`, including `ContentRef`, exact length,
SHA-256, chunk size, and chunk count. The receiver writes to a private temporary
file while validating transfer ID, contiguous offsets, chunk indexes, per-frame
lengths, and epoch. `content_commit` succeeds only when the final byte count and
SHA-256 match the declaration; commit is an atomic rename into managed storage.

Disconnect, cancellation, an old epoch, a digest mismatch, a duplicate or
missing chunk, or a length overflow aborts and removes the incomplete file.
Idempotent reads may restart from byte zero after reconnect. v2 does not provide
cross-reconnect resumable writes.

Temporary phone content defaults to a 15-minute lease. Private host AppShot
artifacts default to one hour. Persisting to MediaStore, a SAF root, or an
explicit host path removes the temporary lease.

## Reconnection and boot

While its process is alive, the Companion reconnects with bounded exponential
backoff and jitter across backgrounding, screen-off, and network changes. After
whole-package process death, backoff resumes only once Android or the user
recreates the process; the supported Galaxy recovery is to relaunch the
Companion and use its visible Retry action. Capabilities therefore report
`process_death_autorestart=false`; `START_STICKY` remains best effort rather than
a resurrection guarantee. Full reconnection begins only after the first local
secure unlock following reboot; v2 does not bypass Android Direct Boot
credential boundaries.

### Binary chunk wire layout

One WebSocket binary message carries exactly one chunk. Integers are unsigned
big-endian. The message is `transfer_id_len:u8`, that many UTF-8 transfer ID
bytes, `link_epoch:u64`, `chunk_index:u64`, `offset:u64`, `length:u32`, then
exactly `length` payload bytes. Transfer IDs contain 1–255 UTF-8 bytes and chunk
payloads are at most 256 KiB. There is no padding or trailing data.

For transfer ID `t1`, epoch 1, index 0, offset 0, and payload `01 02 03`, the
complete frame hex is:

```text
02743100000000000000010000000000000000000000000000000000000003010203
```

Chunks must arrive in declared index and contiguous-offset order. Every
non-final chunk is the declared chunk size; the final chunk is exactly the
remaining declared bytes. A malformed binary message closes the link and aborts
all incomplete transfers. Control frames take priority when selecting the next
WebSocket message; an already-started message is not preempted.

## Bounded camera capture

The V1 camera descriptor advertises `max_capture_size` as 1920x1080,
`max_video_duration_ms` as 60000, and `automatic_media_transfer` as `false`.
Portrait 1080x1920 is the same allowed resolution. Larger requested capture
sizes are rejected; the Companion configures CameraX at or below this bound and
rejects any output that nevertheless exceeds it. A video recording stops at
the one-minute wall-clock limit and retains its final result for `video_stop`.

Photo, video-stop, and preview-frame completion return a temporary phone-local
`ContentRef`. They do not enqueue the captured bytes on the WebSocket. A caller
that wants a host file must issue `phone_content(operation="export_host_file")`
with that content ID; only that explicit request initiates a finite binary
transfer. Capture and transfer are therefore independently observable and
controllable operations.

No runtime ADB, Shizuku, shell broker, root, Device Owner, or AppServer hook is
part of this protocol.
