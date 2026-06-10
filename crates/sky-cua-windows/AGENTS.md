# sky-cua-windows Guide

`sky-cua-windows` implements the native Windows desktop backend for the
shared Computer Use contract. Keep Windows-specific Win32/UIA/GDI/SendInput
details inside this crate and expose only `DesktopBackend` behavior upward.

## Conventions

- The public MCP/service model lives in `sky-cua-platform`; do not duplicate
  contract types here.
- Keep v1 fallback roles blunt: real window bounds are useful, fabricated
  widget semantics are not.
- Prefer semantic UI Automation actions where implemented, with `SendInput`
  as the physical fallback.
- Keep screenshot coordinates in screenshot pixels, matching Linux behavior.
