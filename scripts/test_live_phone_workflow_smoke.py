"""Unit tests for the phone-use workflow smoke's pure helpers.

These cover the adb resumed-activity parser, the per-workflow ground-truth
evaluation, the prompt builders, the soft answer check, and the CLI option
round-trip — all without a device, agent CLI, or MCP server.
"""

from __future__ import annotations

import live_phone_workflow_smoke as wf

# Real ``dumpsys activity activities`` resumed-activity lines (API 36 emulator).
LAUNCHER_DUMP = (
    "      topResumedActivity=ActivityRecord{91851576 u0 "
    "com.google.android.apps.nexuslauncher/.NexusLauncherActivity t2}\n"
    "  ResumedActivity: ActivityRecord{91851576 u0 "
    "com.google.android.apps.nexuslauncher/.NexusLauncherActivity t2}\n"
)
SETTINGS_ACCESSIBILITY_DUMP = (
    "      topResumedActivity=ActivityRecord{abc u0 "
    "com.android.settings/.Settings$AccessibilitySettingsActivity t5}\n"
)
SETTINGS_HOME_DUMP = (
    "      topResumedActivity=ActivityRecord{abc u0 com.android.settings/.Settings t5}\n"
)
CHROME_DUMP = (
    "  mResumedActivity: ActivityRecord{def u0 "
    "com.android.chrome/com.google.android.apps.chrome.Main t9}\n"
)


def test_parse_resumed_activity_settings_accessibility() -> None:
    resumed = wf.parse_resumed_activity(SETTINGS_ACCESSIBILITY_DUMP)
    assert resumed is not None
    assert resumed.package == "com.android.settings"
    assert resumed.activity == ".Settings$AccessibilitySettingsActivity"
    assert resumed.component == ("com.android.settings/.Settings$AccessibilitySettingsActivity")


def test_parse_resumed_activity_chrome_and_launcher() -> None:
    chrome = wf.parse_resumed_activity(CHROME_DUMP)
    assert chrome is not None and chrome.package == "com.android.chrome"
    launcher = wf.parse_resumed_activity(LAUNCHER_DUMP)
    assert launcher is not None
    assert launcher.package == "com.google.android.apps.nexuslauncher"


def test_parse_resumed_activity_prefers_top_resumed_marker() -> None:
    # When both topResumedActivity and a later marker disagree, the top marker wins.
    dump = (
        "      topResumedActivity=ActivityRecord{a u0 com.android.chrome/.Main t1}\n"
        "  ResumedActivity: ActivityRecord{b u0 com.android.settings/.Settings t2}\n"
    )
    resumed = wf.parse_resumed_activity(dump)
    assert resumed is not None and resumed.package == "com.android.chrome"


def test_parse_resumed_activity_none_when_absent() -> None:
    assert wf.parse_resumed_activity("no resumed activity here") is None
    assert wf.parse_resumed_activity("") is None


def test_parse_resumed_activity_equals_marker_form() -> None:
    # The ``mResumedActivity=`` (equals, no colon) marker form must also parse.
    dump = "  mResumedActivity=ActivityRecord{x u0 com.android.settings/.Settings t1}\n"
    resumed = wf.parse_resumed_activity(dump)
    assert resumed is not None and resumed.package == "com.android.settings"


def test_parse_resumed_activity_null_value_falls_through() -> None:
    # Mid-transition dumps print ``topResumedActivity=null``; that line must not
    # match, and a valid lower-preference marker should still win.
    assert wf.parse_resumed_activity("      topResumedActivity=null\n") is None
    dump = (
        "      topResumedActivity=null\n"
        "  mResumedActivity: ActivityRecord{y u0 com.android.chrome/.Main t2}\n"
    )
    resumed = wf.parse_resumed_activity(dump)
    assert resumed is not None and resumed.package == "com.android.chrome"


def test_evaluate_foreground_settings_requires_accessibility_screen() -> None:
    settings = wf.WORKFLOWS["settings"]
    on_screen = wf.evaluate_foreground(
        settings, wf.parse_resumed_activity(SETTINGS_ACCESSIBILITY_DUMP)
    )
    assert on_screen.ok
    # Reaching only the top-level Settings is not enough for the settings workflow.
    top_level = wf.evaluate_foreground(settings, wf.parse_resumed_activity(SETTINGS_HOME_DUMP))
    assert not top_level.ok
    assert "Accessibility" in top_level.detail


