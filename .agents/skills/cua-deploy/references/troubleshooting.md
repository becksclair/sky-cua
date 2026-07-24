# Deployment troubleshooting

Load only after the selected lane fails or produces an unexpected result.

- **Build cannot find `packages/browser-use/build`:** run the canonical command
  from the real checkout. Do not prebuild Browser Use manually or build from a
  temporary checkout; `install.py build` owns and reuses durable outputs.
- **Built binary uses unsupported CPU instructions:** inspect machine-wide
  Cargo configuration. Rebuild with a suitable explicit `RUSTFLAGS` portable
  target override; do not turn it into an installer selector.
- **Install asks for a manifest hash, release ID, verification, activation, or
  rollback command:** obsolete instructions or an obsolete artifact are in
  use. Refresh from current `main` and use only `python3 install.py build` or
  `python3 install.py install`.
- **Installed paths contain `releases/` or `current`:** the target still has the
  retired generation layout. A current install should replace the fixed
  `${XDG_DATA_HOME:-~/.local/share}/sky-cua` tree directly.
- **OpenClaw cannot find managed plugins:** verify the fixed marketplace path,
  then inspect OpenClaw's pre-thread reconciliation. Do not add marketplace
  selectors, standalone `sky_cua` MCP injection, or a stock fallback.
- **Computer Use selects SSH forwarding instead of Xpra:** inspect live Xpra
  process environment, X11 socket, D-Bus address, and Xauthority. The client
  should self-discover and repair the detached launch environment.
- **Browser Use is disconnected:** verify the current extension is loaded from
  the fixed root, its native messaging manifest points at the stable launcher,
  and the browser reports `extension_native_host`; do not label it as IAB.
- **AT-SPI is wedged:** load `rare-operations.md`; refresh accessibility only
  when proven necessary, then relaunch affected GTK applications.
- **A prerequisite fails:** stop dependent install, live acceptance, commit, or
  push steps and report the first failed command.
