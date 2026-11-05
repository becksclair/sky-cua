# Plan 011: Fix the agent guides that point at moved or removed code, and add a test that keeps their paths honest

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/AGENTS.md crates/sky-cua-service/AGENTS.md crates/sky-cua-client/AGENTS.md crates/sky-cua-linux/AGENTS.md android/phone-companion/AGENTS.md skills/phone-use/AGENTS.md ROADMAP.md scripts/test_env_key_contract.py`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live files before proceeding; on a semantic mismatch, treat it as a
> STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

Every agent working in this repository is told to read the nearest
`AGENTS.md` first and treat it as authority. Five of those guides cite source
paths that no longer exist after the 2026-07-08 god-file splits
(`src/daemon.rs` → `src/daemon/mod.rs`, `src/mcp_tools/definitions.rs` →
`src/mcp_tools/definitions/`, `src/backend.rs` → `src/backend/`), and the
Android companion guide describes a loopback HTTP RPC server, an `rpc/`
package, `RpcController` and `SetupActivity` that were all removed on
2026-08-24, while saying nothing about the shipped `direct/` security surface
(HMAC device secret, enrollment, content provider). It also states toolchain
and SDK versions two releases behind the build files. An agent editing
companion security code is handed the wrong threat model. `ROADMAP.md:36`
links a plan file that never existed for work that has shipped.

The fix is small text edits plus one pytest, `scripts/test_agent_guides.py`,
that scans every tracked `AGENTS.md` for backticked repository paths and
fails when one does not exist. A prototype of that check on 2026-11-03
examined 111 path tokens and flagged exactly the five stale ones listed
below and nothing else, so it can land green once the guides are fixed.

## Current state

Stale path citations (prototype scan output; file:line: token):

- `crates/AGENTS.md:21` `src/daemon.rs` — in "Daemon state and IPC serving:
  `crates/sky-cua-service/src/**` (request dispatch in `src/daemon.rs`)."
  Live: `crates/sky-cua-service/src/daemon/mod.rs` (dispatch), with
  `daemon/{agent_cursor,appshot,browser,capture_reuse,desktop,phone,session_presence}.rs`
  and `daemon/tests.rs`.
- `crates/AGENTS.md:44` `portal/pipewire.rs` — in "keep serialized patterns
  like `serial_test` in `portal/pipewire.rs`." Live path relative to the
  crate: `crates/sky-cua-linux/src/portal/pipewire.rs`.
- `crates/sky-cua-client/AGENTS.md:17` `src/mcp_tools/definitions.rs` — in
  "- `src/mcp_tools/definitions.rs` — advertised grouped tool definitions
  and schemas." Live: `src/mcp_tools/definitions/` containing
  `mod.rs, common.rs, status.rs, desktop.rs, browser.rs, phone.rs, contract_fixtures.rs, fixture_tests.rs`.
- `crates/sky-cua-linux/AGENTS.md:12` `src/backend.rs` — in "- `src/backend.rs`
  — orchestration, fallback snapshot geometry, and the cached
  `accessibility_connection` pattern for expensive connections." Live:
  `src/backend/` containing `mod.rs, desktop_backend.rs, action_runtime.rs, elements.rs, tests.rs`.
- `crates/sky-cua-service/AGENTS.md:9` `src/daemon.rs` — in "- `src/daemon.rs`
  — request dispatch; `src/ipc_server.rs` — Unix-socket server; ...". Live:
  `src/daemon/mod.rs` and the submodules above.

`crates/AGENTS.md` Layout (lines 6-25) also never mentions three of the ten
crates: `sky-cua-chrome-host` (Chrome native-messaging host),
`sky-cua-cosmic-helper` (COSMIC toplevel helper), `sky-cua-input-helper`
(root uinput helper, runs as a systemd system service). The other seven are
listed or linked.

`android/phone-companion/AGENTS.md` (full text read 2026-11-03):

- Intro says the companion is "exposed to the sky-cua host over a
  localhost-only RPC endpoint reached through host-managed `adb forward`".
  Live: the companion connects outbound to the host over a direct-LAN
  websocket (`ws://` or `wss://`) after QR enrollment (`skycua://enroll`),
  authenticating with a per-host HMAC device secret and link epoch stored in
  `direct/CredentialStore.kt`. Evidence: `docs/features/phone-use.md:476`
  ("Legacy RPC token delivery ... and the `SetupActivity`/`RpcServer` were
  removed 2026-08-24. CompanionDirect now uses a persistent HMAC device
  secret + epoch ... over `ws://`/`wss://`"); the `direct/` package contents
  below.
- "Authoritative contract" points at `docs/runtime/phone-companion-protocol.md`
  and "the setup-intent token bootstrap". That document still describes the
  removed transport (audit finding `DOCS-01`, out of scope here). The current
  wire contract is `docs/runtime/phone-control-v2.md` with the bind/transport
  rules in `docs/runtime/direct-lan.md`.
- Toolchain says "AGP 9.2.1 + Gradle 9.5.1" and "`compileSdk = 36`,
  `targetSdk = 36`, `minSdk = 30`". Live: `gradle/libs.versions.toml:2`
  `agp = "9.4.1"`; `gradle/wrapper/gradle-wrapper.properties:4`
  `gradle-9.8.1-bin.zip`; `app/build.gradle.kts:158,164,165`
  `compileSdk = 37`, `minSdk = 33`, `targetSdk = 36`.
- Structure lists `protocol/`, `json/`, `rpc/`, `service/` (with
  `RpcController`), `overlay/`, `app/`, `screenshot/`, `setup/` +
  `SetupActivity`. Live packages under
  `app/src/main/java/com/skycua/phonecompanion/`: `app/`, `appshot/`,
  `camera/`, `direct/`, `json/`, `overlay/`, `protocol/` (`Dispatcher.kt`,
  `Methods.kt`, `Protocol.kt`, `TokenStore.kt`), `screenshot/`, `service/`
  (`CameraController.kt`, `ClipboardController.kt`, `DeviceMethodHandler.kt`,
  `SkyAccessibilityService.kt`, `SkyImeService.kt`,
  `SkyNotificationListenerService.kt`, `SmsController.kt`,
  `StorageController.kt`), `ui/`; top-level activities `CaptureActivity.kt`,
  `EnrollmentActivity.kt`, `EnrollmentFlowViewModel.kt`, `HostListActivity.kt`,
  `MainActivity.kt`, `SafGrantActivity.kt`. `direct/` contains `Auth.kt`,
  `CompanionContentProvider.kt`, `ContentTransfer.kt`,
  `ContentTransferReceiver.kt`, `CredentialStore.kt`, `DirectDispatcher.kt`,
  `DirectLink.kt`, `DirectLinkRuntime.kt`, `Enrollment.kt`, `LinkEpoch.kt`,
  `MultiHostDirectLinkPool.kt`, `OkHttpDirectSocket.kt`. There is no `rpc/`
  or `setup/` package and no `RpcController` or `SetupActivity`.
- Security says "The RPC server binds loopback only and validates the token
  on every call". There is no RPC server.

`skills/phone-use/AGENTS.md:17-18` says "The companion wire contract is
authoritative in `docs/runtime/phone-companion-protocol.md` (repo-side
reference; ...)" with the line break after "contract is".

Two doc claims the plan's evidence relies on are themselves stale and must not
be repeated in the guide: `docs/features/phone-use.md:476` says
`phone-companion-protocol.md` is "now direct-only" (it is not; its title is
still "Phone companion RPC protocol (v1)" and its Transport section is
`adb forward` plus `POST /rpc`), and `docs/runtime/phone-control-v2.md:3-5`
says it "coexists with" v1. Both belong to `DOCS-01/02`.

The companion's authentication model, from `direct/DirectLink.kt:263-300` and
`docs/runtime/phone-control-v2.md:85-122`: one mutual HMAC-SHA256
challenge/response handshake per link (`auth_hello` → `auth_challenge` with a
server proof → `auth_proof` → `auth_ok`), under the per-host device secret,
with a strictly increasing link epoch persisted on `auth_ok`. Non-handshake
frames that arrive before the link is authenticated fail it. Individual
method frames after that carry no per-frame auth. `protocol/TokenStore.kt`
is a v1 remnant: `DirectDispatcher.kt:223-229` installs a fixed internal
token (`DIRECT_TOKEN`) so the v1 `Dispatcher` can be reused; it is not a
credential and its KDoc is stale. JDK pinning is `jvmToolchain(21)` at
`app/build.gradle.kts:197`.

`ROADMAP.md:36` (verbatim):

```
  - [ ] Privileged helper fast path for Wayland input — [`plans/privileged_linux_uinput_helper.md`](plans/privileged_linux_uinput_helper.md)
```

The file never existed (`git log --all --diff-filter=D -- plans/privileged_linux_uinput_helper.md`
prints nothing). The work shipped: `crates/sky-cua-input-helper/`,
`scripts/install_mcp_server.py --input-helper`, `README.md:105-107`,
`docs/features/linux-virtual-input.md:68-73`.

Python test conventions: tests live in `scripts/test_<subsystem>.py`, are
typed for basedpyright `standard`, and use `REPO_ROOT = Path(__file__).resolve().parents[1]`
(see `scripts/test_env_key_contract.py:28`, a regex-scanning contract test
that is the closest exemplar). `scripts/conftest.py` has an autouse fixture;
nothing special is needed for a pure filesystem test.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `uv run ruff format --check scripts` | `N files already formatted` |
| Lint | `uv run ruff check scripts` | `All checks passed!` |
| Types | `uv run basedpyright` | `0 errors, 0 warnings, 0 notes` |
| New test only | `uv run pytest scripts/test_agent_guides.py -q` | fails after Step 1 with the five expected citations; passes after Step 3 |
| All Python | `uv run pytest --ignore=scripts/test_browser_control_acceptance.py -q` | all pass |
| List guides | (unescaped form, copy from prose, not from this table) | `git ls-files | grep -E '(^|/)AGENTS\.md$' | wc -l` prints 17 at `e928a40` |

## Scope

**In scope** (the only files you should modify):
- `crates/AGENTS.md`
- `crates/sky-cua-service/AGENTS.md`
- `crates/sky-cua-client/AGENTS.md`
- `crates/sky-cua-linux/AGENTS.md`
- `android/phone-companion/AGENTS.md`
- `skills/phone-use/AGENTS.md` (one sentence)
- `ROADMAP.md` (line 36 only)
- `scripts/test_agent_guides.py` (create)
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- `docs/runtime/phone-companion-protocol.md` and
  `docs/runtime/phone-control-v2.md` (audit findings `DOCS-01/02`; separate
  documentation work).
- `NOTES.md`, `docs/features/**`, `plans/*.md` ExecPlans (`DOCS-03/04/06`).
- Creating `AGENTS.md` files for the three unlisted crates. Mention them in
  `crates/AGENTS.md`; writing new guides needs the crate owners' knowledge.
- `skills/*/SKILL.md` (installed copies cannot reach repo paths; they are
  deliberately path-free).
- Any source code.

## Git workflow

- Branch: `bex/agent-guide-paths`
- Commit only if the operator authorised it; style:
  `docs(agents): fix moved paths, rewrite the companion guide, guard guide paths with a test`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Write the guard test first (it should fail on the five stale paths)

Create `scripts/test_agent_guides.py`:

```python
"""Keeps every tracked AGENTS.md honest about the repository paths it cites.

Agents are told to read the nearest AGENTS.md first and treat it as
authority. After file splits and removals, stale paths in those guides send
agents to code that no longer exists. This test scans backticked tokens in
every tracked AGENTS.md and fails when a token that looks like a repository
path does not exist, resolved against the repo root, the guide's own
directory, or any immediate subdirectory of the guide's directory (so a
crate-group guide may cite `src/foo.rs` meaning "inside one of these crates").
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# A backticked token is treated as a repository path when it starts with one
# of these prefixes, or contains a slash and ends with a source/doc extension.
PATH_PREFIXES = (
    "crates/",
    "scripts/",
    "docs/",
    "src/",
    "resources/",
    "skills/",
    "android/",
    "plans/",
    "runtime/",
    "packages/",
    ".config/",
    "bin/",
    ".github/",
    ".gitea/",
)
PATH_EXTENSIONS = (".rs", ".py", ".md", ".kt", ".ts", ".json", ".toml", ".yml", ".yaml", ".service")
TOKEN_RE = re.compile(r"`([^`\s]+)`")
SAFE_TOKEN_RE = re.compile(r"^[A-Za-z0-9_./-]+$")


def tracked_agent_guides() -> list[Path]:
    output = subprocess.run(
        ["git", "ls-files", "--", "AGENTS.md", "*/AGENTS.md"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return sorted({REPO_ROOT / line for line in output.splitlines() if line})


def looks_like_repo_path(token: str) -> bool:
    if "..." in token or not SAFE_TOKEN_RE.match(token):
        return False
    return token.startswith(PATH_PREFIXES) or ("/" in token and token.endswith(PATH_EXTENSIONS))


def resolves(token: str, guide: Path) -> bool:
    guide_dir = guide.parent
    candidates = [REPO_ROOT / token, guide_dir / token]
    candidates.extend(child / token for child in guide_dir.iterdir() if child.is_dir())
    return any(candidate.exists() for candidate in candidates)


def stale_path_citations() -> list[str]:
    stale: list[str] = []
    for guide in tracked_agent_guides():
        for line_number, line in enumerate(guide.read_text(encoding="utf-8").splitlines(), 1):
            for match in TOKEN_RE.finditer(line):
                token = match.group(1)
                if looks_like_repo_path(token) and not resolves(token, guide):
                    stale.append(f"{guide.relative_to(REPO_ROOT)}:{line_number}: `{token}`")
    return stale


def test_agent_guides_exist() -> None:
    assert len(tracked_agent_guides()) >= 10


def test_agent_guide_paths_exist() -> None:
    stale = stale_path_citations()
    assert not stale, "AGENTS.md cites paths that do not exist:\n" + "\n".join(stale)
```

Run `uv run ruff format scripts/test_agent_guides.py` (writing a new test
file is in scope; formatting it is part of writing it). `git ls-files -- AGENTS.md '*/AGENTS.md'`
lists the 17 guides on git 2.56 (the `*/` glob recurses under git's default
pathspec rules; `**/AGENTS.md` would be redundant and misses the root file).
This exact file passed `ruff check`, `ruff format --check` and basedpyright
`standard` on 2026-11-03.

**Verify**: `uv run pytest scripts/test_agent_guides.py -q` → `test_agent_guide_paths_exist`
FAILS listing exactly these five:

```
crates/AGENTS.md:21: `src/daemon.rs`
crates/AGENTS.md:44: `portal/pipewire.rs`
crates/sky-cua-client/AGENTS.md:17: `src/mcp_tools/definitions.rs`
crates/sky-cua-linux/AGENTS.md:12: `src/backend.rs`
crates/sky-cua-service/AGENTS.md:9: `src/daemon.rs`
```

If it lists anything else, STOP (the heuristic or the tree differs from the
prototype).

### Step 2: Fix the four crate guides

The "before" strings below are quoted with their real line wraps; match
across the wrap.

- `crates/AGENTS.md:20-21`, before:
  ```
  - Daemon state and IPC serving: `crates/sky-cua-service/src/**` (request
    dispatch in `src/daemon.rs`).
  ```
  after:
  ```
  - Daemon state and IPC serving: `crates/sky-cua-service/src/**` (request
    dispatch in `src/daemon/mod.rs`; per-surface handlers live beside it in
    `src/daemon/`).
  ```
- `crates/AGENTS.md:44`: `` `portal/pipewire.rs` `` → `` `src/portal/pipewire.rs` ``.
- `crates/AGENTS.md` Layout: add three bullets after the overlay-host bullet
  (ends line 18):
  - "Chrome native-messaging host (extension to daemon bridge): `crates/sky-cua-chrome-host/`."
  - "COSMIC toplevel helper (Wayland `cosmic-protocols` client used for window discovery on COSMIC): `crates/sky-cua-cosmic-helper/`."
  - "Root uinput helper (systemd system service; keyboard injection and pointer observation over `/run/sky-cua/input-helper.sock`): `crates/sky-cua-input-helper/`."
- `crates/sky-cua-service/AGENTS.md:9`, before (single line):
  `` - `src/daemon.rs` — request dispatch; `src/ipc_server.rs` — Unix-socket ``
  after:
  `` - `src/daemon/mod.rs` — request dispatch (surface handlers in `src/daemon/desktop.rs`, `src/daemon/phone.rs`, `src/daemon/browser.rs`, `src/daemon/appshot.rs`); `src/ipc_server.rs` — Unix-socket ``
- `crates/sky-cua-client/AGENTS.md:17-18`, before:
  ```
  - `src/mcp_tools/definitions.rs` — advertised grouped tool definitions and
    schemas.
  ```
  after:
  ```
  - `src/mcp_tools/definitions/` — advertised grouped tool definitions and
    schemas (`mod.rs` assembles them; `desktop.rs`, `browser.rs`, `phone.rs`,
    `status.rs` per surface; `fixture_tests.rs` pins the contract fixtures).
  ```
- `crates/sky-cua-linux/AGENTS.md:12-13`, before:
  ```
  - `src/backend.rs` — orchestration, fallback snapshot geometry, and the
    cached `accessibility_connection` pattern for expensive connections.
  ```
  after:
  ```
  - `src/backend/` — orchestration (`mod.rs`), the `DesktopBackend` impl
    (`desktop_backend.rs`), fallback snapshot geometry, and the cached
    `accessibility_connection` pattern for expensive connections.
  ```

**Verify**: `uv run pytest scripts/test_agent_guides.py -q` → the failure
list now contains no `crates/` entries (it should pass entirely; the Android
guide cites no checked paths today).

### Step 3: Rewrite the Android companion guide's stale sections

Edit `android/phone-companion/AGENTS.md` using only the facts in "Current
state" (open the files to confirm anything you add). Do not write backticked
partial paths such as `direct/Auth.kt`: they look like paths to the test but
do not resolve from the guide's directory, so the test fails. Use bare class
names, or the full `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/...`
path when you want the test to guard it.

- Intro paragraph (lines 3-7; the sentence wraps after "forwarding,"):
  replace the whole paragraph with: "The Android companion app for
  `phone-use`. It is the rich phone backend (phone-native cursor overlay,
  accessibility-tree snapshots, native gesture dispatch, accessibility
  screenshots, notification forwarding, camera, clipboard, SMS and storage),
  connected to the sky-cua host over the CompanionDirect lane: after QR
  enrollment (`skycua://enroll`) the app opens an outbound `ws://`/`wss://`
  link to each enrolled host and authenticates with a per-host HMAC device
  secret and link epoch."
- "Authoritative contract": point at `docs/runtime/phone-control-v2.md`
  (frames, handshake, methods) and `docs/runtime/direct-lan.md` (bind and
  transport rules); say that `docs/runtime/phone-companion-protocol.md`
  describes the removed v1 transport and is being reconciled separately. Do
  not call it "direct-only" and do not say v1 "coexists". Drop "the
  setup-intent token bootstrap".
- Toolchain: `AGP 9.4.1 + Gradle 9.8.1 (wrapper checksum pinned)`;
  `compileSdk = 37`, `targetSdk = 36`, `minSdk = 33`; JDK 21 via
  `jvmToolchain(21)` in `app/build.gradle.kts`. With `minSdk = 33` the
  "API 24+ / API 30+" gating clauses are moot; keep only
  `takeScreenshotOfWindow` (API 34+). Keep the `ANDROID_SDK_ROOT` line.
- Structure: replace the list with exactly this (descriptions were checked
  against the files on 2026-11-03):

  ```
  All sources are under `app/src/main/java/com/skycua/phonecompanion/`.

  - `direct/` — CompanionDirect: host link and transport (`DirectLink`,
    `DirectLinkRuntime`, `MultiHostDirectLinkPool`, `OkHttpDirectSocket`);
    enrollment and credentials (`Enrollment`, `CredentialStore`, `Auth`,
    `LinkEpoch`); request dispatch (`DirectDispatcher`); content transfer
    (`ContentTransfer`, `ContentTransferReceiver`, `CompanionContentProvider`).
  - `protocol/` — method names, param/result types and validation, and the
    method `Dispatcher` that `DirectDispatcher` wraps. `TokenStore` is a v1
    remnant filled with a fixed internal token; it is not a credential (its
    KDoc is stale).
  - `json/` — small dependency-free JSON model/parser/writer.
  - `service/` — `SkyAccessibilityService`, `SkyImeService`,
    `SkyNotificationListenerService`, `DeviceMethodHandler` (wires methods to
    live services), and the camera, clipboard, SMS and storage controllers.
  - `overlay/` — pass-through agent cursor overlay (controller, view, AGSL
    renderers, window flags).
  - `camera/` — camera capture service and contract. `app/` —
    PackageManager app management. `appshot/` — phone AppShot DTO and capture
    policy. `screenshot/` — accessibility screenshot failure classification.
    `ui/` — grid/pointer/interaction playground activities.
  - Top-level activities: `MainActivity` (status UI), `EnrollmentActivity` +
    `EnrollmentFlowViewModel` (QR enrollment), `HostListActivity` (paired
    hosts, per-host remove), `CaptureActivity` (visible surface for remote
    camera), `SafGrantActivity` (one-time SAF tree picker for remote storage).
  ```

- Security: replace the section body with: "Never log, persist outside
  `CredentialStore`, or commit device secrets, enrollment payloads,
  notification content, accessibility dumps, or screenshots. The overlay is
  non-focusable and non-touchable so taps pass through. Each host link is
  mutually authenticated before any method frame is dispatched: an
  HMAC-SHA256 challenge/response under the per-host device secret, with a
  strictly increasing link epoch persisted on `auth_ok` (`direct/` package:
  `Auth`, `DirectLink`, `LinkEpoch`). Frames other than the handshake that
  arrive before the link is authenticated fail it. Cleartext `ws://` is
  accepted only for loopback, Tailscale addresses, or `*.ts.net` names."

Also edit `skills/phone-use/AGENTS.md:17-18`. Before (wraps after "is"):

```
  `crates/sky-cua-service/src/phone/`. The companion wire contract is
  authoritative in `docs/runtime/phone-companion-protocol.md` (repo-side
```

after:

```
  `crates/sky-cua-service/src/phone/`. The companion wire contract is
  authoritative in `docs/runtime/phone-control-v2.md`, with transport rules in
  `docs/runtime/direct-lan.md` (repo-side
```

**Verify**: `uv run pytest scripts/test_agent_guides.py -q` → both tests pass;
`grep -n 'rpc/\|RpcController\|SetupActivity\|adb forward\|loopback\|direct-only' android/phone-companion/AGENTS.md` → no matches.

### Step 4: Fix the ROADMAP dead link

Replace `ROADMAP.md:36` with:

```
  - [x] Privileged helper fast path for Wayland input (root `sky-cua-input-helper` service) — [`docs/features/linux-virtual-input.md`](docs/features/linux-virtual-input.md)
```

**Verify**: `grep -n 'privileged_linux_uinput_helper' ROADMAP.md` → no
matches; `test -e docs/features/linux-virtual-input.md` → exit 0.

### Step 5: Full Python gate

**Verify**: `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest --ignore=scripts/test_browser_control_acceptance.py -q`
→ all clean, all pass.

## Test plan

- `scripts/test_agent_guides.py::test_agent_guide_paths_exist`, observed to
  fail with exactly the five stale citations before Step 2 and pass after
  Step 3.
- `scripts/test_agent_guides.py::test_agent_guides_exist` (sanity: the scan
  found guides at all).

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `uv run pytest scripts/test_agent_guides.py -q` exits 0
- [ ] `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright` all clean
- [ ] `grep -n 'src/daemon\.rs' crates/AGENTS.md crates/sky-cua-service/AGENTS.md; grep -n 'definitions\.rs' crates/sky-cua-client/AGENTS.md; grep -n 'src/backend\.rs' crates/sky-cua-linux/AGENTS.md` prints nothing (at `e928a40` it prints exactly the four stale lines; `crates/sky-cua-platform/src/backend.rs` is real and is cited only in files this command does not scan)
- [ ] `grep -n 'sky-cua-chrome-host\|sky-cua-cosmic-helper\|sky-cua-input-helper' crates/AGENTS.md` shows all three
- [ ] `grep -n 'rpc/\|RpcController\|SetupActivity\|adb forward\|direct-only' android/phone-companion/AGENTS.md` prints nothing
- [ ] `grep -n 'privileged_linux_uinput_helper' ROADMAP.md` prints nothing
- [ ] `git status --porcelain -- crates/ android/ skills/ ROADMAP.md scripts/` lists exactly the eight in-scope files (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- Step 1's failure list differs from the five expected citations (report the
  extra or missing entries; do not loosen the heuristic to make it pass).
- A fact in "Current state" about the Android tree is wrong when you open
  the files (for example a package listed here is missing). Report the
  discrepancy rather than guessing the new layout.
- `git ls-files` pathspecs behave unexpectedly and the test finds fewer than
  10 guides.
- You are tempted to edit `docs/runtime/*.md` to make the Android guide's
  pointers "fully correct"; that is `DOCS-01/02`, not this plan.

## Maintenance notes

- The test deliberately checks only tokens that look like paths. Tokens such
  as `goals/` in the root guide's "do not create" list are not matched
  because they have no extension and no known prefix; keep anti-pattern
  examples extension-free or `...`-marked so they stay exempt. If a guide
  needs to cite a path that intentionally does not exist, write it without
  backticks.
- Extending the same scan to `docs/features/**` and `docs/runtime/**` would
  catch the 101 stale citations the audit found there (`DOCS-06`); do it as
  a separate change with its own exemption list, since research docs are
  dated snapshots.
- A reviewer should read the rewritten Android Structure and Security
  sections against the tree, not the plan: the plan's facts were correct on
  2026-11-03.
- The test resolves paths against the working tree, not the tracked tree, so
  an untracked local directory (for example `android/phone-companion/build/`,
  an immediate child the test searches) could mask a stale citation on one
  machine. Harmless today; if it ever matters, resolve against
  `git ls-files` output instead.
