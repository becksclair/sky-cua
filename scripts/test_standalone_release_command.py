from __future__ import annotations

import json
import subprocess
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

import _standalone_release_command as release_command
import standalone_release

Step = tuple[tuple[str, ...], int, str]
DEFAULT_RELEASE_VERSION = release_command.calculate_release_version(
    standalone_release.PRODUCT_VERSION
)
DEFAULT_RELEASE_MAJOR, _DEFAULT_RELEASE_MINOR, _DEFAULT_RELEASE_PATCH = (
    release_command.parse_stable_version(DEFAULT_RELEASE_VERSION)
)
HIGHER_REMOTE_VERSION = f"{DEFAULT_RELEASE_MAJOR + 1}.0.0"
PUSH_URL = "ssh://example.invalid/sky-cua.git"


class ScriptedRunner:
    def __init__(self, steps: Sequence[Step]) -> None:
        self.steps = list(steps)
        self.calls: list[tuple[str, ...]] = []

    def __call__(self, command: Sequence[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
        actual = tuple(command)
        self.calls.append(actual)
        assert kwargs["check"] is False
        assert kwargs["capture_output"] is True
        assert kwargs["text"] is True
        assert self.steps, f"unexpected command: {actual}"
        expected, returncode, stdout = self.steps.pop(0)
        assert actual == expected
        stderr = "scripted failure" if returncode else ""
        return subprocess.CompletedProcess(command, returncode, stdout, stderr)


def _write(path: Path, content: str = "fixture\n", *, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    if executable:
        path.chmod(0o755)


def _run_release(**kwargs: Any) -> int:
    return release_command.release_command(
        product_version=standalone_release.PRODUCT_VERSION, **kwargs
    )


def _release_source(version: str) -> str:
    return (
        "from pathlib import Path\n\n"
        f'PRODUCT_VERSION = "{version}"\n'
        'UNRELATED = "preserve exactly"\n'
    )


def _release_checkout(tmp_path: Path) -> Path:
    repo = tmp_path / "release-repo"
    (repo / ".git").mkdir(parents=True)
    _write(repo / "scripts/build_plugin.py")
    _write(repo / release_command.VERSION_PATH, _release_source(standalone_release.PRODUCT_VERSION))
    return repo


def _preflight_steps(target: str = DEFAULT_RELEASE_VERSION) -> list[Step]:
    old_head = "a" * 40
    tag = f"standalone-v{target}"
    return [
        (("git", "status", "--porcelain=v1", "--untracked-files=all"), 0, ""),
        (("git", "symbolic-ref", "--quiet", "--short", "HEAD"), 0, "main\n"),
        (("git", "config", "--get", "branch.main.remote"), 0, "upstream\n"),
        (("git", "config", "--get", "branch.main.merge"), 0, "refs/heads/main\n"),
        (("git", "remote", "get-url", "--push", "--all", "upstream"), 0, f"{PUSH_URL}\n"),
        (("git", "rev-parse", "HEAD"), 0, f"{old_head}\n"),
        (
            ("git", "ls-remote", "--exit-code", PUSH_URL, "refs/heads/main"),
            0,
            f"{old_head}\trefs/heads/main\n",
        ),
        (("git", "show-ref", "--verify", "--quiet", f"refs/tags/{tag}"), 1, ""),
        (
            (
                "git",
                "ls-remote",
                "--tags",
                "--refs",
                PUSH_URL,
                "refs/tags/standalone-v*",
            ),
            0,
            "",
        ),
    ]


def _successful_release_steps(target: str = DEFAULT_RELEASE_VERSION) -> list[Step]:
    relative = release_command.VERSION_PATH.as_posix()
    commit = "b" * 40
    tag_object = "c" * 40
    tag = f"standalone-v{target}"
    tag_ref = f"refs/tags/{tag}"
    return [
        *_preflight_steps(target),
        (("just", "verify-python"), 0, "verified\n"),
        (
            ("git", "status", "--porcelain=v1", "--untracked-files=all"),
            0,
            f" M {relative}\n",
        ),
        (("git", "diff", "--name-only"), 0, f"{relative}\n"),
        (("git", "diff", "--cached", "--name-only"), 0, ""),
        (("git", "diff", "--check", "--", relative), 0, ""),
        (("git", "add", "--", relative), 0, ""),
        (
            ("git", "status", "--porcelain=v1", "--untracked-files=all"),
            0,
            f"M  {relative}\n",
        ),
        (("git", "diff", "--name-only"), 0, ""),
        (("git", "diff", "--cached", "--name-only"), 0, f"{relative}\n"),
        (("git", "diff", "--cached", "--check", "--", relative), 0, ""),
        (
            ("git", "commit", "-m", f"Release standalone v{target}", "--", relative),
            0,
            "committed\n",
        ),
        (("git", "status", "--porcelain=v1", "--untracked-files=all"), 0, ""),
        (("git", "rev-parse", "HEAD"), 0, f"{commit}\n"),
        (
            ("git", "diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"),
            0,
            f"{relative}\n",
        ),
        (("git", "show", f"HEAD:{relative}"), 0, _release_source(target)),
        (("git", "rev-parse", "HEAD^"), 0, f"{'a' * 40}\n"),
        (
            ("git", "tag", "-a", tag, "-m", f"sky-cua standalone v{target}"),
            0,
            "",
        ),
        (("git", "rev-parse", tag_ref), 0, f"{tag_object}\n"),
        (("git", "cat-file", "-t", tag_ref), 0, "tag\n"),
        (("git", "rev-list", "-n", "1", tag_ref), 0, f"{commit}\n"),
        (
            (
                "git",
                "push",
                "--atomic",
                "--no-follow-tags",
                PUSH_URL,
                "refs/heads/main:refs/heads/main",
                f"{tag_ref}:{tag_ref}",
            ),
            0,
            "",
        ),
        (
            (
                "git",
                "ls-remote",
                "--exit-code",
                PUSH_URL,
                "refs/heads/main",
                tag_ref,
                f"{tag_ref}^{{}}",
            ),
            0,
            (f"{commit}\trefs/heads/main\n{tag_object}\t{tag_ref}\n{commit}\t{tag_ref}^{{}}\n"),
        ),
    ]


@pytest.mark.parametrize(
    ("current", "bump", "explicit", "expected"),
    [
        ("1.2.3", "patch", None, "1.2.4"),
        ("1.2.3", "minor", None, "1.3.0"),
        ("1.2.3", "major", None, "2.0.0"),
        ("1.2.3", "minor", "1.2.4", "1.2.4"),
        ("0.1.1", "minor", None, "0.2.0"),
    ],
)
def test_calculate_release_version(
    current: str, bump: str, explicit: str | None, expected: str
) -> None:
    assert (
        release_command.calculate_release_version(current, bump=bump, explicit=explicit) == expected
    )


@pytest.mark.parametrize("explicit", ["1.2.3", "1.2.2", "1.0.0"])
def test_explicit_release_version_must_strictly_increase(explicit: str) -> None:
    with pytest.raises(ValueError, match="must be greater"):
        release_command.calculate_release_version("1.2.3", explicit=explicit)


def test_release_cli_defaults_minor_and_rejects_conflicts_or_unstable_versions() -> None:
    parser = standalone_release._argument_parser()
    args = parser.parse_args(["release"])
    assert (args.command, args.bump, args.version) == ("release", "minor", None)

    with pytest.raises(SystemExit):
        parser.parse_args(["release", "--patch", "--major"])
    with pytest.raises(SystemExit):
        parser.parse_args(["release", "--version", "1.2.3-rc.1"])
    with pytest.raises(SystemExit):
        parser.parse_args(["release", "--version", "01.2.3"])


@pytest.mark.parametrize(
    "destination",
    ["https://example.invalid/repo.git", "git@example.invalid:repo.git", "helper::repo"],
)
def test_concrete_repository_preserves_unambiguous_urls(tmp_path: Path, destination: str) -> None:
    assert release_command._concrete_repository(destination, repo_root=tmp_path) == destination


def test_concrete_repository_absolutizes_relative_local_path(tmp_path: Path) -> None:
    assert release_command._concrete_repository("release-target", repo_root=tmp_path) == str(
        tmp_path / "release-target"
    )


@pytest.mark.parametrize(
    ("steps", "message"),
    [
        (
            [(("git", "status", "--porcelain=v1", "--untracked-files=all"), 0, "?? new\n")],
            "clean checkout",
        ),
        (
            [
                (("git", "status", "--porcelain=v1", "--untracked-files=all"), 0, ""),
                (("git", "symbolic-ref", "--quiet", "--short", "HEAD"), 0, "topic\n"),
            ],
            "branch main",
        ),
        (
            [
                (("git", "status", "--porcelain=v1", "--untracked-files=all"), 0, ""),
                (("git", "symbolic-ref", "--quiet", "--short", "HEAD"), 1, ""),
            ],
            "checking current branch failed",
        ),
        (
            [
                *_preflight_steps()[:2],
                (("git", "config", "--get", "branch.main.remote"), 1, ""),
            ],
            "reading main upstream remote failed",
        ),
        (
            [
                *_preflight_steps()[:3],
                (("git", "config", "--get", "branch.main.merge"), 0, "refs/heads/topic\n"),
            ],
            "upstream must target",
        ),
        (
            [
                *_preflight_steps()[:4],
                (
                    ("git", "remote", "get-url", "--push", "--all", "upstream"),
                    0,
                    "ssh://one.invalid/repo.git\nssh://two.invalid/repo.git\n",
                ),
            ],
            "must resolve to exactly one push destination",
        ),
        (
            [
                *_preflight_steps()[:6],
                (
                    ("git", "ls-remote", "--exit-code", PUSH_URL, "refs/heads/main"),
                    0,
                    f"{'d' * 40}\trefs/heads/main\n",
                ),
            ],
            "must exactly equal remote main",
        ),
        (
            [
                *_preflight_steps()[:7],
                (
                    (
                        "git",
                        "show-ref",
                        "--verify",
                        "--quiet",
                        f"refs/tags/standalone-v{DEFAULT_RELEASE_VERSION}",
                    ),
                    0,
                    "",
                ),
            ],
            "local tag already exists",
        ),
        (
            [
                *_preflight_steps()[:8],
                (
                    (
                        "git",
                        "ls-remote",
                        "--tags",
                        "--refs",
                        PUSH_URL,
                        "refs/tags/standalone-v*",
                    ),
                    0,
                    f"{'e' * 40}\trefs/tags/standalone-v{DEFAULT_RELEASE_VERSION}\n",
                ),
            ],
            "remote tag already exists",
        ),
        (
            [
                *_preflight_steps()[:8],
                (
                    (
                        "git",
                        "ls-remote",
                        "--tags",
                        "--refs",
                        PUSH_URL,
                        "refs/tags/standalone-v*",
                    ),
                    0,
                    f"{'e' * 40}\trefs/tags/standalone-v{HIGHER_REMOTE_VERSION}\n",
                ),
            ],
            f"must be greater than published standalone version {HIGHER_REMOTE_VERSION}",
        ),
    ],
)
def test_release_preflight_rejections(tmp_path: Path, steps: Sequence[Step], message: str) -> None:
    repo = _release_checkout(tmp_path)
    runner = ScriptedRunner(steps)

    with pytest.raises(release_command.ReleaseError, match=message):
        release_command._release_preflight(DEFAULT_RELEASE_VERSION, runner=runner, repo_root=repo)

    assert not runner.steps


def test_release_happy_path_has_exact_order_and_scope(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    repo = _release_checkout(tmp_path)
    original = (repo / release_command.VERSION_PATH).read_text(encoding="utf-8")
    runner = ScriptedRunner(_successful_release_steps())

    assert _run_release(runner=runner, repo_root=repo) == 0

    assert not runner.steps
    rewritten = (repo / release_command.VERSION_PATH).read_text(encoding="utf-8")
    assert rewritten == original.replace(
        f'PRODUCT_VERSION = "{standalone_release.PRODUCT_VERSION}"',
        f'PRODUCT_VERSION = "{DEFAULT_RELEASE_VERSION}"',
    )
    report = json.loads(capsys.readouterr().out)
    assert report == {
        "branch": "main",
        "commit": "b" * 40,
        "remote": "upstream",
        "status": "pushed",
        "tag": f"standalone-v{DEFAULT_RELEASE_VERSION}",
        "version": DEFAULT_RELEASE_VERSION,
    }


@pytest.mark.parametrize(
    "failed_command",
    [("just", "verify-python"), ("git", "commit"), ("git", "tag"), ("git", "push")],
)
def test_release_stops_at_failure_without_cleanup_or_later_side_effects(
    tmp_path: Path, failed_command: tuple[str, ...]
) -> None:
    repo = _release_checkout(tmp_path)
    steps = _successful_release_steps()
    failure_index = next(
        index
        for index, (command, _returncode, _stdout) in enumerate(steps)
        if command[:2] == failed_command
    )
    command, _returncode, _stdout = steps[failure_index]
    steps[failure_index] = (command, 1, "")
    runner = ScriptedRunner(steps[: failure_index + 1])

    with pytest.raises(release_command.ReleaseError, match="failed"):
        _run_release(runner=runner, repo_root=repo)

    assert not runner.steps
    assert f'PRODUCT_VERSION = "{DEFAULT_RELEASE_VERSION}"' in (
        repo / release_command.VERSION_PATH
    ).read_text(encoding="utf-8")
    assert not any(call[:2] == ("git", "reset") for call in runner.calls)
    assert not any(call[:2] == ("git", "clean") for call in runner.calls)


def test_release_rejects_verify_changes_outside_version_file(tmp_path: Path) -> None:
    repo = _release_checkout(tmp_path)
    steps = [
        *_preflight_steps(),
        (("just", "verify-python"), 0, ""),
        (
            ("git", "status", "--porcelain=v1", "--untracked-files=all"),
            0,
            " M scripts/standalone_release.py\n?? generated.txt\n",
        ),
        (("git", "diff", "--name-only"), 0, "scripts/standalone_release.py\n"),
        (("git", "diff", "--cached", "--name-only"), 0, ""),
    ]
    runner = ScriptedRunner(steps)

    with pytest.raises(release_command.ReleaseError, match="release scope"):
        _run_release(runner=runner, repo_root=repo)

    assert not runner.steps
    assert not any(call[:2] == ("git", "add") for call in runner.calls)


def test_release_rejects_remote_readback_mismatch_after_atomic_push(tmp_path: Path) -> None:
    repo = _release_checkout(tmp_path)
    steps = _successful_release_steps()
    command, returncode, output = steps[-1]
    steps[-1] = (command, returncode, output.replace("b" * 40, "d" * 40, 1))
    runner = ScriptedRunner(steps)

    with pytest.raises(release_command.ReleaseError, match="remote release refs"):
        _run_release(runner=runner, repo_root=repo)

    assert not runner.steps


def test_release_end_to_end_with_isolated_git_remote(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    intended_remote = repo / "release-target"
    shadow_first = tmp_path / "shadow-first.git"
    shadow_second = tmp_path / "shadow-second.git"
    _write(repo / "scripts/build_plugin.py")
    version_path = repo / release_command.VERSION_PATH
    _write(version_path, _release_source(standalone_release.PRODUCT_VERSION))

    def git(*arguments: str, cwd: Path = repo) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *arguments],
            cwd=cwd,
            check=True,
            capture_output=True,
            text=True,
        )

    git("init", "--initial-branch=main")
    git("config", "user.name", "Release Test")
    git("config", "user.email", "release-test@example.invalid")
    git("add", "scripts/build_plugin.py", release_command.VERSION_PATH.as_posix())
    git("commit", "-m", "Initial fixture")
    git("init", "--bare", intended_remote.name)
    _write(repo / ".git/info/exclude", f"{intended_remote.name}/\n")
    git("remote", "add", "origin", intended_remote.name)
    git("push", "--set-upstream", "origin", "main")
    git("init", "--bare", str(shadow_first), cwd=tmp_path)
    git("init", "--bare", str(shadow_second), cwd=tmp_path)
    git("remote", "add", intended_remote.name, str(shadow_first))
    git("remote", "set-url", "--add", "--push", intended_remote.name, str(shadow_first))
    git("remote", "set-url", "--add", "--push", intended_remote.name, str(shadow_second))
    git("tag", "-a", "unrelated-v1", "-m", "Unrelated reachable tag")
    git("config", "push.followTags", "true")

    def runner(command: Sequence[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
        if list(command) == ["just", "verify-python"]:
            return subprocess.CompletedProcess(command, 0, "verified\n", "")
        return subprocess.run(list(command), **kwargs)

    assert _run_release(runner=runner, repo_root=repo) == 0

    tag = f"standalone-v{DEFAULT_RELEASE_VERSION}"
    release_commit = git("rev-parse", "HEAD").stdout.strip()
    assert git("status", "--porcelain=v1", "--untracked-files=all").stdout == ""
    assert git("show", "-s", "--format=%s", "HEAD").stdout.strip() == (
        f"Release standalone v{DEFAULT_RELEASE_VERSION}"
    )
    assert git("cat-file", "-t", f"refs/tags/{tag}").stdout.strip() == "tag"
    assert git("rev-list", "-n", "1", f"refs/tags/{tag}").stdout.strip() == release_commit
    remote_refs = git(
        "ls-remote",
        str(intended_remote),
        "refs/heads/main",
        f"refs/tags/{tag}^{{}}",
    ).stdout
    assert f"{release_commit}\trefs/heads/main" in remote_refs
    assert f"{release_commit}\trefs/tags/{tag}^{{}}" in remote_refs
    assert git("ls-remote", str(intended_remote), "refs/tags/unrelated-v1").stdout == ""
    assert git("ls-remote", str(shadow_first), f"refs/tags/{tag}").stdout == ""
    assert git("ls-remote", str(shadow_second), f"refs/tags/{tag}").stdout == ""
    assert version_path.read_text(encoding="utf-8") == _release_source(DEFAULT_RELEASE_VERSION)


def test_release_rejects_version_below_existing_remote_release(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    remote = tmp_path / "remote.git"
    _write(repo / "scripts/build_plugin.py")
    version_path = repo / release_command.VERSION_PATH
    original_source = _release_source(standalone_release.PRODUCT_VERSION)
    _write(version_path, original_source)

    def git(*arguments: str, cwd: Path = repo) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *arguments],
            cwd=cwd,
            check=True,
            capture_output=True,
            text=True,
        )

    git("init", "--initial-branch=main")
    git("config", "user.name", "Release Test")
    git("config", "user.email", "release-test@example.invalid")
    git("add", "scripts/build_plugin.py", release_command.VERSION_PATH.as_posix())
    git("commit", "-m", "Initial fixture")
    initial_commit = git("rev-parse", "HEAD").stdout.strip()
    higher_tag = f"standalone-v{HIGHER_REMOTE_VERSION}"
    git("tag", "-a", higher_tag, "-m", "Existing higher release")
    git("init", "--bare", str(remote), cwd=tmp_path)
    git("remote", "add", "origin", str(remote))
    git("push", "--set-upstream", "origin", "main")
    git("push", "origin", f"refs/tags/{higher_tag}")

    with pytest.raises(release_command.ReleaseError, match="must be greater"):
        _run_release(runner=subprocess.run, repo_root=repo)

    assert git("rev-parse", "HEAD").stdout.strip() == initial_commit
    assert version_path.read_text(encoding="utf-8") == original_source
    assert git("tag", "--list", f"standalone-v{DEFAULT_RELEASE_VERSION}").stdout == ""


def test_release_rejects_multiple_push_destinations_before_writing(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    first_remote = tmp_path / "first.git"
    second_remote = tmp_path / "second.git"
    _write(repo / "scripts/build_plugin.py")
    version_path = repo / release_command.VERSION_PATH
    original_source = _release_source(standalone_release.PRODUCT_VERSION)
    _write(version_path, original_source)

    def git(*arguments: str, cwd: Path = repo) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *arguments],
            cwd=cwd,
            check=True,
            capture_output=True,
            text=True,
        )

    git("init", "--initial-branch=main")
    git("config", "user.name", "Release Test")
    git("config", "user.email", "release-test@example.invalid")
    git("add", "scripts/build_plugin.py", release_command.VERSION_PATH.as_posix())
    git("commit", "-m", "Initial fixture")
    initial_commit = git("rev-parse", "HEAD").stdout.strip()
    git("init", "--bare", str(first_remote), cwd=tmp_path)
    git("init", "--bare", str(second_remote), cwd=tmp_path)
    git("remote", "add", "origin", str(first_remote))
    git("push", "--set-upstream", "origin", "main")
    git("remote", "set-url", "--add", "--push", "origin", str(first_remote))
    git("remote", "set-url", "--add", "--push", "origin", str(second_remote))

    with pytest.raises(release_command.ReleaseError, match="exactly one push destination"):
        _run_release(runner=subprocess.run, repo_root=repo)

    assert git("rev-parse", "HEAD").stdout.strip() == initial_commit
    assert version_path.read_text(encoding="utf-8") == original_source
    target_ref = f"refs/tags/standalone-v{DEFAULT_RELEASE_VERSION}"
    assert git("ls-remote", str(first_remote), target_ref).stdout == ""
    assert git("ls-remote", str(second_remote), target_ref).stdout == ""
