#!/usr/bin/env python3
"""Live phone-use *workflow* smoke for sky-cua.

Crystallizes the live agentic workflows proven on the API-36 emulator on
2026-06-20 — the ones an external agent ran end-to-end against a ready device
through the `phone_*` MCP tools — into a wired, repeatable smoke:

* ``settings`` — open the Settings app and navigate into the Accessibility
  screen.
* ``browser`` — open Chrome, dismiss any first-run interstitial, and search the
  web for a fixed query.

Both are *agentic*: an external agent CLI (``claude`` by default) is told to
accomplish the task using ONLY the sky-cua phone MCP tools, then the harness
proves the outcome by GROUND TRUTH that is independent of the agent's own
claims — the device's resumed activity, read straight from adb. The settings
workflow must land on ``com.android.settings`` *Accessibility*; the browser
workflow must land on ``com.android.chrome``. The browser ground truth proves the
agent reached the browser, not that a specific search executed: web-page text is
not in the accessibility tree (and is GPU-garbled on this emulator), so the
agent's prose answer is a soft signal only (it may even reflect the model's prior
knowledge, since it cannot read the result page here), never the gate. Use
``--require-tool-evidence`` with a JSON-mode agent to additionally gate that the
agent drove the device with a real phone action tool (any of the workflow's
evidence tools — efficient agents may reach Settings via ``phone_open_settings``
rather than tapping).

Every run also probes the phone-native agent pointer overlay through a pure MCP
probe: the companion must advertise the native overlay plane, a screenshot must
report it live, and a benign device-space tap + swipe must route
``backend=companion`` (the path that fires the on-device glow / tap-ripple /
swipe-trail). This is the "pointer overlay works throughout" assertion.

The harness is hardware-dependent: when a prerequisite is missing (no adb, no
device, no companion, no agent CLI) the affected step SKIPS with an explicit
reason rather than failing. It is non-destructive — it navigates UI and runs a
web search; it installs nothing and never shells out to adb on the agent's
behalf.

No screenshots, RPC tokens, notification bodies, or accessibility dumps are
persisted; only bounded sanitized metadata.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path
from xml.sax import saxutils

from _agent_mcp_smoke import make_artifact_dir, run_agent
from _mcp_stdio import McpClient
from deploy_freshness import deployed_client_path

# Reuse the existing phone smokes' typed driver, outcome model, sanitization, and
# MCP-result inspection so all three phone harnesses stay in lockstep.
from live_phone_companion_setup_smoke import (
    REPO_ROOT,
    SetupFailure,
    SetupSkip,
    SetupSmokeOptions,
    _run_adb,
    phone_tools_invoked,
    resolve_serial,
)
from live_phone_use_smoke import (
    PhoneSmoke,
    PhoneSmokeOptions,
    SmokeFailure,
    SmokeSkip,
    StepResult,
    adb_binary,
    bounded_metadata,
    first_diagnostic_message,
    overlay_step,
    resolve_client_path,
    result_is_error,
    sanitize_serial,
    step_fail,
    step_pass,
    step_skip,
    structured,
    summarize_counts,
)

SMOKE_NAME = "phone_workflow_smoke"

# Agents that can drive the workflow. Claude Code reliably surfaces the sky-cua
# phone MCP tools; opencode does not currently expose them to the agent (it sees
# the phone-use skill and falls back to bash/exploration), so it is not the
# default. The adb ground-truth gate validates whichever agent is used.
AGENT_CHOICES = ("claude", "opencode", "pi", "openclaw")
DEFAULT_AGENT = "claude"

# The fixed search query and the answer keyword the browser workflow expects to
# surface. The keyword is a soft signal only (web text is not in the a11y tree).
BROWSER_QUERY = "tallest mountain in the world"
BROWSER_ANSWER_KEYWORD = "everest"

SETTINGS_PACKAGE = "com.android.settings"
# The Accessibility screen lands on ``.Settings$AccessibilitySettingsActivity``
# via the direct intent, but on the generic ``.SubSettings`` host when reached by
# manual navigation (and OEM skins vary too). When the resumed activity name lacks
# this substring, the ground truth falls back to confirming the screen's toolbar
# title via uiautomator (``screen_title_visible``), so either nav path passes.
SETTINGS_SCREEN_SUBSTRING = "Accessibility"
CHROME_PACKAGE = "com.android.chrome"

# phone_* tools that ACTUATE the device — navigation, input, and app/intent
# control — as opposed to observation tools (connect/screenshot/accessibility_tree/
# app_current/...). Invoking any one proves the agent drove the device through
# phone-use rather than faking it via the shell. Enumerating per-workflow "must use
# X" lists is brittle: an efficient agent legitimately reaches a screen via
# phone_open_settings or phone_app_launch instead of tapping, so the evidence gate
# accepts any actuation tool and lets the per-workflow ground truth prove the
# outcome.
PHONE_DRIVE_TOOLS: tuple[str, ...] = (
    "phone_tap",
    "phone_swipe",
    "phone_type_text",
    "phone_press_key",
    "phone_app_launch",
    "phone_app_open_intent",
    "phone_open_settings",
)


# ---------------------------------------------------------------------------
# Workflow registry (pure data + prompt builders)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Workflow:
    """One agentic phone workflow and the ground truth that proves it."""

    name: str
    #: adb resumed-activity package the device must end on.
    target_package: str
    #: Optional substring the resumed-activity component must contain (the finer
    #: screen check). ``None`` means reaching the package is the whole gate.
    screen_substring: str | None
    #: ``phone_*`` actuation tools that count as the agent having driven the device
    #: for this workflow — ANY one is sufficient. Defaults to ``PHONE_DRIVE_TOOLS``
    #: (every navigation/input/app-control tool) so efficient paths still pass. Soft
    #: evidence unless ``--require-tool-evidence``.
    evidence_tools: tuple[str, ...]
    #: Soft answer keyword expected in the agent transcript, or ``None``.
    answer_keyword: str | None
    #: Builds the single-shot task prompt for a given device serial.
    prompt_builder: Callable[[str], str]

    def prompt(self, serial: str) -> str:
        return self.prompt_builder(serial)


def _settings_prompt(serial: str) -> str:
    return (
        "You are driving a ready Android device through the sky-cua phone MCP tools "
        "(phone_connect, phone_screenshot, phone_accessibility_tree, phone_tap, phone_swipe, "
        "phone_type_text, phone_press_key, phone_app_launch, phone_open_settings). The device is "
        "already set up (companion installed, permissions granted): do NOT install anything and do "
        "NOT shell out to adb — use ONLY the sky-cua phone MCP tools. "
        f"Task: connect to the device with serial {serial}, open the Settings app, and navigate "
        "into the Accessibility screen. Prefer the accessibility tree for native UI coordinates. "
        "When you reach the Accessibility screen, leave the device on it (do not press Home or "
        "Back afterwards). Report a JSON object with keys: reached_accessibility (boolean) and "
        "services (a short list of the accessibility services/options you saw)."
    )


def _browser_prompt(serial: str) -> str:
    return (
        "You are driving a ready Android device through the sky-cua phone MCP tools "
        "(phone_connect, phone_screenshot, phone_accessibility_tree, phone_tap, phone_swipe, "
        "phone_type_text, phone_press_key, phone_app_launch). The device is already set up "
        "(companion installed, permissions granted): do NOT install anything and do NOT shell out "
        "to adb — use ONLY the sky-cua phone MCP tools. Prefer the accessibility tree for native UI "
        "coordinates; the web page pixels may render poorly, so rely on the native Chrome UI. "
        f"Task: connect to the device with serial {serial}, open Chrome, dismiss any first-run or "
        "sign-in interstitial, tap the address/search bar, and search the web for "
        f"'{BROWSER_QUERY}'. Stay in Chrome on the results (do not press Home afterwards). Report a "
        "JSON object with keys: searched (boolean) and answer (the mountain name from the results, "
        "or null if you could not read it)."
    )


WORKFLOWS: dict[str, Workflow] = {
    "settings": Workflow(
        name="settings",
        target_package=SETTINGS_PACKAGE,
        screen_substring=SETTINGS_SCREEN_SUBSTRING,
        evidence_tools=PHONE_DRIVE_TOOLS,
        answer_keyword=None,
        prompt_builder=_settings_prompt,
    ),
    "browser": Workflow(
        name="browser",
        target_package=CHROME_PACKAGE,
        screen_substring=None,
        evidence_tools=PHONE_DRIVE_TOOLS,
        answer_keyword=BROWSER_ANSWER_KEYWORD,
        prompt_builder=_browser_prompt,
    ),
}

WORKFLOW_FULL = "full"
WORKFLOW_SEQUENCE: tuple[str, ...] = ("settings", "browser")
ALL_WORKFLOWS: tuple[str, ...] = (*WORKFLOW_SEQUENCE, WORKFLOW_FULL)


def workflows_for(selection: str) -> tuple[str, ...]:
    """Expand a requested workflow selection into concrete workflow names."""
    if selection == WORKFLOW_FULL:
        return WORKFLOW_SEQUENCE
    return (selection,)


# ---------------------------------------------------------------------------
# Ground-truth parsing (pure, unit-tested)
# ---------------------------------------------------------------------------

# Inside an ActivityRecord the resumed component appears as
# ``u0 <package>/<activity> t<task>`` — e.g.
# ``u0 com.android.settings/.Settings$AccessibilitySettingsActivity t5``.
_RESUMED_COMPONENT_RE = re.compile(r"u\d+\s+([A-Za-z0-9_.]+)/([^\s}]+)")
# Resumed-activity lines, in descending preference. ``dumpsys activity
# activities`` prints these; format varies by Android version.
_RESUMED_LINE_MARKERS = (
    "topResumedActivity=",
    "mResumedActivity:",
    "ResumedActivity:",
    "mResumedActivity=",
)


@dataclass(frozen=True)
class ResumedActivity:
    """The device's foreground (resumed) activity component."""

    package: str
    activity: str

    @property
    def component(self) -> str:
        return f"{self.package}/{self.activity}"


