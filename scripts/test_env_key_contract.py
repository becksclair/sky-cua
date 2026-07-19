"""Guards Rust<->Python drift in the `SKY_CUA_*` env-key contract.

Plan 007 deduplicated the Rust-side declarations of these keys down to one
canonical `const` per key in `sky-cua-platform`. That does nothing, though,
to stop the *Python* side (the installer's forwarding allowlist, the
checked-in `.mcp.json`, and the agent smoke harnesses) from drifting away
from the Rust source of truth: a rename or typo on either side is invisible
until a live smoke fails on a silently-unforwarded toggle.

This module builds key sets by regex-scanning source rather than importing it
(works in CI without a Rust build) and asserts both directions:

- Every key any Python forwarding structure references must exist in the
  Rust set (`test_python_referenced_keys_exist_in_rust`) -- catches
  renames/typos, the observed failure mode.
- Every Rust environment read must appear in the installer/`.mcp.json`
  forwarding surface, or be in the commented `KNOWN_NOT_FORWARDED`
  exemption below with a one-line reason
  (`test_forwarding_relevant_rust_keys_are_forwarded_or_exempted`) -- a new
  key now forces a forward-or-exempt decision instead of silently missing
  the installer.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# Matches a quoted `SKY_CUA_*` string literal in either language ("..." in
# both Rust and Python; Rust never uses single quotes for &str literals but
# the pattern tolerates it for robustness).
SKY_CUA_KEY_PATTERN = re.compile(r'["\'](SKY_CUA_[A-Z0-9_]+)["\']')
RUST_ENV_LITERAL_READ_PATTERN = re.compile(
    r'(?:std::)?env::(?:var|var_os)\(\s*["\'](SKY_CUA_[A-Z0-9_]+)["\']\s*\)'
)
RUST_STRING_CONST_PATTERN = re.compile(
    r"\b(?:const|static)\s+([A-Z][A-Z0-9_]*)\s*:[^=;]+="
    r'\s*["\'](SKY_CUA_[A-Z0-9_]+)["\']\s*;'
)
RUST_ENV_CONST_READ_PATTERN = re.compile(
    r"(?:std::)?env::(?:var|var_os)\(\s*([A-Z][A-Z0-9_]*)\s*\)"
)

# Python-side structures that are meant to track the Rust env-key contract:
# the installer's default MCP server env forwarding list, the module that
# defines a few of the constants it references, and the checked-in `.mcp.json`
# (the most complete forwarding list; also covers phone-use and
# isolated-desktop keys the installer's Python default omits). Scanning only
# these named structures (not every literal in `scripts/**/*.py`) keeps this
# check aimed at the actual allowlists the plan is guarding, instead of
# tripping on unrelated Python-only harness knobs (VM SSH targets, smoke
# model selection, judge thresholds, ...) that were never meant to mirror a
# Rust env key.
INSTALLER_FILES = (
    "scripts/install_mcp_server.py",
    "scripts/_install_shared.py",
)
MCP_JSON_PATH = ".mcp.json"

# Additional Python-side allowlists the plan calls out as hardcoded mirrors
# of the Rust contract. Unlike the installer files above (small, entirely
# env-key focused), these two files are large multi-purpose harnesses with
# plenty of unrelated SKY_CUA_* literals (VM SSH targets, smoke model
# selection, judge thresholds, ...) that were never meant to mirror a Rust
# env key, so each entry names the specific module-level structure to scan
# rather than the whole file.
SMOKE_ALLOWLIST_STRUCTURES = (
    ("scripts/_agent_mcp_smoke.py", "SKY_CUA_RUNTIME_ENV_ALLOWLIST = {"),
    ("scripts/run_gui_testing_vm_smoke.py", "AGENT_AUTH_ENV_KEYS = ("),
    ("scripts/run_gui_testing_vm_smoke.py", "MCP_LAUNCH_POLICY_ENV_KEYS = ("),
)

# Keys referenced by the Python allowlists above with no Rust-literal match,
# each with a reason. Keep this list short; if it grows, the scan is probably
# too broad rather than every entry being legitimate.
KNOWN_PYTHON_ONLY: dict[str, str] = {
    # Agent-smoke-harness-only knobs: control the Python smoke runner itself
    # (raw log retention, per-agent model override); never read by Rust.
    "SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG": "Python smoke-harness knob, not a Rust env key",
    "SKY_CUA_SMOKE_OPENCODE_MODEL": "Python smoke-harness knob, not a Rust env key",
    "SKY_CUA_SMOKE_PI_MODEL": "Python smoke-harness knob, not a Rust env key",
    # Real Rust runtime keys, but the suffix is assembled at runtime
    # (`format!("SKY_CUA_XKB_{suffix}")` in virtual_input.rs) rather than
    # declared as a quoted literal, so the Rust-side literal scan below
    # cannot see them.
    "SKY_CUA_XKB_LAYOUT": 'Rust builds this key via format!("SKY_CUA_XKB_{suffix}"), not a literal',
    "SKY_CUA_XKB_MODEL": 'Rust builds this key via format!("SKY_CUA_XKB_{suffix}"), not a literal',
    "SKY_CUA_XKB_OPTIONS": 'Rust builds this key via format!("SKY_CUA_XKB_{suffix}"), not a literal',
    "SKY_CUA_XKB_RULES": 'Rust builds this key via format!("SKY_CUA_XKB_{suffix}"), not a literal',
    "SKY_CUA_XKB_VARIANT": 'Rust builds this key via format!("SKY_CUA_XKB_{suffix}"), not a literal',
}

# Rust-read keys not in the installer/.mcp.json forwarding surface, each
# with a one-line reason. Built empirically: run
# test_forwarding_relevant_rust_keys_are_forwarded_or_exempted, move every
# reported miss here with a reason, confirm green. A *new* unforwarded key
# fails the test until it is either added to install_mcp_server.py /
# .mcp.json or exempted here with a reason -- the forced decision point this
# plan exists to create.
KNOWN_NOT_FORWARDED: dict[str, str] = {
    "SKY_CUA_BROWSER_USE_SESSIONS_DIR": "read only by the standalone chrome-host native-messaging process, launched by the browser itself, not through the MCP host env",
    "SKY_CUA_BROWSER_USE_SOCKET_DIR": "forwarded through the client's dedicated launch-environment repair path (service_launcher.rs), not the generic env_vars allowlist",
    "SKY_CUA_CAPTURE_DIR": "overlay-host motion/gesture capture-harness dev knob (renderer/motion_capture.rs, renderer/shaders.rs), not an operator toggle",
    "SKY_CUA_CAPTURE_GESTURES": "overlay-host gesture capture-harness dev knob (renderer/shaders.rs), not an operator toggle",
    "SKY_CUA_CAPTURE_MOTION": "overlay-host motion capture-harness dev knob (renderer/motion_capture.rs), not an operator toggle",
    "SKY_CUA_CHROME_HOST_COMPAT_CODEX": "read only by the standalone chrome-host native-messaging process, not through the MCP host env",
    "SKY_CUA_CHROME_HOST_NAME": "read only by the standalone chrome-host native-messaging process, not through the MCP host env",
    "SKY_CUA_CHROME_HOST_TRACE": "chrome-host debug tracing toggle, set alongside the native-messaging manifest, not through the MCP host env",
    "SKY_CUA_CLIENT_CLEARED_SESSION_ENV_KEYS": "set by the client for its own spawned service child; never sourced externally",
    "SKY_CUA_CLIENT_SESSION_ENV_REPAIRS": "set by the client for its own spawned service child; never sourced externally",
    "SKY_CUA_COSMIC_CURSOR_BRIDGE": "internal IPC path between overlay-host and sky-cua-cosmic-helper, not operator config",
    "SKY_CUA_COSMIC_CURSOR_READY": "internal IPC path between overlay-host and sky-cua-cosmic-helper, not operator config",
    "SKY_CUA_COSMIC_CURSOR_STATE": "internal IPC path between overlay-host and sky-cua-cosmic-helper, not operator config",
    "SKY_CUA_DAEMON_LOG_PATH": "set by the client for its own spawned service child (resolved per-endpoint log path); never sourced externally",
    "SKY_CUA_FORCE_PIPEWIRE_CAPTURE_FAILURE": "test-only fault injection (portal/pipewire.rs)",
    "SKY_CUA_INPUT_HELPER_SOCKET_MODE": "input-helper systemd socket permission, set once at install time alongside SKY_CUA_INPUT_HELPER_SOCKET_GROUP into /etc/sky-cua/input-helper.env; the privileged sky-cua-input-helper.service never inherits from the MCP-launched process tree, so forwarding it through the per-invocation env_vars allowlist would be a no-op, not a real gap",
    "SKY_CUA_MCP_HOST": "deprecated client-ingress compatibility fallback; sky-cua-owned adapters set SKY_CUA_MCP_CALLER_PROVENANCE instead",
    "SKY_CUA_PHONE_COMMAND_TIMEOUT_MS": "internal phone-backend command timeout tuning, not a documented operator override",
    "SKY_CUA_POINTER_TRACKING_DEBUG": "overlay-host pointer-tracking debug logging toggle, not an operator toggle",
    "SKY_CUA_TEST_BRIDGE_REQUEST_TIMEOUT_MS": "test-only (browser/transport.rs test fixture)",
    "SKY_CUA_UPDATE_MCP_FIXTURES": "dev-only MCP tool-fixture regeneration toggle (mcp_tools/definitions.rs)",
}


def _keys_in_text(text: str) -> set[str]:
    return set(SKY_CUA_KEY_PATTERN.findall(text))


def _extract_balanced_block(text: str, start_marker: str) -> str:
    """Return `text` from `start_marker` through its balanced closing bracket.

    `start_marker` must end in one of `([{`; used to scope a regex scan to a
    single module-level list/set/tuple/dict literal instead of the whole file.
    """
    start = text.index(start_marker)
    depth = 0
    opened = False
    for index, char in enumerate(text[start:], start=start):
        if char in "([{":
            depth += 1
            opened = True
        elif char in ")]}":
            depth -= 1
            if opened and depth == 0:
                return text[start : index + 1]
    raise ValueError(f"unbalanced block starting at {start_marker!r}")


def rust_declared_keys() -> set[str]:
    """Every quoted `SKY_CUA_*` string literal anywhere under `crates/`."""
    keys: set[str] = set()
    for path in (REPO_ROOT / "crates").glob("**/*.rs"):
        keys |= _keys_in_text(path.read_text(encoding="utf-8", errors="ignore"))
    return keys


def rust_environment_keys() -> set[str]:
    """Rust SKY_CUA_* values that are environment reads, not diagnostic strings.

    Direct string-literal reads and constants passed directly to ``env::var`` /
    ``env::var_os`` are detected. Env-named constants are also included because
    small typed parsing helpers commonly receive them one call above the actual
    generic environment read.
    """
    source_texts = [
        path.read_text(encoding="utf-8", errors="ignore")
        for path in (REPO_ROOT / "crates").glob("**/*.rs")
    ]
    constants: dict[str, str] = {}
    for text in source_texts:
        for name, value in RUST_STRING_CONST_PATTERN.findall(text):
            constants[name] = value

    keys: set[str] = set()
    for text in source_texts:
        keys.update(RUST_ENV_LITERAL_READ_PATTERN.findall(text))
        keys.update(
            constants[name]
            for name in RUST_ENV_CONST_READ_PATTERN.findall(text)
            if name in constants
        )
    keys.update(value for name, value in constants.items() if "ENV" in name)
    return keys


def python_allowlist_keys() -> set[str]:
    """Every quoted `SKY_CUA_*` literal in the Python/JSON forwarding allowlists."""
    keys: set[str] = installer_forwarded_keys()
    for rel_path, marker in SMOKE_ALLOWLIST_STRUCTURES:
        text = (REPO_ROOT / rel_path).read_text(encoding="utf-8")
        keys |= _keys_in_text(_extract_balanced_block(text, marker))
    return keys


def installer_forwarded_keys() -> set[str]:
    """Keys forwarded by the installer default env_vars list or `.mcp.json`."""
    keys: set[str] = set()
    for rel_path in INSTALLER_FILES:
        keys |= _keys_in_text((REPO_ROOT / rel_path).read_text(encoding="utf-8"))
    mcp_json = json.loads((REPO_ROOT / MCP_JSON_PATH).read_text(encoding="utf-8"))
    env_vars = mcp_json["mcpServers"]["computer-use"]["env_vars"]
    keys |= {key for key in env_vars if key.startswith("SKY_CUA_")}
    return keys


def test_python_referenced_keys_exist_in_rust() -> None:
    rust_keys = rust_declared_keys()
    python_keys = python_allowlist_keys()
    drifted = python_keys - rust_keys - set(KNOWN_PYTHON_ONLY)
    assert not drifted, (
        "Python allowlist(s) reference SKY_CUA_* keys with no matching Rust "
        f"literal (rename/typo?): {sorted(drifted)}"
    )


def test_known_python_only_keys_are_still_absent_from_rust() -> None:
    # Keeps KNOWN_PYTHON_ONLY honest: if a Rust literal for one of these
    # shows up later (e.g. the XKB suffix stops being format!()-assembled),
    # the exemption is stale and should be removed.
    rust_keys = rust_declared_keys()
    stale = sorted(key for key in KNOWN_PYTHON_ONLY if key in rust_keys)
    assert not stale, f"KNOWN_PYTHON_ONLY entries now have a Rust literal, remove them: {stale}"


def test_forwarding_relevant_rust_keys_are_forwarded_or_exempted() -> None:
    rust_keys = rust_environment_keys()
    forwarded = installer_forwarded_keys()
    unforwarded = rust_keys - forwarded - set(KNOWN_NOT_FORWARDED)
    assert not unforwarded, (
        "Rust-read SKY_CUA_* environment keys missing from the installer/.mcp.json "
        "forwarding surface, and not in KNOWN_NOT_FORWARDED: "
        f"{sorted(unforwarded)}"
    )


def test_known_not_forwarded_keys_are_still_unforwarded() -> None:
    # Keeps KNOWN_NOT_FORWARDED honest: if a key gets added to the installer
    # or .mcp.json later, its exemption entry is stale and should be removed.
    forwarded = installer_forwarded_keys()
    stale = sorted(key for key in KNOWN_NOT_FORWARDED if key in forwarded)
    assert not stale, f"KNOWN_NOT_FORWARDED entries are now forwarded, remove them: {stale}"


def test_exemption_lists_stay_small() -> None:
    # A large exemption list is a sign the scan is too broad or the
    # forwarding contract needs a real fix, not more exemptions. See Plan
    # 007's STOP condition: report to a maintainer instead of padding this
    # past ~30.
    assert len(KNOWN_NOT_FORWARDED) <= 30, (
        f"KNOWN_NOT_FORWARDED has grown to {len(KNOWN_NOT_FORWARDED)} entries; "
        "re-evaluate the forwarding contract instead of adding more exemptions"
    )


def test_diagnostic_error_codes_are_not_classified_as_environment_keys() -> None:
    diagnostic_codes = {
        "SKY_CUA_ACTION_OUTCOME_UNKNOWN",
        "SKY_CUA_DEADLINE_EXCEEDED",
        "SKY_CUA_FRAME_TOO_LARGE",
        "SKY_CUA_INTERNAL",
        "SKY_CUA_INVALID_ARGUMENT",
        "SKY_CUA_INVALID_CONTEXT",
        "SKY_CUA_INVALID_REQUEST",
        "SKY_CUA_TARGET_UNAVAILABLE",
        "SKY_CUA_TURN_CANCELLED",
    }
    assert diagnostic_codes <= rust_declared_keys()
    assert diagnostic_codes.isdisjoint(rust_environment_keys())


def test_browser_control_env_contract_is_forwarded_without_removed_transport() -> None:
    forwarded = installer_forwarded_keys()
    assert {
        "SKY_CUA_BROWSER_CONTROL_MODE",
        "SKY_CUA_CODEX_BROWSER_SOCKET_PATH",
    } <= forwarded
    assert "SKY_CUA_BROWSER_BRIDGE_TRANSPORT" not in forwarded
