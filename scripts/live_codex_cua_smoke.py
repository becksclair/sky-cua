#!/usr/bin/env python3
"""Single-run codex CUA smoke: exercise every computer-use + browser-use tool.

This is the substantive codex-CLI gate. In one ``codex exec`` run the agent must
drive the full sky-cua tool surface against live fixtures:

- extension install: the harness opens Chrome at chrome://extensions (no
  preloaded extension) and registers the native-messaging host; the AGENT then
  installs the Codex extension itself with computer-use ("Developer mode" ->
  "Load unpacked" -> the folder chooser), which is what unlocks the browser tools;
- desktop (GTK pointer fixture): click, secondary-click, drag, scroll, type,
  editable readback, a checkbox toggle, a combo-box semantic select, window
  activation, and desktop observation/screenshots; and
- browser (the live Chrome tab the agent just enabled): open/claim/navigate a tab,
  move the cursor, click, type, press a key, scroll, observe the page, and
  screenshot it — including reading a pixels-only token rendered on a page <canvas>
  (the model-image vision proof folded in from the retired WebP readback smoke).

After the run a deterministic gate proves, from the transcript, that every
required tool/operation was called with no unrecovered tool errors (a transient
error followed by a successful retry of the same operation is recovery, not
failure), cross-checked against the fixtures' ground truth. The qualitative host
judge (live_agent_perf_judge.py)
runs separately on the host over the artifacts this smoke writes.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import secrets
import shutil
import subprocess
import time
from contextlib import ExitStack
from pathlib import Path
from typing import Any

from _chrome_bridge import (
    DEFAULT_EXTENSION_ID,
    browser_command,
    default_extension_dir,
    install_temp_manifest,
    launch_browser,
    restore_manifest,
    serve_html_fixture,
    terminate_browser,
    wait_for_devtools_port,
    wait_for_extension_target,
    wait_for_socket,
)
from _codex_exec import (
    DESKTOP_E2E_EXEC_ARGS,
    make_artifact_dir,
    plugin_mention,
    prepare_chatgpt_plugin_test_home,
    read_last_message,
    require_computer_use_tool_call,
    run_codex_exec,
    transcript_mcp_tool_calls,
    with_plugin_mention,
)
from _cua_coverage import analyze_coverage
from _model_profiles import model_profile, required_reasoning_effort
from _plugin_bundle import REPO_ROOT
from live_desktop_smoke import load_state, run_pointer_fixture, wait_for_stable_pointer_fixture

RESULT_SCHEMA = REPO_ROOT / "scripts" / "schemas" / "cua_full_smoke_result.json"
# This profile remains separately named even when it matches codex_exec, so the
# YAML states which model each harness owns.
_CODEX_CUA_PROFILE = model_profile("codex_cua")
DEFAULT_CUA_MODEL = _CODEX_CUA_PROFILE.model
DEFAULT_CUA_REASONING_EFFORT = required_reasoning_effort("codex_cua")
# Default native-host socket dir. The deployed MCP server discovers sockets here
# (and via /proc), so launching Chrome against this dir needs no .mcp.json change.
DEFAULT_SOCKET_DIR = Path("/tmp/codex-browser-use")
HOST_BINARY_CANDIDATES = (
    REPO_ROOT / "target/release/sky-cua-chrome-host",
    REPO_ROOT / "target/debug/sky-cua-chrome-host",
)


def require_real_wayland_session() -> None:
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR")
    wayland_display = os.environ.get("WAYLAND_DISPLAY")
    if not runtime_dir or not wayland_display:
        raise SystemExit(
            "codex CUA smoke requires a real Wayland session (XDG_RUNTIME_DIR + WAYLAND_DISPLAY)"
        )
    socket_path = Path(runtime_dir) / wayland_display
    if not socket_path.is_socket():
        raise SystemExit(f"Wayland session socket not found: {socket_path}")


def resolve_host_binary(override: Path | None) -> Path:
    candidates = [override] if override is not None else list(HOST_BINARY_CANDIDATES)
    for candidate in candidates:
        resolved = candidate.expanduser().resolve()
        if resolved.exists() and os.access(resolved, os.X_OK):
            return resolved
    raise SystemExit(
        "sky-cua-chrome-host binary not found; build the plugin (target/release) "
        f"or pass --host-path. Looked at: {[str(c) for c in candidates]}"
    )


def clean_stale_sockets(socket_dir: Path) -> None:
    socket_dir.mkdir(parents=True, exist_ok=True)
    for stale in socket_dir.glob("extension-*.sock"):
        with contextlib.suppress(OSError):
            stale.unlink()


def browser_fixture_html(token: str) -> bytes:
    # #field/#submit/#marker exercise browser_input type/click/press_key; #scrollbox
    # exercises browser_scroll; the <canvas> renders the token as pixels only (not in
    # the DOM text / accessibility tree) so the agent must read it from the screenshot.
    token_js = json.dumps(token)
    return (
        "<!doctype html><html><head><meta charset='utf-8'>"
        "<title>sky-cua browser fixture</title></head>"
        "<body style='font-family:sans-serif;margin:24px'>"
        "<h1>sky-cua browser fixture</h1>"
        "<input id='field' placeholder='type here' style='font-size:20px'>"
        "<button id='submit' style='font-size:20px'>Submit</button>"
        "<div id='marker' style='font-size:24px;font-weight:bold'></div>"
        "<div id='scrollbox' style='height:180px;width:360px;overflow:auto;border:2px solid #333'>"
        "<div style='height:2000px'>scrollable region</div></div>"
        "<canvas id='token' width='640' height='160'></canvas>"
        "<script>"
        "var submit=function(){"
        "document.getElementById('marker').textContent=document.getElementById('field').value;};"
        "document.getElementById('submit').addEventListener('click',submit);"
        "document.getElementById('field').addEventListener('keydown',function(e){"
        "if(e.key==='Enter'){submit();}});"
        "var ctx=document.getElementById('token').getContext('2d');"
        "ctx.fillStyle='#ffffff';ctx.fillRect(0,0,640,160);"
        "ctx.fillStyle='#000000';ctx.font='bold 72px monospace';"
        f"ctx.fillText({token_js},20,104);"
        "</script></body></html>"
    ).encode()


def build_prompt(
    *,
    window_title: str,
    fixture_url: str,
    entry_sentinel: str,
    browser_sentinel: str,
    extension_dir: str,
) -> str:
    return f"""