def parse_resumed_activity(dumpsys_text: str) -> ResumedActivity | None:
    """Extract the resumed activity component from ``dumpsys activity activities``.

    Scans for the resumed-activity marker lines in preference order and returns
    the first that yields a ``<package>/<activity>`` component.
    """
    for marker in _RESUMED_LINE_MARKERS:
        for line in dumpsys_text.splitlines():
            if marker not in line:
                continue
            match = _RESUMED_COMPONENT_RE.search(line)
            if match:
                return ResumedActivity(package=match.group(1), activity=match.group(2))
    return None


@dataclass(frozen=True)
class ForegroundCheck:
    """Whether the resumed activity satisfies a workflow's ground truth."""

    ok: bool
    detail: str


def evaluate_foreground(workflow: Workflow, resumed: ResumedActivity | None) -> ForegroundCheck:
    """Decide whether the resumed activity proves the workflow reached its target."""
    if resumed is None:
        return ForegroundCheck(ok=False, detail="no resumed activity parsed")
    component = resumed.component
    if resumed.package != workflow.target_package:
        return ForegroundCheck(
            ok=False,
            detail=f"foreground={bounded_metadata(component)} expected={workflow.target_package}",
        )
    if workflow.screen_substring is not None and workflow.screen_substring not in component:
        return ForegroundCheck(
            ok=False,
            detail=(
                f"foreground={bounded_metadata(component)} "
                f"missing screen '{workflow.screen_substring}'"
            ),
        )
    return ForegroundCheck(ok=True, detail=f"foreground={bounded_metadata(component)}")


