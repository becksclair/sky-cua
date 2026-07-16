# Cursor planes

Read this reference only when interpreting cursor state, scrcpy mirroring, or
phone overlay behavior.

- Treat the screenshot-synthetic cursor as composited into returned screenshots after a successful action.
- Allow the screenshot-synthetic cursor on ADB-only sessions.
- Do not double-composite it when the captured frame already includes the native overlay.
- Treat the host-visible overlay as the phone-native cursor mirrored into a host-mapped scrcpy window.
- Do not treat the host as drawing the host-visible overlay.
- Treat the host-visible overlay as true only when a live mirror is host-mapped and companion RPC can draw it.
- Treat the phone-native overlay as drawn on the device by the companion accessibility service.
- Treat its full-screen overlay as non-focusable and non-touchable.
- Treat its active edge glow as visual activity feedback rather than session validity.
