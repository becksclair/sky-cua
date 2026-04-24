from __future__ import annotations

from pathlib import Path

from _plugin_bundle import REPO_ROOT

TIDAL_PLAYLIST_NAME = "Codex Favorites"
TIDAL_SONG_COUNT = 5
TIDAL_WORKFLOW_MODEL = "gpt-5.5"
TIDAL_WORKFLOW_REASONING_EFFORT = "low"
TIDAL_APP_SERVER_TIMEOUT_SECONDS = 420.0
TIDAL_RESULT_SCHEMA = REPO_ROOT / "scripts" / "schemas" / "tidal_playlist_result.json"


def tidal_playlist_prompt(*, app_server: bool) -> str:
    goal_verb = "focus the running" if app_server else "open or focus the"
    pre_playlist_recovery = (
        "- If you are in search results, a track-detail page, or any other unrelated view before the playlist exists, get back to the library/sidebar flow instead of continuing there.\n"
        if app_server
        else ""
    )
    lane_rules = (
        """
Assumptions:
- TIDAL is already installed, running, and logged in.
- Do not use shell commands, process inspection, media APIs, or shell tricks to fake visibility or completion. This harness is specifically proving the installed computer-use plugin path.

Rules:
- Always start from a fresh `get_app_state` so you have the latest screenshot path.
- Inspect any returned `screenshot_path` with `view_image` before committing to a target in TIDAL.
- Use the control tree for structure when it exists, but do not wait for perfect semantics. If the tree is thin, fallback-only, or ambiguous, use the screenshot to decide where to click, scroll, drag, or type.
- Treat the screenshot as truth for on-screen targeting. The fallback regions are only anchors that narrow your visual search space.
"""
        if app_server
        else """
Rules:
- Use shell commands only to launch the app if needed.
- Do not use `list_mcp_resources`, process inspection, media APIs, or shell tricks to fake playlist inspection or completion; use the computer-use plugin for inspection and interaction.
- Follow the skill's hybrid policy: tree for structure, screenshots for truth, and physical gestures for media UI when that is the more reliable path.
"""
    ).strip()

    app_server_only_rules = (
        """
- When the obvious next move is unclear, explore the UI with `perform_secondary_action`, visible “more” buttons, or action clusters rather than giving up. Context menus are a legitimate discovery tool here.
- If you can see the right target on the screenshot but it is not a semantic element, use coordinate-based `click`, `scroll`, `drag`, or text-entry actions rather than bailing out.
- Keep the exploration disciplined: if two successive fresh screenshots show no material progress after exploratory clicks or right-clicks, switch tactics or classify the result honestly instead of looping.
- If you cannot honestly verify the final state from a fresh screenshot, classify the result as `blocked_app_state` or `unable_to_verify` instead of pretending success.
- Return the final `screenshot_path` from the verification snapshot, not an earlier one.
"""
        if app_server
        else """
- When the next move is unclear, explore with `perform_secondary_action`, visible “more” buttons, or other context actions instead of stalling out.
- If two successive fresh screenshots show no material progress after exploratory actions, switch tactics or classify the result honestly instead of looping.
- If you are blocked by login state, missing portal approval, or another app-state issue, or you cannot honestly verify the final screenshot state, classify the result honestly instead of pretending success.
"""
    ).strip()

    return f"""
Goal: {goal_verb} TIDAL desktop app, find or create a playlist named `{TIDAL_PLAYLIST_NAME}`, add exactly {TIDAL_SONG_COUNT} songs you personally like, and return only the schema result.

Required workflow:
- The MCP server is named `computer-use`; in Codex tool calls this may appear as namespaced tools like `mcp__computer_use__list_apps`.
- Use the computer-use MCP tools directly: `mcp__computer_use__list_apps`, `mcp__computer_use__get_app_state`, `mcp__computer_use__click`, `mcp__computer_use__perform_secondary_action`, `mcp__computer_use__scroll`, `mcp__computer_use__type_text`, `mcp__computer_use__press_key`, `mcp__computer_use__drag`, and `mcp__computer_use__set_value` as appropriate.
- Re-run `get_app_state` after meaningful actions instead of assuming the UI updated.
- The final proof must be a fresh plugin `screenshot_path` showing the `{TIDAL_PLAYLIST_NAME}` playlist open with {TIDAL_SONG_COUNT} tracks in it.
- Work in phases:
  1. first find or create the `{TIDAL_PLAYLIST_NAME}` playlist
  2. only after the playlist exists, add tracks to it
  3. return to the playlist and verify the growing track count
  4. end on a final verification screenshot of that playlist with {TIDAL_SONG_COUNT} tracks
{pre_playlist_recovery}- If `{TIDAL_PLAYLIST_NAME}` is already visible in the sidebar or playlist rail, reuse it immediately instead of trying to create it again.

{tidal_interaction_rules()}

{lane_rules}
- When you know where TIDAL is in the desktop screenshot, visually focus on the TIDAL window while keeping all pointer coordinates in the original full-screenshot coordinate space.
- When you are unsure how to create, rename, or search for something, look for visible text-entry controls first: search fields, inline rename fields, modal text boxes, or obvious text inputs. Focus them, type deliberately, then reacquire state.
- Never type placeholder or probe text such as `test` into a field. Only type deliberate values that directly serve the workflow, such as the exact playlist name or an intentionally chosen song search.
{app_server_only_rules}
- While looking for or creating the playlist, prioritize the sidebar, sidebar list, sidebar row band, and nearby action controls over the main content area.
- Treat visible playlist rows and item counts in the sidebar as strong evidence. If `{TIDAL_PLAYLIST_NAME}` is visibly listed there, click that row first.
- Do not start searching for songs or opening track pages until `{TIDAL_PLAYLIST_NAME}` definitely exists.
- Once the playlist exists, add tracks one at a time with a deliberate method such as a visible `Add` button on a track page, a row context menu, a visible add action, or drag-and-drop; after each addition, return to the playlist and verify progress before adding the next track.
- If you are already on a track or album page and there is an obvious visible `Add` action, prefer that over wandering back into search unless it clearly fails.
- If the current page already shows a track list with visible add affordances such as `+`, `Add`, or a row-level context menu, use those to add tracks to `{TIDAL_PLAYLIST_NAME}` before you consider changing the global search query.
- If `{TIDAL_PLAYLIST_NAME}` is selected in the sidebar but the main page does not change, try one more deliberate click on the visible playlist row or use a row-level secondary click/context menu; do not immediately pivot into the global search field.
- After opening any context menu, submenu, dialog, or transient sheet, immediately re-run `get_app_state` and inspect the fresh screenshot before the next click.
- Use right-click flows on playlist rows, track rows, and obvious list items whenever that looks more reliable than hunting for a toolbar action.
- If two successive fresh screenshots still show an unrelated page such as search results or a track detail while the playlist does not exist yet, stop poking the main content area and navigate back to the sidebar/library flow.
""".strip()