def title_fallback_substring(workflow: Workflow, resumed: ResumedActivity | None) -> str | None:
    """The screen title to confirm via the toolbar, or ``None`` when the title
    fallback does not apply.

    Returns the workflow's ``screen_substring`` only when the device is in the
    target app (the load-bearing package guard: a launcher or wrong-app state must
    never be upgraded by a stray on-screen title node). Returning the substring
    itself — rather than a bare bool — lets the single caller that needs it pass a
    narrowed ``str`` to ``screen_title_visible`` without re-checking ``None``, so the
    guard lives in exactly one place.
    """
    if (
        workflow.screen_substring is not None
        and resumed is not None
        and resumed.package == workflow.target_package
    ):
        return workflow.screen_substring
    return None


def resolve_foreground(
    workflow: Workflow,
    resumed: ResumedActivity | None,
    *,
    title_visible: bool,
) -> ForegroundCheck:
    """Final ground-truth decision, allowing a toolbar-title fallback.

    The base check is the resumed activity name. When the agent is in the target
    app but on a generic host activity whose name lacks the screen substring (e.g.
    the Accessibility screen reached by manual navigation lands under
    ``.SubSettings``), an on-screen toolbar-title confirmation (``title_visible``,
    supplied by ``screen_title_visible``) upgrades the result to a pass. The package
    guard (``title_fallback_substring``) is load-bearing: a launcher or wrong-app
    state can never be upgraded, because the title can only be trusted once the
    device is in the target app.
    """
    base = evaluate_foreground(workflow, resumed)
    fallback_title = title_fallback_substring(workflow, resumed)
    if not base.ok and title_visible and fallback_title is not None:
        return ForegroundCheck(
            ok=True,
            detail=f"{base.detail}; screen title '{fallback_title}' confirmed via uiautomator",
        )
    return base


