---
name: computer-use-workflows
description: Use when an agent should operate desktop applications through the sky-cua MCP computer-use runtime. This skill teaches a hybrid workflow that combines control-tree reads, screenshots or other visual confirmation, and physical desktop actions across Linux, macOS, and Windows. Prefer it for app automation, graphical workflows, drag and drop, sliders, canvases, right-click flows, double-click flows, and any UI where semantic APIs are incomplete or misleading.
---

# Computer Use Workflows

Use this skill when the installed sky-cua computer-use runtime is the right tool for the job.

## Core stance

Treat the accessibility tree as structure, not gospel.

- Use `list_apps` and `get_app_state` to discover the focused app, candidate controls, diagnostics, semantic actions, and the latest screenshot path.
- If `get_app_state` gives you a `screenshot_path`, inspect it with `view_image` before you commit to a target in a murky UI.
- Start from the screenshot. The tree is optional structure, not a prerequisite for action.
- Use full `get_app_state` for discovery and debugging. Once the target app/window is known, prefer `get_app_state` with `detail: "compact"` for repeated screenshot-first action loops so old verbose state does not bloat later model steps.
- If the tree is thin, missing, or fallback-only, use the screenshot to decide where to click, drag, scroll, or type.
- If the tree and the screenshot disagree, trust what is actually on screen.
- Treat action-tool success as transport success, not UI truth. Re-check state after meaningful actions.
- In fallback-only or visually murky apps, prefer a tight loop: one action, fresh `get_app_state`, inspect the new screenshot, then decide the next action.
- When the next move is unclear, use visible text controls and context menus as discovery tools instead of freezing. Search bars, inline rename fields, modal text boxes, kebab menus, and right-click affordances are often the fastest honest path through a murky UI.
- Use text fields deliberately, not superstitiously. Never type placeholder junk like `test` just to see what happens; only enter a value that directly serves the workflow.
- Before typing into a visible field, inspect the current screenshot and decide whether the field is empty, selected, or already contains text. If stale or unrelated text is present, clear or select it before typing; do not append blindly.
- If the required object is confirmed missing or deleted, create or recreate it directly instead of wandering through unrelated search results first.
- When you know the target app or window inside a full desktop screenshot, visually focus on that window while keeping all coordinate actions in the current screenshot's pixel coordinate space.
- For `click`/`drag` x/y coordinates, use the pixels of the screenshot returned by the current snapshot. The runtime maps those screenshot pixels back to the desktop or stream before moving the pointer.

## Interaction ladder

1. **Take a fresh screenshot-backed state snapshot**
   - call `get_app_state` first so you have the current screenshot path, diagnostics, and any visible element anchors
   - after the first full orientation pass, use `detail: "compact"` unless you specifically need verbose element descriptions, environment details, or app guidance
   - re-run it after meaningful actions instead of steering from stale geometry

2. **Use the tree to understand the app when it exists**
   - identify the focused app and window
   - find candidate controls, text fields, lists, and regions
   - harvest labels, roles, values, and element bounds

3. **Use screenshots or visual confirmation to verify the target**
   - inspect the current `screenshot_path` with `view_image` when precise on-screen targeting matters
   - confirm that the intended control is visibly present
   - before text entry, confirm the focused or target field and whether it already contains text
   - confirm dialog state, tab selection, and transient UI that may lag behind the tree
   - verify the outcome after actions that are visually obvious but semantically thin

4. **Choose the action lane that fits the widget**
    - when an element advertises a matching `semantic_actions` entry, prefer the narrow semantic primitive: `focus_element`, `activate_element`, `select_element`, `expand_element`, `collapse_element`, `toggle_element`, or `set_value`
    - use `activate_element` for default app-chrome actions such as toolbar buttons, menu buttons, and dialog buttons; use `select_element` for tabs, list items, radio options, and selectable rows
    - use `click` when the desired action is inherently pointer-shaped, when the tree only gives a rough visual anchor, or when no narrower semantic primitive is exposed
    - prefer physical actions for sliders, drag and drop, canvases, splitters, media rows, custom-painted widgets, right-click menus, double-click actions, and “obvious on screen, murky in the tree” affordances
    - when the tree exposes only fallback regions, use them as rough anchors for visual search instead of pretending they are real widgets
    - if you can identify the target on the screenshot but not in the tree, use coordinate-based physical actions rather than giving up
    - if you are unsure how to trigger an action, check for visible search fields, title/name inputs, “new” affordances, overflow buttons, or row-level context menus before assuming the UI is blocked

## Text entry policy

For text replacement, the safest default is usually:

1. get a fresh screenshot-backed state snapshot
2. inspect the screenshot and identify the exact field or editor
3. decide whether the field is empty, already selected, or contains stale text
4. click to focus the field or editor
5. select existing contents with `Cmd + A / Ctrl + A` or clear with `Delete / Backspace` when replacement is intended
6. type the new text
7. re-check state before assuming the edit landed

Use semantic `set_value` only when the target exposes a proven semantic write path. Otherwise, keyboard-driven replacement is usually the grown-up choice.

If you are not sure how an app wants something to be created, renamed, filtered, or found:

1. look for visible text-entry controls first
2. inspect the screenshot for the field's current text, placeholder, cursor, and selection state
3. click to focus the field the screenshot actually shows
4. clear stale or unrelated contents before typing when replacement is intended
5. type the intended name or search term
6. reacquire state before deciding the UI ignored you

Do not use global search just because it is easy to hit. If the current page already shows visible row-level add affordances or the target playlist row is plainly on screen, prefer those paths before changing the global query.

## Verification policy

After these kinds of actions, run `get_app_state` again before acting again:

- playback or networked media actions
- navigation that changes panes or routes
- save or export flows
- list mutations such as add/remove/reorder
- context-menu or submenu actions
- transient dialogs, sheets, inline popovers, or rename states
- anything where the app is known to update lazily

For fallback-only Wayland surfaces and other murky UIs:

- after any click, scroll, drag, keypress, or text entry that could change the visible state, reacquire state before the next action
- after opening a context menu or submenu, reacquire a fresh screenshot before the next click
- after a right-click selection gesture, verify which row is actually highlighted before choosing the menu item
- after text entry into a search or rename field, reacquire state before assuming the visible results or title changed
- if a click on a visibly listed playlist or row did not change the main pane, try one more deliberate click or a row-level context menu before pivoting into unrelated search
- if the target object is missing after a direct check, switch to the create/recreate flow immediately instead of continuing in search, playback, or unrelated detail pages

## Platform notation

Write shortcuts in paired cross-platform form when possible:

- `Cmd + A / Ctrl + A`
- `Cmd + F / Ctrl + F`
- `Return / Enter`
- `Delete / Backspace` only when the distinction matters

## Read next when needed

- For practical interaction patterns adapted from existing computer-use workflows plus Linux-specific examples, read `references/hybrid-patterns.md`.
- For app-specific advice, inspect the packaged `resources/app-instructions/*.md` guidance or the snapshot's `app_guidance` field.
