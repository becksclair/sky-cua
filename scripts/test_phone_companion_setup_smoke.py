"""Unit tests for the phone-companion setup smoke's pure helpers.

These cover the tool-evidence parsing, the device-side ground-truth parsers, the
safe cold-reset list math, and the proof-condition logic — all without a device,
agent CLI, or MCP server.
"""

from __future__ import annotations

import json

import live_phone_companion_setup_smoke as setup


def test_agent_setup_prompt_names_serial_and_required_tools() -> None:
    prompt = setup.agent_setup_prompt("emulator-5554")
    assert "emulator-5554" in prompt
    for tool in setup.REQUIRED_AGENT_TOOLS:
        assert tool in prompt
    assert "do not shell out to adb" in prompt


def test_phone_tool_base_name_strips_namespaces() -> None:
    assert setup._phone_tool_base_name("phone_connect") == "phone_connect"
    assert setup._phone_tool_base_name("mcp__sky-cua__phone_connect") == "phone_connect"
    assert setup._phone_tool_base_name("mcp__sky_cua__phone_install_companion") == (
        "phone_install_companion"
    )
    assert setup._phone_tool_base_name("sky_cua_phone_observe") == "phone_observe"
    # opencode names MCP tools with the server's own spelling (hyphen possible).
    assert setup._phone_tool_base_name("sky-cua_phone_connect") == "phone_connect"
    assert setup._phone_tool_base_name("computer-use_phone_status") == "phone_status"
    assert setup._phone_tool_base_name("click") is None
    assert setup._phone_tool_base_name("mcp__sky-cua__click") is None


def test_redaction_keeps_phone_tool_names() -> None:
    import _agent_mcp_smoke

    # Tool names are non-sensitive: the redaction must keep phone tool identities
    # (any namespace spelling) so a phone smoke can prove the agent used them.
    for name in (
        "phone_connect",
        "sky_cua_phone_install_companion",
        "sky-cua_phone_connect",
        "mcp__sky-cua__phone_companion_status",
    ):
        assert _agent_mcp_smoke.safe_tool_identity_field("tool", name), name
    assert not _agent_mcp_smoke.safe_tool_identity_field("tool", "bash")
    assert not _agent_mcp_smoke.safe_tool_identity_field("tool", "read")


def test_phone_tools_invoked_parses_jsonl_events() -> None:
    transcript = "\n".join(
        [
            json.dumps({"type": "tool_use_start", "toolName": "mcp__sky-cua__phone_connect"}),
            "not json at all",
            json.dumps({"type": "tool", "tool": {"name": "phone_install_companion"}}),
            json.dumps({"type": "agentMessage", "name": "phone_status"}),
        ]
    )
    invoked = setup.phone_tools_invoked(transcript)
    assert invoked == {"phone_connect", "phone_install_companion", "phone_status"}


def test_phone_tools_invoked_parses_whole_document_json() -> None:
    transcript = json.dumps(
        {"events": [{"toolName": "phone_connect"}, {"tool_name": "phone_install_companion"}]}
    )
    assert setup.phone_tools_invoked(transcript) == {"phone_connect", "phone_install_companion"}


def test_phone_tools_invoked_empty_when_no_phone_tools() -> None:
    transcript = json.dumps({"type": "tool", "name": "click"}) + "\nplain text\n"
    assert setup.phone_tools_invoked(transcript) == set()


def test_proc_net_has_listening_port_matches_rpc_port() -> None:
    proc_net = (
        "  sl  local_address rem_address   st\n"
        "   0: 0100007F:BA43 00000000:0000 0A 00000000:00000000\n"
        "   1: 0100007F:1F90 00000000:0000 0A 00000000:00000000\n"
    )
    assert setup.proc_net_has_listening_port(proc_net, "BA43")
    assert not setup.proc_net_has_listening_port(proc_net, "BA44")


def test_proc_net_has_listening_port_case_insensitive() -> None:
    proc_net = "   0: 0100007F:ba43 00000000:0000 0A\n"
    assert setup.proc_net_has_listening_port(proc_net, "BA43")


def test_accessibility_service_bound() -> None:
    dump = (
        "     Enabled services:{{com.skycua.phonecompanion/"
        "com.skycua.phonecompanion.service.SkyAccessibilityService}}\n"
    )
    assert setup.accessibility_service_bound(dump, setup.ACCESSIBILITY_COMPONENT)
    assert not setup.accessibility_service_bound(
        "Enabled services:{}", setup.ACCESSIBILITY_COMPONENT
    )


def test_notification_listener_bound() -> None:
    dump = (
        "      ComponentInfo{com.skycua.phonecompanion/"
        "com.skycua.phonecompanion.service.SkyNotificationListenerService} (user 0): "
        "android.service.notification.INotificationListener$Stub$Proxy@d86af2f\n"
    )
    assert setup.notification_listener_bound(dump, setup.NOTIFICATION_COMPONENT)
    assert not setup.notification_listener_bound("nothing here", setup.NOTIFICATION_COMPONENT)