def transcript_mentions_keyword(transcript: str, keyword: str) -> bool:
    """Case-insensitive check that the agent transcript mentions ``keyword``."""
    return keyword.lower() in transcript.lower()


def format_result_line(results: Iterable[StepResult]) -> str:
    """Render the final ``RESULT phone_workflow_smoke ...`` summary line."""
    passed, skipped, failed = summarize_counts(results)
    return f"RESULT {SMOKE_NAME} passed={passed} skipped={skipped} failed={failed}"


# ---------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------


@dataclass
class WorkflowSmokeOptions:
    """Operator-supplied targeting for the workflow smoke."""

    selection: str = WORKFLOW_FULL
    agent: str = DEFAULT_AGENT
    model: str | None = None
    serial: str | None = None
    installed: bool = False
    require_tool_evidence: bool = False
    agent_timeout: float = 300.0
    skip_overlay_probe: bool = False


# ---------------------------------------------------------------------------
# adb ground truth + phone runtime env
# ---------------------------------------------------------------------------


def capture_resumed_activity(adb: str, serial: str) -> ResumedActivity | None:
    """Read the device's current resumed activity straight from adb."""
    dump = _run_adb(adb, serial, ["shell", "dumpsys", "activity", "activities"])
    return parse_resumed_activity(dump)


def dump_has_screen_title(dump_xml: str, title: str) -> bool:
    """Whether a uiautomator XML dump shows ``title`` as the screen's toolbar title.

    Settings sub-screens render a collapsing toolbar whose title is exposed as an
    exact ``content-desc`` node — e.g. the Accessibility screen shows
    ``content-desc="Accessibility"`` whether it is hosted by
    ``.Settings$AccessibilitySettingsActivity`` (direct intent) or the generic
    ``.SubSettings`` host (manual navigation). The match is intentionally
    ``content-desc`` only and NOT ``text``: a parent screen (e.g. top-level
    Settings) lists the child screen as a ``text="Accessibility"`` row, so matching
    ``text`` would wrongly credit merely opening Settings as reaching the screen.
    The title is XML-attribute escaped so a screen name containing ``&``/``<``/``>``
    (e.g. "Display & text") still matches the escaped dump.
    """
    needle = saxutils.escape(title)
    return f'content-desc="{needle}"' in dump_xml


#: On-device path the uiautomator dump is written to, then read back and removed.
_UIAUTOMATOR_DUMP_PATH = "/sdcard/sky-cua-uiautomator-dump.xml"


def screen_title_visible(adb: str, serial: str, title: str) -> bool:
    """Best-effort check that the foreground screen's toolbar title is ``title``.

    Robust to the host activity (reads on-screen content via uiautomator). Dumps to
    an on-device file and reads it back with ``cat`` — the canonical, portable form
    (the ``uiautomator dump /dev/tty`` stdout trick is mangled by some shells) —
    then removes the file. The dump is only inspected for the title substring and is
    never persisted host-side. Returns False when the dump is unavailable rather
    than raising, so an unreadable dump can never upgrade a miss.

    The path is removed BEFORE the dump as well as after: every adb call is
    best-effort (``check=False``), so a stale file left by an interrupted prior call
    must not be readable — otherwise a failed ``uiautomator dump`` (e.g. mid-
    animation) could ``cat`` an old screen's XML and wrongly confirm the title.

    A hung adb call (``_run_adb`` lets ``subprocess`` raise on timeout) is treated as
    "title not confirmed" rather than propagating: this is a best-effort upgrade of
    an already-failed activity-name check, so a flaky dump must not collapse the
    whole workflow into one error.
    """
    try:
        _run_adb(adb, serial, ["shell", "rm", "-f", _UIAUTOMATOR_DUMP_PATH])
        _run_adb(adb, serial, ["shell", "uiautomator", "dump", _UIAUTOMATOR_DUMP_PATH])
        dump = _run_adb(adb, serial, ["shell", "cat", _UIAUTOMATOR_DUMP_PATH])
        _run_adb(adb, serial, ["shell", "rm", "-f", _UIAUTOMATOR_DUMP_PATH])
    except (subprocess.SubprocessError, OSError):
        return False
    return dump_has_screen_title(dump, title)


