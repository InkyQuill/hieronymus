from __future__ import annotations

import json
from dataclasses import replace
from datetime import UTC, datetime, timedelta

from hieronymus.config import HieronymusConfig
from hieronymus.db import connect
from hieronymus.dream_audit import DreamAuditStore
from hieronymus.dream_autostart import DreamAutostart
from hieronymus.dream_config import default_dream_config, load_dream_config, save_dream_config
from hieronymus.dreaming import DreamService
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.workspace import WorkspaceStore


class CapturingDictProvider:
    name = "capturing"

    def __init__(self) -> None:
        self.calls: list[list[int]] = []

    def crystallize(self, context, memories):
        memory_ids = [memory.id for memory in memories]
        self.calls.append(memory_ids)
        first_id = memory_ids[0]
        return {
            "concepts": [
                {
                    "canonical_name": f"Concept {first_id}",
                    "description": "Accepted concept.",
                    "confidence": 0.8,
                }
            ],
            "facets": [
                {
                    "concept_name": f"Concept {first_id}",
                    "value": f"Facet {first_id}",
                    "kind": "alias",
                    "language_tags": "malformed",
                }
            ],
            "crystals": [
                {
                    "body": f"Fallback body for memory {first_id}.",
                    "source_memory_ids": [first_id],
                    "concept_names": [f"Concept {first_id}"],
                }
            ],
        }


def _context(config: HieronymusConfig) -> TranslationContext:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="ru",
    )
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
        task_type="translate",
        volume="1",
        chapter="2",
    )


def _completed_session(
    config: HieronymusConfig,
    context: TranslationContext,
    *,
    memories: int,
) -> list[int]:
    workspace = WorkspaceStore(config)
    session = workspace.start_session(context)
    memory_ids = [
        workspace.add_short_term_memory(
            session.id,
            "user",
            "note",
            f"Bounded audit memory {index}.",
        )
        for index in range(memories)
    ]
    workspace.complete_session(session.id)
    return memory_ids


def _save_dreaming_config(
    config: HieronymusConfig,
    *,
    min_pending: int = 3,
    max_pending: int = 10,
    per_cycle: int = 2,
    max_changed: int = 200,
    max_related_concepts: int = 80,
    max_related_per_concept: int = 20,
    max_total_affected: int = 500,
) -> None:
    save_dream_config(
        config,
        replace(
            default_dream_config(),
            enabled=True,
            min_pending_short_term_memories=min_pending,
            max_pending_short_term_memories=max_pending,
            max_short_term_memories_per_cycle=per_cycle,
            max_changed_crystals_per_cycle=max_changed,
            max_related_concepts_per_cycle=max_related_concepts,
            max_related_crystals_per_concept=max_related_per_concept,
            max_total_affected_crystals=max_total_affected,
        ),
    )


def _phase_input_counts(config: HieronymusConfig) -> list[int]:
    with connect(config.database_path) as conn:
        rows = conn.execute(
            "select input_count from dream_phase_runs order by id",
        ).fetchall()
    return [int(row["input_count"]) for row in rows]


def _pending_memory_count(config: HieronymusConfig) -> int:
    with connect(config.database_path) as conn:
        row = conn.execute(
            "select count(*) from short_term_memories where archived_at is null",
        ).fetchone()
    return int(row[0])


def test_default_affected_set_caps_are_configured(config: HieronymusConfig) -> None:
    dream_config = load_dream_config(config)

    assert dream_config.max_changed_crystals_per_cycle == 200
    assert dream_config.max_related_concepts_per_cycle == 80
    assert dream_config.max_related_crystals_per_concept == 20
    assert dream_config.max_total_affected_crystals == 500


def test_manual_dream_all_uses_capped_prompts_and_processes_final_small_batch(
    config: HieronymusConfig,
) -> None:
    _save_dreaming_config(config, min_pending=4, max_pending=10, per_cycle=2)
    memory_ids = _completed_session(config, _context(config), memories=5)
    provider = CapturingDictProvider()

    run = DreamService(config, provider).run_all(owner="admin")

    assert run.status == "completed"
    assert provider.calls == [memory_ids[:2], memory_ids[2:4], memory_ids[4:]]
    assert _phase_input_counts(config) == [2, 2, 1]
    assert _pending_memory_count(config) == 0


def test_autostart_scheduled_threshold_and_stale_override(
    config: HieronymusConfig,
    monkeypatch,
) -> None:
    _save_dreaming_config(config, min_pending=3, max_pending=10, per_cycle=2)
    memory_ids = _completed_session(config, _context(config), memories=2)
    provider = CapturingDictProvider()
    monkeypatch.setattr("hieronymus.dream_autostart.resolve_provider", lambda _config: provider)
    base = datetime(2026, 6, 10, 12, 0, tzinfo=UTC)

    for offset in range(5):
        assert DreamAutostart(config).run_due(now=base + timedelta(minutes=30 * offset)) == {
            "ran": False,
            "reason": "not_enough_memories",
            "cycles": 0,
        }
        assert provider.calls == []

    result = DreamAutostart(config).run_due(now=base + timedelta(minutes=150))

    assert result == {"ran": True, "reason": "backlog_escape", "cycles": 1}
    assert provider.calls == [memory_ids]
    assert _pending_memory_count(config) == 0


