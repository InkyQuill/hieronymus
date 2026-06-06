from __future__ import annotations

import asyncio
import tomllib

import pytest

from hieronymus.config import load_config
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def test_mcp_tools_wrap_core_services(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))
    series = Registry(load_config()).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    from hieronymus import mcp_server

    proposed = mcp_server.hieronymus_termbase_propose(
        series.slug,
        "character",
        "ユン",
        "Yun",
        tags=["name"],
        notes="Main character.",
    )
    assert proposed == {"term_id": 1}

    approved = mcp_server.hieronymus_termbase_approve(series.slug, proposed["term_id"])
    assert approved == {"term_id": 1, "approved": True}

    termbase = Termbase(
        load_config(),
        TranslationContext(
            series_slug=series.slug,
            source_language=series.source_language,
            target_language=series.target_language,
            task_type="translation",
        ),
    )
    termbase.add_alias(
        proposed["term_id"],
        kind="forbidden_variant",
        text="Yuun",
        language="en",
    )

    contract = mcp_server.hieronymus_termbase_contract(series.slug, "ユン walked home.")
    assert contract == [
        {
            "id": 1,
            "category": "character",
            "source_text": "ユン",
            "canonical_translation": "Yun",
            "forbidden_variants": ["Yuun"],
            "tags": ["name"],
            "notes": "Main character.",
        }
    ]

    findings = mcp_server.hieronymus_termbase_validate(
        series.slug,
        "ユン walked home.",
        "Yuun walked home.",
    )
    assert findings == [
        {
            "term_id": 1,
            "kind": "forbidden_variant",
            "severity": "high",
            "expected": "Yun",
            "observed": "Yuun",
            "message": "Use 'Yun' for 'ユン'; 'Yuun' is forbidden.",
        },
        {
            "term_id": 1,
            "kind": "missing_canonical",
            "severity": "medium",
            "expected": "Yun",
            "observed": "",
            "message": (
                "Raw text contains 'ユン', but translation does not contain approved form 'Yun'."
            ),
        },
    ]

    added = mcp_server.hieronymus_memory_add(
        series.slug,
        "translation_rationale",
        "Use Yun for ユン.",
        source_ref="chapter-1",
        importance=4,
    )
    assert added == {"memory_id": 1}

    memories = mcp_server.hieronymus_memory_search(series.slug, "Yun", limit=5)
    assert memories == [
        {
            "id": 1,
            "kind": "translation_rationale",
            "text": "Use Yun for ユン.",
            "importance": 4,
            "source_ref": "",
        }
    ]


def test_mcp_script_entrypoint_is_declared():
    with open("pyproject.toml", "rb") as pyproject:
        data = tomllib.load(pyproject)

    assert data["project"]["scripts"]["hieronymus-mcp"] == "hieronymus.mcp_server:main"


def test_mcp_main_is_callable():
    from hieronymus.mcp_server import main

    assert callable(main)


def test_mcp_tools_raise_clear_error_when_data_root_is_file(monkeypatch, tmp_path):
    data_root = tmp_path / "data-root-file"
    data_root.write_text("not a directory", encoding="utf-8")
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(data_root))

    from hieronymus import mcp_server

    with pytest.raises(ValueError, match=f"data root is not a directory: {data_root}"):
        mcp_server.hieronymus_memory_search("only-sense-online", "Yun")


def test_mcp_server_registers_expected_tool_names():
    from hieronymus.mcp_server import server

    tools = asyncio.run(server.list_tools())

    assert {tool.name for tool in tools} == {
        "hieronymus_termbase_contract",
        "hieronymus_termbase_validate",
        "hieronymus_termbase_propose",
        "hieronymus_termbase_approve",
        "hieronymus_memory_search",
        "hieronymus_memory_add",
    }
