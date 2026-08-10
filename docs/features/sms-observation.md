# Observation-only SMS ingress

## Status

Implemented and live-proven as of 2026-08-10. The reviewed host runtime and
companion were installed locally, `READ_SMS` was granted, and the named
CompanionDirect profile completed an empty future-window probe plus two strict
Sky-Comms SMS-only syncs.

## Summary

The operator can read bounded Android `Telephony.Sms` pages through an
authenticated CompanionDirect link. The lane is observation-only and uses
named profiles so it never selects an implicit phone, serial, or fallback
transport.

## Contract surface

The stable operator command is:

```text
sky-cua-client phone sms query --profile NAME --start-ms ... --end-ms ... [--limit ...] [--cursor ...]
```

Success and failure use schema `sky-cua.sms-query.v1`; success carries
`profile`, stable `device_id`, `transport = companion_direct`,
`access = observation_only`, and a one-page `page` object. Raw message fields
remain nullable. Failure is nonzero and carries a structured `error` code.

Named profiles live under `[phone.profiles.<name>]` and contain only
`device_id`, `transport`, `access`, and `required_capabilities`. Direct-link
credentials remain in the existing private state store.

```toml
[phone.profiles.sky-comms]
device_id = "<stable enrolled CompanionDirect device ID>"
transport = "companion_direct"
access = "observation_only"
required_capabilities = ["sms.read"]
```

## Behavior

The host validates the named profile, requires phone support to be enabled and
the authenticated direct device to currently advertise `sms.read`, and then
dispatches only `sms.query` over `phone-control.v2`. The Android side reads
`Telephony.Sms.CONTENT_URI` through `ContentResolver` with `date >= start_ms`,
`date < end_ms`, `type = MESSAGE_TYPE_INBOX`, keyset order `date ASC, _id ASC`,
and a bounded 1..500 page. The provider projection contains only the portable
identity/ingress subset; optional AOSP columns omitted by OEM restricted views
remain present in the wire response as null.
The opaque cursor carries the fixed query window and last key. Provider,
permission, transport, deadline, or decode errors fail the complete request;
the host never forwards partial messages or a cursor.

The companion declares only `READ_SMS` for this lane. It does not send SMS,
write provider rows, or become the default handler. The main activity exposes
permission state and an explicit request action; no automatic grant logic is
present.

## Source paths

- Rust request/response DTOs: `crates/sky-cua-platform/src/model/phone.rs`
- Named profile resolution: `crates/sky-cua-platform/src/config.rs`
- Strict host routing: `crates/sky-cua-service/src/phone/manager/mod.rs`
- Android direct dispatch and provider reader:
  `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/direct/DirectDispatcher.kt`,
  `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/service/SmsController.kt`
- Operator CLI: `crates/sky-cua-client/src/operator_cli.rs`
- Wire contract: `docs/runtime/phone-control-v2.md` and
  `docs/runtime/phone-companion-protocol.md`

## Verification

- `cargo test -p sky-cua-platform`
- `cargo test -p sky-cua-client sms`
- `JAVA_HOME=/usr/lib/jvm/java-21-openjdk ./gradlew :app:testDebugUnitTest`
- Cross-language request/response/error fixtures under
  `docs/runtime/fixtures/phone-control-v2/`

Live acceptance used one enrolled physical Android device. The first isolated
Sky-Comms sync imported 171 inbound messages from one exhausted page; the
immediate replay added no rows and advanced the checkpoint. The acceptance did
not send SMS or write provider rows.

## Known limitations

- The query intentionally reports `snapshot = false`; it does not promise a
  transactionally stable provider snapshot across pages.

## Related

- Runtime contract: [`../runtime/phone-control-v2.md`](../runtime/phone-control-v2.md)
- Phone feature baseline: [`phone-use.md`](phone-use.md)
- ROADMAP: Android phone control phase
