from __future__ import annotations

import pytest

from hieronymus.dream_config import default_dream_config
from hieronymus.dream_workflows import (
    CRYSTALLIZATION_PHASE,
    PHASE_PROMPTS,
    REINFORCEMENT_COMPACTION_PHASE,
    RELATION_DISCOVERY_PHASE,
    build_phase_prompt,
    resolve_enabled_workflows,
)


def test_default_workflows_have_separate_phase_prompts() -> None:
    assert set(PHASE_PROMPTS) == {
        CRYSTALLIZATION_PHASE,
        RELATION_DISCOVERY_PHASE,
        REINFORCEMENT_COMPACTION_PHASE,
    }
    assert "Convert short-term memories" in PHASE_PROMPTS[CRYSTALLIZATION_PHASE]
    assert (
        "reinforce, combine, supersede, or decay" in PHASE_PROMPTS[REINFORCEMENT_COMPACTION_PHASE]
    )
    assert len(set(PHASE_PROMPTS.values())) == 3


def test_default_enabled_workflows_exclude_disabled_relation_discovery() -> None:
    resolved = resolve_enabled_workflows(default_dream_config())

    assert set(resolved) == {
        CRYSTALLIZATION_PHASE,
        REINFORCEMENT_COMPACTION_PHASE,
    }
    assert resolved[CRYSTALLIZATION_PHASE].provider == "anthropic"
    assert resolved[REINFORCEMENT_COMPACTION_PHASE].provider == "ollama"
    assert RELATION_DISCOVERY_PHASE not in resolved


def test_phase_prompt_includes_general_prompt_format_constraint_and_input() -> None:
    prompt = build_phase_prompt(
        default_dream_config(),
        CRYSTALLIZATION_PHASE,
        '{"pending_short_term_memory_ids": [1, 2]}',
    )

    assert "Use English as the primary searchable memory language." in prompt
    assert "Return a single JSON object." in prompt
    assert '{"pending_short_term_memory_ids": [1, 2]}' in prompt


def test_unknown_phase_raises() -> None:
    with pytest.raises(ValueError, match="unknown dream workflow phase: missing"):
        build_phase_prompt(default_dream_config(), "missing", "{}")
