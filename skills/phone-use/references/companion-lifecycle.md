# Companion lifecycle and setup

Read this reference only when companion status, installation, permissions,
bootstrap, or setup needs diagnosis.

- Use companion status to inspect installed version, signature match, permission grants, RPC reachability, and bootstrap diagnostics.
- Installing or reinstalling the companion also attempts to enable its accessibility service and notification listener over ADB without clobbering existing device services.
- Treat a manual-setup diagnostic as proof that Android gated the grant and finish the grant on the device.
- Use setup/open-settings for accessibility, notification access, overlay permission, app details, wireless debugging, or battery optimization when the action menu or diagnostics reports that permission disabled.
- Treat the companion as mandatory for visible coordinate control, the accessibility tree, notifications, and coordinate gestures.
- Treat missing companion access for those capabilities as `backend=none` with `PhoneCompanionRequired` and no produced result.
- Treat ADB as the path for text/key input, app management, setup, and screenshot fallback.
