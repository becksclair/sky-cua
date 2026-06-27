# Codex CUA tool-use gate and performance judge

## Status

Shipped (code + harness). Last verified: local Python gate (ruff/basedpyright/
pytest) on the consolidation change set; live VM run is the standing acceptance
gate (KDE Wayland `testing-vm`).

## Summary

One `codex exec` run exercises the full sky-cua computer-use **and** browser-use
tool surface against live fixtures, a deterministic gate proves from the
transcript that the right tools were called with no errors, and a host-side
gpt-5.5 judge scores the agent's tool-use quality and emits an actionable triage
list. This replaced four redundant agent dialog-dismiss runs and the scattered
single-purpose codex smokes.

## Contract surface

- Profile `codex-cua` in `scripts/run_gui_testing_vm_smoke.py` (dispatch
  `codex-cua-judge`). `--profile codex-cua` runs the VM smoke and the host judge;
  `--profile all` runs only the deterministic VM gate (no host auth in the VM).
- Deterministic gate result: `coverage-summary.json` next to the codex transcript
  (`tools_seen`, `operations_seen`, `surfaces_seen`, `missing_*`, `errors`,
  `ground_truth`, `ok`). Stable pointer at
  `artifacts/codex-e2e/codex-cua/latest.json`.
- Judge verdict schema: `scripts/schemas/agent_perf_judge_verdict.json`
  (`score` 0-100, `subscores` four 0-25 dimensions, `pass`, `summary`, `triage`).
  CLI always writes `judge-verdict.json` and `judge-triage.json`.
- Smoke result schema: `scripts/schemas/cua_full_smoke_result.json`.
- Tunables: `SKY_CUA_JUDGE_THRESHOLD` / `--threshold` (default 70);
  `SKY_CUA_SMOKE_OPENCODE_MODEL` / `SKY_CUA_SMOKE_PI_MODEL` for the wiring-check
  model (default `opencode/deepseek-v4-flash-free`); `--model` / `--reasoning-effort`
  on the smoke and judge.

## Behavior

1. The `codex-cua` VM profile builds the native-host binary, then
   `scripts/live_codex_cua_smoke.py` launches the GTK pointer fixture (now with a
   check button and combo box for the semantic tools) and a loopback HTML page,
   registers the native-messaging host manifest, and opens Chrome at
   `chrome://extensions` **without** preloading the extension. The **agent installs
   the Codex extension itself** with computer-use (Developer mode → Load unpacked →
   the folder chooser) — that is what unlocks the browser tools, so a failed
   install shows up as a browser-coverage miss the judge can explain. The harness
   records `extension_loaded_by_agent` / `native_host_socket_up` in the summary.
   (Chrome 137+ disables the `--load-extension` switch, so the UI path is also the
   durable one.)
2. One `codex exec` run (default gpt-5.5/low) drives every required tool against
   the **production `computer-use@openai-bundled` compat plugin**. The runner
   stages the openai-bundled marketplace into the VM
   (`--openai-bundled-resource-root`, default the host's
   `codex-desktop-linux/codex-app/resources`); `prepare_chatgpt_plugin_test_home`
   then auto-enables the compat id. The compat plugin points at the *same*
   `sky-cua-client` server with the *same* `computer-use`/`browser-use`/`phone-use`
   skills, so the tool surface (`observe`, `capture_desktop`, `desktop_*`,
   `browser_*`) is identical to the dev `sky-cua@local` fallback used when the
   marketplace is absent. The smoke records the resolved surface
   (`plugin_surface`) in `coverage-summary.json`.
3. The deterministic gate (`scripts/_cua_coverage.py`) parses
   `transcript_mcp_tool_calls`, merges started/completed items, and fails on any
   missing required tool/operation/surface, any tool error, or any fixture
   ground-truth mismatch. It always writes `coverage-summary.json`.
4. The runner pulls the transcript + summary + last-message to the host and runs
   `scripts/live_agent_perf_judge.py`. The judge condenses the transcript
   (image/result payloads stripped, head/tail elision, char budget), runs in an
   isolated tool-free codex home with host gpt-5.5 auth, scores against the rubric
   with the coverage matrix as ground truth, hard-fails below the threshold, and
   always emits triage. It runs even when the deterministic gate failed.
5. Overall success = deterministic gate passed AND judge passed.

## Source paths

- `scripts/live_codex_cua_smoke.py` — the single-run smoke + deterministic gate orchestration.
- `scripts/_cua_coverage.py` — required tool/operation/surface sets + coverage/no-error analysis.
- `scripts/_chrome_bridge.py` — Chrome launch, extension load, native-host manifest, socket wait, HTML fixture server (shared with `live_chrome_host_client_smoke.py`).
- `scripts/_agent_perf_judge.py`, `scripts/live_agent_perf_judge.py` — host judge.
- `scripts/schemas/cua_full_smoke_result.json`, `scripts/schemas/agent_perf_judge_verdict.json`.
- `scripts/testing-vm/profiles/codex-cua.sh`, `run-profile.sh`, `all.sh` — VM dispatch.
- `scripts/run_gui_testing_vm_smoke.py` — `codex-cua` descriptor + `run_codex_cua_judge_profile`.
- `scripts/gtk_pointer_smoke_fixture.py` — fixture (check button + combo box added).
- `scripts/live_agent_mcp_smoke.py` — `--mode wiring` for the opencode/pi checks.

## Verification

- Host (no VM): `uv run ruff format --check scripts && uv run ruff check scripts &&
  uv run basedpyright && uv run pytest`. Unit tests cover `_cua_coverage`
  (coverage/no-error/merge) and `_agent_perf_judge` (condenser + threshold) in
  `scripts/test_live_smoke_helpers.py`.
- Live (VM, KDE Wayland): re-sync host codex auth first (see the
  `testing-vm-codex-auth` memory), then `--profile codex-cua --sync-codex-settings`.
  Confirm Chrome/extension/native-host comes up, `coverage-summary.json.ok` is
  true, and `judge-verdict.json` scores >= threshold.

## Known limitations

- The browser half depends on a live Chrome + extension + native-host socket in
  the VM; that bring-up is the most fragile dependency (the profile fails loudly
  if the socket never appears).
- Exact operation/tool spellings and the free model id (`opencode/deepseek-v4-flash-free`)
  are validated on the first live VM run; the required set is tuned honestly if a
  fixture cannot prove a tool.
- The judge is an LLM and is non-deterministic near the threshold; it is layered
  on top of the deterministic gate, not a replacement for it.

## Related

- `docs/operations/gui-desktop-test-harness.md` — operator runbook.
- ROADMAP "Diagnostics and operator UX".
