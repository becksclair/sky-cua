# App Instructions Guide

`resources/app-instructions/` contains app-specific guidance and
machine-readable action policy for the Linux backend and MCP client.
Markdown guides are packaged into the plugin and may appear in
`get_app_state.app_guidance`.

## Conventions

- Register every guidance file in `index.json`; it drives both markdown
  lookup and backend action policy (`crates/sky-cua-linux/src/app_policy.rs`),
  so validate both readers after changes — and build the plugin so packaging
  failures show up early.
- Use desktop-file IDs as canonical `key` values where possible; keep
  aliases lowercase and practical (`kate`, `kwrite`, `dolphin`, `firefox`).
- Only add machine policy fields when backend code reads them; do not invent
  per-app policies in markdown if code needs a structured knob.
- Keep markdown short and behavioral. `Kate.md`/`KWrite.md` are the editor
  pattern (`set_value_fallback`); `Firefox.md` is the cautious web-content
  pattern.
- Keep fallback descriptions honest: guidance teaches strategy, it does not
  pretend an app has better accessibility than it does. Never claim semantic
  write support without a live/backend proof.

## Checks

```bash
cargo test -p sky-cua-platform app_instructions
cargo test -p sky-cua-linux app_policy
python3 scripts/build_plugin.py
```
