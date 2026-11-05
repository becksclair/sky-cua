# Plan 013: Put the standalone installer's `~/.codex/config.toml` rewriter under test and make its write atomic

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- scripts/standalone_release.py scripts/test_standalone_release.py`
> (no `..HEAD`, so uncommitted edits count). If either file changed since
> this plan was written, compare the "Current state" excerpts against the
> live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are verbatim unless marked abridged.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

`python3 install.py install` (the standalone install) ends by rewriting the
user's real `~/.codex/config.toml` through `_update_codex_config_for_bundled`
in `scripts/standalone_release.py`, the most-changed code file in the
repository (54 commits, 31 in the last 90 days). That function and its regex-based
`_upsert_toml_key` have no test: the only test that reaches the install path
monkeypatches `_install_codex_plugins` away. It also writes the file with a
plain `write_text` even though an atomic writer (`_write_text_atomically`)
already exists in the same file and is used for the Pi and OpenClaw configs.
A regex regression here, or an interrupted write, corrupts the config that
Codex needs to start. The deliverable is characterization tests that pin the
current output for realistic inputs and check the result parses as TOML,
plus the one-line switch to the atomic writer. Two latent defects the tests
will document (and that are then fixed because the fix is trivial and
covered): a table header followed by a comment (`[features] # x`) is not
recognised, so a duplicate `[features]` table gets appended; and
`re.sub(line, ...)` treats the rendered value as a replacement template, so
backslashes are collapsed and, more realistically on this Linux-only
installer, a non-ASCII home path (`/home/josé/.codex`, which `json.dumps`
renders as `\u00e9`) raises `re.PatternError: bad escape \u` on every
reinstall once the `source` key exists.

## Current state

`scripts/standalone_release.py`:

- Line 47: `PLUGIN_NAMES = ("computer-use", "browser")`.
- Lines 558-559:

  ```python
  def _toml_string(value: Path | str) -> str:
      return json.dumps(str(value))
  ```

  (JSON escaping is valid TOML basic-string escaping for ASCII and BMP
  characters: `\uXXXX` escapes are legal TOML. `json.dumps` defaults to
  `ensure_ascii=True`, so a non-ASCII path becomes `\u00e9`-style escapes,
  which is fine for TOML but is what trips `re.sub` below. Characters outside
  the BMP (emoji) become surrogate pairs, which TOML rejects; see Maintenance.)

