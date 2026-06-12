from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "pr.yml"
EXPECTED_FRONTEND_BUN_VERSION = "1.3.14"
CHECKOUT_SHA = "34e114876b0b11c390a56381ad16ebd13914f8d5"
SETUP_UV_SHA = "d0d8abe699bfb85fec6de9f7adb5ae17292296ff"
SETUP_PYTHON_SHA = "a309ff8b426b58ec0e2a45f0f869d46889d02405"
SETUP_BUN_SHA = "0c5077e51419868618aeaa5fe8019c62421857d6"


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
    return [
        line for step in _step_blocks(lines) for line in step if line.strip().startswith("- uses:")
    ]


def test_pr_workflow_runs_on_pull_requests_to_main() -> None:
    lines = _workflow_lines()

    trigger = _block_after(lines, _find_line(lines, "on:"))
    assert "  pull_request:" in trigger
    assert "    branches: [main]" in trigger

    concurrency = _block_after(lines, _find_line(lines, "concurrency:"))
    assert concurrency == [
        "  group: pr-${{ github.ref }}",
        "  cancel-in-progress: true",
    ]


def test_pr_workflow_backend_job_runs_python_checks() -> None:
    lines = _workflow_lines()
    backend = _block_after(lines, _find_line(lines, "  backend:"))

    assert "    runs-on: ubuntu-latest" in backend
    assert "    permissions:" in backend
    assert "      contents: read" in backend

    assert _uses_lines(backend) == [
        f"      - uses: actions/checkout@{CHECKOUT_SHA}",
        f"      - uses: astral-sh/setup-uv@{SETUP_UV_SHA}",
        f"      - uses: actions/setup-python@{SETUP_PYTHON_SHA}",
    ]
    assert "          persist-credentials: false" in backend

    for command in [
        "      - run: uv sync --dev",
        "      - run: uv run pytest",
        "      - run: uv run ruff check .",
        "      - run: uv run ruff format --check .",
    ]:
        assert command in backend


def test_pr_workflow_frontend_job_runs_bun_tests_and_build() -> None:
    lines = _workflow_lines()
    frontend = _block_after(lines, _find_line(lines, "  frontend:"))

    assert "    runs-on: ubuntu-latest" in frontend
    assert "    permissions:" in frontend
    assert "      contents: read" in frontend
    assert _uses_lines(frontend) == [
        f"      - uses: actions/checkout@{CHECKOUT_SHA}",
        f"      - uses: oven-sh/setup-bun@{SETUP_BUN_SHA}",
    ]
    assert "          persist-credentials: false" in frontend

    steps = _step_blocks(frontend)
    bun_step = next(
        (
            step
            for step in steps
            if "setup-bun" in (_step_value(step, "uses") or "").lower()
            or "setup-bun" in (_step_value(step, "name") or "").lower()
        ),
        None,
    )
    assert bun_step is not None
    bun_uses = _step_value(bun_step, "uses")
    assert bun_uses is not None
    assert bun_uses.startswith("oven-sh/setup-bun@")
    assert len(bun_uses.rsplit("@", maxsplit=1)[1]) == 40
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_FRONTEND_BUN_VERSION}"'

    assert "      - run: bun install --cwd frontend --frozen-lockfile" in frontend
    assert "      - run: bun run --cwd frontend format" in frontend
    assert "      - run: bun run --cwd frontend typecheck" in frontend
    assert "      - run: bun --cwd frontend test" in frontend
    assert "      - run: bun run --cwd frontend build" in frontend
