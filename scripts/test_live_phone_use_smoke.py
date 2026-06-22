"""Tests for the phone-use live smoke harness (no real device required).

Covers the profile registry and skip-reason logic, PASS/SKIP/FAIL/RESULT line
formatting, argparse wiring, sanitization, and the MCP result parsing helpers,
all with fakes/monkeypatch for the MCP transport so nothing touches adb, scrcpy,
or a phone.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

import pytest

import live_phone_use_smoke as smoke

# ---------------------------------------------------------------------------
# Fakes
# ---------------------------------------------------------------------------


class FakeClient:
    """Stand-in for McpClient: records tools_call invocations, returns canned maps."""

    def __init__(
        self,
        responses: dict[str, dict[str, Any]] | None = None,
        *,
        tools: list[dict[str, Any]] | None = None,
    ) -> None:
        self.responses = responses or {}
        self.tools = tools or []
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.initialized = False
        self.closed = False

    def initialize(self) -> None:
        self.initialized = True

    def close(self) -> None:
        self.closed = True

    def tools_list(self) -> list[dict[str, Any]]:
        return self.tools

    def tools_call(self, request_id: int, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        self.calls.append((name, arguments))
        return self.responses.get(name, {"structuredContent": {}, "isError": False})


def ok(structured: dict[str, Any]) -> dict[str, Any]:
    return {"structuredContent": structured, "isError": False}


def err(structured: dict[str, Any]) -> dict[str, Any]:
    return {"structuredContent": structured, "isError": True}


def compact_ok(
    tool: str, branch: str, legacy_tool: str, structured: dict[str, Any]
) -> dict[str, Any]:
    return {
        "structuredContent": {
            "profile": "compact",
            "tool": tool,
            "branch": branch,
            "legacy_tool": legacy_tool,
            "result": structured,
        },
        "isError": False,
    }


def make_smoke(
    responses: dict[str, dict[str, Any]] | None = None,
    *,
    profile: str = smoke.PROFILE_FALLBACK,
    serial: str | None = None,
    wireless_host: str | None = None,
    pair_host: str | None = None,
    pairing_code: str | None = None,
    tool_profile: str = smoke.TOOL_PROFILE_LEGACY,
) -> tuple[smoke.PhoneSmoke, FakeClient]:
    client = FakeClient(responses)
    options = smoke.PhoneSmokeOptions(
        profile=profile,
        serial=serial,
        wireless_host=wireless_host,
        pair_host=pair_host,
        pairing_code=pairing_code,
        tool_profile=tool_profile,
    )
    return smoke.PhoneSmoke(client, options), client  # pyright: ignore[reportArgumentType]


# ---------------------------------------------------------------------------
# Line formatting
# ---------------------------------------------------------------------------


def test_format_step_line_includes_status_profile_and_name() -> None:
    line = smoke.format_step_line(
        "adb-usb", smoke.step_pass("phone_connect", "serial=emulator-5554")
    )
    assert line == "PASS adb-usb.phone_connect serial=emulator-5554"


def test_format_step_line_without_detail() -> None:
    assert smoke.format_step_line("fallback", smoke.step_pass("x")) == "PASS fallback.x"


def test_format_skip_and_fail_lines() -> None:
    skip = smoke.format_step_line("companion", smoke.step_skip("companion", "no device"))
    fail = smoke.format_step_line("adb-usb", smoke.step_fail("phone_tap", "boom"))
    assert skip == "SKIP companion.companion no device"
    assert fail == "FAIL adb-usb.phone_tap boom"


def test_summarize_counts() -> None:
    results = [
        smoke.step_pass("a"),
        smoke.step_pass("b"),
        smoke.step_skip("c", "r"),
        smoke.step_fail("d", "x"),
    ]
    assert smoke.summarize_counts(results) == (2, 1, 1)


def test_format_result_line_matches_plan_sample_shape() -> None:
    results = [smoke.step_pass("a"), smoke.step_skip("b", "r"), smoke.step_fail("c", "x")]
    assert (
        smoke.format_result_line(results)
        == "RESULT full_phone_use_smoke passed=1 skipped=1 failed=1"
    )


def test_format_result_line_all_pass_zero_failed() -> None:
    results = [smoke.step_pass("a"), smoke.step_pass("b")]
    assert smoke.format_result_line(results) == (
        "RESULT full_phone_use_smoke passed=2 skipped=0 failed=0"
    )


# ---------------------------------------------------------------------------
# Sanitization
# ---------------------------------------------------------------------------


def test_sanitize_serial_collapses_host_port() -> None:
    assert smoke.sanitize_serial("172.16.255.58:38781") == "172-16-255-58-38781"


def test_sanitize_serial_handles_none_and_empty() -> None:
    assert smoke.sanitize_serial(None) == "unknown"
    assert smoke.sanitize_serial("") == "unknown"
    assert smoke.sanitize_serial(":::") == "unknown"


def test_sanitize_serial_truncates_long_values() -> None:
    assert len(smoke.sanitize_serial("a" * 200)) == 48


def test_bounded_metadata_bools_and_none() -> None:
    assert smoke.bounded_metadata(None) == "none"
    assert smoke.bounded_metadata(True) == "true"
    assert smoke.bounded_metadata(False) == "false"


def test_bounded_metadata_collapses_whitespace_and_truncates() -> None:
    assert smoke.bounded_metadata("a\n  b\tc") == "a b c"
    assert smoke.bounded_metadata("x" * 200, max_len=10) == "x" * 10


# ---------------------------------------------------------------------------
# MCP result parsing helpers
# ---------------------------------------------------------------------------


def test_result_is_error() -> None:
    assert smoke.result_is_error({"isError": True}) is True
    assert smoke.result_is_error({"isError": False}) is False
    assert smoke.result_is_error({}) is False


def test_structured_returns_map_or_empty() -> None:
    assert smoke.structured({"structuredContent": {"a": 1}}) == {"a": 1}
    assert smoke.structured({"structuredContent": ["not", "a", "map"]}) == {}
    assert smoke.structured({}) == {}


def test_diagnostic_codes_extracted() -> None:
    result = {
        "structuredContent": {
            "diagnostics": [
                {"code": "PhoneDeviceUnavailable", "message": "x"},
                {"message": "no code here"},
                "garbage",
            ]
        }
    }
    assert smoke.diagnostic_codes(result) == ["PhoneDeviceUnavailable"]


def test_first_diagnostic_message_is_bounded() -> None:
    result = {"structuredContent": {"diagnostics": [{"code": "X", "message": "  the   message  "}]}}
    assert smoke.first_diagnostic_message(result) == "the message"


def test_first_diagnostic_message_empty_when_absent() -> None:
    assert smoke.first_diagnostic_message({"structuredContent": {}}) == ""


def test_tool_names_from_raw_tools_list() -> None:
    assert smoke.tool_names_from_list([{"name": "phone_observe"}, {"name": "get_app_state"}]) == {
        "phone_observe",
        "get_app_state",
    }


def test_require_expected_phone_tools_passes_with_complete_surface() -> None:
    client = FakeClient(tools=[{"name": name} for name in smoke.EXPECTED_PHONE_TOOLS])
    result = smoke.require_expected_phone_tools(client)  # pyright: ignore[reportArgumentType]
    assert result.passed
    assert "phone_tools=" in result.detail


def test_require_expected_phone_tools_passes_with_compact_surface() -> None:
    client = FakeClient(tools=[{"name": name} for name in smoke.COMPACT_EXPECTED_PHONE_TOOLS])
    result = smoke.require_expected_phone_tools(
        client,  # pyright: ignore[reportArgumentType]
        tool_profile=smoke.TOOL_PROFILE_COMPACT,
    )
    assert result.passed
    assert "compact_phone_tools=" in result.detail


def test_require_expected_phone_tools_fails_when_missing() -> None:
    client = FakeClient(tools=[{"name": "phone_observe"}])
    with pytest.raises(smoke.SmokeFailure, match="missing phone tools"):
        smoke.require_expected_phone_tools(client)  # pyright: ignore[reportArgumentType]


def test_compact_smoke_maps_phone_calls_and_unwraps_results() -> None:
    driver, client = make_smoke(
        {
            "status": compact_ok(
                "status",
                "phone",
                "phone_status",
                {"adb_available": True, "devices": [{"serial": "emulator-5554"}]},
            )
        },
        tool_profile=smoke.TOOL_PROFILE_COMPACT,
    )

    result = driver.status(refresh_devices=True)

    assert smoke.structured(result)["adb_available"] is True
    assert client.calls == [
        ("status", {"refresh_devices": True, "component": "phone"}),
    ]


def test_compact_smoke_maps_phone_actions() -> None:
    driver, client = make_smoke(tool_profile=smoke.TOOL_PROFILE_COMPACT)

    driver.connect("emulator-5554")
    driver.screenshot("session")
    driver.tap("session", "snap", 1.0, 2.0)
    driver.notification_reply("session", "event", "action", "reply")

    assert client.calls == [
        ("phone_connection", {"serial": "emulator-5554", "operation": "connect"}),
        ("capture_screen", {"session_id": "session", "surface": "phone"}),
        (
            "phone_pointer",
            {
                "x": 1.0,
                "y": 2.0,
                "session_id": "session",
                "phone_snapshot_id": "snap",
                "operation": "tap",
            },
        ),
        (
            "phone_notification_reply",
            {
                "event_id": "event",
                "action_id": "action",
                "text": "reply",
                "session_id": "session",
            },
        ),
    ]


# ---------------------------------------------------------------------------
# Profile registry
# ---------------------------------------------------------------------------


def test_profile_registry_covers_all_non_full_profiles() -> None:
    assert set(smoke.PROFILES) == set(smoke.FULL_PROFILE_SEQUENCE)


def test_full_is_not_in_the_registry() -> None:
    assert smoke.PROFILE_FULL not in smoke.PROFILES


def test_profiles_for_full_expands_to_every_profile() -> None:
    assert smoke.profiles_for(smoke.PROFILE_FULL) == smoke.FULL_PROFILE_SEQUENCE


def test_profiles_for_single_profile() -> None:
    assert smoke.profiles_for(smoke.PROFILE_COMPANION) == (smoke.PROFILE_COMPANION,)


def test_all_profiles_includes_full() -> None:
    assert smoke.PROFILE_FULL in smoke.ALL_PROFILES
    for name in smoke.FULL_PROFILE_SEQUENCE:
        assert name in smoke.ALL_PROFILES


# ---------------------------------------------------------------------------
# Prerequisite probing / skip-reason logic
# ---------------------------------------------------------------------------


def test_require_devices_or_skip_skips_without_adb(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: None)
    driver, _ = make_smoke()
    with pytest.raises(smoke.SmokeSkip, match="adb not found"):
        smoke.require_devices_or_skip(driver)


def test_require_devices_or_skip_skips_without_authorized_device(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {"phone_list_devices": ok({"devices": [{"serial": "x", "state": "unauthorized"}]})}
    )
    with pytest.raises(smoke.SmokeSkip, match="no authorized adb device"):
        smoke.require_devices_or_skip(driver)


def test_require_devices_or_skip_returns_authorized_devices(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_list_devices": ok(
                {
                    "devices": [
                        {"serial": "good", "state": "device"},
                        {"serial": "bad", "state": "offline"},
                    ]
                }
            )
        }
    )
    devices = smoke.require_devices_or_skip(driver)
    assert [device["serial"] for device in devices] == ["good"]


def test_choose_serial_prefers_explicit_serial() -> None:
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_ADB_USB, serial="chosen")
    devices = [{"serial": "device-default", "connection_kind": "usb"}]
    assert smoke.choose_serial(options, devices, wireless=False) == "chosen"


def test_choose_serial_falls_back_to_first_device() -> None:
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_ADB_USB)
    devices = [{"serial": "device-default", "connection_kind": "usb"}]
    assert smoke.choose_serial(options, devices, wireless=False) == "device-default"


def test_choose_serial_wireless_prefers_wireless_host() -> None:
    options = smoke.PhoneSmokeOptions(
        profile=smoke.PROFILE_ADB_WIRELESS, wireless_host="10.0.0.5:5555"
    )
    devices = [{"serial": "usb-serial", "connection_kind": "usb"}]
    assert smoke.choose_serial(options, devices, wireless=True) == "10.0.0.5:5555"


def test_choose_serial_wireless_finds_wireless_device() -> None:
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_ADB_WIRELESS)
    devices = [
        {"serial": "usb-serial", "connection_kind": "usb"},
        {"serial": "10.0.0.9:5555", "connection_kind": "wireless"},
    ]
    assert smoke.choose_serial(options, devices, wireless=True) == "10.0.0.9:5555"


def test_choose_serial_wireless_skips_without_wireless() -> None:
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_ADB_WIRELESS)
    devices = [{"serial": "usb-serial", "connection_kind": "usb"}]
    with pytest.raises(smoke.SmokeSkip, match="no wireless adb device"):
        smoke.choose_serial(options, devices, wireless=True)


# ---------------------------------------------------------------------------
# connect_session
# ---------------------------------------------------------------------------


def test_connect_session_returns_session_id() -> None:
    driver, _ = make_smoke({"phone_connect": ok({"session_id": "sess-1"})})
    session_id, _ = smoke.connect_session(driver, "serial-1")
    assert session_id == "sess-1"


def test_connect_session_raises_on_error() -> None:
    driver, _ = make_smoke(
        {"phone_connect": err({"diagnostics": [{"code": "X", "message": "nope"}]})}
    )
    with pytest.raises(smoke.SmokeFailure, match="phone_connect failed"):
        smoke.connect_session(driver, "serial-1")


def test_connect_session_raises_without_session_id() -> None:
    driver, _ = make_smoke({"phone_connect": ok({})})
    with pytest.raises(smoke.SmokeFailure, match="no session_id"):
        smoke.connect_session(driver, "serial-1")


# ---------------------------------------------------------------------------
# Profiles via run_profile (skip/fail collapsing)
# ---------------------------------------------------------------------------


def test_run_profile_collapses_skip_to_single_line(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: None)
    driver, _ = make_smoke()
    results = smoke.run_profile(
        driver,
        smoke.PROFILE_COMPANION,
        driver._options,  # pyright: ignore[reportPrivateUsage]
    )
    assert len(results) == 1
    assert results[0].skipped
    assert "adb not found" in results[0].detail


def test_run_profile_collapses_unexpected_error_to_fail() -> None:
    def boom(_smoke: smoke.PhoneSmoke, _options: smoke.PhoneSmokeOptions) -> list[Any]:
        raise ValueError("kaboom")

    driver, _ = make_smoke()
    with pytest.MonkeyPatch().context() as patch:
        patch.setitem(smoke.PROFILES, smoke.PROFILE_FALLBACK, boom)
        results = smoke.run_profile(driver, smoke.PROFILE_FALLBACK, driver._options)  # pyright: ignore[reportPrivateUsage]
    assert len(results) == 1
    assert results[0].failed
    assert "unexpected error" in results[0].detail


def test_profile_fallback_passes_with_status_fields() -> None:
    driver, _ = make_smoke(
        {
            "phone_status": ok(
                {
                    "adb_available": True,
                    "scrcpy_available": False,
                    "companion_enabled": True,
                }
            )
        }
    )
    results = smoke.profile_fallback(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    assert len(results) == 1
    assert results[0].passed
    assert "adb=true" in results[0].detail


def test_profile_fallback_fails_when_status_field_missing() -> None:
    driver, _ = make_smoke({"phone_status": ok({"scrcpy_available": True})})
    with pytest.raises(smoke.SmokeFailure, match="omitted the adb_available"):
        smoke.profile_fallback(driver, driver._options)  # pyright: ignore[reportPrivateUsage]


def test_overlay_step_fails_when_gesture_falls_back_to_adb() -> None:
    driver, client = make_smoke(
        {
            "phone_screenshot": ok(
                {
                    "phone_snapshot_id": "snap-1",
                    "cursor_capabilities": {"phone_native_overlay": True},
                }
            ),
            "phone_tap": ok({"backend": "adb"}),
            "phone_swipe": ok({"backend": "companion"}),
        }
    )
    with pytest.raises(smoke.SmokeFailure, match="overlay tap used adb"):
        smoke._overlay_step(  # pyright: ignore[reportPrivateUsage]
            driver,
            "sess-1",
            {"rpc_reachable": True, "native_overlay": True},
        )
    tap_call = next(args for name, args in client.calls if name == "phone_tap")
    assert tap_call["use_device_coordinates"] is True


def test_profile_adversarial_passes_when_surface_rejects(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_connect": err({"diagnostics": [{"code": "X", "message": "no"}]}),
            "phone_tap": err({"diagnostics": [{"code": "Y", "message": "stale"}]}),
        }
    )
    results = smoke.profile_adversarial(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    by_name = {step.name: step for step in results}
    assert by_name["wrong_serial_rejected"].passed
    assert by_name["stale_snapshot_rejected"].passed
    # The drift steps skip (named) because no device-change flags were passed.
    assert by_name["orientation_mismatch_rejected"].skipped
    assert by_name["resolution_mismatch_rejected"].skipped
    assert not any(step.failed for step in results)


def test_profile_adversarial_fails_when_bogus_serial_accepted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_connect": ok({"session_id": "leaked"}),
            "phone_tap": err({"diagnostics": [{"code": "Y", "message": "stale"}]}),
        }
    )
    results = smoke.profile_adversarial(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    statuses = [step.status for step in results]
    assert smoke.StepStatus.FAIL in statuses


def test_profile_adversarial_skips_without_adb(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: None)
    driver, _ = make_smoke()
    with pytest.raises(smoke.SmokeSkip, match="adb not found"):
        smoke.profile_adversarial(driver, driver._options)  # pyright: ignore[reportPrivateUsage]


def test_profile_pair_wireless_skips_without_endpoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke()
    with pytest.raises(smoke.SmokeSkip, match="pairing endpoint"):
        smoke.profile_pair_wireless(driver, driver._options)  # pyright: ignore[reportPrivateUsage]


def test_profile_pair_wireless_skips_without_code(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(profile=smoke.PROFILE_PAIR_WIRELESS, pair_host="h:1")
    with pytest.raises(smoke.SmokeSkip, match="pairing code"):
        smoke.profile_pair_wireless(driver, driver._options)  # pyright: ignore[reportPrivateUsage]


def test_profile_pair_wireless_never_echoes_code(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, client = make_smoke(
        {"phone_pair_wireless": ok({"paired": True})},
        profile=smoke.PROFILE_PAIR_WIRELESS,
        pair_host="172.16.0.9:37000",
        pairing_code="123456",
    )
    results = smoke.profile_pair_wireless(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    assert results[0].passed
    # The code is sent to the tool, but never appears in any printed line.
    assert "123456" not in smoke.format_step_line("pair-wireless", results[0])
    sent_args = next(args for name, args in client.calls if name == "phone_pair_wireless")
    assert sent_args["pairing_code"] == "123456"


def test_profile_scrcpy_skips_without_scrcpy(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(smoke, "scrcpy_binary", lambda: None)
    driver, _ = make_smoke()
    with pytest.raises(smoke.SmokeSkip, match="scrcpy not found"):
        smoke.profile_scrcpy(driver, driver._options)  # pyright: ignore[reportPrivateUsage]


def test_profile_companion_skips_when_not_installed(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_list_devices": ok({"devices": [{"serial": "d", "state": "device"}]}),
            "phone_connect": ok({"session_id": "s"}),
            "phone_companion_status": ok({"companion": {"installed": False}}),
            "phone_disconnect": ok({"disconnected": True}),
        }
    )
    with pytest.raises(smoke.SmokeSkip, match="companion app not installed"):
        smoke.profile_companion(driver, driver._options)  # pyright: ignore[reportPrivateUsage]


def test_profile_adb_usb_full_pass_path(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_status": ok({"adb_available": True, "devices": [{"serial": "d"}]}),
            "phone_list_devices": ok({"devices": [{"serial": "d", "state": "device"}]}),
            "phone_connect": ok({"session_id": "s"}),
            "phone_observe": ok(
                {"capability_profile_id": "prof-1", "available_actions": [1, 2, 3]}
            ),
            "phone_screenshot": ok(
                {
                    "phone_snapshot_id": "snap-1",
                    "device_size": {"width": 1080, "height": 2400},
                }
            ),
            "phone_tap": ok({}),
            "phone_app_current": ok({"current_app": {"package_name": "com.android.chrome"}}),
            "phone_app_list": ok({"apps": [1, 2]}),
            "phone_disconnect": ok({"disconnected": True}),
        },
        profile=smoke.PROFILE_ADB_USB,
    )
    results = smoke.profile_adb_usb(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    names = [step.name for step in results]
    assert "phone_connect" in names
    assert "phone_screenshot" in names
    assert "phone_disconnect" in names
    assert all(step.passed for step in results)


# ---------------------------------------------------------------------------
# run_smoke end-to-end with a fake client
# ---------------------------------------------------------------------------


def test_run_smoke_emits_step_and_result_lines(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: None)
    fake = FakeClient(
        {
            "phone_status": ok(
                {
                    "adb_available": True,
                    "scrcpy_available": False,
                    "companion_enabled": True,
                }
            )
        }
    )

    def factory() -> Any:
        return fake

    lines: list[str] = []
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_FALLBACK)
    report = smoke.run_smoke(options, client_factory=factory, emit=lines.append)
    assert fake.initialized is True
    assert fake.closed is True
    assert any(line.startswith("PASS fallback.phone_status_fallback") for line in lines)
    assert lines[-1] == "RESULT full_phone_use_smoke passed=1 skipped=0 failed=0"
    assert report.failed_count == 0


def test_run_smoke_full_profile_skips_hardware_lanes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # No adb, no scrcpy: every hardware lane skips with a reason, fallback passes.
    monkeypatch.setattr(smoke, "adb_binary", lambda: None)
    monkeypatch.setattr(smoke, "scrcpy_binary", lambda: None)
    fake = FakeClient({"phone_status": ok({"adb_available": False, "scrcpy_available": False})})

    def factory() -> Any:
        return fake

    lines: list[str] = []
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_FULL)
    report = smoke.run_smoke(options, client_factory=factory, emit=lines.append)
    assert report.failed_count == 0
    result_line = lines[-1]
    assert result_line.startswith("RESULT full_phone_use_smoke")
    assert "failed=0" in result_line
    # Every hardware lane should have skipped, not failed.
    assert any("SKIP adb-usb" in line for line in lines)
    assert any("SKIP companion" in line for line in lines)


def test_run_smoke_closes_client_on_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    fake = FakeClient()

    def explode(_self: smoke.PhoneSmoke) -> Any:
        raise RuntimeError("init blew up")

    monkeypatch.setattr(FakeClient, "initialize", explode)

    def factory() -> Any:
        return fake

    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_FALLBACK)
    with pytest.raises(RuntimeError, match="init blew up"):
        smoke.run_smoke(options, client_factory=factory, emit=lambda _line: None)
    assert fake.closed is True


# ---------------------------------------------------------------------------
# argparse wiring
# ---------------------------------------------------------------------------


def test_build_parser_defaults_to_full() -> None:
    args = smoke.build_parser().parse_args([])
    assert args.profile == smoke.PROFILE_FULL
    assert args.serial is None


def test_build_parser_rejects_unknown_profile() -> None:
    with pytest.raises(SystemExit):
        smoke.build_parser().parse_args(["--profile", "nope"])


def test_build_parser_accepts_targeting_flags() -> None:
    args = smoke.build_parser().parse_args(
        [
            "--profile",
            "adb-wireless",
            "--serial",
            "emulator-5554",
            "--wireless-host",
            "10.0.0.2:5555",
            "--pair-host",
            "10.0.0.2:37000",
            "--pairing-code",
            "000000",
        ]
    )
    assert args.profile == "adb-wireless"
    assert args.wireless_host == "10.0.0.2:5555"
    assert args.pair_host == "10.0.0.2:37000"


def test_options_from_args_maps_every_field() -> None:
    args = smoke.build_parser().parse_args(
        ["--profile", "adb-usb", "--serial", "abc", "--pair-host", "h:1"]
    )
    options = smoke.options_from_args(args)
    assert options.profile == "adb-usb"
    assert options.serial == "abc"
    assert options.pair_host == "h:1"
    assert options.wireless_host is None


def test_main_returns_nonzero_when_a_step_fails(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fake_run(
        _options: smoke.PhoneSmokeOptions,
        *,
        client_factory: Callable[[], Any] | None = None,
        emit: Callable[[str], None] = print,
    ) -> smoke.RunReport:
        report = smoke.RunReport()
        report.add(smoke.step_fail("x", "boom"))
        return report

    monkeypatch.setattr(smoke, "run_smoke", fake_run)
    assert smoke.main(["--profile", "fallback"]) == 1


def test_main_returns_zero_on_clean_run(monkeypatch: pytest.MonkeyPatch) -> None:
    def fake_run(
        _options: smoke.PhoneSmokeOptions,
        *,
        client_factory: Callable[[], Any] | None = None,
        emit: Callable[[str], None] = print,
    ) -> smoke.RunReport:
        report = smoke.RunReport()
        report.add(smoke.step_pass("x"))
        report.add(smoke.step_skip("y", "r"))
        return report

    monkeypatch.setattr(smoke, "run_smoke", fake_run)
    assert smoke.main(["--profile", "full"]) == 0


# ---------------------------------------------------------------------------
# Installed-surface client selection
# ---------------------------------------------------------------------------


def test_resolve_client_path_defaults_to_dev_build() -> None:
    assert smoke.resolve_client_path(installed=False) == smoke.DEV_CLIENT


def test_resolve_client_path_installed_returns_staged_when_present(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(type(smoke.INSTALLED_CLIENT), "exists", lambda _self: True)
    assert smoke.resolve_client_path(installed=True) == smoke.INSTALLED_CLIENT


def test_resolve_client_path_installed_errors_when_absent(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(type(smoke.INSTALLED_CLIENT), "exists", lambda _self: False)
    with pytest.raises(FileNotFoundError, match="staged client is missing"):
        smoke.resolve_client_path(installed=True)


def test_build_parser_accepts_installed_flag() -> None:
    args = smoke.build_parser().parse_args(["--profile", "full", "--installed"])
    assert args.installed is True


def test_options_from_args_installed_flag_sets_option() -> None:
    args = smoke.build_parser().parse_args(["--profile", "full", "--installed"])
    assert smoke.options_from_args(args).installed is True


def test_options_from_args_installed_defaults_false() -> None:
    args = smoke.build_parser().parse_args(["--profile", "full"])
    assert smoke.options_from_args(args).installed is False


def test_options_from_args_installed_env_opt_in(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(smoke.INSTALLED_ENV_VAR, "1")
    args = smoke.build_parser().parse_args(["--profile", "full"])
    assert smoke.options_from_args(args).installed is True


def test_options_from_args_installed_env_falsey(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(smoke.INSTALLED_ENV_VAR, "0")
    args = smoke.build_parser().parse_args(["--profile", "full"])
    assert smoke.options_from_args(args).installed is False


# ---------------------------------------------------------------------------
# Notification inspection helpers
# ---------------------------------------------------------------------------


def test_notification_events_returns_maps_only() -> None:
    result = ok({"events": [{"event_id": "e1"}, "garbage", {"event_id": "e2"}]})
    events = smoke.notification_events(result)
    assert [event["event_id"] for event in events] == ["e1", "e2"]


def test_first_openable_event_picks_can_open() -> None:
    events = [
        {"event_id": "e1", "can_open": False},
        {"event_id": "e2", "can_open": True},
    ]
    picked = smoke.first_openable_event(events)
    assert picked is not None
    assert picked["event_id"] == "e2"


def test_first_openable_event_none_when_all_closed() -> None:
    assert smoke.first_openable_event([{"event_id": "e1", "can_open": False}]) is None


def test_first_reply_action_finds_reply() -> None:
    event = {
        "event_id": "e1",
        "actions": [
            {"action_id": "open", "is_reply": False},
            {"action_id": "reply", "is_reply": True},
        ],
    }
    assert smoke.first_reply_action(event) == "reply"


def test_first_reply_action_none_without_reply() -> None:
    event = {"event_id": "e1", "actions": [{"action_id": "open", "is_reply": False}]}
    assert smoke.first_reply_action(event) is None


def test_first_action_id_returns_first() -> None:
    event = {"actions": [{"action_id": "a1"}, {"action_id": "a2"}]}
    assert smoke.first_action_id(event) == "a1"


# ---------------------------------------------------------------------------
# profile_companion notification steps
# ---------------------------------------------------------------------------


def _companion_responses(extra: dict[str, dict[str, Any]]) -> dict[str, dict[str, Any]]:
    base: dict[str, dict[str, Any]] = {
        "phone_list_devices": ok({"devices": [{"serial": "d", "state": "device"}]}),
        "phone_connect": ok({"session_id": "s"}),
        "phone_companion_status": ok(
            {
                "companion": {
                    "installed": True,
                    "accessibility_enabled": True,
                    "rpc_reachable": True,
                    "native_overlay": True,
                }
            }
        ),
        # Phone-native agent overlay step prerequisites: a screenshot that reports
        # the native overlay cursor plane, plus benign tap/swipe dispatches.
        "phone_screenshot": ok(
            {
                "phone_snapshot_id": "snap-1",
                "cursor_capabilities": {"phone_native_overlay": True},
            }
        ),
        "phone_tap": ok({"backend": "companion"}),
        "phone_swipe": ok({"backend": "companion"}),
        "phone_disconnect": ok({"disconnected": True}),
    }
    base.update(extra)
    return base


def test_profile_companion_runs_notification_steps(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    event = {
        "event_id": "evt-1",
        "can_open": True,
        "can_dismiss": True,
        "actions": [
            {"action_id": "act-1", "is_reply": False},
            {"action_id": "reply-1", "is_reply": True},
        ],
    }
    driver, _ = make_smoke(
        _companion_responses(
            {
                "phone_notifications": ok({"listener_enabled": True, "events": [event]}),
                "phone_notification_open": ok({"ok": True}),
                "phone_notification_dismiss": ok({"ok": True}),
                "phone_notification_action": ok({"ok": True}),
                "phone_notification_reply": ok({"ok": True}),
            }
        ),
        profile=smoke.PROFILE_COMPANION,
    )
    results = smoke.profile_companion(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    names = {step.name: step.status for step in results}
    assert names["phone_overlay"] == smoke.StepStatus.PASS
    assert names["phone_notifications"] == smoke.StepStatus.PASS
    assert names["phone_notification_open"] == smoke.StepStatus.PASS
    assert names["phone_notification_action"] == smoke.StepStatus.PASS
    assert names["phone_notification_reply"] == smoke.StepStatus.PASS
    assert names["phone_notification_dismiss"] == smoke.StepStatus.PASS


def test_profile_companion_notification_steps_skip_named_when_listener_disabled(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        _companion_responses(
            {"phone_notifications": ok({"listener_enabled": False, "events": []})}
        ),
        profile=smoke.PROFILE_COMPANION,
    )
    results = smoke.profile_companion(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    observe = next(step for step in results if step.name == "phone_notifications")
    assert observe.skipped
    assert "listener not enabled" in observe.detail


def test_profile_companion_notification_steps_skip_named_without_events(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        _companion_responses({"phone_notifications": ok({"listener_enabled": True, "events": []})}),
        profile=smoke.PROFILE_COMPANION,
    )
    results = smoke.profile_companion(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    by_name = {step.name: step for step in results}
    assert by_name["phone_notifications"].passed
    assert by_name["phone_notification_open"].skipped
    assert "no openable notification" in by_name["phone_notification_open"].detail
    assert by_name["phone_notification_reply"].skipped
    assert "no inline-reply" in by_name["phone_notification_reply"].detail
    assert by_name["phone_notification_dismiss"].skipped


# ---------------------------------------------------------------------------
# Phone-native agent overlay helpers + step
# ---------------------------------------------------------------------------


def test_cursor_capabilities_returns_map_or_empty() -> None:
    assert smoke.cursor_capabilities(
        ok({"cursor_capabilities": {"phone_native_overlay": True}})
    ) == {"phone_native_overlay": True}
    assert smoke.cursor_capabilities(ok({})) == {}
    assert smoke.cursor_capabilities(ok({"cursor_capabilities": ["nope"]})) == {}


def test_companion_native_overlay_reads_bool_or_none() -> None:
    assert smoke.companion_native_overlay(ok({"companion": {"native_overlay": True}})) is True
    assert smoke.companion_native_overlay(ok({"companion": {"native_overlay": False}})) is False
    assert smoke.companion_native_overlay(ok({"companion": {}})) is None
    assert smoke.companion_native_overlay(ok({})) is None


def test_overlay_step_passes_and_fires_tap_and_swipe(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, client = make_smoke(
        _companion_responses({"phone_notifications": ok({"listener_enabled": True, "events": []})}),
        profile=smoke.PROFILE_COMPANION,
    )
    caps = {"installed": True, "rpc_reachable": True, "native_overlay": True}
    result = smoke._overlay_step(driver, "s", caps)  # pyright: ignore[reportPrivateUsage]
    assert result.passed
    assert "native_overlay=true" in result.detail
    assert "gesture_backend=companion" in result.detail
    # The overlay path drives both per-action animations via real dispatches.
    called = [name for name, _ in client.calls]
    assert "phone_tap" in called
    assert "phone_swipe" in called


def test_overlay_step_skips_when_companion_lacks_native_overlay() -> None:
    driver, _ = make_smoke()
    caps = {"installed": True, "rpc_reachable": True, "native_overlay": False}
    with pytest.raises(smoke.SmokeSkip, match="does not advertise the native overlay"):
        smoke._overlay_step(driver, "s", caps)  # pyright: ignore[reportPrivateUsage]


def test_overlay_step_skips_when_rpc_unreachable() -> None:
    driver, _ = make_smoke()
    caps = {"installed": True, "rpc_reachable": False, "native_overlay": True}
    with pytest.raises(smoke.SmokeSkip, match="rpc not reachable"):
        smoke._overlay_step(driver, "s", caps)  # pyright: ignore[reportPrivateUsage]


def test_overlay_step_skips_when_screenshot_omits_plane() -> None:
    driver, _ = make_smoke(
        {"phone_screenshot": ok({"phone_snapshot_id": "snap-1", "cursor_capabilities": {}})}
    )
    caps = {"installed": True, "rpc_reachable": True, "native_overlay": True}
    with pytest.raises(smoke.SmokeSkip, match="did not report the phone_native_overlay"):
        smoke._overlay_step(driver, "s", caps)  # pyright: ignore[reportPrivateUsage]


def test_overlay_step_skips_when_tap_rejected() -> None:
    driver, _ = make_smoke(
        {
            "phone_screenshot": ok(
                {
                    "phone_snapshot_id": "snap-1",
                    "cursor_capabilities": {"phone_native_overlay": True},
                }
            ),
            "phone_tap": err({"diagnostics": [{"code": "X", "message": "rejected"}]}),
        }
    )
    caps = {"installed": True, "rpc_reachable": True, "native_overlay": True}
    with pytest.raises(smoke.SmokeSkip, match="overlay tap rejected"):
        smoke._overlay_step(driver, "s", caps)  # pyright: ignore[reportPrivateUsage]


def test_profile_companion_overlay_step_skips_named_without_plane(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    responses = _companion_responses(
        {
            "phone_notifications": ok({"listener_enabled": True, "events": []}),
            # Companion installed but without the native overlay plane.
            "phone_companion_status": ok(
                {"companion": {"installed": True, "rpc_reachable": True, "native_overlay": False}}
            ),
        }
    )
    driver, _ = make_smoke(responses, profile=smoke.PROFILE_COMPANION)
    results = smoke.profile_companion(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    overlay = next(step for step in results if step.name == "phone_overlay")
    assert overlay.skipped
    assert "does not advertise the native overlay" in overlay.detail
    # A missing overlay plane must not fail the rest of the companion profile.
    assert not any(step.failed for step in results)


# ---------------------------------------------------------------------------
# profile_adversarial snapshot-drift steps
# ---------------------------------------------------------------------------


def test_profile_adversarial_drift_steps_skip_named_without_flags(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_connect": err({"diagnostics": [{"code": "X", "message": "no"}]}),
            "phone_tap": err({"diagnostics": [{"code": "Y", "message": "stale"}]}),
        },
        profile=smoke.PROFILE_ADVERSARIAL,
    )
    results = smoke.profile_adversarial(driver, driver._options)  # pyright: ignore[reportPrivateUsage]
    by_name = {step.name: step for step in results}
    assert by_name["orientation_mismatch_rejected"].skipped
    assert "--device-can-rotate" in by_name["orientation_mismatch_rejected"].detail
    assert by_name["resolution_mismatch_rejected"].skipped
    assert "--device-can-resize" in by_name["resolution_mismatch_rejected"].detail


def test_snapshot_drift_step_passes_on_expected_code(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_list_devices": ok({"devices": [{"serial": "d", "state": "device"}]}),
            "phone_connect": ok({"session_id": "s"}),
            "phone_screenshot": ok({"phone_snapshot_id": "snap-1"}),
            "phone_tap": err(
                {
                    "diagnostics": [
                        {"code": smoke.SNAPSHOT_ORIENTATION_MISMATCH_CODE, "message": "rot"}
                    ]
                }
            ),
            "phone_disconnect": ok({"disconnected": True}),
        },
        profile=smoke.PROFILE_ADVERSARIAL,
    )
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_ADVERSARIAL, rotate_device=True)
    result = smoke._snapshot_drift_step(  # pyright: ignore[reportPrivateUsage]
        driver,
        options,
        enabled=True,
        enable_flag="--device-can-rotate",
        change_label="rotate",
        expected_code=smoke.SNAPSHOT_ORIENTATION_MISMATCH_CODE,
        step_name="orientation_mismatch_rejected",
    )
    assert result.passed
    assert smoke.SNAPSHOT_ORIENTATION_MISMATCH_CODE in result.detail


def test_snapshot_drift_step_fails_when_tap_accepted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_list_devices": ok({"devices": [{"serial": "d", "state": "device"}]}),
            "phone_connect": ok({"session_id": "s"}),
            "phone_screenshot": ok({"phone_snapshot_id": "snap-1"}),
            "phone_tap": ok({}),
            "phone_disconnect": ok({"disconnected": True}),
        },
        profile=smoke.PROFILE_ADVERSARIAL,
    )
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_ADVERSARIAL, resize_device=True)
    result = smoke._snapshot_drift_step(  # pyright: ignore[reportPrivateUsage]
        driver,
        options,
        enabled=True,
        enable_flag="--device-can-resize",
        change_label="resize",
        expected_code=smoke.SNAPSHOT_RESOLUTION_MISMATCH_CODE,
        step_name="resolution_mismatch_rejected",
    )
    assert result.failed
    assert "accepted" in result.detail


def test_snapshot_drift_step_fails_on_wrong_code(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(smoke, "adb_binary", lambda: "/usr/bin/adb")
    driver, _ = make_smoke(
        {
            "phone_list_devices": ok({"devices": [{"serial": "d", "state": "device"}]}),
            "phone_connect": ok({"session_id": "s"}),
            "phone_screenshot": ok({"phone_snapshot_id": "snap-1"}),
            "phone_tap": err({"diagnostics": [{"code": "SomeOtherCode", "message": "no"}]}),
            "phone_disconnect": ok({"disconnected": True}),
        },
        profile=smoke.PROFILE_ADVERSARIAL,
    )
    options = smoke.PhoneSmokeOptions(profile=smoke.PROFILE_ADVERSARIAL, rotate_device=True)
    result = smoke._snapshot_drift_step(  # pyright: ignore[reportPrivateUsage]
        driver,
        options,
        enabled=True,
        enable_flag="--device-can-rotate",
        change_label="rotate",
        expected_code=smoke.SNAPSHOT_ORIENTATION_MISMATCH_CODE,
        step_name="orientation_mismatch_rejected",
    )
    assert result.failed
    assert "expected" in result.detail


def test_build_parser_accepts_device_change_flags() -> None:
    args = smoke.build_parser().parse_args(
        ["--profile", "adversarial", "--device-can-rotate", "--device-can-resize"]
    )
    assert args.device_can_rotate is True
    assert args.device_can_resize is True
    options = smoke.options_from_args(args)
    assert options.rotate_device is True
    assert options.resize_device is True
