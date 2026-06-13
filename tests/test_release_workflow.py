import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
EXPECTED_RELEASE_BUN_VERSION = "1.3.14"
CHECKOUT_SHA = "34e114876b0b11c390a56381ad16ebd13914f8d5"
SETUP_UV_SHA = "d0d8abe699bfb85fec6de9f7adb5ae17292296ff"
SETUP_PYTHON_SHA = "a309ff8b426b58ec0e2a45f0f869d46889d02405"
SETUP_BUN_SHA = "0c5077e51419868618aeaa5fe8019c62421857d6"


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


def _uses_lines(lines: list[str]) -> list[str]:
    uses: list[str] = []
    for step in _step_blocks(lines):
        if any(line.strip().startswith("- uses:") for line in step):
            value = _step_value(step, "uses")
            if value is not None:
                uses.append(value)
    return uses


def _assert_pinned_uses(lines: list[str], expected: list[str]) -> None:
    uses = _uses_lines(lines)
    for action in expected:
        assert action in uses
    assert all("@" in action and len(action.rsplit("@", maxsplit=1)[1]) == 40 for action in uses)
    assert not any(action.endswith(("@v4", "@v6")) for action in uses)


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

    _assert_pinned_uses(
        verify,
        [
            f"actions/checkout@{CHECKOUT_SHA}",
            f"astral-sh/setup-uv@{SETUP_UV_SHA}",
            f"actions/setup-python@{SETUP_PYTHON_SHA}",
            f"oven-sh/setup-bun@{SETUP_BUN_SHA}",
        ],
    )

    checkout = next(
        step
        for step in _step_blocks(verify)
        if _step_value(step, "uses") == f"actions/checkout@{CHECKOUT_SHA}"
    )
    assert "        with:" in checkout
    assert "          persist-credentials: false" in checkout
    assert not any(line.strip().startswith("token:") for line in checkout)

    bun_step = next(
        step
        for step in _step_blocks(verify)
        if _step_value(step, "uses") == f"oven-sh/setup-bun@{SETUP_BUN_SHA}"
    )
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_RELEASE_BUN_VERSION}"'

    verify_runs = [line for line in verify if line.startswith("      - run: ")]
    for command in [
        "      - run: uv sync --dev",
        "      - run: bun install --cwd frontend --frozen-lockfile",
        "      - run: bun run --cwd frontend build",
        "      - run: uv run pytest",
        "      - run: uv run ruff check .",
        "      - run: uv run ruff format --check .",
    ]:
        assert command in verify_runs

    assert verify_runs.index("      - run: uv sync --dev") < verify_runs.index(
        "      - run: bun install --cwd frontend --frozen-lockfile"
    )
    assert verify_runs.index(
        "      - run: bun install --cwd frontend --frozen-lockfile"
    ) < verify_runs.index("      - run: bun run --cwd frontend build")
    assert verify_runs.index("      - run: bun run --cwd frontend build") < verify_runs.index(
        "      - run: uv run pytest"
    )


def test_release_workflow_release_job_publishes_after_verification() -> None:
    lines = _workflow_lines()
    release = _block_after(lines, _find_line(lines, "  release:"))

    assert "    needs: verify" in release
    assert "    permissions:" in release
    assert "      contents: write" in release

    _assert_pinned_uses(
        release,
        [
            f"actions/checkout@{CHECKOUT_SHA}",
            f"astral-sh/setup-uv@{SETUP_UV_SHA}",
            f"actions/setup-python@{SETUP_PYTHON_SHA}",
            f"oven-sh/setup-bun@{SETUP_BUN_SHA}",
        ],
    )

    checkout = next(
        step
        for step in _step_blocks(release)
        if _step_value(step, "uses") == f"actions/checkout@{CHECKOUT_SHA}"
    )
    assert "        with:" in checkout
    assert "          fetch-depth: 0" in checkout
    assert "          persist-credentials: false" in checkout
    assert "          token: ${{ secrets.GITHUB_TOKEN }}" in checkout

    release_runs = [line for line in release if line.startswith("      - run: ")]
    assert release_runs == [
        "      - run: uv sync --dev",
        "      - run: uv run python -m hieronymus.release_guard",
        "      - run: uv run semantic-release version",
        "      - run: uv run python -m hieronymus.release_guard",
        "      - run: uv run semantic-release publish",
    ]

    for command in [
        "      - run: uv run semantic-release version",
        "      - run: uv run semantic-release publish",
    ]:
        step = _step_block(release, command)
        assert "        env:" in step
        assert "          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}" in step

    steps = _step_blocks(release)
    bun_step = next(
        step for step in steps if _step_value(step, "uses") == f"oven-sh/setup-bun@{SETUP_BUN_SHA}"
    )
    assert _step_value(bun_step, "bun-version") == f'"{EXPECTED_RELEASE_BUN_VERSION}"'


