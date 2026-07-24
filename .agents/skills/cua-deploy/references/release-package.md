# Standalone package details

Load this reference only when building or installing the self-contained archive
for a clean target.

## Build

Run from the checkout:

```bash
python3 install.py build
```

The command owns Browser Use, model-documentation, Node runtime, Rust runtime,
plugin, skill, and archive construction. It reuses durable outputs beneath
`target/`, `out/`, and `dist/`; do not prebuild Browser Use or copy the checkout
to a temporary directory.

The single output archive is:

```text
dist/sky-cua-linux-x64-glibc.tar.gz
```

Inspect that it contains one top-level `sky-cua-linux-x64-glibc/` tree with
`install.py`, `RELEASE.json`, stable `bin/` programs, exactly one Chrome/Codex
extension, Browser client/native host, both Codex plugins and marketplace,
three skills, documentation, and runtime assets. It must not contain
`releases/`, `current`, activation receipts, or promotion journals.

## Target install

```bash
tar xzf sky-cua-linux-x64-glibc.tar.gz
cd sky-cua-linux-x64-glibc
python3 install.py install
```

The artifact validates itself and directly replaces
`${XDG_DATA_HOME:-~/.local/share}/sky-cua`, then projects stable launchers,
skills, native messaging manifests, detected host integrations, and the global
OpenClaw `node_repl` definition. There is no release ID, manifest hash,
`verify`, `ensure`, `verify-activation`, staging, backup, or rollback command.

OpenClaw separately reconciles `computer-use@openai-bundled` and
`browser-use@openai-bundled` into each agent Codex home from:

```text
~/.local/share/sky-cua/codex/openai-bundled/.agents/plugins/marketplace.json
```

For exact command selection, load `command-and-flag-catalog.md`.
