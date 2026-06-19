# Phone-use capability detection and target-device research

## Context

The target devices for `phone-use` are Bex's Galaxy S26 Ultra and Redmi Pad 15
Pro tablet. The feature should adapt to the device and OS it actually connects
to rather than hard-coding assumptions from marketing specs. Android vendors,
Android versions, accessibility settings, notification permissions, companion
app version, root/Shizuku/device-owner state, scrcpy availability, display
rotation, and wireless ADB state can all change what actions are possible.

Therefore every `phone_connect` session should detect and cache a
`PhoneCapabilityProfile`, then every observation and action response should
report what is currently available, what backend will be used, and why a
capability is unavailable.

## API Findings

ADB is the bootstrap authority. Android's official adb documentation describes
ADB as a versatile command-line tool for communicating with devices, including
installing/debugging apps and accessing a Unix shell. The adb manpage documents
`adb install`, `install-multiple`, and `install-multi-package`; `adb install -r`
replaces an existing application, and `-g` grants runtime permissions at install
where supported. This makes ADB the right mechanism for companion install/update,
version checks, wireless connect, port forwarding, fallback screenshots, and
recovery. The companion should be a single APK in v1 so `adb install -r` is
enough for auto-install/update. General app install tools must preserve
`install-multiple` and `install-multi-package` for split APKs and package sets,
and must model downgrade, test APK, and runtime permission grant flags as
explicit options rather than hiding them behind a generic install call.

AccessibilityService is the companion's main rich-control API. The official
AccessibilityService docs show `getRootInActiveWindow()` can retrieve the active
window root when the service declares `canRetrieveWindowContent`. `dispatchGesture`
was added in API 24, requires the service's gesture capability, and dispatches
touch gestures while cancelling other gestures in progress. `takeScreenshot` was
added in API 30, requires screenshot capability metadata, and returns a screenshot
for a display. `takeScreenshotOfWindow` is an API 34 path to capture a target
window without covering it with an accessibility overlay. Screenshots can still
fail on secure windows, disabled service capabilities, OEM policy, or throttling.
This supports native gestures, semantic screen trees, native cursor overlay, and
native screenshots, but the capability and failure reason must be detected per
device/API level.

NotificationListenerService is enough for notification events and many
notification actions. The docs say `onNotificationPosted` provides a
StatusBarNotification and ranking map for newly posted notifications, and
`onNotificationRemoved` reports removals. `cancelNotification(key)` dismisses a
single notification on API 21+. Notification actions and RemoteInput are exposed
through the Notification object. Opening a notification means sending its
content PendingIntent; invoking an action means sending the action PendingIntent;
inline reply requires RemoteInput results to be attached before the PendingIntent
is sent. These operations can fail if the PendingIntent is null, canceled,
expired, immutable in a way that ignores extras, redacted, or OEM-filtered.
Therefore notifications should be modeled as structured events with stable IDs
and capability-gated actions.

PackageManager supports the app-management slice. `getLaunchIntentForPackage`
returns an intent to launch the front-door activity for a package when available,
`getLaunchIntentSenderForPackage` is useful on API 33+ where an IntentSender
launch path is preferable, and `queryIntentActivities` retrieves activities that
can handle an intent. For the companion, package visibility on modern Android
means the app should declare needed queries or rely on ADB/package-manager shell
fallbacks such as `pm list packages` for complete app inventory where necessary.
The host should treat app-management capabilities as detected, not assumed.

MediaProjection is not the first screen-capture path. Android 14 tightened
MediaProjection by requiring user consent for each capture session for apps
targeting API 34+, and foreground-service requirements apply. Accessibility
screenshots plus ADB/scrcpy fallback are better for the initial companion.

## Target Devices

Samsung has official Galaxy S26 Ultra pages, and public Samsung pages describe a
6.9 inch display, high-end camera system, large battery, Galaxy AI features, and
One UI generation details. This confirms the phone is a high-end modern Samsung
device where recent Android accessibility and notification APIs are likely to be
available, but the implementation must still detect the actual API level,
permissions, and Samsung-specific behavior at runtime.