def test_release_workflow_guards_alpha_version_before_publish() -> None:
    lines = _workflow_lines()
    release = _block_after(lines, _find_line(lines, "  release:"))

    guard_command = "      - run: uv run python -m hieronymus.release_guard"
    computed_guard_name = "      - name: Check next release version"
    version_command = "      - run: uv run semantic-release version"
    publish_command = "      - run: uv run semantic-release publish"

    guard_indexes = [index for index, line in enumerate(release) if line == guard_command]
    version_indexes = [index for index, line in enumerate(release) if line == version_command]
    computed_guard_index = release.index(computed_guard_name)
    publish_index = release.index(publish_command)
    computed_guard = _step_block(release, computed_guard_name)

    assert '          NEXT_VERSION="$(uv run semantic-release version --print)"' in computed_guard
    assert (
        '          uv run python -m hieronymus.release_guard --version "$NEXT_VERSION"'
        in computed_guard
    )
    assert "        env:" in computed_guard
    assert "          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}" in computed_guard
    assert len(guard_indexes) == 2
    assert len(version_indexes) == 1
    version_index = version_indexes[0]
    assert guard_indexes[0] < computed_guard_index < version_index
    assert version_index < guard_indexes[1] < publish_index


def test_pyproject_configures_semantic_release() -> None:
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    pyproject = tomllib.loads(pyproject_text)

    assert "python-semantic-release" in pyproject_text

    semantic_release = pyproject["tool"]["semantic_release"]
    assert semantic_release["version_toml"] == ["pyproject.toml:project.version"]
    assert semantic_release["version_variables"] == ["src/hieronymus/__init__.py:__version__"]
    assert semantic_release["tag_format"] == "v{version}"
    assert semantic_release["build_command"] == (
        "bun install --cwd frontend --frozen-lockfile && bun run --cwd frontend build && uv build"
    )
    assert semantic_release["commit_message"] == "chore(release): v{version}"
    assert "branch" not in semantic_release
    assert "changelog_file" not in semantic_release

    assert semantic_release["branches"]["main"] == {
        "match": "main",
        "prerelease": False,
    }
    assert semantic_release["changelog"]["default_templates"]["changelog_file"] == "CHANGELOG.md"


def test_project_metadata_stays_on_alpha_version_line() -> None:
    pyproject_text = (ROOT / "pyproject.toml").read_text()
    pyproject = tomllib.loads(pyproject_text)
    init_text = (ROOT / "src" / "hieronymus" / "__init__.py").read_text()
    lockfile = tomllib.loads((ROOT / "uv.lock").read_text())
    hieronymus_package = next(
        package
        for package in lockfile["package"]
        if package["name"] == "hieronymus" and package.get("source") == {"editable": "."}
    )
    project_version = pyproject["project"]["version"]

    assert project_version.startswith("0.")
    assert f'__version__ = "{project_version}"' in init_text
    assert hieronymus_package["source"] == {"editable": "."}
    assert hieronymus_package["version"] == project_version
    assert "α" not in project_version
    assert "α" not in init_text
    assert "α" not in hieronymus_package["version"]
    assert '"1.0.0"' not in init_text
    assert '"1.1.0"' not in init_text
    assert hieronymus_package["version"] not in {"1.0.0", "1.1.0"}
