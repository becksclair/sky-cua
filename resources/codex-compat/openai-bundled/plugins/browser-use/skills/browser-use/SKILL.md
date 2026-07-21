---
name: browser-use
description: "Control page content in the user's external Chrome or Chromium browser through sky-cua's extension native host and node_repl. Do not use for an in-app browser, browser chrome, OS dialogs, or phone UI."
---

# Browser Use

Use the `@heliasar/browser-use` package from `node_repl` for page content in
the user's external Chrome-family browser. The runtime attaches through the
installed sky-cua Chrome/Chromium extension and its native messaging host.

- List browsers first and require `transport="extension_native_host"`.
- Treat an `iab:` id, `host_provided_iab`, or any in-app-browser result as unavailable.
- Reuse an appropriate existing tab before opening a new tab.
- Use Computer Use for browser chrome, extension pages, permission prompts,
  native file pickers, and other OS UI.
- Browser coordinates are CSS pixels and must never be reused as desktop coordinates.
- Re-observe after navigation, scrolling, resizing, or other visible transitions.
- Verify consequential actions from a fresh browser observation.
