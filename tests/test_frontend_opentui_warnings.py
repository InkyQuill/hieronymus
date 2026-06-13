from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
WARNING_MARKERS = (
    "not wrapped in act",
    "Possible EventTarget memory leak detected",
    "TerminalConsoleCache",
)


def test_frontend_opentui_tests_do_not_emit_lifecycle_warnings() -> None:
    if shutil.which("bun") is None:
        pytest.skip("Bun is not installed")

    result = subprocess.run(
        ["bun", "--cwd", "frontend", "test"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
        timeout=30,
    )

    assert result.returncode == 0, result.stdout
    warning_lines = [
        line
        for line in result.stdout.splitlines()
        if any(marker in line for marker in WARNING_MARKERS)
    ]
    assert warning_lines == []
