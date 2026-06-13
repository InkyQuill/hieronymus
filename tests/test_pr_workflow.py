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
    uses: list[str] = []
    for step in _step_blocks(lines):
        if any(line.strip().startswith("- uses:") for line in step):
            value = _step_value(step, "uses")
            if value is not None:
                uses.append(value)
    return uses


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
        f"actions/checkout@{CHECKOUT_SHA}",
        f"astral-sh/setup-uv@{SETUP_UV_SHA}",
        f"actions/setup-python@{SETUP_PYTHON_SHA}",
        f"oven-sh/setup-bun@{SETUP_BUN_SHA}",
    ]
    assert "          persist-credentials: false" in backend

    steps = _step_blocks(backend)
    bun_step = next(
        (
            step
            for step in steps
            if _step_value(step, "uses") == f"oven-sh/setup-bun@{SETUP_BUN_SHA}"
        ),
        None,
    )
    assert bun_step is not None
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_FRONTEND_BUN_VERSION}"'

    backend_runs = [line for line in backend if line.startswith("      - run: ")]
    assert backend_runs == [
        "      - run: uv sync --dev",
        "      - run: bun install --cwd frontend --frozen-lockfile",
        "      - run: bun run --cwd frontend build",
        "      - run: uv run pytest",
        "      - run: uv run ruff check .",
        "      - run: uv run ruff format --check .",
    ]
    assert backend_runs.index("      - run: bun install --cwd frontend --frozen-lockfile") < (
        backend_runs.index("      - run: uv run pytest")
    )
    assert backend_runs.index("      - run: bun run --cwd frontend build") < backend_runs.index(
        "      - run: uv run pytest"
    )


def test_pr_workflow_frontend_job_runs_bun_tests_and_build() -> None:
    lines = _workflow_lines()
    frontend = _block_after(lines, _find_line(lines, "  frontend:"))

    assert "    runs-on: ubuntu-latest" in frontend
    assert "    permissions:" in frontend
    assert "      contents: read" in frontend
    assert _uses_lines(frontend) == [
        f"actions/checkout@{CHECKOUT_SHA}",
        f"oven-sh/setup-bun@{SETUP_BUN_SHA}",
    ]
    assert "          persist-credentials: false" in frontend

    steps = _step_blocks(frontend)
    bun_step = next(
        (
            step
            for step in steps
            if _step_value(step, "uses") == f"oven-sh/setup-bun@{SETUP_BUN_SHA}"
        ),
        None,
    )
    assert bun_step is not None
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_FRONTEND_BUN_VERSION}"'

    assert "      - run: bun install --cwd frontend --frozen-lockfile" in frontend
    assert "      - run: bun run --cwd frontend format" in frontend
    assert "      - run: bun run --cwd frontend typecheck" in frontend
    assert "      - run: bun run --cwd frontend test" in frontend
    assert "      - run: bun run --cwd frontend build" in frontend