def test_evaluate_foreground_browser_reaches_chrome() -> None:
    browser = wf.WORKFLOWS["browser"]
    reached = wf.evaluate_foreground(browser, wf.parse_resumed_activity(CHROME_DUMP))
    assert reached.ok
    # The launcher is not Chrome.
    missed = wf.evaluate_foreground(browser, wf.parse_resumed_activity(LAUNCHER_DUMP))
    assert not missed.ok


def test_evaluate_foreground_none_resumed() -> None:
    check = wf.evaluate_foreground(wf.WORKFLOWS["settings"], None)
    assert not check.ok
    assert "no resumed activity" in check.detail


def test_dump_has_screen_title_matches_toolbar_content_desc() -> None:
    # The collapsing-toolbar title is the screen's content-desc node.
    dump = '<node content-desc="Accessibility" resource-id="...collapsing_toolbar"/>'
    assert wf.dump_has_screen_title(dump, "Accessibility")


def test_dump_has_screen_title_rejects_text_list_row() -> None:
    # A parent screen (e.g. top-level Settings) lists the child as a text row; that
    # must NOT count as reaching the screen — only the toolbar content-desc does.
    text_row = '<node text="Accessibility" resource-id="android:id/title"/>'
    assert not wf.dump_has_screen_title(text_row, "Accessibility")


def test_dump_has_screen_title_ignores_list_items_and_empty() -> None:
    # A list item like "Accessibility Menu" must not satisfy the exact title check.
    dump = '<node content-desc="Accessibility Menu" /><node text="Magnification" />'
    assert not wf.dump_has_screen_title(dump, "Accessibility")
    assert not wf.dump_has_screen_title("", "Accessibility")


def test_dump_has_screen_title_xml_escapes_title() -> None:
    # A screen name with XML-special chars matches the entity-escaped dump.
    dump = '<node content-desc="Display &amp; text" />'
    assert wf.dump_has_screen_title(dump, "Display & text")


def test_screen_title_visible_returns_false_on_adb_error(monkeypatch) -> None:
    # A hung/failed adb call must degrade to "title not confirmed", not raise and
    # collapse the whole workflow (the function documents this best-effort contract).
    import subprocess

    def boom(*_args, **_kwargs):
        raise subprocess.TimeoutExpired(cmd="adb", timeout=30)

    monkeypatch.setattr(wf, "_run_adb", boom)
    assert wf.screen_title_visible("adb", "emulator-5554", "Accessibility") is False


def test_title_fallback_substring_guards_on_package() -> None:
    settings = wf.WORKFLOWS["settings"]
    in_settings = wf.parse_resumed_activity(
        "  topResumedActivity=ActivityRecord{a u0 com.android.settings/.SubSettings t1}\n"
    )
    # In the target app with a finer screen check → the title to confirm.
    assert wf.title_fallback_substring(settings, in_settings) == "Accessibility"
    # Wrong app / launcher / no resumed activity → no fallback (package guard).
    assert wf.title_fallback_substring(settings, wf.parse_resumed_activity(LAUNCHER_DUMP)) is None
    assert wf.title_fallback_substring(settings, None) is None
    # A workflow without a finer screen check (browser) never uses the fallback.
    assert (
        wf.title_fallback_substring(wf.WORKFLOWS["browser"], wf.parse_resumed_activity(CHROME_DUMP))
        is None
    )


def test_resolve_foreground_upgrades_subsettings_via_title() -> None:
    settings = wf.WORKFLOWS["settings"]
    # Manual nav lands Accessibility under the generic .SubSettings host.
    subsettings = wf.parse_resumed_activity(
        "  topResumedActivity=ActivityRecord{a u0 com.android.settings/.SubSettings t1}\n"
    )
    # Base activity-name check fails, but the toolbar title confirms the screen.
    assert not wf.evaluate_foreground(settings, subsettings).ok
    upgraded = wf.resolve_foreground(settings, subsettings, title_visible=True)
    assert upgraded.ok
    assert "confirmed via uiautomator" in upgraded.detail
    # Without the title confirmation it stays a miss.
    assert not wf.resolve_foreground(settings, subsettings, title_visible=False).ok


def test_resolve_foreground_title_cannot_upgrade_wrong_app() -> None:
    settings = wf.WORKFLOWS["settings"]
    # On the launcher, a stray "Accessibility" title must never upgrade a miss:
    # the package guard rejects it.
    launcher = wf.parse_resumed_activity(
        "  topResumedActivity=ActivityRecord{a u0 "
        "com.google.android.apps.nexuslauncher/.NexusLauncherActivity t2}\n"
    )
    assert not wf.resolve_foreground(settings, launcher, title_visible=True).ok
    assert wf.resolve_foreground(settings, None, title_visible=True).ok is False


