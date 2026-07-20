# Phone JavaScript

Import `{ phone, createPhoneClient }` from `@heliasar/sky-cua/phone`. The lazy `phone` facade and explicit client use the normal sky-cua service socket. Call `list_devices()`, connect or bind an existing session selector, then retain the returned `PhoneDeviceSession` across `js` calls.

Phone operations cover discovery and connection, screenshots, touch/gesture/text/key input, apps, notifications, and companion-backed actions when advertised. Screenshot results expose bytes/data URL/local-file helpers suitable for `nodeRepl.emitImage`.

Every operation forwards current `nodeRepl.requestMeta` provenance. A bound session becomes invalid after explicit or observed disconnect and never silently reconnects. Structured service errors are preserved. Mutation calls are never retried after an ambiguous write.

`PhoneClient.close()` permanently closes the client transport. Read `client.disconnected` to inspect that terminal state. A bound `PhoneDeviceSession` also exposes `.disconnected`; it becomes true after `session.disconnect()`, an observed no-session response, or client closure. Create a new client explicitly when a later workflow needs a fresh lifecycle.
