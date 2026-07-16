# Android phone companion

Load only when a build-bearing local deploy reports companion status, a
companion rebuild option is requested, or the user asks to finish companion
device setup. A Gradle-only companion build is outside `cua-deploy`.

## Build and staging

The build-bearing `scripts/deploy_plugin.py` companion lane is toolchain-gated
and change-detected. With JDK 21 and the Android SDK available, it rebuilds
only when companion sources changed since the last staged APK, then stages:

```text
resources/android/phone-companion.apk
resources/android/phone-companion.json
```

Without the toolchain it logs a note and reuses any existing staged APK.
`--force-companion` forces a rebuild; `--no-companion` skips the lane. Override
discovery with `SKY_CUA_COMPANION_JAVA_HOME` and
`SKY_CUA_COMPANION_ANDROID_SDK_ROOT`.

## Handoff

Installing the staged APK and enabling its accessibility and notification
listener services is a runtime-tool concern. When the deploy reports a bundled
companion and connected ADB devices:

1. Ask the user which device or devices to set up; never assume and never
   install on every connected device.
2. For each chosen serial, call
   `phone_connection(operation="connect", serial=...)` and retain its returned
   `session_id`.
3. Call `phone_setup(operation="install_companion", session_id=...)`.
4. Confirm with `status(component="phone_companion", session_id=...)`.

Do not use raw `adb install`: it bypasses the service-enable logic in the Rust
runtime tool path. The companion flow is documented in
`docs/features/phone-use.md` in the checkout.
