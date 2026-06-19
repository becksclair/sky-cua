# Sky Phone Companion (android/phone-companion)

The Android companion app for `phone-use`. It is the preferred rich backend
after the ADB bootstrap: phone-native cursor overlay, accessibility-tree
snapshots, native gesture dispatch, accessibility screenshots, and notification
forwarding, exposed to the sky-cua host over a localhost-only RPC endpoint
reached through host-managed `adb forward`.

## Authoritative contract

The wire protocol is defined by `docs/runtime/phone-companion-protocol.md`. That
document is the source of truth for envelope shapes, the protocol version, method
names, error codes, and the setup-intent token bootstrap. Do not change wire
behavior here without updating that document and the Rust host side in lockstep.

## Toolchain

- JDK 21 is required for Gradle/AGP. The default system `java` (26) is rejected
  by AGP. Build with `JAVA_HOME=/usr/lib/jvm/java-21-openjdk`.
- `ANDROID_SDK_ROOT=$HOME/Android/Sdk`. `local.properties` (gitignored) also
  carries `sdk.dir`.
- AGP 9.2.1 + Gradle 9.5.1. AGP 9 has built-in Kotlin support; the standalone
  Kotlin Gradle plugin must not be applied.
- `compileSdk = 36`, `targetSdk = 36`, `minSdk = 30`. Features are runtime-gated
  by `Build.VERSION.SDK_INT`: `dispatchGesture` (API 24+), `takeScreenshot`
  (API 30+), `takeScreenshotOfWindow` (API 34+).

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

- `protocol/` — wire DTOs, envelope codec, method param/result types and
  validation, dispatcher, token store. Pure JVM, fully unit-testable.
- `json/` — a small dependency-free JSON model/parser/writer so the protocol
  layer needs no Android runtime in tests and the RPC server has no heavy dep.
- `rpc/` — the localhost-only HTTP/1.1 `POST /rpc` server over `ServerSocket`,
  one request per connection, body capped at 32 MiB.
- `service/` — `SkyAccessibilityService` (overlay/gesture/tree/screenshot),
  `SkyNotificationListenerService`, `DeviceMethodHandler` (wires the protocol
  handler to live services), and `RpcController` (server lifecycle + token).
- `overlay/` — pass-through cursor overlay window flags and the cursor view.
- `app/` — PackageManager-based app management.
- `screenshot/` — accessibility screenshot failure classification.
- `setup/` + `SetupActivity` — receive the ephemeral RPC token from the host
  setup intent. `MainActivity` is the operator status UI.

## Security

Never log, persist, or commit RPC tokens, notification content, accessibility
dumps, or screenshots. The overlay is non-focusable and non-touchable so taps
pass through. The RPC server binds loopback only and validates the token on
every call before method dispatch.
