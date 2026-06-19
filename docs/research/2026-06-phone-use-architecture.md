# Phone-use architecture research

## Context

The question is how to design `phone-use` so an agent can control a real Android phone through sky-cua, including wireless use. The first sketch used scrcpy as the obvious bridge: mirror the phone into a desktop window, then let sky-cua control that window. That remains useful, but it is not sufficient as the architecture boundary because wireless pairing, device discovery, reconnect behavior, and fallback screenshots are ADB concerns rather than window concerns.

## Investigation

sky-cua currently ships one host-facing MCP server named `computer-use` from `./bin/sky-cua-client mcp`. Browser tools are already a tool family inside that same server rather than a separate MCP process. The compat plugin contract also says wrappers should keep the MCP server name `computer-use`; Codex Desktop enables a single Computer Use plugin identity and points it at the sky-cua payload. That makes a separate enabled `phone-use` MCP server a poor first fit for this repository.

Android Debug Bridge is the connection authority. Google's adb documentation describes adb as the host client/server plus device daemon used to communicate with devices, list attached devices, run shell commands, install/debug apps, and select a target by serial. It also documents Android 11+ wireless debugging with one-time pairing by QR or pairing code, same-network requirements, `adb pair ipaddr:port`, post-pairing wireless use similar to USB, `adb devices -l`, `ANDROID_SERIAL`, and mDNS diagnostics through `adb mdns track-services --proto-text`. For Android 10 and lower, the documented wireless path still starts with USB, runs `adb tcpip 5555`, disconnects USB, then uses `adb connect device_ip:5555`.

scrcpy is the strongest local mirroring and control implementation. The official Genymobile README describes scrcpy 4.0 as mirroring Android video/audio over USB or TCP/IP and allowing keyboard/mouse control without root or a phone-side app. Its connection docs mirror adb's selection model: serial, USB, TCP/IP, `ANDROID_SERIAL`, and `--tcpip`, including automatic TCP/IP setup when USB is attached or direct connection to a known TCP/IP listener. Its video docs show why scrcpy is better than polling screenshots: the device encodes a real video stream, can limit size, frame rate, codec, orientation, crop, and buffering, and can send video to V4L2 on Linux. The developer docs make the downside clear: the scrcpy client/server protocol is internal and may change between versions, with exact client/server version matching required.

scrcpy window control is the lowest-risk acceleration path because it uses supported scrcpy CLI behavior and sky-cua's existing desktop screenshot/input stack. It is not the cleanest long-term agent surface because it depends on host window bounds, decorations, capture geometry, compositor behavior, and letterboxing. It is also not headless. Linux V4L2 output can avoid a playback window, but it requires `v4l2loopback` and is Linux-specific. Directly consuming the raw scrcpy protocol would give the cleanest fast screenshots and input, but it binds sky-cua to an explicitly internal, version-changing protocol.

ADB-only control is slower but architecturally important. It can list devices, pair/connect wirelessly, launch shell commands, capture screenshots through device shell commands, inject keyevents/taps/swipes/text, inspect installed packages, and recover from missing scrcpy. It should be the baseline backend because it works over USB and wireless, does not require a host window, and gives useful diagnostics when scrcpy is unavailable. It is not enough alone for a premium CUA experience because repeated screenshots are slower than a stream, text/clipboard behavior is weaker, and UI semantics are limited unless we add uiautomator dumps or an accessibility/appium layer.

Existing mobile MCP projects reinforce the two-layer shape. `JuanCF/scrcpy-mcp` presents Android control through ADB plus scrcpy, advertises a scrcpy-first fast path and ADB fallback, and includes screen, input, app, UI, shell, file, and clipboard tools. Appium MCP and WebdriverIO MCP focus on mobile app automation through Appium drivers, element finding, and test workflows across Android/iOS. Those are valuable references for a later semantic layer, but they are heavier than necessary for general "control my phone" use and bring a larger Java/Appium/toolchain footprint.

## Conclusion

Build `phone-use` inside this repository and expose it from the same `sky-cua-client mcp` process as a new `phone_*` tool family plus `skills/phone-use/SKILL.md`. Do not start with a new project or a second enabled MCP server. sky-cua already owns the agent-facing CUA runtime, plugin packaging, installed-surface proof, screenshot delivery conventions, and tool annotations. A separate project would duplicate the hard parts and make Codex Desktop compatibility worse.

Internally, make the phone runtime separable. Add a phone model under `sky-cua-platform`, a service-owned `PhoneManager`, and a distinct module or crate boundary such as `crates/sky-cua-service/src/phone/` now and possibly `crates/sky-cua-phone/` once it grows. The boundary should make it possible to ship a standalone `phone-use` MCP later, but that should not be the first delivery shape.

Use two backends from the beginning:

1. ADB baseline backend: required for discovery, wireless pairing, connect/reconnect, device status, shell diagnostics, fallback screenshot, fallback input, app/package helpers, and live-smoke setup.
2. scrcpy acceleration backend: optional but preferred for low-latency visual control. Start with supported scrcpy CLI/window behavior and optionally V4L2 on Linux; defer direct scrcpy protocol consumption until there is a compelling performance reason and a strict version-gating plan.

Wireless should be a first-class workflow:

- `phone_pair_wireless` or `phone_wireless_pair` accepts the Android 11+ pairing host:port and code, runs `adb pair`, and reports paired state.
- `phone_connect` accepts a serial, `host:port`, or discovered mDNS target and can connect over USB, Android 11+ wireless debugging, or legacy `adb tcpip` after USB bootstrap.
- `phone_status` reports adb version, server status, mDNS readiness, paired/discovered devices, selected serial, and active scrcpy sessions.
- `phone_list_devices` distinguishes USB, emulator, legacy TCP/IP, and wireless debugging devices instead of treating every serial string as equal.
- `phone_disconnect` should disconnect wireless ADB and/or stop managed scrcpy according to explicit ownership flags.

The agent-facing tool contract should stay small at first: status, list devices, wireless pair/connect, connect, disconnect, screenshot, tap, swipe, type text, press key, launch app. Add broader shell/file/clipboard tools later only with separate annotations and explicit risk boundaries.

## Implications

The current ExecPlan should be changed from "USB scrcpy first" to "ADB wireless-aware baseline plus scrcpy acceleration." USB is still the easiest bootstrap path and should be supported, but wireless pairing and reconnect are core requirements, not a later enhancement.

The first live proof should have two lanes: an ADB-only wireless smoke that can run without a scrcpy window, and a scrcpy-accelerated smoke that proves low-latency screenshots/actions when scrcpy is available. Installed-MCP proof remains required because sky-cua's packaging and compat layer are part of the product.
