import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
EXPECTED_RELEASE_BUN_VERSION = "1.3.14"


def _workflow_lines() -> list[str]:
    return WORKFLOW.read_text().splitlines()


def _indent(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def _find_line(lines: list[str], expected: str) -> int:
    return next(index for index, line in enumerate(lines) if line == expected)


def _block_after(lines: list[str], start: int) -> list[str]:
    start_indent = _indent(lines[start])
    block: list[str] = []

    for line in lines[start + 1 :]:
        if not line:
            continue
        if line and _indent(line) <= start_indent:
            break
        block.append(line)

    return block


def _step_block(lines: list[str], uses_or_run_line: str) -> list[str]:
    step_start = _find_line(lines, uses_or_run_line)
    return [lines[step_start], *_block_after(lines, step_start)]


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


def test_release_workflow_serializes_main_releases() -> None:
    lines = _workflow_lines()

    trigger = _block_after(lines, _find_line(lines, "on:"))
    assert "  push:" in trigger
    assert "    branches: [main]" in trigger

    concurrency = _block_after(lines, _find_line(lines, "concurrency:"))
    assert concurrency == [
        "  group: release-${{ github.ref }}",
        "  cancel-in-progress: false",
    ]

    assert "permissions:" not in lines


def test_release_workflow_verify_job_uses_read_only_credentials() -> None:
    lines = _workflow_lines()
    verify = _block_after(lines, _find_line(lines, "  verify:"))

    assert "    runs-on: ubuntu-latest" in verify
    assert "    permissions:" in verify
    assert "      contents: read" in verify

    checkout = _step_block(verify, "      - uses: actions/checkout@v4")
    assert "        with:" in checkout
    assert "          persist-credentials: false" in checkout
    assert not any(line.strip().startswith("token:") for line in checkout)

    expected_runs = [
        "      - run: uv sync --dev",
        "      - run: uv run pytest",
        "      - run: uv run ruff check .",
        "      - run: uv run ruff format --check .",
    ]
    for command in expected_runs:
        assert command in verify


def test_release_workflow_release_job_publishes_after_verification() -> None:
    lines = _workflow_lines()
    release = _block_after(lines, _find_line(lines, "  release:"))

    assert "    needs: verify" in release
    assert "    permissions:" in release
    assert "      contents: write" in release

    checkout = _step_block(release, "      - uses: actions/checkout@v4")
    assert "        with:" in checkout
    assert "          fetch-depth: 0" in checkout
    assert "          token: ${{ secrets.GITHUB_TOKEN }}" in checkout

    expected_runs = [
        "      - run: uv sync --dev",
        "      - run: uv run semantic-release version",
        "      - run: uv run semantic-release publish",
    ]
    for command in expected_runs:
        assert command in release

    for command in [
        "      - run: uv run semantic-release version",
        "      - run: uv run semantic-release publish",
    ]:
        step = _step_block(release, command)
        assert "        env:" in step
        assert "          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}" in step

    steps = _step_blocks(release)
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
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_RELEASE_BUN_VERSION}"'


def test_pyproject_configures_semantic_release() -> None:
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    pyproject = tomllib.loads(pyproject_text)

    assert "python-semantic-release" in pyproject_text

    semantic_release = pyproject["tool"]["semantic_release"]
    assert semantic_release["version_toml"] == ["pyproject.toml:project.version"]
    assert semantic_release["version_variables"] == ["src/hieronymus/__init__.py:__version__"]
    assert semantic_release["tag_format"] == "v{version}"
    assert semantic_release["build_command"] == (
        "bun --cwd frontend install --frozen-lockfile && bun --cwd frontend run build && uv build"
    )
    assert semantic_release["commit_message"] == "chore(release): v{version}"
    assert "branch" not in semantic_release
    assert "changelog_file" not in semantic_release

    assert semantic_release["branches"]["main"] == {
        "match": "main",
        "prerelease": False,
    }
    assert semantic_release["changelog"]["default_templates"]["changelog_file"] == "CHANGELOG.md"