def tidal_interaction_rules() -> str:
    return f"""
TIDAL-specific discipline:
- First prove whether `{TIDAL_PLAYLIST_NAME}` exists from the sidebar/library/playlist rail. If it is missing, deleted, or not visible after a direct check, create it immediately before searching for songs.
- Do not search for songs, open track pages, or keep working in unrelated results until `{TIDAL_PLAYLIST_NAME}` is open or its creation flow is actively in progress.
- Before any `type_text`, run `get_app_state`, inspect the screenshot, and confirm the exact field you are about to type into. If the field contains a stale query, playlist name, title text, or any unrelated value, clear or select it before typing.
- If the window title, search field, or visible search-results header contains an unexpected old query, treat the search field as dirty and clear it before entering the next song or playlist name.
- In TIDAL's fallback-only surface, use one visible action at a time: after every click, scroll, drag, `press_key`, `type_text`, menu open, or dialog action, run `get_app_state` and inspect the fresh screenshot before continuing.
- After the first full TIDAL orientation snapshot, prefer `get_app_state` with `detail: "compact"` for repeated verification loops unless you specifically need verbose element descriptions.
- If TIDAL reports that content is unavailable, treat the current add attempt as failed, return to a verified playlist/search state, and choose another visible track instead of counting it.
- After each song add, return to `{TIDAL_PLAYLIST_NAME}` and verify the visible track count before trying the next song.
""".strip()


def validate_tidal_result(
    message: dict,
    *,
    artifact_dir: Path,
    require_screenshot: bool = False,
) -> None:
    if message.get("status") != "completed":
        raise SystemExit(f"Tidal workflow did not complete: {message}; inspect {artifact_dir}")
    if message.get("playlist_name") != TIDAL_PLAYLIST_NAME:
        raise SystemExit(
            f"Tidal workflow returned the wrong playlist name: {message}; inspect {artifact_dir}"
        )
    if len(message.get("song_titles", [])) != TIDAL_SONG_COUNT:
        raise SystemExit(
            f"Tidal workflow did not return {TIDAL_SONG_COUNT} songs: {message}; inspect {artifact_dir}"
        )
    screenshot_path = message.get("screenshot_path")
    if require_screenshot and (not screenshot_path or not Path(screenshot_path).exists()):
        raise SystemExit(
            f"Tidal workflow did not return a real final screenshot: {message}; inspect {artifact_dir}"
        )
