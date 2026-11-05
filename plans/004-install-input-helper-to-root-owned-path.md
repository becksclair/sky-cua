# Plan 004: Install the root input helper to a root-owned path instead of running a user-writable binary as root

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- scripts/install_mcp_server.py scripts/test_install_flows.py resources/systemd/sky-cua-input-helper.service README.md docs/features/linux-virtual-input.md`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are abridged (the live code is ruff-formatted with one
> list element per line); formatting differences are not drift.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

`scripts/install_mcp_server.py --input-helper` writes a systemd system unit
whose `ExecStart` points at the helper binary inside the install target,
which defaults to `~/.local/share/sky-cua/bin/sky-cua-input-helper`. The unit
runs as root (it must: it owns `/dev/uinput`). Anything that can write as the
user, which on this machine includes every agent shell, MCP host, and package
post-install script, can replace that file and run as root on the next unit
restart or reboot. The packaged unit at
`resources/systemd/sky-cua-input-helper.service` already uses `/usr/bin`, so
the installer's generated unit is the outlier. The generated unit also
interpolated the user-supplied path unquoted; using a fixed system path
removes that failure mode. The fix is to copy the binary to a root-owned
location with `install -o root -g root -m 0755` and point `ExecStart` there.

The location matters: on the primary development host `/usr/local` itself is
owned by the user (`stat` on 2026-11-03: `bex:bex 755 /usr/local`,
`bex:bex 755 /usr/local/lib`, while `/usr/local/libexec`, `/usr/lib`,
`/usr/libexec`, `/opt` are `root:root 755`). A user who can write `/usr/local`
can rename `/usr/local/libexec` aside and recreate the path, so anything under
`/usr/local` does not close the hole there. The plan therefore uses
`/usr/lib/sky-cua/` (Arch convention for a package's internal executables;
nothing in the repo ships a package using that path) and adds a guard that
refuses to install under any existing ancestor that is not root-owned and
write-protected. Nothing in the helper protocol or socket layout changes.

## Current state

- `scripts/install_mcp_server.py`
  - Lines 897-964, `install_input_helper_service(target_dir, *, socket_group=None)`
    (abridged; the live code is ruff-formatted across more lines, content identical):

    ```python
    helper_path = target_dir / entrypoint_path(current_platform(), "sky-cua-input-helper")
    if not helper_path.exists():
        raise FileNotFoundError(...)
    group = socket_group or default_input_helper_group()
    env_path = target_dir / "input-helper.env"
    service_path = target_dir / "sky-cua-input-helper.service"
    env_path.write_text("\n".join([
        "SKY_CUA_INPUT_HELPER_SOCKET=/run/sky-cua/input-helper.sock",
        "SKY_CUA_INPUT_HELPER_SOCKET_MODE=0660",
        f"SKY_CUA_INPUT_HELPER_SOCKET_GROUP={group}",
        "",
    ]), encoding="utf-8")
    service_path.write_text("\n".join([
        "[Unit]",
        "Description=sky-cua privileged uinput helper",
        "After=systemd-udevd.service",
        "",
        "[Service]",
        "Type=simple",
        "EnvironmentFile=/etc/sky-cua/input-helper.env",
        f"ExecStart={helper_path} serve",
        "Restart=on-failure",
        "RestartSec=1s",
        "RuntimeDirectory=sky-cua",
        "RuntimeDirectoryMode=0775",
        "",
        "[Install]",
        "WantedBy=multi-user.target",
        "",
    ]), encoding="utf-8")
    run_sudo(["install", "-D", "-m", "0644", str(env_path), "/etc/sky-cua/input-helper.env"])
    run_sudo(["install", "-D", "-m", "0644", str(service_path), "/etc/systemd/system/sky-cua-input-helper.service"])
    run_sudo(["systemctl", "daemon-reload"])
    run_sudo(["systemctl", "enable", "--now", "sky-cua-input-helper.service"])
    run_sudo(["systemctl", "restart", "sky-cua-input-helper.service"])
    print("Installed and started sky-cua-input-helper.service " f"(socket group: {group}, socket: /run/sky-cua/input-helper.sock).")
    ```

  - Lines 967-975: `default_input_helper_group()` reads
    `SKY_CUA_INPUT_HELPER_SOCKET_GROUP` or the current gid's group name;
    `run_sudo(args)` is `subprocess.run(["sudo", *args], check=True)`. Both
    are module-level functions looked up at call time, so tests can
    monkeypatch them as `install_mcp_server.run_sudo` etc.
  - `target_dir` defaults to `DEFAULT_LOCAL_INSTALL_DIR = Path.home() / ".local" / "share" / "sky-cua"`
    (`scripts/_install_shared.py:28`, imported at line 38).
  - Module constants live near line 70-76 (`CLAUDE_CODE_DENY_RULES`,
    `CLAUDE_CODE_ALLOW_RULES`, `AT_SPI_RESTART_TIMEOUT_SECONDS = 5` on line
    74, `MCP_BROWSER_EVAL_ENV`, ...). `stat` is imported (line 28), `Path`
    (line 32), `shlex` (line 26). `LINUX_X64 = "linux-x64"` is defined in
    `_plugin_bundle.py:37` and imported into this module at line 54.
  - Line 257: `entrypoint_path(platform_id, name)` returns `Path("bin") / name`
    (`.exe` on Windows). Line 132: `current_platform()` wraps
    `current_runtime_platform()` from `_plugin_bundle`.
  - Lines 1121-1122 (inside the main install function) call
    `install_input_helper_service(target_dir, socket_group=input_helper_group)`.
    CLI flags `--input-helper` and `--input-helper-group` are defined at
    lines 1263 and 1271.
  - `shlex` is already imported (line 26).

- `resources/systemd/sky-cua-input-helper.service` — the packaged unit, with
  `ExecStart=/usr/bin/sky-cua-input-helper serve`. It is staged by
  `scripts/build_plugin.py:55` but no script installs it; it documents the
  intended shape. Leave it unchanged.

- `scripts/test_install_flows.py` — no test references
  `install_input_helper_service` or `run_sudo`
  (`grep -n 'run_sudo\|install_input_helper' scripts/test_*.py` → nothing).
  The exemplar for faking subprocess calls is
  `test_refresh_accessibility_bus_restarts_user_atspi` (lines 345-378): it
  monkeypatches `install_mcp_server.sys.platform`, `install_mcp_server.shutil.which`
  and `install_mcp_server.subprocess.run` with a `fake_run(argv, **kwargs)`
  that records `argv` and returns `subprocess.CompletedProcess(argv, returncode=0, stdout="", stderr="")`,
  then asserts the exact recorded `calls` list.

- Docs that describe the install: `README.md` lines 105-107 and 202-204;
  `docs/features/linux-virtual-input.md` lines 68-73 (says the root service
  "owns `/dev/uinput`, exposes `/run/sky-cua/input-helper.sock`"). Neither
  states where the binary lives; add one sentence to each.

- `crates/sky-cua-input-helper/src/server.rs` lines 118-139: the helper
  serves any peer that can open the 0660 socket (group-gated, no peer
  credential check). Out of scope here; see Maintenance.

Conventions: Python is typed for basedpyright `standard`; tests are
per-subsystem (`scripts/AGENTS.md`); commit style is conventional, e.g.
`fix(release): rebuild a stale phone companion before bundling`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `uv run ruff format --check scripts` | `N files already formatted` |
| Lint | `uv run ruff check scripts` | `All checks passed!` |
| Types | `uv run basedpyright` | `0 errors, 0 warnings, 0 notes` |
| Tests (file) | `uv run pytest scripts/test_install_flows.py -q` | all pass |
| Tests (all) | `uv run pytest --ignore=scripts/test_browser_control_acceptance.py -q` | all pass |

## Scope

**In scope** (the only files you should modify):
- `scripts/install_mcp_server.py` (the module constant block, the
  `install_input_helper_service` function, and one new function
  `require_root_owned_ancestors`)
- `scripts/test_install_flows.py` (add tests)
- `README.md` (lines 105-107 or 202-204: one sentence)
- `docs/features/linux-virtual-input.md` (lines 68-73: one sentence)

**Out of scope** (do NOT touch):
- `crates/sky-cua-input-helper/**`. Adding a peer-credential check is a
  hardening change the project has explicitly deprioritised; it is recorded
  under Maintenance.
- `resources/systemd/sky-cua-input-helper.service`.
- `scripts/standalone_release.py`, `install.py`, `scripts/deploy_plugin.py`:
  none of them installs the unit (verified by grep at `e928a40`).
- The socket group default and `RuntimeDirectoryMode`.
- Uninstall logic. There is none today; adding it is a separate decision.

## Git workflow

- Branch: `bex/input-helper-root-owned-binary`
- Commit only if the operator authorised it; style:
  `fix(install): run the uinput helper from a root-owned path`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Copy the helper to a root-owned path and point the unit at it

In `scripts/install_mcp_server.py`:

1. Add a module-level constant on the line after
   `AT_SPI_RESTART_TIMEOUT_SECONDS = 5` (line 74):

   ```python
   INPUT_HELPER_SYSTEM_PATH = Path("/usr/lib/sky-cua/sky-cua-input-helper")
   ```

2. Add a guard function next to `run_sudo` (after line 975):

   ```python
   def require_root_owned_ancestors(path: Path) -> None:
       """Refuse a system path whose existing ancestors a non-root user could replace."""
       for ancestor in path.parents:
           if not ancestor.exists():
               continue
           info = ancestor.stat()
           if info.st_uid != 0 or info.st_mode & (stat.S_IWGRP | stat.S_IWOTH):
               raise RuntimeError(
                   f"{ancestor} is not root-owned and write-protected; refusing to install "
                   "the root input helper under it"
               )
   ```

   In `install_input_helper_service`, call
   `require_root_owned_ancestors(INPUT_HELPER_SYSTEM_PATH)` right after the
   `helper_path.exists()` check, before anything is written and before any
   `run_sudo`.

3. Change the `ExecStart` line to the fixed system path (no `shlex.quote`:
   systemd has its own command-line syntax, expanding `$VAR` and treating `%`
   as a specifier, so POSIX shell quoting is the wrong tool and the constant
   contains nothing that needs quoting):

   ```python
   f"ExecStart={INPUT_HELPER_SYSTEM_PATH} serve",
   ```

4. Before the two existing `run_sudo(["install", ...])` calls, add:

   ```python
   run_sudo(
       [
           "install",
           "-D",
           "-o",
           "root",
           "-g",
           "root",
           "-m",
           "0755",
           str(helper_path),
           str(INPUT_HELPER_SYSTEM_PATH),
       ]
   )
   ```

   GNU coreutils `install -D` creates missing parent directories (verified
   with coreutils 9.12 on the dev host; it also replaces a running binary
   without "Text file busy"). Ordering: binary, env file, unit file,
   `daemon-reload`, `enable --now`, `restart`. The existing `restart` after
   `enable --now` is what picks up a replaced binary on re-install; keep it.

5. Update the final `print(...)` to mention the installed path, e.g.
   `f"(binary: {INPUT_HELPER_SYSTEM_PATH}, socket group: {group}, socket: /run/sky-cua/input-helper.sock)."`

**Verify**: `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright`
→ all clean.

### Step 2: Add installer tests

In `scripts/test_install_flows.py`, add `import os` to the imports (needed
for test 3) and three tests modelled on
`test_refresh_accessibility_bus_restarts_user_atspi`:

1. `test_install_input_helper_service_installs_root_owned_binary(tmp_path, monkeypatch, capsys)`:
   - `target_dir = tmp_path / "sky-cua"`; create
     `target_dir / "bin" / "sky-cua-input-helper"` (write any bytes).
   - `monkeypatch.setattr(install_mcp_server.sys, "platform", "linux")`
   - `monkeypatch.setattr(install_mcp_server, "current_platform", lambda: install_mcp_server.LINUX_X64)`
     (`install_mcp_server.LINUX_X64` exists; it is imported at line 54).
   - `monkeypatch.setattr(install_mcp_server, "require_root_owned_ancestors", lambda _path: None)`
     (the real `/usr/lib` check must not run against the test host).
   - Record sudo calls: `calls: list[list[str]] = []` and
     `monkeypatch.setattr(install_mcp_server, "run_sudo", lambda args: calls.append(list(args)))`.
   - Call `install_mcp_server.install_input_helper_service(target_dir, socket_group="sky-test")`
     (passing `socket_group` means `default_input_helper_group()` and `grp`
     are never called).
   - Assert the rendered unit `(target_dir / "sky-cua-input-helper.service").read_text()`
     contains `ExecStart=/usr/lib/sky-cua/sky-cua-input-helper serve` and
     does **not** contain `str(target_dir)`.
   - Assert the env file contains `SKY_CUA_INPUT_HELPER_SOCKET_GROUP=sky-test`.
   - Assert the complete sudo sequence:

     ```python
     assert calls == [
         [
             "install", "-D", "-o", "root", "-g", "root", "-m", "0755",
             str(target_dir / "bin" / "sky-cua-input-helper"),
             str(install_mcp_server.INPUT_HELPER_SYSTEM_PATH),
         ],
         ["install", "-D", "-m", "0644", str(target_dir / "input-helper.env"), "/etc/sky-cua/input-helper.env"],
         [
             "install", "-D", "-m", "0644",
             str(target_dir / "sky-cua-input-helper.service"),
             "/etc/systemd/system/sky-cua-input-helper.service",
         ],
         ["systemctl", "daemon-reload"],
         ["systemctl", "enable", "--now", "sky-cua-input-helper.service"],
         ["systemctl", "restart", "sky-cua-input-helper.service"],
     ]
     ```

     (ruff will reflow this; let `uv run ruff format scripts` do it.)
   - Assert `str(install_mcp_server.INPUT_HELPER_SYSTEM_PATH) in capsys.readouterr().out`.

2. `test_install_input_helper_service_requires_built_binary(tmp_path, monkeypatch)`:
   same platform patches, empty `target_dir`, assert
   `pytest.raises(FileNotFoundError)` and that no sudo call was recorded.

3. `test_require_root_owned_ancestors_rejects_user_owned_parent(tmp_path)`,
   decorated `@pytest.mark.skipif(os.geteuid() == 0, reason="runs as root")`:
   assert `pytest.raises(RuntimeError)` for
   `install_mcp_server.require_root_owned_ancestors(tmp_path / "x" / "helper")`
   (`tmp_path` is user-owned, so the first existing ancestor fails the check).

**Verify**: `uv run pytest scripts/test_install_flows.py -q` → all pass,
3 new tests.

### Step 3: Document the binary location

- `README.md`, after line 106 ("root and exposes `/run/sky-cua/input-helper.sock`."),
  add: "The installer copies the helper to
  `/usr/lib/sky-cua/sky-cua-input-helper` (root-owned) and the unit runs that
  copy, never the user-writable install tree; it refuses to install under a
  parent directory that is not root-owned."
- `docs/features/linux-virtual-input.md`, in the "Privileged helper" bullet
  (lines 68-73), add the same fact in one sentence.

**Verify**: `grep -n '/usr/lib/sky-cua' README.md docs/features/linux-virtual-input.md scripts/install_mcp_server.py`
→ at least one match in each file.

## Test plan

- New: `test_install_input_helper_service_installs_root_owned_binary`,
  `test_install_input_helper_service_requires_built_binary`,
  `test_require_root_owned_ancestors_rejects_user_owned_parent` in
  `scripts/test_install_flows.py`.
- Pattern: `test_refresh_accessibility_bus_restarts_user_atspi` (same file).
- Verification: `uv run pytest scripts/test_install_flows.py -q` → all pass.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `uv run ruff format --check scripts` and `uv run ruff check scripts` exit 0
- [ ] `uv run basedpyright` reports 0 errors
- [ ] `uv run pytest scripts/test_install_flows.py -q` exits 0 with the 3 new tests
- [ ] `grep -n 'ExecStart={helper_path}' scripts/install_mcp_server.py` prints nothing
- [ ] `grep -n 'INPUT_HELPER_SYSTEM_PATH' scripts/install_mcp_server.py` shows at least the constant, the guard call, the `install` call, the `ExecStart` line and the `print`
- [ ] `grep -n 'def require_root_owned_ancestors' scripts/install_mcp_server.py` matches
- [ ] `git status --porcelain -- scripts/ README.md docs/` lists exactly the four in-scope files (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- `install_input_helper_service` is called from a file other than
  `install_mcp_server.py` (check with `grep -rn install_input_helper_service scripts/`);
  report the caller before changing behaviour.
- The test cannot monkeypatch `current_platform` or `run_sudo` as module
  attributes (they are module-level functions at `e928a40`).
- You are tempted to run the installer for real. Do not; it needs `sudo` and
  modifies the host. The tests must cover the rendered files and the command
  sequence only.

## Maintenance notes

- Re-running the installer overwrites the root-owned copy with the current
  build via `install`, then `systemctl restart` picks it up. If the helper
  ever gains a protocol version bump, remember the running unit is the copy,
  not the install tree.
- `require_root_owned_ancestors` checks only ancestors that already exist.
  `/usr/lib` is root-owned on every mainstream distribution; on a host where
  it is not, the installer now refuses rather than installing a root unit
  that the user could hijack. That refusal is the correct outcome.
- The socket is still 0660 and gated only by group membership; the helper
  does no `SO_PEERCRED` check (`crates/sky-cua-input-helper/src/server.rs:118-139`).
  Where the default group is broad (`users`), every member can inject input.
  Deliberately not addressed here per the project's hardening policy; a
  dedicated `sky-cua-input` group would be the smallest next step.
- A reviewer should check the sudo command ordering in the test matches the
  code exactly; the test is the only guard against someone re-adding the
  user-path `ExecStart`.
- The testing VM harness stops the unit by name
  (`scripts/run_gui_testing_vm_smoke.py:265`, `:1243`); the name is unchanged.
