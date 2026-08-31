"""Generate a deterministic inventory of the FastMCP tool registry."""

from __future__ import annotations

import argparse
import asyncio
import json
import tempfile
from pathlib import Path

from hieronymus import mcp_server
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext

_PROTOCOL_REVISION = "2026-07-28"
_TRANSPORTS = ["stdio", "streamable-http"]
_PRIVATE_BRIDGE = {
    "path": "/api/mcp/{operation}",
    "classification": "private_python_bridge",
    "disposition": "remove",
    "adr": "docs/adr/0015-mcp-protocol-and-transport.md",
}
_SERIES_ARGUMENTS = {
    "slug": "synthetic-series",
    "title": "Synthetic Series",
    "source_language": "ja",
    "target_language": "en",
    "language_tags": ["ja", "en"],
}
_DATA_ROOT_ERROR = {
    "error": {
        "type": "ValueError",
        "message": "data root is not a directory: <DATA_ROOT>",
    }
}


def _sort_object_keys(value: object) -> object:
    if isinstance(value, dict):
        return {key: _sort_object_keys(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        return [_sort_object_keys(item) for item in value]
    return value


async def registered_tools() -> list[dict[str, object]]:
    """Return the authoritative FastMCP registry in stable name order."""
    tools = await mcp_server.server.list_tools()
    return sorted(
        (
            {
                "name": tool.name,
                "description": tool.description or "",
                "input_schema": _sort_object_keys(tool.inputSchema),
            }
            for tool in tools
        ),
        key=lambda item: str(item["name"]),
    )


def snapshot_mcp() -> dict[str, object]:
    """Return the registered MCP tools and ADR-pinned transport declarations."""
    tools = asyncio.run(registered_tools())
    return {
        "protocol_revision": _PROTOCOL_REVISION,
        "transports": list(_TRANSPORTS),
        "derived_tool_count": len(tools),
        "tools": tools,
        "private_python_bridge": dict(_PRIVATE_BRIDGE),
    }


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def _invoke(
    config: HieronymusConfig,
    tool_name: str,
    arguments: dict[str, object],
) -> object:
    operation = tool_name.removeprefix("hieronymus_")
    return mcp_server.invoke_daemon_operation(config, operation, arguments)


def _create_series(config: HieronymusConfig) -> dict[str, object]:
    result = _invoke(config, "hieronymus_series_create", dict(_SERIES_ARGUMENTS))
    assert isinstance(result, dict)
    return result


def _create_concept(config: HieronymusConfig) -> dict[str, object]:
    result = _invoke(
        config,
        "hieronymus_concept_create",
        {
            "canonical_name": "Sense",
            "description": "Synthetic game-system concept.",
            "confidence": 0.8,
            "semantic_tags": ["ability"],
        },
    )
    assert isinstance(result, dict)
    return result


def _create_facet(config: HieronymusConfig, concept_id: int) -> dict[str, object]:
    result = _invoke(
        config,
        "hieronymus_concept_facet_add",
        {
            "concept_id": concept_id,
            "value": "センス",
            "language": "ja",
            "language_tags": ["ja"],
            "kind": "name",
            "confidence": 0.9,
            "story_scopes": ["volume:1"],
            "semantic_tags": ["ability"],
        },
    )
    assert isinstance(result, dict)
    return result


def _create_crystal(config: HieronymusConfig, *, crystal_type: str = "rule") -> int:
    _create_series(config)
    context = TranslationContext(
        series_slug=str(_SERIES_ARGUMENTS["slug"]),
        source_language=str(_SERIES_ARGUMENTS["source_language"]),
        target_language=str(_SERIES_ARGUMENTS["target_language"]),
        task_type="translation",
    )
    return CrystalStore(config).add_crystal(
        context,
        crystal_type=crystal_type,
        text="センス is translated as Sense, not Feeling.",
        title="Sense rendering",
        strength=0.8,
        confidence=0.9,
    )


def _start_session(config: HieronymusConfig) -> int:
    _create_series(config)
    result = _invoke(
        config,
        "hieronymus_session_start",
        {"series_slug": _SERIES_ARGUMENTS["slug"], "volume": "1", "chapter": "2"},
    )
    assert isinstance(result, dict)
    return int(result["session_id"])


def _propose_term(config: HieronymusConfig) -> int:
    _create_series(config)
    result = _invoke(
        config,
        "hieronymus_termbase_propose",
        {
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "category": "character",
            "source_text": "ユン",
            "canonical_translation": "Yun",
            "tags": ["name"],
            "notes": "Synthetic character name.",
        },
    )
    assert isinstance(result, dict)
    return int(result["term_id"])


def _fixture_case(tool_name: str, root: Path) -> tuple[dict[str, object], object]:
    config = HieronymusConfig(data_root=root / "data")

    if tool_name == "hieronymus_status":
        arguments: dict[str, object] = {}
        return arguments, {"service": {"available": True, "mode": "local-http"}}
    if tool_name in {"hieronymus_series_create", "hieronymus_series_init"}:
        arguments = dict(_SERIES_ARGUMENTS)
    elif tool_name == "hieronymus_series_list":
        _create_series(config)
        arguments = {}
    elif tool_name == "hieronymus_series_set_language_tags":
        _create_series(config)
        arguments = {"series_id": 1, "language_tags": ["ja", "en", "fantasy"]}
    elif tool_name == "hieronymus_concept_list":
        _create_concept(config)
        arguments = {"status": "candidate", "semantic_tag": "ability"}
    elif tool_name == "hieronymus_concept_get":
        concept = _create_concept(config)
        arguments = {"concept_id": concept["id"]}
    elif tool_name == "hieronymus_concept_create":
        arguments = {
            "canonical_name": "Sense",
            "description": "Synthetic game-system concept.",
            "confidence": 0.8,
            "semantic_tags": ["ability"],
        }
    elif tool_name == "hieronymus_concept_update":
        concept = _create_concept(config)
        arguments = {
            "concept_id": concept["id"],
            "description": "Approved synthetic concept.",
            "status": "established",
            "confidence": 0.9,
        }
    elif tool_name == "hieronymus_concept_archive":
        concept = _create_concept(config)
        arguments = {"concept_id": concept["id"], "reason": "Synthetic archive case."}
    elif tool_name == "hieronymus_concept_merge":
        source = _create_concept(config)
        target = _invoke(config, "hieronymus_concept_create", {"canonical_name": "Ability"})
        assert isinstance(target, dict)
        arguments = {
            "source_concept_id": source["id"],
            "target_concept_id": target["id"],
            "reason": "Synthetic merge case.",
        }
    elif tool_name == "hieronymus_concept_rename":
        concept = _create_concept(config)
        arguments = {"concept_id": concept["id"], "new_label": "Sense Ability"}
    elif tool_name == "hieronymus_concept_facet_add":
        concept = _create_concept(config)
        arguments = {
            "concept_id": concept["id"],
            "value": "センス",
            "language": "ja",
            "language_tags": ["ja"],
            "kind": "name",
            "confidence": 0.9,
            "story_scopes": ["volume:1"],
            "semantic_tags": ["ability"],
        }
    elif tool_name == "hieronymus_concept_facet_update":
        concept = _create_concept(config)
        facet = _create_facet(config, int(concept["id"]))
        arguments = {
            "facet_id": facet["id"],
            "value": "Sense",
            "language": "en",
            "language_tags": ["en"],
            "semantic_tags": ["canonical"],
        }
    elif tool_name == "hieronymus_concept_facet_list":
        concept = _create_concept(config)
        _create_facet(config, int(concept["id"]))
        arguments = {"concept_id": concept["id"]}
    elif tool_name == "hieronymus_concept_facet_set_canonical":
        concept = _create_concept(config)
        facet = _create_facet(config, int(concept["id"]))
        arguments = {"concept_id": concept["id"], "facet_id": facet["id"]}
    elif tool_name == "hieronymus_concept_semantic_tags_set":
        concept = _create_concept(config)
        arguments = {"concept_id": concept["id"], "semantic_tags": ["ability", "ui"]}
    elif tool_name == "hieronymus_crystal_link_concept":
        crystal_id = _create_crystal(config)
        concept = _create_concept(config)
        arguments = {
            "crystal_id": crystal_id,
            "concept_id": concept["id"],
            "confidence": 0.9,
        }
    elif tool_name == "hieronymus_crystal_story_scopes_set":
        crystal_id = _create_crystal(config)
        arguments = {
            "crystal_id": crystal_id,
            "story_scopes": ["volume:1", "chapter:2"],
            "confidence": 0.9,
        }
    elif tool_name == "hieronymus_crystal_semantic_tags_set":
        crystal_id = _create_crystal(config)
        arguments = {
            "crystal_id": crystal_id,
            "semantic_tags": ["ability", "ui"],
            "confidence": 0.9,
        }
    elif tool_name == "hieronymus_rule_crystals_list":
        _create_crystal(config)
        arguments = {"status": "active", "series_slug": _SERIES_ARGUMENTS["slug"]}
    elif tool_name == "hieronymus_rule_crystal_archive":
        arguments = {"crystal_id": _create_crystal(config)}
    elif tool_name == "hieronymus_rule_crystal_validate":
        arguments = {"crystal_id": _create_crystal(config)}
    elif tool_name == "hieronymus_termbase_propose":
        _create_series(config)
        arguments = {
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "category": "character",
            "source_text": "ユン",
            "canonical_translation": "Yun",
            "tags": ["name"],
            "notes": "Synthetic character name.",
        }
    elif tool_name == "hieronymus_termbase_approve":
        term_id = _propose_term(config)
        arguments = {"series_slug": _SERIES_ARGUMENTS["slug"], "term_id": term_id}
    elif tool_name in {"hieronymus_termbase_contract", "hieronymus_termbase_validate"}:
        term_id = _propose_term(config)
        _invoke(
            config,
            "hieronymus_termbase_approve",
            {"series_slug": _SERIES_ARGUMENTS["slug"], "term_id": term_id},
        )
        arguments = {
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "raw_text": "ユン walked home.",
        }
        if tool_name == "hieronymus_termbase_validate":
            arguments["translated_text"] = "Yun walked home."
    elif tool_name == "hieronymus_memory_search":
        _create_series(config)
        arguments = {"series_slug": _SERIES_ARGUMENTS["slug"], "query": "Sense"}
    elif tool_name == "hieronymus_rag_import":
        _create_series(config)
        source = root / "chapter.txt"
        source.write_text("Cooking Talent appears here.", encoding="utf-8")
        arguments = {
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "path": str(source),
            "source_ref": "chapter.txt",
            "language_tags": ["en"],
        }
    elif tool_name == "hieronymus_rag_search":
        _create_series(config)
        source = root / "chapter.txt"
        source.write_text("Cooking Talent appears here.", encoding="utf-8")
        _invoke(
            config,
            "hieronymus_rag_import",
            {
                "series_slug": _SERIES_ARGUMENTS["slug"],
                "path": str(source),
                "source_ref": "chapter.txt",
            },
        )
        arguments = {
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "query": "Cooking Talent",
        }
    elif tool_name == "hieronymus_memory_add":
        _create_series(config)
        arguments = {
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "kind": "correction",
            "text": "Use Sense, not Feeling.",
            "source_ref": "chapter-2",
            "importance": 5,
        }
    elif tool_name == "hieronymus_session_start":
        _create_series(config)
        arguments = {
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "volume": "1",
            "chapter": "2",
        }
    elif tool_name == "hieronymus_session_complete":
        arguments = {"session_id": _start_session(config)}
    elif tool_name == "hieronymus_short_term_add":
        arguments = {
            "session_id": _start_session(config),
            "kind": "correction",
            "text": "Use Sense as a game-system term.",
            "source_role": "agent",
            "source_ref": "chapter-2",
            "metadata": {"line": 12},
        }
    elif tool_name == "hieronymus_short_term_add_batch":
        arguments = {
            "session_id": _start_session(config),
            "items": [
                {
                    "source_role": "agent",
                    "kind": "reading-conclusion",
                    "text": "The narrator distrusts the council.",
                    "source_ref": "chapter-1",
                },
                {
                    "source_role": "agent",
                    "kind": "reading-conclusion",
                    "text": "The council controls the city.",
                    "source_ref": "chapter-1",
                },
            ],
        }
    elif tool_name == "hieronymus_recall":
        arguments = {
            "session_id": _start_session(config),
            "series_slug": _SERIES_ARGUMENTS["slug"],
            "query": "Sense UI",
        }
    elif tool_name == "hieronymus_feedback":
        arguments = {
            "session_id": _start_session(config),
            "correction_text": "Use Sense, not Feeling.",
        }
    elif tool_name == "hieronymus_dream":
        session_id = _start_session(config)
        _invoke(
            config,
            "hieronymus_short_term_add",
            {
                "session_id": session_id,
                "kind": "correction",
                "text": "Use Sense as a game-system term.",
            },
        )
        _invoke(config, "hieronymus_session_complete", {"session_id": session_id})
        arguments = {}
    elif tool_name == "hieronymus_concept_proposals_list":
        arguments = {}
    else:
        raise ValueError(f"missing synthetic MCP fixture case: {tool_name}")

    output = _invoke(config, tool_name, arguments)
    normalized_arguments = _replace_root(arguments, root)
    normalized_output = _replace_root(output, root)
    assert isinstance(normalized_arguments, dict)
    return normalized_arguments, normalized_output


def _replace_root(value: object, root: Path) -> object:
    if isinstance(value, str):
        return value.replace(str(root), "<SYNTHETIC_ROOT>")
    if isinstance(value, dict):
        return {key: _replace_root(item, root) for key, item in value.items()}
    if isinstance(value, list):
        return [_replace_root(item, root) for item in value]
    if isinstance(value, tuple):
        return [_replace_root(item, root) for item in value]
    return value


def _behavior_tests(tool_name: str) -> list[str]:
    tests = ["tests/compatibility/test_mcp_inventory.py"]
    if "rag_" in tool_name:
        tests.append("tests/test_mcp_rag.py")
    elif "short_term_" in tool_name:
        tests.append("tests/test_mcp_agent_ingestion.py")
    elif tool_name in {
        "hieronymus_series_list",
        "hieronymus_series_set_language_tags",
    }:
        tests.append("tests/test_series_language_tags.py")
    elif tool_name == "hieronymus_concept_proposals_list":
        tests.append("tests/test_mcp_server.py")
    elif any(token in tool_name for token in ("concept_", "crystal_", "series_")):
        tests.append("tests/test_mcp_memory_primitives.py")
    else:
        tests.append("tests/test_mcp_server.py")
    return tests


def _write_tool_fixtures(repo_root: Path, tools: list[dict[str, object]]) -> None:
    fixture_root = repo_root / "compatibility/fixtures/mcp"
    for tool in tools:
        tool_name = str(tool["name"])
        with tempfile.TemporaryDirectory(prefix="hieronymus-mcp-compat-") as directory:
            success_input, success_output = _fixture_case(tool_name, Path(directory))
        tool_root = fixture_root / tool_name
        _write_json(tool_root / "success.input.json", success_input)
        _write_json(tool_root / "success.output.json", success_output)
        _write_json(tool_root / "error.output.json", _DATA_ROOT_ERROR)


def _merge_manifest(repo_root: Path, tools: list[dict[str, object]]) -> None:
    manifest_path = repo_root / "compatibility/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    contracts = manifest["contracts"]
    if not isinstance(contracts, list):
        raise ValueError("manifest contracts must be an array")
    preserved = [
        contract
        for contract in contracts
        if not (isinstance(contract, dict) and str(contract.get("id", "")).startswith("mcp.tool."))
    ]
    mcp_contracts = []
    for tool in tools:
        tool_name = str(tool["name"])
        mcp_contracts.append(
            {
                "id": f"mcp.tool.{tool_name}",
                "surface": "mcp",
                "acceptance_owner": "Pavel Obruchnikov <me@inkyquill.net>",
                "technical_owner": "daemon-mcp-security",
                "python_entry_point": f"hieronymus.mcp_server:{tool_name}",
                "tests": _behavior_tests(tool_name),
                "fixture": (f"compatibility/fixtures/mcp/{tool_name}/success.input.json"),
                "rust_test_target": (f"crates/hiero-mcp/tests/registry_contract.rs::{tool_name}"),
                "disposition": "preserve",
            }
        )
    manifest["contracts"] = [*preserved, *mcp_contracts]
    _write_json(manifest_path, manifest)


def write_snapshot(repo_root: Path) -> dict[str, object]:
    """Write the normalized MCP registry and protocol metadata fixtures."""
    snapshot = snapshot_mcp()
    _write_json(repo_root / "compatibility/snapshots/mcp.json", snapshot)
    _write_json(
        repo_root / "compatibility/fixtures/mcp/protocol.json",
        {
            "protocol_revision": snapshot["protocol_revision"],
            "transports": snapshot["transports"],
            "private_python_bridge": snapshot["private_python_bridge"],
        },
    )
    tools = snapshot["tools"]
    assert isinstance(tools, list)
    _write_tool_fixtures(repo_root, tools)
    _merge_manifest(repo_root, tools)
    return snapshot


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="write compatibility files")
    args = parser.parse_args()
    repo_root = Path(__file__).resolve().parents[2]
    snapshot = write_snapshot(repo_root) if args.write else snapshot_mcp()
    if not args.write:
        print(json.dumps(snapshot, indent=2, sort_keys=True, ensure_ascii=False))


if __name__ == "__main__":
    main()
