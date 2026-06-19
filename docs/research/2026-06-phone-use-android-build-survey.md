# Phone-use Phase 0 build and host-tooling survey

This is the Phase 0 proof-of-feasibility survey required by
`plans/phone-use.md`. It records the real repo-local Android build
conventions, host tool availability, the chosen companion app location, and
the connected target device, as observed on 2026-06-17.

## Repo state

- No pre-existing Android/Gradle/Kotlin project exists in the workspace
  (`rg --files` for `^android/`, `gradle`, `*.kt`, `AndroidManifest` returned
  nothing). The companion app is therefore greenfield.
- The Rust workspace already exposes a `browser` tool family mirrored at every
  layer (`crates/sky-cua-platform/src/model/browser.rs`,
  `crates/sky-cua-service/src/browser/`,
  `crates/sky-cua-client/src/mcp_tools/browser*`). `phone-use` mirrors that
  shape; it is not a new MCP server.

## Host executables

- `adb`: Android Debug Bridge version 1.0.41 (`/usr/sbin/adb`). Present and
  required-capable.
- `scrcpy`: 4.0 (`/usr/sbin/scrcpy`). Present, matches the version the plan's
  research validated.
- `cargo` / `rustc`: 1.95.0, workspace edition 2024.

## Android toolchain

The Android build is feasible in this environment without new system installs:

- JDKs available via mise and system: **21.0.2** (mise
  `~/.local/share/mise/installs/java/21.0.2`, system
  `/usr/lib/jvm/java-21-openjdk`), 25.0.2, and 26.0.1 (the default `java`).
  The Android Gradle Plugin does not support JDK 25/26; the companion build
  must pin **`JAVA_HOME` to JDK 21** even though the interactive default is 26.
- Android SDK at `~/Android/Sdk`:
  - build-tools: 35.0.0, 36.0.0, 36.1.0, 37.0.0 (37.0.0-rc2)
  - platforms: android-34, android-35, android-36, android-36.1, android-37.0
  - NDK: 27.2, 29.0, 30.0; CMake present
- `~/.gradle` exists and is populated (caches, daemon, wrapper, native,
  `gradle.properties`), so a project Gradle wrapper can resolve a distribution.
  There is no `gradle` on `PATH`; the companion project must ship its own
  `./gradlew` wrapper.

Conclusion: the companion app can compile and target **API 36 / Android 16**
directly. `compileSdk = 36`, `targetSdk = 36`, with `minSdk` chosen to cover
the gesture/screenshot/overlay APIs the plan relies on (gesture dispatch API
24+, accessibility screenshot API 30+, `takeScreenshotOfWindow` API 34+).

## Chosen companion app location and build command

- Location: `android/phone-companion/` (Decision Log already anticipated this
  path; nothing in the repo conflicts with it).
- Package id: `com.skycua.phonecompanion` (per plan; no repo-local naming
  convention requires otherwise).
- Build command (single-APK debug artifact for the auto-install path):

      JAVA_HOME=/usr/lib/jvm/java-21-openjdk \
        ANDROID_SDK_ROOT=$HOME/Android/Sdk \
        android/phone-companion/gradlew -p android/phone-companion assembleDebug

- The build must emit build metadata next to the APK (package id, versionCode,
  versionName, APK relative path, APK SHA-256, signing certificate SHA-256) for
  the host `phone_connect` signature/version checks.

## Connected target device (live-proof capability)

A real device is attached over **wireless ADB**, so Phase 2/3 and (subject to
permission enablement) companion/scrcpy live proof can run in this environment:

- serial `172.16.255.58:38781` (wireless), `transport_id:10`
- `ro.product.model = SM-S948B` (Samsung Galaxy)
- `ro.build.version.release = 16`, `ro.build.version.sdk = 36`
- product `m3qxeea`, device codename `m3q`

This satisfies a real Android 16 / API 36 device for capability-profile and
live-smoke proof. The Redmi Pad tablet target is not attached; per the plan's
acceptance rule, the Redmi/HyperOS-3.1 lane remains blocked until that tablet
is connected and confirmed via `getprop`.

## Security note

Device serial, IP, and model are recorded here as non-sensitive host
diagnostics. No pairing codes, RPC tokens, screenshots, notification content,
or accessibility dumps are stored in this survey or any committed artifact.
