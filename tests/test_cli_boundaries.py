from __future__ import annotations

from pathlib import Path

from hieronymus.cli_boundaries import DIRECT_STORE_COMMANDS, DIRECT_STORE_MCP_ADAPTER


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


def test_mcp_direct_store_adapter_boundary_is_explicit() -> None:
    assert DIRECT_STORE_MCP_ADAPTER.name == "hieronymus-mcp"
    assert DIRECT_STORE_MCP_ADAPTER.consumer == "agent-automation"
    assert "stdio MCP adapter" in DIRECT_STORE_MCP_ADAPTER.reason
    assert "human CLI output" not in DIRECT_STORE_MCP_ADAPTER.reason


def test_service_toolkit_mentions_every_direct_store_command() -> None:
    docs = Path("docs/service-toolkit.md").read_text(encoding="utf-8")

    for entry in DIRECT_STORE_COMMANDS:
        assert f"`hiero {entry.name}`" in docs
        assert entry.reason in docs
    assert DIRECT_STORE_MCP_ADAPTER.reason in docs


def test_memory_dreaming_session_start_json_examples_request_json() -> None:
    docs = Path("docs/memory-dreaming.md").read_text(encoding="utf-8")

    for line in docs.splitlines():
        if line.startswith("hieronymus session-start "):
            assert "--json" in line