def test_autostart_urgent_drains_all_capped_batches(
    config: HieronymusConfig,
    monkeypatch,
) -> None:
    _save_dreaming_config(config, min_pending=5, max_pending=5, per_cycle=2)
    memory_ids = _completed_session(config, _context(config), memories=5)
    provider = CapturingDictProvider()
    monkeypatch.setattr("hieronymus.dream_autostart.resolve_provider", lambda _config: provider)

    result = DreamAutostart(config).run_due(now=datetime(2026, 6, 10, 12, 0, tzinfo=UTC))

    assert result == {"ran": True, "reason": "urgent", "cycles": 1}
    assert provider.calls == [memory_ids[:2], memory_ids[2:4], memory_ids[4:]]
    assert _pending_memory_count(config) == 0


def test_affected_set_and_phase_audit_payloads_are_bounded_and_complete(
    config: HieronymusConfig,
) -> None:
    _save_dreaming_config(
        config,
        min_pending=1,
        max_pending=10,
        per_cycle=2,
        max_changed=1,
        max_related_concepts=1,
        max_related_per_concept=1,
        max_total_affected=1,
    )
    memory_ids = _completed_session(config, _context(config), memories=2)
    provider = CapturingDictProvider()

    run = DreamService(config, provider).run_all(owner="admin")

    entries = DreamAuditStore(config).list_for_run(run.id)
    event_types = [entry.event_type for entry in entries]
    assert event_types == [
        "provider_request",
        "parse_warnings",
        "provider_response",
        "phase_completed",
    ]
    phase_payload = entries[-1].payload
    expected_keys = {
        "trigger_type",
        "threshold_state",
        "selected_short_term_memory_ids",
        "phase_name",
        "prompt_version",
        "provider_profile",
        "model",
        "request_summary",
        "response_summary",
        "parse_warnings",
        "accepted_entries",
        "rejected_entries",
        "confidence_penalties",
        "created_crystals",
        "created_concepts",
        "created_facets",
        "created_links",
        "superseded_crystals",
        "reinforced_crystals",
        "decayed_crystals",
        "searched_related_candidates",
        "affected_memory_set",
        "skipped_candidates",
    }
    assert expected_keys <= set(phase_payload)
    assert phase_payload["trigger_type"] == "manual"
    assert phase_payload["threshold_state"]["pending_short_term_memories"] == 2
    assert phase_payload["threshold_state"]["minimum_met"] is True
    assert phase_payload["selected_short_term_memory_ids"] == memory_ids
    assert phase_payload["phase_name"] == "crystallization"
    assert phase_payload["prompt_version"] == "crystallization:v1"
    assert phase_payload["provider_profile"] == "capturing"
    assert phase_payload["model"] == "capturing"
    assert phase_payload["request_summary"]["memory_count"] == 2
    assert phase_payload["response_summary"]["parse_warning_count"] >= 1
    assert phase_payload["accepted_entries"]["crystals"] == 1
    assert phase_payload["accepted_entries"]["concepts"] == 1
    assert phase_payload["accepted_entries"]["facets"] == 1
    assert phase_payload["rejected_entries"] == []
    assert phase_payload["confidence_penalties"]
    assert len(phase_payload["affected_memory_set"]["changed_crystal_ids"]) <= 1
    assert len(phase_payload["searched_related_candidates"]["concept_ids"]) <= 1
    assert all(
        len(item["crystal_ids"]) <= 1
        for item in phase_payload["searched_related_candidates"]["crystals_by_concept"]
    )
    assert phase_payload["affected_memory_set"]["total_crystal_count"] <= 1


def test_audit_lookup_returns_phase_entries_and_parse_warning_records(
    config: HieronymusConfig,
) -> None:
    _save_dreaming_config(config, min_pending=1, max_pending=10, per_cycle=1)
    _completed_session(config, _context(config), memories=2)
    provider = CapturingDictProvider()

    run = DreamService(config, provider).run_all(owner="admin")

    entries = DreamAuditStore(config).list_for_run(run.id)
    assert [entry.event_type for entry in entries] == [
        "provider_request",
        "parse_warnings",
        "provider_response",
        "phase_completed",
        "provider_request",
        "parse_warnings",
        "provider_response",
        "phase_completed",
    ]
    assert all(entry.phase_run_id is not None for entry in entries)
    assert all(json.dumps(entry.payload, ensure_ascii=False) for entry in entries)