I did not find an official Xiaomi page for the exact string "Redmi Pad 15 Pro".
Official Xiaomi tablet pages currently surface products such as Redmi Pad Pro,
Redmi Pad 2 Pro, Xiaomi Pad 7 Pro, Xiaomi Pad 8, and Xiaomi Pad 8 Pro. The
closest official match is Redmi Pad 2 Pro / Redmi Pad 2 Pro 5G. Xiaomi's global
FAQs say the first batch of both Redmi Pad 2 Pro and Redmi Pad 2 Pro 5G runs
Xiaomi HyperOS 2.2 based on Android 15. Bex's tablet is expected to already be on
HyperOS 3.1, and public rollout/ROM sources describe HyperOS 3.x / 3.1 for these
tablet lines as Android 16 based. Therefore the tablet compatibility lane should
include Android 16 / API 36 as the practical target, while retaining Android 15 /
API 35 as the documented launch-software baseline. The implementation still must
identify the actual tablet through ADB properties and companion capability
reports at connection time.

Android's own SDK platform release notes label Android 16 as Android API 36 and
Android 15 as Android API 35. The companion app should compile against API 36
when available, and target API 36 if the chosen Android Gradle tooling supports
it cleanly. If the repo's available toolchain only supports API 35 initially,
that is acceptable for the first sideloaded prototype only if runtime smoke
tests pass on the HyperOS 3.1 tablet and an explicit API 36 upgrade task remains
in the plan. For personal sideloading, Google Play target requirements are not
the distribution constraint, but they are still a useful modern-target baseline.

Evidence strength for the tablet OS should be recorded explicitly. Xiaomi's own
FAQ is strong evidence for the Redmi Pad 2 Pro first-batch Android 15 baseline.
Bex's device report is the strongest evidence for the actual target tablet state
until ADB is connected. Public Xiaomi HyperOS 3 pages and rollout reporting
support the Android 16 generation for HyperOS 3.x. A third-party Xiaomi ROM
tracker lists Redmi Pad 2 Pro 5G EEA stable as HyperOS 3.1 / Android 16.0, which
is useful corroboration but must not replace live `adb shell getprop` proof.

For both target devices, the implementation should collect at session start:

- `ro.product.manufacturer`
- `ro.product.brand`
- `ro.product.model`
- `ro.product.device`
- `ro.build.version.sdk`
- `ro.build.version.release`
- display size and density
- current orientation
- whether the device is phone/tablet/foldable-like from screen metrics
- ADB connection kind
- companion installed version and signature/hash
- companion RPC reachability
- accessibility service enabled state
- notification listener enabled state
- screenshot API availability
- gesture dispatch availability
- phone-native overlay availability
- notification action/inline-reply availability
- current foreground app/package
- root/Shizuku/device-owner capability when detectable

## Recommendation

Add a required session-start capability detection phase inside `phone_connect`.
The capability profile should be cached in the service for the lifetime of the
session, invalidated on reconnect, companion update, permission-state change,
orientation/display change, RPC failure, wireless disconnect, and explicit
`phone_refresh_capabilities`.

The MCP tool list can stay static for install/plugin compatibility, but
`phone_observe`, `phone_status`, and every action response should expose dynamic
`available_actions` and `unavailable_actions` with reasons. This gives the agent
a tailored action menu without depending on dynamic MCP tool registration.

The backend routing recommendation is:

- `phone_observe`: aggregate screenshot, snapshot id, current app, capability
  profile, accessibility summary, notifications, cursor state, and available
  actions. Prefer companion semantic state when available; use scrcpy for
  low-latency visual frames when active/requested; use ADB fallback.
- screenshots: companion screenshot when available, scrcpy when explicitly
  requested or active and mapped, ADB fallback.
- coordinate actions: translate from snapshot to device coordinates, then use
  companion gestures when available, scrcpy when the snapshot came from scrcpy
  and host mapping is current, ADB fallback.
- text input: companion input/IME path when available, scrcpy keyboard when
  active, ADB text fallback.
- Android keys: companion when available, ADB keyevent fallback, scrcpy only
  where clearly better.
- notifications: events are observable; open, dismiss, invoke action, and inline
  reply are supported only by explicit notification/action IDs and only when the
  capability profile says the action is available.
- app management: v1 includes foreground app, list launchable apps, launch by
  package, launch activity/deep link/intent URI, force-stop, install/update APK,
  and open setup/settings screens. Clear app data/cache is deferred.

The first implementation should be capability-aware and operator-privileged, but
not privilege-dependent. Detect root, Shizuku, and device-owner/profile-owner
state in v1; use them only for low-risk setup automation and diagnostics until
the baseline is proven.