def _phone_runtime_env(adb: str) -> dict[str, str]:
    """Phone runtime env shared by the agent's MCP child and the overlay probe.

    Pins adb and enables phone-use, with companion auto-install OFF (the device is
    already set up and the smoke installs nothing). Only keys on the agent env
    allowlist (``_agent_mcp_smoke.agent_environment``) reach the agent's MCP child;
    the agent is steered onto the accessibility tree for native UI through the task
    prompt, not an image toggle. The overlay probe forces images off itself in
    ``_client_factory`` (where it actually takes effect over the direct MCP pipe).
    """
    return {
        "SKY_CUA_PHONE": "1",
        "SKY_CUA_ADB": adb,
        "SKY_CUA_PHONE_COMPANION_AUTO_INSTALL": "0",
        "SKY_CUA_PHONE_COMPANION_OPERATOR_MODE": "0",
    }


def _overlay_client_path(*, installed: bool) -> Path:
    """Client the overlay probe drives.

    Defaults to the locally deployed runtime — the exact binary the agent reaches
    through its MCP config, so the overlay assertion validates the same surface
    the workflows ran on (and that the deploy-freshness gate just stamped fresh).
    ``installed`` selects the staged bundle client for packaging proof instead.
    """
    if installed:
        return resolve_client_path(installed=True)
    return deployed_client_path()


def _client_factory(adb: str, *, installed: bool) -> McpClient:
    base_env = dict(os.environ)
    base_env.update(_phone_runtime_env(adb))
    # The probe drives the MCP client directly (no agent env allowlist in the way),
    # so force text-only delivery here: the overlay assertion reads only structured
    # fields, and this keeps screenshot base64 off the stdio pipe.
    base_env["SKY_CUA_MODEL_SUPPORTS_IMAGES"] = "false"
    client_path = _overlay_client_path(installed=installed)
    client = McpClient([str(client_path), "mcp"], base_env=base_env, read_timeout=60.0)
    client.initialize()
    return client


# ---------------------------------------------------------------------------
# Overlay probe (independent device-level proof of the pointer overlay plane)
# ---------------------------------------------------------------------------


def overlay_probe(adb: str, serial: str, options: WorkflowSmokeOptions) -> StepResult:
    """Prove the phone-native agent pointer overlay plane is live and routed.

    Connects through a pure MCP probe, reads companion status, then runs the
    canonical overlay assertion (`overlay_step`): the companion advertises the
    native overlay plane, a screenshot reports it live, and a benign device-space
    tap + swipe route ``backend=companion`` (the overlay-gesture path). SKIPS with
    a named reason when the companion/overlay is unavailable; FAILS only when the
    overlay path is present but a gesture routes to the wrong backend.
    """
    client = _client_factory(adb, installed=options.installed)
    try:
        smoke = PhoneSmoke(client, PhoneSmokeOptions(profile="companion", serial=serial))
        connect = smoke.connect(serial)
        if result_is_error(connect):
            return step_skip(
                "overlay_probe",
                f"phone_connect failed: {first_diagnostic_message(connect) or 'no diagnostic'}",
            )
        session_id = structured(connect).get("session_id")
        if not isinstance(session_id, str) or not session_id:
            return step_skip("overlay_probe", "phone_connect returned no session_id")
        companion = smoke.companion_status(session_id)
        companion_caps = structured(companion).get("companion")
        companion_caps = companion_caps if isinstance(companion_caps, dict) else {}
        try:
            return overlay_step(smoke, session_id, companion_caps)
        except SmokeSkip as skip:
            return step_skip("overlay_probe", str(skip))
        except SmokeFailure as failure:
            return step_fail("overlay_probe", str(failure))
        finally:
            smoke.disconnect(session_id)
    finally:
        client.close()