def test_accessibility_list_without_removes_only_our_component() -> None:
    other = "com.google.android.marvin.talkback/com.google.android.marvin.talkback.TalkBackService"
    existing = f"{other}:{setup.ACCESSIBILITY_COMPONENT}"
    assert setup.accessibility_list_without(existing, setup.ACCESSIBILITY_COMPONENT) == other


def test_accessibility_list_without_handles_null_and_empty() -> None:
    assert setup.accessibility_list_without("null", setup.ACCESSIBILITY_COMPONENT) == ""
    assert setup.accessibility_list_without("", setup.ACCESSIBILITY_COMPONENT) == ""
    assert (
        setup.accessibility_list_without(
            setup.ACCESSIBILITY_COMPONENT, setup.ACCESSIBILITY_COMPONENT
        )
        == ""
    )


def test_accessibility_list_without_matches_short_form() -> None:
    short = f"{setup.COMPANION_PACKAGE}/.service.SkyAccessibilityService"
    assert setup.accessibility_list_without(short, setup.ACCESSIBILITY_COMPONENT) == ""


def test_companion_setup_complete() -> None:
    complete = {
        "installed": True,
        "rpc_reachable": True,
        "accessibility_enabled": True,
        "notification_listener_enabled": True,
    }
    assert setup.companion_setup_complete(complete).ok
    partial = {**complete, "rpc_reachable": False, "accessibility_enabled": False}
    check = setup.companion_setup_complete(partial)
    assert not check.ok
    assert set(check.missing) == {"rpc_reachable", "accessibility_enabled"}


def test_ground_truth_setup_complete_and_missing() -> None:
    good = setup.GroundTruth(
        companion_installed=True,
        companion_version="0.1.1",
        accessibility_bound=True,
        notification_bound=True,
        rpc_listening=True,
    )
    assert good.setup_complete
    assert good.missing == ()
    bad = setup.GroundTruth(
        companion_installed=True,
        companion_version=None,
        accessibility_bound=False,
        notification_bound=True,
        rpc_listening=False,
    )
    assert not bad.setup_complete
    assert set(bad.missing) == {"accessibility_bound", "rpc_listening"}


def test_format_result_line() -> None:
    results = [
        setup.step_pass("device", "serial=emulator-5554"),
        setup.step_skip("agent_tool_evidence", "missing=phone_install_companion"),
        setup.step_fail("ground_truth", "missing=rpc_listening"),
    ]
    line = setup.format_result_line(results)
    assert line == "RESULT phone_companion_setup_smoke passed=1 skipped=1 failed=1"


def test_resolve_serial_prefers_explicit() -> None:
    options = setup.SetupSmokeOptions(serial="emulator-5554")
    assert setup.resolve_serial(options, "adb") == "emulator-5554"


def test_resolve_serial_single_device(monkeypatch) -> None:
    def fake_run(*_args, **_kwargs):
        return type("P", (), {"stdout": "List of devices attached\nemulator-5554\tdevice\n"})()

    monkeypatch.setattr(setup.subprocess, "run", fake_run)
    assert setup.resolve_serial(setup.SetupSmokeOptions(), "adb") == "emulator-5554"


def test_resolve_serial_skips_when_none_or_ambiguous(monkeypatch) -> None:
    def none_run(*_args, **_kwargs):
        return type("P", (), {"stdout": "List of devices attached\n"})()

    monkeypatch.setattr(setup.subprocess, "run", none_run)
    import pytest

    with pytest.raises(setup.SetupSkip, match="no authorized adb device"):
        setup.resolve_serial(setup.SetupSmokeOptions(), "adb")

    def two_run(*_args, **_kwargs):
        return type(
            "P",
            (),
            {"stdout": "List of devices attached\nemulator-5554\tdevice\n1.2.3.4:5555\tdevice\n"},
        )()

    monkeypatch.setattr(setup.subprocess, "run", two_run)
    with pytest.raises(setup.SetupSkip, match="2 devices attached"):
        setup.resolve_serial(setup.SetupSmokeOptions(), "adb")


def test_options_from_args_round_trip() -> None:
    parser = setup.build_parser()
    args = parser.parse_args(
        ["--driver", "direct", "--cold-reset", "services", "--serial", "x", "--no-build"]
    )
    options = setup.options_from_args(args)
    assert options.driver == "direct"
    assert options.cold_reset == "services"
    assert options.serial == "x"
    assert options.build_companion is False
    assert options.agent == setup.DEFAULT_AGENT


def test_default_options() -> None:
    options = setup.options_from_args(setup.build_parser().parse_args([]))
    assert options.driver == setup.DRIVER_AGENT
    assert options.agent == "claude"
    assert options.cold_reset == setup.COLD_RESET_FULL
    assert options.build_companion is True
