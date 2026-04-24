# Hybrid Computer Use Patterns

These patterns are adapted from the bundled macOS Computer Use guidance and expanded so they make sense across Linux, macOS, and Windows.

## Screenshot first, tree second

The model should not wait for a perfect control tree before it does anything useful.

- take a fresh screenshot-backed state snapshot first
- if a `screenshot_path` is available, inspect it with `view_image`
- use the tree for names, bounds, and candidate regions when it exists
- after initial orientation, prefer compact app-state snapshots for repeated action verification; keep full snapshots for debugging or when verbose element descriptions are actually needed
- if the tree is sparse, fallback-only, or plainly wrong, use the screenshot to decide the target directly
- once the screenshot confirms the target, coordinate-based physical actions are fair game
- treat fallback regions as visual anchors that narrow the search space, not as guaranteed widgets
- in fallback-heavy apps, take one action at a time and reacquire a fresh screenshot before the next click, keypress, scroll, drag, or text entry
- when the target is one window inside a full desktop screenshot, visually narrow your attention to that window but keep coordinate actions in the current screenshot pixel coordinate space
- for `click` and `drag`, explicit x/y coordinates are screenshot pixels from the snapshot image; the plugin converts them to desktop or stream coordinates before moving the pointer

That is especially important for custom media apps, games, design tools, and native Wayland surfaces that expose only partial accessibility coverage.

## Spreadsheet-like editing

When changing a cell or table value:

- click the cell to focus it
- if replacement is intended, use a full-selection shortcut such as `Cmd + A / Ctrl + A` or the app's known replacement gesture
- type the new value
- verify the result with fresh state instead of assuming the write landed

This is often more reliable than trying to force a semantic write into a widget that only sort of behaves like a text field.

## Block editors and document surfaces

For editors like Notion-style block UIs, Builder panes, or other structured documents:

- use the tree to locate likely blocks, headings, and editable regions
- click the visible block or title to focus it
- use `Return / Enter` if the app enters edit mode on keypress instead of direct typing
- insert one logical chunk at a time, then re-check state

If `Cmd + A / Ctrl + A` has layered selection semantics, assume the first press selects the local block and a second press may select the whole document.

## Search and filter flows

For media apps, browsers, and file managers:

- first confirm the intended search field is actually focused
- inspect whether the search field already contains a stale query before typing
- clear or select stale text when replacement is intended
- use text entry for the search term
- re-run state before concluding that the app returned no results; search UIs are often lazy or network-backed

This matters especially in media apps where pressing `Return / Enter` in the wrong place can trigger playback instead of search.

## Text controls as discovery tools

When the app goal is “create, find, rename, or filter something” and the obvious button is not jumping out:

- look for visible text-entry controls first
- treat search bars, inline rename boxes, modal title fields, and obvious empty text controls as exploratory tools, not just semantic form fields
- inspect the field the screenshot actually shows before typing into it
- if the field contains stale or unrelated text, clear or select it before typing the deliberate value
- click to focus the field, type one deliberate value, then reacquire state
- if the first text field did not do the right thing, do not blindly keep typing into the void; re-check the screenshot and pick the next plausible text target
- never type placeholder junk like `test` into a field just to probe the UI; every typed value should be an intentional name, search term, or edit that directly serves the task
- if the object you need is confirmed missing or deleted, move to the create/recreate path promptly instead of continuing through unrelated search results

This is especially useful in custom media apps where the playlist/search/create affordance is clearer on screen than in the tree.

## Sliders, timers, and custom controls

If the UI exposes a slider, dial, canvas, split view divider, or other custom control:

- prefer physical interaction
- use the tree for rough targeting and the screenshot for geometric confirmation
- drag or click at visible locations instead of assuming a semantic value write exists

For timer-style fields, clicking to focus and typing digits can be more reliable than direct semantic value changes when the widget is custom-drawn.

## Media rows and context menus

For track lists, playlist rows, file rows, and similar list items:

- single-click for selection/focus
- double-click when the app's obvious visible behavior is “open” or “play”
- right-click when the visible affordance is a context menu or when a “More” button is unavailable
- re-check state before repeating playback or queue actions

If the menu path matters:

- right-click the row you can actually see on the screenshot
- reacquire state before picking submenu items or destructive actions
- if a menu or submenu appeared, use the fresh screenshot instead of trusting where you think it ought to be
- if a row-level overflow button or action cluster is visible, it is often safer than trying to hit a tiny semantic node that is not really there
- if the current page already shows track rows with visible `+`, `Add`, or row-level menu affordances, use those before jumping to a global search field just because it is there
- if the target playlist row is visibly listed with an item count, treat that as strong evidence and work from that visible row before trying more speculative navigation
- if a required playlist, folder, or collection is not visible after a direct sidebar/library check, create it before searching for content that depends on it

Fallback-only Wayland surfaces especially need this discipline, because the right action often lives in a context menu long before the tree admits it.

## Graphical apps and canvases

For apps like Krita, GIMP, or other image/design tools:

- use the tree to find the app, dialogs, and large named regions
- use screenshots and physical input for canvas work, save dialogs, drag gestures, and custom tool surfaces
- never assume the tree alone can tell you where a painted button, slider thumb, or canvas affordance actually is

## Linux-specific notes worth keeping

- XWayland editors may need keyboard input routed through the X11 lane rather than the portal keyboard lane.
- Native Wayland apps can expose good structure but still lie about actionable screen bounds on some surfaces.
- Fallback-only Wayland windows often need a fresh screenshot after every context-menu, submenu, popover, or inline rename step. Do not chain clicks through transient UI off stale geometry.
- If the visible button is clear and the semantic click wedges, stop being romantic about the API and click the damned button.