- Lines 562-580 (verbatim):

  ```python
  def _upsert_toml_key(config_text: str, header: str, key: str, rendered_value: str) -> str:
      import re

      section_re = re.compile(
          rf"(^\[{re.escape(header)}\]\r?\n)(.*?)(?=^\[|\Z)", re.MULTILINE | re.DOTALL
      )
      match = section_re.search(config_text)
      line = f"{key} = {rendered_value}"
      if match is None:
          separator = "" if not config_text or config_text.endswith("\n") else "\n"
          return f"{config_text}{separator}\n[{header}]\n{line}\n"
      body = match.group(2)
      key_re = re.compile(rf"^{re.escape(key)}\s*=.*$", re.MULTILINE)
      if key_re.search(body):
          new_body = key_re.sub(line, body, count=1)
      else:
          suffix = "" if body.endswith(("\n", "\r\n")) or body == "" else "\n"
          new_body = f"{body}{suffix}{line}\n"
      return config_text[: match.start(2)] + new_body + config_text[match.end(2) :]
  ```

  Two things to notice: `section_re` requires the header line to end right
  after `]` (optional `\r`) then newline, so `[features] # comment` or
  `[features]   ` (trailing spaces) never matches and a second `[features]`
  table is appended (invalid TOML, duplicate table). And `key_re.sub(line, ...)`
  passes `line` as a replacement template, so a backslash in `rendered_value`
  is interpreted by `re`: `\\` collapses to `\` (a Windows path becomes
  `"C:\Users\x\.codex"`, which `tomllib` then rejects with "Invalid hex
  value" on `\U`), and `\u00e9` from a non-ASCII path raises
  `re.PatternError: bad escape \u`. Both only fire when the key already
  exists (the reinstall path).

- Lines 639-675 (verbatim):

  ```python
  def _update_codex_config_for_bundled(codex_home: Path) -> None:
      from datetime import UTC, datetime

      config_path = codex_home / "config.toml"
      try:
          config_text = config_path.read_text(encoding="utf-8")
      except FileNotFoundError:
          config_text = ""
      marketplace_root = codex_home / ".tmp/bundled-marketplaces/openai-bundled"
      config_text = _upsert_toml_key(config_text, "features", "plugins", "true")
      config_text = _upsert_toml_key(
          config_text,
          "marketplaces.openai-bundled",
          "last_updated",
          _toml_string(datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")),
      )
      config_text = _upsert_toml_key(
          config_text,
          "marketplaces.openai-bundled",
          "source_type",
          '"local"',
      )
      config_text = _upsert_toml_key(
          config_text,
          "marketplaces.openai-bundled",
          "source",
          _toml_string(marketplace_root),
      )
      for plugin_id in (f"{name}@openai-bundled" for name in PLUGIN_NAMES):
          config_text = _upsert_toml_key(
              config_text,
              f'plugins."{plugin_id}"',
              "enabled",
              "true",
          )
      config_path.parent.mkdir(parents=True, exist_ok=True)
      config_path.write_text(config_text, encoding="utf-8")
  ```

- Caller: `_install_codex_plugins` (lines 679-697) returns `()` when
  `which("codex")` is `None`, else derives `codex_home = home/.codex` (from
  `home` or `env["HOME"]`), calls `_sync_codex_bundled_marketplace(install_root, codex_home)`
  (which returns early when `install_root/codex/openai-bundled/.agents/plugins/marketplace.json`
  is missing) and then `_update_codex_config_for_bundled(codex_home)`,
  returning `PLUGIN_NAMES`.

- Lines 776-787, the atomic writer already used at 876 and 938:

  ```python
  def _write_text_atomically(path: Path, text: str) -> None:
      write_path = path.resolve(strict=False) if path.is_symlink() else path
      write_path.parent.mkdir(parents=True, exist_ok=True)
      mode = write_path.stat().st_mode if write_path.exists() else 0o600
      temporary = write_path.with_name(f".{write_path.name}.tmp-{os.getpid()}")
      _remove_path(temporary)
      try:
          temporary.write_text(text, encoding="utf-8")
          temporary.chmod(mode)
          os.replace(temporary, write_path)
      finally:
          _remove_path(temporary)
  ```

- `tomllib` is already imported in `standalone_release.py` (and in several
  test modules), so parse-validation needs no new dependency.

`scripts/test_standalone_release.py`:

- Imports `standalone_release`, `Any` (line 10) and defines `_write(path, content)`
  (lines 26-31). The install-path test around lines 730-740 monkeypatches
  `standalone_release._install_codex_plugins` with a fake, so nothing
  exercises the rewriter.
- Exemplar for a config-rewrite test that checks idempotence and byte
  preservation: `test_openclaw_no_prompt_permissions_are_idempotent_and_preserve_agent_config`
  (lines 818-848): writes a config with `_write`, calls the function twice,
  asserts `config.count('approval_policy = "never"') == 1` and that unrelated
  tables survive. Note it asserts the inode is unchanged on a no-op rerun;
  the new Codex test below asserts the opposite, because `last_updated`
  changes on every call so every call rewrites the file.

Conventions: Python typed for basedpyright `standard`; tests per subsystem;
commit style conventional, e.g. `fix(release): rebuild a stale phone companion before bundling`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `uv run ruff format --check scripts` | `N files already formatted` |
| Lint | `uv run ruff check scripts` | `All checks passed!` |
| Types | `uv run basedpyright` | `0 errors, 0 warnings, 0 notes` |
| This test file | `uv run pytest scripts/test_standalone_release.py -q` | all pass |
| All Python | `uv run pytest --ignore=scripts/test_browser_control_acceptance.py -q` | all pass |

## Scope

**In scope** (the only files you should modify):
- `scripts/standalone_release.py` (`_upsert_toml_key` and `_update_codex_config_for_bundled` only)
- `scripts/test_standalone_release.py` (add tests)
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- Making `_update_codex_config_for_bundled` call `_plugin_bundle.update_codex_config`
  (the single-enabled-plugin invariant the other installer enforces). That
  is audit finding `DEBT-15`, a behaviour change in which plugins end up
  enabled; it needs the maintainer's decision.
- Replacing the regex upserter with a TOML library round-trip. `tomllib` is
  read-only and there is no TOML writer dependency; preserving the user's
  formatting is the point of the regex approach.
- `_set_top_level_toml_string`, the Pi/OpenClaw/Hermes config writers.
- Any other function in `standalone_release.py`.

## Git workflow

- Branch: `bex/standalone-codex-config-tests`
- Commit only if the operator authorised it; style:
  `test(release): characterize the Codex config rewriter and write it atomically`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Characterization tests for `_upsert_toml_key` (pure function)

Add to `scripts/test_standalone_release.py`:

```python
def _toml_ok(text: str) -> dict[str, Any]:
    import tomllib

    return tomllib.loads(text)


def test_upsert_toml_key_creates_table_and_key_in_empty_text() -> None:
    out = standalone_release._upsert_toml_key("", "features", "plugins", "true")
    assert out == "\n[features]\nplugins = true\n"
    assert _toml_ok(out) == {"features": {"plugins": True}}


def test_upsert_toml_key_replaces_existing_key_in_place_and_preserves_neighbours() -> None:
    before = 'model = "gpt-5.6-luna"\n\n[features]\nplugins = false\nother = 1\n\n[plugins.example]\nenabled = true\n'
    out = standalone_release._upsert_toml_key(before, "features", "plugins", "true")
    assert out == 'model = "gpt-5.6-luna"\n\n[features]\nplugins = true\nother = 1\n\n[plugins.example]\nenabled = true\n'
    assert _toml_ok(out)["features"] == {"plugins": True, "other": 1}


def test_upsert_toml_key_appends_key_to_existing_table_without_trailing_newline() -> None:
    before = "[features]\nother = 1"
    out = standalone_release._upsert_toml_key(before, "features", "plugins", "true")
    assert _toml_ok(out) == {"features": {"other": 1, "plugins": True}}
    assert out.endswith("plugins = true\n")


def test_upsert_toml_key_handles_crlf_input() -> None:
    before = "[features]\r\nother = 1\r\n"
    out = standalone_release._upsert_toml_key(before, "features", "plugins", "true")
    assert _toml_ok(out) == {"features": {"other": 1, "plugins": True}}


def test_upsert_toml_key_quoted_dotted_header_round_trips() -> None:
    out = standalone_release._upsert_toml_key(
        "", 'plugins."computer-use@openai-bundled"', "enabled", "true"
    )
    assert _toml_ok(out) == {"plugins": {"computer-use@openai-bundled": {"enabled": True}}}


def test_upsert_toml_key_matches_header_with_trailing_comment_or_spaces() -> None:
    # Codex and humans both write `[features] # note` or `[features]   `.
    for header_line in ("[features] # enabled by hand", "[features]   "):
        before = f"{header_line}\nother = 1\n"
        out = standalone_release._upsert_toml_key(before, "features", "plugins", "true")
        parsed = _toml_ok(out)  # a duplicate [features] table would raise here
        assert parsed == {"features": {"other": 1, "plugins": True}}, out
        assert out.count("[features]") == 1, out


def test_upsert_toml_key_inserts_rendered_values_literally() -> None:
    # Existing-key path: the rendered value must survive re.sub untouched.
    # A Windows path exercises backslashes; a non-ASCII home path exercises the
    # `\u00e9` escapes json.dumps emits (the realistic Linux reinstall case).
    for raw in (r"C:\Users\x\.codex", "/home/josé/.codex"):
        rendered = standalone_release._toml_string(raw)
        before = "[marketplaces.openai-bundled]\nsource = \"old\"\n"
        out = standalone_release._upsert_toml_key(
            before, "marketplaces.openai-bundled", "source", rendered
        )
        assert _toml_ok(out)["marketplaces"]["openai-bundled"]["source"] == raw, out
```

The last two tests FAIL at `e928a40` (duplicate table → `tomllib` raises;
Windows path → `re.sub` collapses `\\` to `\` and `tomllib` raises
`TOMLDecodeError: Invalid hex value`; non-ASCII path → `re.PatternError: bad escape \u`).
That is expected; Step 3 fixes both. Run `uv run ruff format scripts` after
adding tests (E501 is ignored in `pyproject.toml`, so long literals only get
reflowed).

**Verify**: `uv run pytest scripts/test_standalone_release.py -q -k upsert_toml_key`
→ 5 pass, 2 fail (`trailing_comment_or_spaces`, `rendered_values_literally`).

### Step 2: Characterization tests for `_update_codex_config_for_bundled`

Add:

```python
def _codex_config_after_install(tmp_path: Path, initial: str | None) -> tuple[Path, str]:
    home = tmp_path / "home"
    codex_home = home / ".codex"
    config_path = codex_home / "config.toml"
    if initial is not None:
        _write(config_path, initial)
    standalone_release._update_codex_config_for_bundled(codex_home)
    return config_path, config_path.read_text(encoding="utf-8")


def test_update_codex_config_for_bundled_creates_valid_config_from_nothing(tmp_path: Path) -> None:
    config_path, text = _codex_config_after_install(tmp_path, None)
    parsed = _toml_ok(text)
    assert parsed["features"] == {"plugins": True}
    marketplace = parsed["marketplaces"]["openai-bundled"]
    assert marketplace["source_type"] == "local"
    assert marketplace["source"] == str(config_path.parent / ".tmp/bundled-marketplaces/openai-bundled")
    assert marketplace["last_updated"].endswith("Z")
    assert parsed["plugins"] == {
        "computer-use@openai-bundled": {"enabled": True},
        "browser@openai-bundled": {"enabled": True},
    }


def test_update_codex_config_for_bundled_preserves_unrelated_content_and_is_idempotent(
    tmp_path: Path,
) -> None:
    initial = (
        'model = "gpt-5.6-luna"\n'
        'approval_policy = "on-request"\n\n'
        "[features]\nplugins = false\nweb_search = true\n\n"
        "[plugins.example]\nenabled = true\n"
    )
    config_path, text = _codex_config_after_install(tmp_path, initial)
    parsed = _toml_ok(text)
    assert parsed["model"] == "gpt-5.6-luna"
    assert parsed["features"] == {"plugins": True, "web_search": True}
    assert parsed["plugins"]["example"] == {"enabled": True}
    assert text.startswith('model = "gpt-5.6-luna"\napproval_policy = "on-request"\n')

    standalone_release._update_codex_config_for_bundled(config_path.parent)
    again = config_path.read_text(encoding="utf-8")
    assert _toml_ok(again)["features"] == parsed["features"]
    assert again.count("[features]") == 1
    assert again.count('[plugins."computer-use@openai-bundled"]') == 1


def test_update_codex_config_for_bundled_writes_atomically_and_keeps_mode(tmp_path: Path) -> None:
    config_path, _ = _codex_config_after_install(tmp_path, 'model = "x"\n')
    # 0o640 rather than the writer's 0o600 default, so a writer that always
    # forces 0o600 would fail this.
    config_path.chmod(0o640)
    inode_before = config_path.stat().st_ino
    standalone_release._update_codex_config_for_bundled(config_path.parent)
    assert config_path.stat().st_mode & 0o777 == 0o640
    assert config_path.stat().st_ino != inode_before  # os.replace of a temp file
    assert not list(config_path.parent.glob(".config.toml.tmp-*"))
```

The atomic-write test FAILS at `e928a40` (in-place `write_text` keeps the
inode). Expected; Step 3 fixes it.

**Verify**: `uv run pytest scripts/test_standalone_release.py -q -k update_codex_config_for_bundled`
→ 2 pass, 1 fail (`writes_atomically_and_keeps_mode`).

### Step 3: Fix the three defects the tests pin

In `scripts/standalone_release.py`:

1. `_upsert_toml_key`: change the header pattern so a header may be followed
   by spaces and/or a `#` comment before the newline:

   ```python
   section_re = re.compile(
       rf"(^\[{re.escape(header)}\][ \t]*(?:#[^\r\n]*)?\r?\n)(.*?)(?=^\[|\Z)",
       re.MULTILINE | re.DOTALL,
   )
   ```

2. `_upsert_toml_key`: insert the replacement literally:

   ```python
   new_body = key_re.sub(lambda _match: line, body, count=1)
   ```

3. `_update_codex_config_for_bundled`: replace the last two lines
   (`config_path.parent.mkdir(...)` and `config_path.write_text(...)`) with
   `_write_text_atomically(config_path, config_text)` (it creates the parent
   and preserves mode, defaulting to `0o600` for a new file). Behaviour
   change, intended: a newly created `config.toml` is now mode `0o600`
   instead of following the umask, and a symlinked `config.toml` (dotfile
   managers) has its target replaced via `os.replace` rather than written
   through; this matches how the Pi and OpenClaw configs are already written.

Run `uv run ruff format scripts`.

**Verify**: `uv run pytest scripts/test_standalone_release.py -q` → `44 passed`
(baseline 34 at `e928a40` plus 10; the three previously failing tests now pass).

### Step 4: Full Python gate

**Verify**: `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest --ignore=scripts/test_browser_control_acceptance.py -q`
→ all clean, all pass.

## Test plan

- Seven `_upsert_toml_key` tests (Step 1), three
  `_update_codex_config_for_bundled` tests (Step 2); three of the ten fail
  before Step 3 and pass after.
- Pattern: `test_openclaw_no_prompt_permissions_are_idempotent_and_preserve_agent_config`.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright` all clean
- [ ] `uv run pytest scripts/test_standalone_release.py -q` reports `44 passed` (34 at `e928a40` plus 10)
- [ ] `sed -n '/def _update_codex_config_for_bundled/,/^def /p' scripts/standalone_release.py | grep -c 'config_path.write_text'` prints `0` (grep exit status 1 is expected; note the fix line `_write_text_atomically(config_path, config_text)` contains `write_text`, which is why the pattern is the full `config_path.write_text`)
- [ ] `grep -c '_write_text_atomically(config_path, config_text)' scripts/standalone_release.py` prints `1`
- [ ] `grep -n 'lambda _match: line' scripts/standalone_release.py` matches
- [ ] `git diff --name-only e928a40 -- scripts/` lists exactly `scripts/standalone_release.py` and `scripts/test_standalone_release.py` (holds before and after an authorised commit)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- A Step 1 or Step 2 test other than the three named ones fails at
  `e928a40` (then current behaviour differs from what the plan pinned;
  report the output rather than editing the assertion to match).
- Any existing test in `test_standalone_release.py` fails after Step 3.
- You find yourself wanting to change which plugins end up enabled; that is
  `DEBT-15`, out of scope.

## Maintenance notes

- The header regex now tolerates `[table] # comment` and trailing spaces; it
  still requires the header at column 0 (TOML allows leading whitespace,
  Codex never writes it). Cases it still mishandles, each verified: a
  `[features]` header as the very last line with no trailing newline gets a
  duplicate table; `[ features ]` (spaces inside the brackets) gets a
  duplicate table; a quoted key (`"plugins" = false`) is not matched and the
  result is invalid TOML; replacing a key on a CRLF line drops its `\r`
  (mixed endings, still valid TOML). Add a test when one of these shows up in
  the field.
- `_toml_string` uses `json.dumps` with `ensure_ascii=True`. Characters
  outside the BMP (an emoji in a home path) become surrogate-pair escapes,
  which TOML rejects even after this plan's fixes. The one-line follow-up is
  `json.dumps(str(value), ensure_ascii=False)` (verified to round-trip `josé`,
  emoji, Windows paths and tabs); it is left out here to keep the plan to
  characterization plus the two pinned fixes.
- `DEBT-15` remains: this writer enables both `computer-use@openai-bundled`
  and `browser@openai-bundled` unconditionally and does not run the
  single-enabled-`computer-use` invariant that `_plugin_bundle.update_codex_config`
  enforces for the other installer. These tests pin that behaviour so a
  deliberate change shows up as a test diff.
- A reviewer should check the Windows-path test still reflects a realistic
  value: `_toml_string` JSON-escapes backslashes, so the rendered TOML is
  `"C:\\Users\\x\\.codex"`, which `tomllib` decodes back to single
  backslashes.
