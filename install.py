#!/usr/bin/env python3
"""One-shot sky-cua installer entry point.

Clone the repository and run `python3 install.py`. The heavy lifting lives in
scripts/installer.py; this shim only validates the interpreter and wires up
the import path so the installer can reuse the existing deploy scripts.
"""

from __future__ import annotations

import sys
from pathlib import Path

# The whole point of this guard is older interpreters that the project's
# minimum version does not cover, so the "outdated version block" lint is
# wrong here by construction.
if sys.version_info < (3, 12):  # noqa: UP036
    raise SystemExit("sky-cua install requires Python 3.12 or newer")

REPO_ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(REPO_ROOT / "scripts"))

from installer import main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(main())