# ---------------------------------------------------------------------------
# Agent workflow driver
# ---------------------------------------------------------------------------


def drive_workflow(
    workflow: Workflow,
    adb: str,
    serial: str,
    options: WorkflowSmokeOptions,
    artifact_dir: Path,
) -> list[StepResult]:
    """Run one workflow through the agent CLI and prove it by adb ground truth."""
    if options.agent != "claude" and shutil.which(options.agent) is None:
        return [step_skip(workflow.name, f"agent CLI '{options.agent}' not found on PATH")]

    # Surface phone runtime config to the agent's MCP child via the allowlisted
    # env (the agent forwards inherited env to its MCP server).
    for key, value in _phone_runtime_env(adb).items():
        os.environ[key] = value
    os.environ.setdefault("SKY_CUA_REPO_ROOT", str(REPO_ROOT))

    prompt = workflow.prompt(serial)
    (artifact_dir / "prompt.txt").write_text(prompt + "\n", encoding="utf-8")
    proc = run_agent(
        options.agent,
        prompt,
        artifact_dir,
        timeout=options.agent_timeout,
        model=options.model,
    )

    stdout_path = artifact_dir / f"{options.agent}.stdout.log"
    transcript = stdout_path.read_text(encoding="utf-8") if stdout_path.exists() else ""

    steps: list[StepResult] = []
    name = workflow.name
    if proc.returncode != 0:
        steps.append(
            step_fail(
                f"{name}.agent_run",
                f"{options.agent} exited rc={proc.returncode}; see {stdout_path.name}",
            )
        )
    else:
        steps.append(step_pass(f"{name}.agent_run", f"{options.agent} rc=0"))

    # Ground truth: the device's resumed activity, independent of any agent claim.
    # When the activity name alone fails to confirm the screen but the agent is in
    # the target app, fall back to an on-screen toolbar-title check (one uiautomator
    # dump, only on that path) so a generic host activity (e.g. .SubSettings) does
    # not produce a false negative.
    resumed = capture_resumed_activity(adb, serial)
    base = evaluate_foreground(workflow, resumed)
    fallback_title = title_fallback_substring(workflow, resumed)
    # Only pay for the uiautomator dump when the activity name failed but the agent
    # is in the target app (fallback_title is the narrowed, non-None screen title).
    title_visible = (
        screen_title_visible(adb, serial, fallback_title)
        if not base.ok and fallback_title is not None
        else False
    )
    foreground = resolve_foreground(workflow, resumed, title_visible=title_visible)
    if foreground.ok:
        steps.append(step_pass(f"{name}.ground_truth", foreground.detail))
    else:
        steps.append(step_fail(f"{name}.ground_truth", foreground.detail))

    # Tool evidence: confirm the agent drove the device with a real phone action
    # tool. ANY of the workflow's evidence tools suffices, so an efficient agent
    # that jumps to Settings via phone_open_settings (instead of tapping) still
    # passes. Soft unless --require-tool-evidence (claude runs plain-text, emitting
    # no structured tool events, so a hard gate there would be a false negative).
    invoked = phone_tools_invoked(transcript)
    evidence_hits = sorted(invoked.intersection(workflow.evidence_tools))
    evidence_detail = f"invoked={','.join(sorted(invoked)) or 'none'}"
    if evidence_hits:
        steps.append(
            step_pass(f"{name}.tool_evidence", f"{evidence_detail} via={','.join(evidence_hits)}")
        )
    else:
        accepted = ",".join(workflow.evidence_tools)
        detail = f"{evidence_detail} none of [{accepted}]"
        if options.require_tool_evidence:
            steps.append(step_fail(f"{name}.tool_evidence", detail))
        else:
            steps.append(step_skip(f"{name}.tool_evidence", detail))

    # Soft answer signal: a PASS means the agent *produced* the expected answer in
    # its transcript — not proof it read it off the page (web text is not in the
    # a11y tree and is GPU-garbled here, so the keyword may reflect the model's
    # prior knowledge). It is recorded for diagnostics and never gates the run: a
    # missing keyword is a named SKIP, never a failure.
    if workflow.answer_keyword is not None:
        if transcript_mentions_keyword(transcript, workflow.answer_keyword):
            steps.append(
                step_pass(
                    f"{name}.answer",
                    f"keyword='{workflow.answer_keyword}' produced by agent (soft)",
                )
            )
        else:
            steps.append(
                step_skip(
                    f"{name}.answer",
                    f"keyword='{workflow.answer_keyword}' not in transcript (web text unreadable)",
                )
            )
    return steps


