# Foreign-extension attach recovery

Read this reference only when browser attach or claim returns the exact
diagnostic `Cannot access a chrome-extension:// URL of different extension`.

- A password-manager overlay on a login page can block attach because Chrome
  refuses debugger access while the tab hosts another extension's frame.
  Bitwarden's inline autofill menu on a credential form is the common case.
- Claim and attach the tab before navigating it into a login flow. An attached
  session survives the overlay appearing.
- If attach is refused on a page already showing a login form, dismiss the
  overlay with `Escape` or click a neutral spot via desktop input.
- Retry the browser action once.
- If the refusal persists, drive that step with desktop input.
