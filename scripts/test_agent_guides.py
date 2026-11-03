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