# ---------------------------------------------------------------------------
# Orchestration
# ---------------------------------------------------------------------------


def run_workflow_smoke(
    options: WorkflowSmokeOptions,
    *,
    emit: Callable[[str], None] = print,
) -> list[StepResult]:
    """Run the selected workflows + overlay probe and emit step lines."""
    steps: list[StepResult] = []

    def record(result: StepResult) -> None:
        steps.append(result)
        emit(
            f"{result.status} {SMOKE_NAME}.{result.name}"
            + (f" {result.detail}" if result.detail.strip() else "")
        )

    try:
        adb = adb_binary()
        if adb is None:
            raise SetupSkip("adb not found on PATH or via SKY_CUA_ADB")
        # resolve_serial only reads ``.serial``; reuse the canonical resolver via a
        # SetupSmokeOptions carrying our serial selection.
        serial = resolve_serial(SetupSmokeOptions(serial=options.serial), adb)
        record(step_pass("device", f"serial={sanitize_serial(serial)}"))

        for workflow_name in workflows_for(options.selection):
            workflow = WORKFLOWS[workflow_name]
            artifact_dir = make_artifact_dir(options.agent, f"phone_workflow_{workflow_name}")
            for result in drive_workflow(workflow, adb, serial, options, artifact_dir):
                record(result)

        if options.skip_overlay_probe:
            record(step_skip("overlay_probe", "overlay probe disabled (--no-overlay-probe)"))
        else:
            record(overlay_probe(adb, serial, options))

    except SetupSkip as skip:
        record(step_skip(SMOKE_NAME, str(skip)))
    except SetupFailure as failure:
        record(step_fail(SMOKE_NAME, str(failure)))
    except Exception as error:  # never crash the run: surface as a FAIL line
        record(step_fail(SMOKE_NAME, bounded_metadata(f"unexpected error: {error}", max_len=180)))

    emit(format_result_line(steps))
    return steps


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Live agentic phone-use workflow smoke (settings + browser)."
    )
    parser.add_argument(
        "--workflow",
        choices=ALL_WORKFLOWS,
        default=WORKFLOW_FULL,
        help="Workflow to run. 'full' runs settings then browser.",
    )
    parser.add_argument(
        "--agent",
        choices=AGENT_CHOICES,
        default=DEFAULT_AGENT,
        help="Agent CLI that drives the workflow via the phone MCP tools.",
    )
    parser.add_argument("--model", default=None, help="Agent model override.")
    parser.add_argument("--serial", default=None, help="Device serial; auto-detected when single.")
    parser.add_argument(
        "--installed",
        action="store_true",
        help="Drive the staged bundle client for the overlay probe instead of the deployed runtime.",
    )
    parser.add_argument(
        "--require-tool-evidence",
        action="store_true",
        help="Fail a workflow when its required phone_* action tools are absent from the transcript.",
    )
    parser.add_argument(
        "--agent-timeout",
        type=float,
        default=300.0,
        help="Seconds to allow each agent workflow to run (default 300).",
    )
    parser.add_argument(
        "--no-overlay-probe",
        action="store_true",
        help="Skip the pointer-overlay MCP probe (e.g. on a device without the companion).",
    )
    return parser


def options_from_args(args: argparse.Namespace) -> WorkflowSmokeOptions:
    return WorkflowSmokeOptions(
        selection=args.workflow,
        agent=args.agent,
        model=args.model,
        serial=args.serial,
        installed=bool(args.installed),
        require_tool_evidence=bool(args.require_tool_evidence),
        agent_timeout=float(args.agent_timeout),
        skip_overlay_probe=bool(args.no_overlay_probe),
    )


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    options = options_from_args(args)
    results = run_workflow_smoke(options)
    _, _, failed = summarize_counts(results)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
