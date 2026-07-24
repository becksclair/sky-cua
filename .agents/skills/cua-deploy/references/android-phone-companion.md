# Android phone companion

Load only when checking companion contents in a standalone package or when the
user asks to finish companion device setup. A Gradle-only companion build is
outside `cua-deploy`.

The canonical distribution build packages the staged companion artifacts:

```text
resources/android/phone-companion.apk
resources/android/phone-companion.json
```

Do not add companion flags to `install.py`; it exposes only `build` and
`install`. If the staged APK itself must be regenerated, follow the Android
project's build instructions as a separate prerequisite, then run the canonical
standalone build or install.

Installing the staged APK and enabling its accessibility and notification
listener services is a runtime-tool concern. When connected ADB devices exist:

1. Ask which device or devices to set up; never install on every connected
   device implicitly.
2. For each chosen serial, call
   `phone_connection(operation="connect", serial=...)` and retain its returned
   `session_id`.
3. Call `phone_setup(operation="install_companion", session_id=...)`.
4. Confirm with `status(component="phone_companion", session_id=...)`.

Do not use raw `adb install`; it bypasses the service-enable logic in the Rust
runtime tool path. See `docs/features/phone-use.md` in the checkout.
