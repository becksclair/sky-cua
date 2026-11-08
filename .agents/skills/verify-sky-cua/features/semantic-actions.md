# Semantic actions

Semantic actions let the agent change application state through the
accessibility tree instead of pixels or keystrokes. The agent observes a
window, gets an AppShot with elements and their advertised `semantic_actions`,
writes a value into an editable element, and activates a control. A fresh
observation then reads the new state back.

## Sub-features

- `sem-observe` returns `appshot_id`, `action_snapshot.snapshot_id`, a window
  image, and `semantic_projection.elements[]` with role, name, value, and
  actions.
- `sem-set-value` writes text into an element advertising `set_value`
  (`desktop_set_value`).
- `sem-activate` activates a button advertising `activate` (`desktop_action`
  operation `activate`).
- `sem-readback` shows the written value in a fresh observation's element
  `value`.
- `sem-focus` and `sem-select` (`desktop_semantic` focus, select, expand,
  collapse) are not yet scripted.
- `sem-perform-action` (`desktop_action` operation `perform_action` with
  `action_index` or `action_name`) is not yet scripted.

## How to get to it (user POV)

- Call MCP `observe {surface: "desktop", window_title | app_id | pid | window_id}`.
- Call MCP `desktop_set_value {appshot_id, snapshot_id, element_index, value}`,
  or target an element with `element_identifier` (its `backend_ref`).
- Call MCP `desktop_action {operation: "activate", appshot_id, snapshot_id, element_index}`.
- Call MCP `desktop_semantic {operation: "focus"|"select"|"expand"|"collapse", ...}`.
- Operator CLI: `sky-cua-client get-app-state --window-title <t> --detail full`
  (read-only).

## Driving it with verify.sh

Preconditions:

- Baseline from `README.md`. `verify.sh doctor` prints `verdict=drivable`.

- **Open the target.** Run `verify.sh drive --journey semantic`. It launches a
  zenity entry dialog titled `sky-cua-verify-semantic-<hex>` (see
  `isolated-desktop.md`).
- **Observe.** `observe {surface: "desktop", window_title: <marker>}`. The check
  `observe returned an appshot_id, snapshot_id, and the entry element` passes.
  The entry is the element whose role contains `text` and whose
  `semantic_actions` include `set_value` (`text box`, value `''`).
- **Set value.** `desktop_set_value {appshot_id, snapshot_id, element_index: <entry>, value: "semantic-<hex>"}`.
  The check `desktop_set_value succeeded` passes.
- **Read back.** A fresh `observe` on the same marker. The check
  `fresh structured readback: entry value == token and != initial` passes with
  `{"initial": "", "final": "semantic-<hex>"}`.
- **Activate OK.** Find `role == "button"` and `name == "OK"` in the fresh
  observation, then call `desktop_action {operation: "activate", appshot_id, snapshot_id, element_index: <OK>}`.
  The check `desktop_action activate OK succeeded` passes.
- **Side effect.** `semantic-zenity-output.txt` equals the token,
  `semantic-zenity-exit.txt` is `0`, and the marker is gone from
  `list_resources` windows (`dialog window is gone afterwards`).
- **Proof.** `images/06-semantic-observe-<snapshot>.png` shows the token in the
  entry. `steps.jsonl` holds the before observe, the set_value and activate
  responses, and the after observe.

## Gotchas

- Buttons in GTK4 zenity report `bounds: null`. Target them by role and name
  with `element_index` and `snapshot_id`, not coordinates.
- `element_index` is only valid with the `snapshot_id` of the observation that
  produced it. Re-observe after any change before targeting again.
- Action responses carry `AgentCursorHostUnavailable` diagnostics on this lane.
  That is expected and harmless.
- `success: true` is not proof. Require the readback and the zenity output and
  exit code.
