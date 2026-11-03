# Sky Phone Companion (android/phone-companion)

The Android companion app for `phone-use`. It is the rich phone backend
(phone-native cursor overlay, accessibility-tree snapshots, native gesture
dispatch, accessibility screenshots, notification forwarding, camera,
clipboard, SMS and storage), connected to the sky-cua host over the
CompanionDirect lane: after QR enrollment (`skycua://enroll`) the app opens an
outbound `ws://`/`wss://` link to each enrolled host and authenticates with a
per-host HMAC device secret and link epoch.

## Authoritative contract

The current wire contract (frames, handshake, methods) is
`docs/runtime/phone-control-v2.md`; bind and transport rules are in
`docs/runtime/direct-lan.md`. `docs/runtime/phone-companion-protocol.md`
describes the removed v1 transport and is being reconciled separately. Do not
change wire behavior here without updating the contract documents and the Rust
host side in lockstep.

## Toolchain

- JDK 21 is required for Gradle/AGP. The default system `java` (26) is rejected
  by AGP. Build with `JAVA_HOME=/usr/lib/jvm/java-21-openjdk`.
- `ANDROID_SDK_ROOT=$HOME/Android/Sdk`. `local.properties` (gitignored) also
  carries `sdk.dir`.
- AGP 9.4.1 + Gradle 9.8.1 (wrapper checksum pinned). AGP 9 has built-in
  Kotlin support; the standalone Kotlin Gradle plugin must not be applied.
- `compileSdk = 37`, `targetSdk = 36`, `minSdk = 33`. JDK 21 is pinned via
  `jvmToolchain(21)` in `app/build.gradle.kts`. Features are runtime-gated by
  `Build.VERSION.SDK_INT`: `takeScreenshotOfWindow` (API 34+).

## Build and test

```bash
export JAVA_HOME=/usr/lib/jvm/java-21-openjdk
export ANDROID_SDK_ROOT="$HOME/Android/Sdk"
./gradlew :app:testDebugUnitTest    # JVM unit tests, no device
./gradlew :app:assembleDebug        # builds the APK, emits build-metadata.json
```

`assembleDebug` is finalized by `emitBuildMetadata`, which writes
`build-metadata.json` (package id, versionCode, versionName, APK relative path,
APK SHA-256, signing certificate SHA-256). The certificate fingerprint comes
from `apksigner verify --print-certs` (v2/v3-aware) with a v1 JAR-signature
fallback. The host uses this metadata for companion identity and install policy.

## Structure

All sources are under `app/src/main/java/com/skycua/phonecompanion/`.

- `direct/` — CompanionDirect: host link and transport (`DirectLink`,
  `DirectLinkRuntime`, `MultiHostDirectLinkPool`, `OkHttpDirectSocket`);
  enrollment and credentials (`Enrollment`, `CredentialStore`, `Auth`,
  `LinkEpoch`); request dispatch (`DirectDispatcher`); content transfer
  (`ContentTransfer`, `ContentTransferReceiver`, `CompanionContentProvider`).
- `protocol/` — method names, param/result types and validation, and the
  method `Dispatcher` that `DirectDispatcher` wraps. `TokenStore` is a v1
  remnant filled with a fixed internal token; it is not a credential (its
  KDoc is stale).
- `json/` — small dependency-free JSON model/parser/writer.
- `service/` — `SkyAccessibilityService`, `SkyImeService`,
  `SkyNotificationListenerService`, `DeviceMethodHandler` (wires methods to
  live services), and the camera, clipboard, SMS and storage controllers.
- `overlay/` — pass-through agent cursor overlay (controller, view, AGSL
  renderers, window flags).
- `camera/` — camera capture service and contract. `app/` —
  PackageManager app management. `appshot/` — phone AppShot DTO and capture
  policy. `screenshot/` — accessibility screenshot failure classification.
  `ui/` — grid/pointer/interaction playground activities.
- Top-level activities: `MainActivity` (status UI), `EnrollmentActivity` +
  `EnrollmentFlowViewModel` (QR enrollment), `HostListActivity` (paired
  hosts, per-host remove), `CaptureActivity` (visible surface for remote
  camera), `SafGrantActivity` (one-time SAF tree picker for remote storage).

## Security

Never log, persist outside `CredentialStore`, or commit device secrets,
enrollment payloads, notification content, accessibility dumps, or
screenshots. The overlay is non-focusable and non-touchable so taps pass
through. Each host link is mutually authenticated before any method frame is
dispatched: an HMAC-SHA256 challenge/response under the per-host device secret,
with a strictly increasing link epoch persisted on `auth_ok` (`direct/`
package: `Auth`, `DirectLink`, `LinkEpoch`). Frames other than the handshake
that arrive before the link is authenticated fail it. Cleartext `ws://` is
accepted only for loopback, private/link-local/Tailscale IP literals, or
`*.ts.net` names; other hosts require `wss://`.