Goal: in ONE run, use the desktop computer-use tools to install the Codex Chrome extension yourself, then use the browser tools it unlocks, then exercise the remaining desktop tools against a test window. Re-observe after each visually meaningful action; use semantic targets where available and screenshot pixels for physical input.

Phase A — install the browser extension through the Chrome UI (a Google Chrome window is already open at the chrome://extensions page; the browser_* tools will NOT work until you do this):
A1. `list_resources(surface="desktop", resource="windows")` then `activate_window` the Google Chrome window by its window_id.
A2. `capture_desktop` and `observe(surface="desktop")` to read the Extensions page.
A3. Turn ON the "Developer mode" toggle (top-right of the page) with desktop_pointer click.
A4. Click the "Load unpacked" button.
A5. A folder chooser dialog opens. Put the absolute directory path into its path field — press the literal key `Ctrl+L` to focus the location field if needed (desktop_keyboard press_key), then desktop_keyboard type_text exactly: {extension_dir}
A6. Confirm/Open the dialog (click Open/Choose, or desktop_keyboard press_key Enter).
A7. Re-`observe(surface="desktop")` the Extensions page and confirm a "Codex" extension card is now present and enabled.

Phase B — browser tools (the extension is now loaded; use the browser_* surface):
B0. The browser tools drive the tab through Chrome's debugger. If Chrome shows any banner, infobar, or prompt about debugging/automation (e.g. "Codex started debugging this browser", "An extension is debugging…", or an Allow/Keep prompt), KEEP it: approve/allow it with desktop_pointer, and never click Cancel/Stop/"Don't allow" — the browser_* tools stop working the moment that debugging connection is cancelled. Re-check for this banner with capture_desktop if a browser action fails.
B1. The only pre-existing tab is the chrome://extensions page. That is a privileged Chrome page, not a CDP page target — it can never be claimed or driven, so do NOT claim it (claiming or navigating it wedges the debugger transport for later tabs). Instead `browser_open` a fresh tab, then `browser_navigate` that opened tab_id to {fixture_url} and use that tab_id for every page action below.
B2. Exercise tab claiming on a drivable tab: `list_resources(surface="browser", resource="tabs")`, then `browser_claim_tab` the http tab you just opened (its tab_id) to confirm claim works, and keep using that tab_id.
B3. `observe(surface="browser", tab_id=...)` and `capture_screen(surface="browser", tab_id=...)` on that tab.
B4. `browser_move_mouse` over the text input, then `browser_input(operation="click")` it, `browser_input(operation="type_text")` exactly `{browser_sentinel}`, and `browser_input(operation="press_key")` Enter (or click Submit) so the page copies your text into the marker.
B5. `browser_scroll` the scroll region.
B6. Read the high-contrast token rendered on the page <canvas> (format `CUA-XXXXXX`) from the browser screenshot image — it is pixels only and is NOT in the page text or DOM.

Phase C — desktop tools (the GTK window titled "{window_title}"):
C1. `activate_window` that window, then `capture_desktop` + `observe(surface="desktop")`.
C2. Click the "Physical click target" button (desktop_pointer click).
C3. Right-click the "Secondary-click region" (desktop_pointer secondary_click).
C4. Drag inside the "Drag region" from one side to the other (desktop_pointer drag).
C5. Scroll the "Scroll region" downward (desktop_scroll).
C6. Focus the text entry and replace its contents with exactly `{entry_sentinel}` (desktop_set_value, or click then desktop_keyboard type_text); confirm via a fresh observe readback.
C7. Press a key in the entry (desktop_keyboard press_key, e.g. End).
C8. Toggle the "Enable smoke option" check button (desktop_toggle).
C9. Expand the "Smoke details" expander (desktop_semantic expand).
C10. Activate the "Physical click target" button via desktop_action (activate or perform_action).
C11. Drag the horizontal slider's thumb to the right with a physical `desktop_pointer` drag (pass `duration_ms` ~600). The thumb starts at the far left (value 0); aim the drag start at the thumb itself and drag to about the right third so the value rises well above the middle. Re-observe to confirm the slider value increased.
C12. Drag-and-drop the "DnD source" chip onto the "Drop zone": a physical `desktop_pointer` drag from the chip to the drop zone (pass `duration_ms` ~600). Re-observe/capture to confirm the drop registered.

Rules:
- Use only sky-cua MCP tools. Do not use shell commands, OCR utilities, xdotool/wmctrl, file reads, or page-source inspection to obtain values; read the entry/marker via tool readback and the canvas token from the screenshot image.
- Desktop and browser coordinate spaces are unrelated; never reuse coordinates across them.

Return the schema result:
- desktop_entry_text: the entry value you proved via desktop readback (should be `{entry_sentinel}`).
- browser_input_text: the text you typed into the browser field (should be `{browser_sentinel}`).
- browser_marker_text: the page marker value after submit.
- vision_token: the exact token read from the canvas image, including the `CUA-` prefix.
- status: `completed` only if the extension installed, every step succeeded, and the token was read from the screenshot; otherwise `blocked`.
""".strip()


def _ground_truth(
    *,
    state: dict[str, Any],
    message: dict[str, Any],
    token: str,
    entry_sentinel: str,
    browser_sentinel: str,
) -> tuple[dict[str, Any], list[str]]:
    entry_text = str(state.get("entry_text", "")) + str(state.get("submitted_text", ""))
    checks = {
        "clicked": bool(state.get("clicked")),
        "secondary_clicked": bool(state.get("secondary_clicked")),
        "drag_completed": bool(state.get("drag_completed")),
        # >= 40 proves a real thumb-tracking drag (a teleport without a grab
        # leaves the value near 0) while giving the agent margin on exactly how
        # far right it drags.
        "slider_dragged": float(state.get("slider_h_value", 0.0) or 0.0) >= 40.0,
        "dnd_dropped": bool(state.get("dnd_dropped")),
        "scrolled": int(state.get("scroll_events", 0) or 0) > 0,
        "checkbox_toggled": bool(state.get("checkbox_toggled")),
        "expander_expanded": bool(state.get("expander_expanded")),
        "entry_sentinel_present": entry_sentinel in entry_text,
        "browser_marker_matches": str(message.get("browser_marker_text", "")) == browser_sentinel,
        "vision_token_matches": str(message.get("vision_token", "")).strip().upper() == token,
    }
    failures = [name for name, ok in checks.items() if not ok]
    return checks, failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--symlink", action="store_true", help="Symlink the built bundle.")
    parser.add_argument(
        "--model",
        default=None,
        help=f"Override the codex model (default {DEFAULT_CUA_MODEL}).",
    )
    parser.add_argument("--reasoning-effort", default=None, help="Override codex reasoning effort.")
    parser.add_argument(
        "--browser", default="chrome", choices=["auto", "chrome", "chromium", "brave"]
    )
    parser.add_argument("--extension-id", default=DEFAULT_EXTENSION_ID)
    parser.add_argument("--host-path", type=Path, default=None, help="sky-cua-chrome-host binary.")
    parser.add_argument(
        "--keep-browser-open", action="store_true", help="Leave the browser running."
    )
    args = parser.parse_args()

    require_real_wayland_session()

    token = "CUA-" + secrets.token_hex(3).upper()
    entry_sentinel = "cua-entry-" + secrets.token_hex(2)
    browser_sentinel = "cua-browser-" + secrets.token_hex(2)
    artifact_dir = make_artifact_dir("codex-cua")

    coverage_summary: dict[str, Any] = {"ok": False, "stage": "setup"}
    result_exit = 1
    try:
        with ExitStack() as stack:
            # Desktop fixture.
            state_path = artifact_dir / "pointer-state.json"
            fixture = run_pointer_fixture(state_path)
            stack.callback(_terminate_process, fixture)
            fixture_state = wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 40)
            window_title = str(fixture_state.get("title") or "sky-cua live pointer smoke")

            # Browser page fixture (HTTP) + live Chrome + extension + native host.
            fixture_url = stack.enter_context(
                serve_html_fixture(browser_fixture_html(token), route="/cua.html")
            )
            browser = browser_command(args.browser)
            extension_src = default_extension_dir().expanduser().resolve()
            if not (extension_src / "manifest.json").exists():
                raise SystemExit(f"chrome extension manifest not found under {extension_src}")
            # Stage the unpacked extension at a stable, simple absolute path the agent
            # can type into the Chrome "Load unpacked" folder chooser.
            extension_dir = Path.home() / ".cache" / "sky-cua" / "codex-extension"
            shutil.rmtree(extension_dir, ignore_errors=True)
            extension_dir.parent.mkdir(parents=True, exist_ok=True)
            shutil.copytree(extension_src, extension_dir)
            host_path = resolve_host_binary(args.host_path)
            socket_dir = DEFAULT_SOCKET_DIR
            clean_stale_sockets(socket_dir)
            sessions_dir = artifact_dir / "browser-sessions"
            sessions_dir.mkdir(parents=True, exist_ok=True)

            # Register the native-messaging host up front so the extension connects to
            # it the moment the agent loads it through the Chrome UI. Chrome searches
            # the launch profile's NativeMessagingHosts/ dir under a custom
            # --user-data-dir, so the manifest must land there too.
            profile_dir = artifact_dir / "profile"
            manifest = install_temp_manifest(
                browser.name, args.extension_id, host_path, user_data_dir=profile_dir
            )
            stack.callback(restore_manifest, manifest)
            # Launch Chrome at chrome://extensions WITHOUT preloading the extension;
            # the agent installs it itself via "Load unpacked" using computer-use.
            proc = launch_browser(
                browser.command,
                user_data_dir=profile_dir,
                extension_dir=extension_dir,
                socket_dir=socket_dir,
                sessions_dir=sessions_dir,
                load_extension=False,
                initial_url="chrome://extensions",
                stderr_path=profile_dir / "chrome_stderr.log",
            )
            stack.callback(terminate_browser, proc, args.keep_browser_open)
            port = wait_for_devtools_port(profile_dir, proc)

            # codex run.
            codex_home = prepare_chatgpt_plugin_test_home(
                artifact_dir=artifact_dir, symlink=args.symlink
            )
            # Record which plugin surface resolved: the production compat
            # `computer-use@openai-bundled` (when its marketplace was staged) or the
            # `sky-cua@local` dev fallback. Proves which surface the run exercised.
            plugin_surface = plugin_mention(codex_home)
            print(f"codex CUA smoke plugin surface: {plugin_surface}", flush=True)
            prompt = with_plugin_mention(
                build_prompt(
                    window_title=window_title,
                    fixture_url=fixture_url,
                    entry_sentinel=entry_sentinel,
                    browser_sentinel=browser_sentinel,
                    extension_dir=str(extension_dir),
                ),
                codex_home,
            )
            result = run_codex_exec(
                prompt=prompt,
                artifact_dir=artifact_dir,
                output_schema=RESULT_SCHEMA,
                model=args.model or DEFAULT_CUA_MODEL,
                reasoning_effort=args.reasoning_effort or DEFAULT_CUA_REASONING_EFFORT,
                extra_env={"CODEX_HOME": str(codex_home), "SKY_CUA_BROWSER": "chrome"},
                extra_args=DESKTOP_E2E_EXEC_ARGS,
            )
            result_exit = result.exit_code

            # The agent was responsible for installing the extension; record whether
            # it succeeded (the browser tool coverage below is the hard gate).
            extension_loaded = _extension_present(port, args.extension_id)
            socket_present = _socket_present(socket_dir)

            # Deterministic gate: coverage + no unrecovered tool errors, from the
            # transcript (recovered retries stay informational, not fatal).
            calls = transcript_mcp_tool_calls(result.transcript_path)
            report = analyze_coverage(calls)
            final_state = load_state(state_path) or {}
            message = (
                read_last_message(result.last_message_path)
                if result.last_message_path.exists()
                else {}
            )
            ground_checks, ground_failures = _ground_truth(
                state=final_state,
                message=message,
                token=token,
                entry_sentinel=entry_sentinel,
                browser_sentinel=browser_sentinel,
            )
            coverage_summary = report.to_summary()
            coverage_summary["exit_code"] = result.exit_code
            coverage_summary["plugin_surface"] = plugin_surface
            coverage_summary["extension_loaded_by_agent"] = extension_loaded
            coverage_summary["native_host_socket_up"] = socket_present
            coverage_summary["ground_truth"] = ground_checks
            coverage_summary["ground_truth_failures"] = ground_failures
            coverage_summary["ok"] = bool(
                report.ok and not ground_failures and result.exit_code == 0
            )
    finally:
        # Surface Chrome's verbose log (debugger/devtools/extension events) at the
        # artifact root so the host dispatch can pull it alongside the transcript.
        for src_name, dst_name in (
            ("chrome_debug.log", "chrome-debug.log"),
            ("chrome_stderr.log", "chrome-stderr.log"),
        ):
            src = artifact_dir / "profile" / src_name
            if src.exists():
                with contextlib.suppress(OSError):
                    shutil.copy2(src, artifact_dir / dst_name)
        # Always write the coverage summary + ready marker so the host judge can run
        # and triage even when this deterministic gate failed.
        (artifact_dir / "coverage-summary.json").write_text(
            json.dumps(coverage_summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        (artifact_dir / "codex-cua-ready.json").write_text(
            json.dumps({"artifact_dir": str(artifact_dir), "exit_code": result_exit}) + "\n",
            encoding="utf-8",
        )
        # Stable pointer at a known path so the host judge dispatch can locate the
        # timestamped artifact dir without scanning.
        (artifact_dir.parent / "latest.json").write_text(
            json.dumps({"artifact_dir": str(artifact_dir), "exit_code": result_exit}) + "\n",
            encoding="utf-8",
        )

    problems: list[str] = []
    if result_exit != 0:
        problems.append(f"codex exec exited with {result_exit}")
    else:
        try:
            require_computer_use_tool_call(
                artifact_dir / "codex-output.jsonl", artifact_dir=artifact_dir
            )
        except RuntimeError as exc:
            problems.append(str(exc))
    if not coverage_summary.get("ok"):
        for key in (
            "missing_tools",
            "missing_operations",
            "missing_surfaces",
            "ground_truth_failures",
        ):
            values = coverage_summary.get(key)
            if values:
                problems.append(f"{key}: {', '.join(values)}")
        if coverage_summary.get("unrecovered_errors"):
            problems.append(
                f"unrecovered tool errors: {len(coverage_summary['unrecovered_errors'])}"
            )

    if problems:
        print(
            "codex CUA smoke FAILED:\n  " + "\n  ".join(problems) + f"\nartifacts: {artifact_dir}",
            flush=True,
        )
        return 1
    print(f"codex CUA smoke passed: full tool surface exercised; artifacts: {artifact_dir}")
    return 0


def _extension_present(port: str, extension_id: str) -> bool:
    """Whether the Codex extension's devtools target is live (the agent loaded it)."""
    try:
        wait_for_extension_target(port, extension_id)
        return True
    except (TimeoutError, RuntimeError, OSError):
        return False


def _socket_present(socket_dir: Path) -> bool:
    """Whether the native-host socket came up (extension connected to the host)."""
    try:
        wait_for_socket(socket_dir)
        return True
    except (TimeoutError, OSError):
        return False


def _terminate_process(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
