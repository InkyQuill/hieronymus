from __future__ import annotations

import tomllib

from hieronymus.config import load_config
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

    termbase = Termbase(series.database_path)
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
            "source_ref": "chapter-1",
        }
    ]


def test_mcp_script_entrypoint_is_declared():
    with open("pyproject.toml", "rb") as pyproject:
        data = tomllib.load(pyproject)

    assert data["project"]["scripts"]["hieronymus-mcp"] == "hieronymus.mcp_server:main"
