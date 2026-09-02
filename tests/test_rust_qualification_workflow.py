"""Line-oriented policy tests for the manual Rust qualification live workflow."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "rust-qualification-live.yml"
CHECKOUT_SHA = "34e114876b0b11c390a56381ad16ebd13914f8d5"
SETUP_UV_SHA = "d0d8abe699bfb85fec6de9f7adb5ae17292296ff"
SETUP_BUN_SHA = "0c5077e51419868618aeaa5fe8019c62421857d6"
RUST_TOOLCHAIN_SHA = "4360b52568e2003a75bf9bc1d59f33a8e3fc893c"
UPLOAD_ARTIFACT_SHA = "ea165f8d65b6e75b540449e92b4886f43607fa02"
EXPECTED_BUN_VERSION = "1.4.0"
EXPECTED_TOOLCHAIN = "1.96.0"


def _workflow_lines() -> list[str]:
    return WORKFLOW.read_text().splitlines()


def _indent(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def _find_line(lines: list[str], expected: str) -> int:
    index = next((index for index, line in enumerate(lines) if line == expected), None)
    if index is None:
        preview = "\n".join(lines[:10])
        raise AssertionError(
            f"Expected workflow line not found: {expected!r}. "
            f"Scanned {len(lines)} lines. First lines:\n{preview}"
        )
    return index


def _block_after(lines: list[str], start: int) -> list[str]:
    start_indent = _indent(lines[start])
    block: list[str] = []

    for line in lines[start + 1 :]:
        if not line:
            continue
        if _indent(line) <= start_indent:
            break
        block.append(line)
    return block


def _step_blocks(lines: list[str]) -> list[list[str]]:
    return [
        [line, *_block_after(lines, index)]
        for index, line in enumerate(lines)
        if line.startswith("      - ")
    ]


def _step_value(step: list[str], key: str) -> str | None:
    prefix = f"{key}: "
    for line in step:
        stripped = line.strip()
        if stripped.startswith(prefix):
            return stripped.removeprefix(prefix)
        if stripped.startswith(f"- {prefix}"):
            return stripped.removeprefix(f"- {prefix}")
    return None


def _uses_lines(lines: list[str]) -> list[str]:
    uses: list[str] = []
    for step in _step_blocks(lines):
        if any(line.strip().startswith("- uses:") for line in step):
            value = _step_value(step, "uses")
            if value is not None:
                uses.append(value)
    return uses


def _step_with_uses(steps: list[list[str]], value: str) -> list[str]:
    step = next(step for step in steps if _step_value(step, "uses") == value)
    assert step is not None
    return step


def test_live_workflow_dispatch_is_the_only_trigger() -> None:
    lines = _workflow_lines()

    trigger = _block_after(lines, _find_line(lines, "on:"))
    assert trigger == ["  workflow_dispatch:"]


def test_live_workflow_job_is_read_only_and_fully_pinned() -> None:
    lines = _workflow_lines()
    job = _block_after(lines, _find_line(lines, "  qualify:"))

    assert "    runs-on: ubuntu-latest" in job
    assert "    permissions:" in job
    assert "      contents: read" in job
    assert _uses_lines(job) == [
        f"actions/checkout@{CHECKOUT_SHA}",
        f"astral-sh/setup-uv@{SETUP_UV_SHA}",
        f"oven-sh/setup-bun@{SETUP_BUN_SHA}",
        f"dtolnay/rust-toolchain@{RUST_TOOLCHAIN_SHA}",
        f"actions/upload-artifact@{UPLOAD_ARTIFACT_SHA}",
    ]
    assert "          persist-credentials: false" in job


def test_live_workflow_action_refs_are_full_commit_shas() -> None:
    for value in _uses_lines(_workflow_lines()):
        _name, separator, ref = value.rpartition("@")
        assert separator == "@", value
        assert re.fullmatch(r"[0-9a-f]{40}", ref) is not None, value


def test_live_workflow_pins_exact_tool_versions() -> None:
    steps = _step_blocks(_workflow_lines())

    bun_step = _step_with_uses(steps, f"oven-sh/setup-bun@{SETUP_BUN_SHA}")
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_BUN_VERSION}"'

    toolchain_step = _step_with_uses(steps, f"dtolnay/rust-toolchain@{RUST_TOOLCHAIN_SHA}")
    assert _step_value(toolchain_step, "toolchain") == f'"{EXPECTED_TOOLCHAIN}"'
    assert _step_value(toolchain_step, "targets") == "x86_64-unknown-linux-gnu"
    assert _step_value(toolchain_step, "components") == "clippy,rustfmt"


def test_live_workflow_acquires_prerequisites_before_offline_live_execution() -> None:
    lines = _workflow_lines()
    job = _block_after(lines, _find_line(lines, "  qualify:"))
    steps = _step_blocks(job)
    names = [_step_value(step, "name") for step in steps]

    checkout_index = names.index(None)
    acquire_index = names.index("Acquire locked prerequisites")
    live_index = names.index("Run isolated live qualification")
    assert checkout_index < acquire_index < live_index

    acquire_step = steps[acquire_index]
    assert "          uv sync --frozen" in acquire_step
    assert "          uv run python -m tools.qualification.acquire semantic-model" in acquire_step
    assert "          uv run python -m tools.qualification.acquire onnx-runtime" in acquire_step
    assert "          bun install --cwd frontend --frozen-lockfile" in acquire_step

    live_step = steps[live_index]
    assert '          HIERONYMUS_QUALIFICATION_LIVE: "1"' in live_step
    assert '          CARGO_NET_OFFLINE: "true"' in live_step
    assert "        run: uv run python -m tools.qualification.run all --write" in live_step

    offline_lines = [line for line in job if "CARGO_NET_OFFLINE" in line]
    assert offline_lines == ['          CARGO_NET_OFFLINE: "true"']


def test_live_workflow_publishes_records_only_as_artifacts() -> None:
    steps = _step_blocks(_workflow_lines())

    upload_step = _step_with_uses(steps, f"actions/upload-artifact@{UPLOAD_ARTIFACT_SHA}")
    assert upload_step == [
        f"      - uses: actions/upload-artifact@{UPLOAD_ARTIFACT_SHA}",
        "        with:",
        "          name: rust-qualification-records",
        "          path: |",
        "            qualification/records/*.json",
        "            docs/qualification/rust/*.md",
    ]


def test_live_workflow_never_commits_or_pushes() -> None:
    text = WORKFLOW.read_text()

    assert "git add" not in text
    assert "git commit" not in text
    assert "git push" not in text
    assert "git config" not in text
    assert "checkout" in text
    assert text.count("persist-credentials: false") == 1