def test_resolve_foreground_passthrough_when_base_ok() -> None:
    settings = wf.WORKFLOWS["settings"]
    resumed = wf.parse_resumed_activity(SETTINGS_ACCESSIBILITY_DUMP)
    # Base check already passes; title_visible is irrelevant and detail is unchanged.
    result = wf.resolve_foreground(settings, resumed, title_visible=False)
    assert result.ok
    assert "confirmed via uiautomator" not in result.detail


def test_settings_prompt_names_serial_and_constraints() -> None:
    prompt = wf.WORKFLOWS["settings"].prompt("emulator-5554")
    assert "emulator-5554" in prompt
    assert "Accessibility" in prompt
    assert "do NOT shell out to adb" in prompt
    assert "phone_tap" in prompt


def test_browser_prompt_names_query_and_constraints() -> None:
    prompt = wf.WORKFLOWS["browser"].prompt("emulator-5554")
    assert "emulator-5554" in prompt
    assert wf.BROWSER_QUERY in prompt
    assert "do NOT shell out to adb" in prompt
    assert "phone_type_text" in prompt


def test_transcript_mentions_keyword_case_insensitive() -> None:
    assert wf.transcript_mentions_keyword("The answer is Mount EVEREST.", "everest")
    assert not wf.transcript_mentions_keyword("K2 is the second tallest.", "everest")


def test_workflows_for_expands_full() -> None:
    assert wf.workflows_for("full") == ("settings", "browser")
    assert wf.workflows_for("settings") == ("settings",)
    assert wf.workflows_for("browser") == ("browser",)


def test_format_result_line_counts() -> None:
    results = [
        wf.step_pass("device", "serial=emulator-5554"),
        wf.step_pass("settings.ground_truth", "foreground=ok"),
        wf.step_skip("browser.answer", "keyword not present"),
        wf.step_fail("settings.agent_run", "rc=1"),
    ]
    line = wf.format_result_line(results)
    assert line == "RESULT phone_workflow_smoke passed=2 skipped=1 failed=1"


def test_options_from_args_round_trip() -> None:
    parser = wf.build_parser()
    args = parser.parse_args(
        [
            "--workflow",
            "browser",
            "--agent",
            "pi",
            "--serial",
            "emulator-5554",
            "--require-tool-evidence",
            "--no-overlay-probe",
            "--agent-timeout",
            "120",
        ]
    )
    options = wf.options_from_args(args)
    assert options.selection == "browser"
    assert options.agent == "pi"
    assert options.serial == "emulator-5554"
    assert options.require_tool_evidence is True
    assert options.skip_overlay_probe is True
    assert options.agent_timeout == 120.0


def test_default_options() -> None:
    options = wf.options_from_args(wf.build_parser().parse_args([]))
    assert options.selection == wf.WORKFLOW_FULL
    assert options.agent == "claude"
    assert options.installed is False
    assert options.require_tool_evidence is False
    assert options.skip_overlay_probe is False


def test_evidence_tools_are_real_phone_tools() -> None:
    # Guard against typos: every evidence tool must be a phone_* action tool, and
    # each workflow accepts at least one (any-of semantics).
    for workflow in wf.WORKFLOWS.values():
        assert workflow.evidence_tools, workflow.name
        for tool in workflow.evidence_tools:
            assert tool.startswith("phone_"), tool


def test_evidence_accepts_efficient_actuation_paths() -> None:
    # Efficient agents reach a screen via phone_open_settings or phone_app_launch
    # (with an intent) instead of tapping — both must count as driving evidence,
    # for every workflow.
    for workflow in wf.WORKFLOWS.values():
        assert "phone_open_settings" in workflow.evidence_tools
        assert "phone_app_launch" in workflow.evidence_tools
        assert "phone_tap" in workflow.evidence_tools


def test_evidence_tools_exclude_observation_only_tools() -> None:
    # Pure observation tools must not satisfy the "agent drove the device" gate.
    for observation in ("phone_connect", "phone_screenshot", "phone_accessibility_tree"):
        assert observation not in wf.PHONE_DRIVE_TOOLS


def test_run_workflow_smoke_skips_without_adb(monkeypatch) -> None:
    # No adb on PATH collapses the whole run to one SKIP line plus the RESULT line.
    monkeypatch.setattr(wf, "adb_binary", lambda: None)
    emitted: list[str] = []
    results = wf.run_workflow_smoke(wf.WorkflowSmokeOptions(), emit=emitted.append)
    assert any(r.skipped and r.name == wf.SMOKE_NAME for r in results)
    assert emitted[-1] == "RESULT phone_workflow_smoke passed=0 skipped=1 failed=0"
