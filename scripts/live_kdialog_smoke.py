#!/usr/bin/env python3
"""Compatibility wrapper for the renamed live desktop smoke harness."""

import runpy
from pathlib import Path

runpy.run_path(str(Path(__file__).with_name("live_desktop_smoke.py")), run_name="__main__")
