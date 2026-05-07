# sky-cua-windows Guide

## Package Identity

`sky-cua-windows` implements the native Windows desktop backend for the shared
Computer Use contract. It should keep Windows-specific Win32/UIA/GDI/SendInput
details inside this crate and expose only `DesktopBackend` behavior upward.

## Setup & Run

```bash
cargo check -p sky-cua-windows
cargo test -p sky-cua-windows
```

## Patterns & Conventions

- Keep the public MCP/service model in `sky-cua-platform`; do not duplicate
  contract types here.
- Keep v1 fallback roles blunt. Real window bounds are useful; fabricated
  widget semantics are not.
- Prefer semantic Windows UI Automation actions where implemented, with
  `SendInput` as the physical fallback.
- Keep screenshot coordinates in screenshot pixels, matching Linux behavior.
