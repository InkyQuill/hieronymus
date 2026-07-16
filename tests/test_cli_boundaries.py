from __future__ import annotations

from pathlib import Path

from click.testing import CliRunner

from hieronymus.cli import main
from hieronymus import cli_boundaries
from hieronymus.cli_boundaries import DIRECT_STORE_COMMANDS


def _normalize_whitespace(value: str) -> str:
    return " ".join(value.split())


def test_direct_store_cli_commands_are_documented() -> None:
    names = [entry.name for entry in DIRECT_STORE_COMMANDS]

    assert names == [
        "init-series",
        "propose-term",
        "validate",
        "remember",
        "session-start",
        "session-complete",
        "remember-short",
        "recall",
        "feedback",
        "dream",
    ]
    assert all(entry.reason.strip() for entry in DIRECT_STORE_COMMANDS)
    assert all(
        entry.consumer in {"human-debug", "agent-automation", "maintenance"}
        for entry in DIRECT_STORE_COMMANDS
    )


def test_mcp_is_not_a_direct_store_boundary() -> None:
    assert not hasattr(cli_boundaries, "DIRECT_STORE_MCP_ADAPTER")


def test_service_toolkit_mentions_every_direct_store_command() -> None:
    docs = Path("docs/service-toolkit.md").read_text(encoding="utf-8")
    normalized_docs = _normalize_whitespace(docs)

    for entry in DIRECT_STORE_COMMANDS:
        assert f"`hiero {entry.name}`" in docs
        assert _normalize_whitespace(entry.reason) in normalized_docs
    assert "hieronymus-mcp` is also a direct-store boundary" not in normalized_docs


def test_service_toolkit_session_complete_example_matches_click_options() -> None:
    docs = Path("docs/service-toolkit.md").read_text(encoding="utf-8")
    result = CliRunner().invoke(main, ["session-complete", "--help"])

    assert "--summary <summary>" not in docs
    assert "provider key env configuration" not in docs
    assert "--event helpful" not in docs
    assert "--event confirmed_by_user" in docs
    assert result.exit_code == 0
    assert "--json" in result.output
    assert "--summary" not in result.output


def test_memory_dreaming_session_start_json_examples_request_json() -> None:
    docs = Path("docs/memory-dreaming.md").read_text(encoding="utf-8")

    for line in docs.splitlines():
        if line.startswith("hieronymus session-start "):
            assert "--json" in line
