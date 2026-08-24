# Direct LAN (phone-control.v2) without Tailscale

## Status

Implemented 2026-08-24. Rust `validate_bind_addr` + Android `EndpointValidator` share one spec; `0.0.0.0/::` wildcard binds and RFC1918/link-local/ULA `ws://` endpoints are allowed. Tailscale path unchanged.

## Validation spec (source of truth for Rust + Kotlin)

```
if scheme == "wss" → allow any host (DNS or IP). TLS handles trust.
if scheme == "ws"  → allow iff host ∈ PRIVATE ∪ LOOPBACK ∪ TAILSCALE else reject `WsRequiresPrivateNetwork`.
else reject.
```

`PRIVATE` for `ws://` cleartext:

- IPv4: `127.0.0.0/8` loopback, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `100.64.0.0/10` CGNAT
- IPv6: `::1`, `fd00::/7` ULA (superset of `fd7a:115c:a1e0::/48`), `fe80::/10` link-local (strip `%zone` before check), `::ffff:/96` → inner IPv4 re-evaluated
- Hostnames: `localhost`, `*.ts.net` allow `ws://`; other DNS requires `wss://` (no DNS resolution in validator; IP literal required for `ws://`).

Rust: `crates/sky-cua-service/src/phone/direct/mod.rs:is_private_ip` + `validate_bind_addr`.
Android: `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/direct/Enrollment.kt:EndpointValidator`.

Parity is enforced by identical parameterized vectors in `sky-cua-service::phone::direct::tests::rejects_wildcard_binds` and `EnrollmentTest.cleartextWebSocketIsLimitedToLoopbackOrTailscale`.

## Bind semantics

- `0.0.0.0` / `[::]` now `Ok` — one socket covers WiFi + ethernet + USB tether (`rndis0`/`usb0`). Previously `InvalidInput`.
- Multicast still rejected. Public routable (`203.0.113.5`, `2001:db8::1`) still `InvalidInput` for cleartext listeners; `wss://` with TLS is the intended public escape hatch.
- Per-interface bind (`192.168.42.10:47684`) still supported.

## Advertisement / enumeration

No mDNS/UPnP/STUN. Host enumerates candidates via `if-addrs` (`crates/sky-cua-service/src/phone/direct/lan.rs:enumerate_lan_candidates`): private ∪ loopback-excluded, `tun*` dropped if non-`tun` candidates exist, sorted `192.168` > `10/8` > `172.16/12` > `fd00::/8` > `169.254`, `%zone` stripped. Wired into `PhoneManager::status` (`crates/sky-cua-service/src/phone/manager/mod.rs:734`) as diagnostic `DirectLanCandidates` (message lists `iface:ip` + listen/advertised, details JSON has `candidates[]`), visible via `status(component="phone")` for QR generation (`skycua://enroll?endpoint=ws://<lan-ip>:<port>&…`). Manual override `direct_advertised_endpoint` / `SKY_CUA_PHONE_DIRECT_ADVERTISED_ENDPOINT` wins. Endpoint is static until re-enroll; `MultiHostDirectLinkPool` `reconnectLoop` (1s) retries but does not mutate endpoint. DHCP churn → `HostUnreachable` diagnostic → re-QR.

## Subnets

- WiFi LAN: operator-assigned `192.168.x.x` / `10.x.x.x`
- Hotspot: `192.168.43.0/24`, `192.168.49.0/24`
- USB tethering (RNDIS): `192.168.42.0/24`, `192.168.44.0/24` on `rndis0`/`usb0`/`enp*` (not `tun*`, so never filtered).
- `fe80::/10` requires zone handling; callers must pass bare IP to validator (zone stripped).

## Security

`ws://` on LAN/tether is cleartext on trusted L2 (WPA2/3 / physical USB). Same HMAC enrollment/auth + epoch fencing as Tailscale; no downgrade to unauth. UI shows: "Cleartext on local network — requires WiFi password/physical access to intercept. Secret still required." Public `ws://` remains rejected.

## Firewall

`ufw allow <port>/tcp` on `wlan0` / `rndis0` / `usb0` (or `firewalld` equivalent). `ss -tlnp` should show `0.0.0.0:<port>` when wildcard bound.

## Related

- Enrollment flow: `crates/sky-cua-service/src/phone/direct/mod.rs:DirectRuntime`, `android/.../direct/Enrollment.kt:EnrollmentRedeemer`
- Credential: `android/.../direct/CredentialStore.kt:HostRecord` (`MAX_PAIRED_HOSTS=8`)
- Platform config: `crates/sky-cua-platform/src/config.rs:direct_listen_addr`, `direct_advertised_endpoint`
