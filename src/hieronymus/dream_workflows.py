from __future__ import annotations

import json

from hieronymus.dream_config import DreamConfig, WorkflowProfile

CRYSTALLIZATION_PHASE = "crystallization"
CONCEPT_DISCOVERY_PHASE = "concept_discovery"
RULE_DISCOVERY_PHASE = "rule_discovery"
CONSOLIDATION_COMPACTION_PHASE = "consolidation_compaction"
DECAY_REINFORCEMENT_REVIEW_PHASE = "decay_reinforcement_review"
RELATION_DISCOVERY_PHASE = CONCEPT_DISCOVERY_PHASE
REINFORCEMENT_COMPACTION_PHASE = DECAY_REINFORCEMENT_REVIEW_PHASE

_ENGLISH_MEMORY_PROSE = (
    "Use English memory prose by default. Japanese, Russian, or other languages may "
    "appear only as terms, names, renderings, quotes, or metadata. Long-term crystals "
    "must be 1-2 sentences. Short-term memories must be 1-6 sentences."
)


PHASE_PROMPTS = {
    CRYSTALLIZATION_PHASE: (
        f"{_ENGLISH_MEMORY_PROSE} Convert short-term memories to concise long-term "
        'memory candidates. Correction memories saying "User told me" should become '
        "rule crystals when they express translation rules. Create concepts, facets, "
        "semantic tags, story scopes, and links when the evidence supports them. Return JSON."
    ),
    CONCEPT_DISCOVERY_PHASE: (
        f"{_ENGLISH_MEMORY_PROSE} Inspect the affected memory set and propose concepts, "
        "facets, concept links, semantic tags, story scopes, and rename candidates. Every "
        "proposal must be supported by snapshot evidence from the supplied snapshot. Return JSON."
    ),
    RULE_DISCOVERY_PHASE: (
        f"{_ENGLISH_MEMORY_PROSE} Discover deterministic translation rules only when "
        "the supplied evidence supports them. Approved termbase entries and user-rule "
        "memories outrank fuzzy memories and speculative thoughts. Return JSON."
    ),
    CONSOLIDATION_COMPACTION_PHASE: (
        f"{_ENGLISH_MEMORY_PROSE} Decide what to combine or supersede. Keep active rule "
        "crystals deterministic, and compact only when the result is clearer than the "
        "inputs. Return JSON maintenance actions."
    ),
    DECAY_REINFORCEMENT_REVIEW_PHASE: (
        f"{_ENGLISH_MEMORY_PROSE} Decide what to reinforce or decay. Active rule crystals "
        "do not decay, but they may be superseded when newer approved evidence supports a "
        "clearer rule. Return JSON maintenance actions."
    ),
}

_PHASE_ALIASES = {
    "relation_discovery": CONCEPT_DISCOVERY_PHASE,
    "reinforcement_compaction": DECAY_REINFORCEMENT_REVIEW_PHASE,
}


def resolve_enabled_workflows(dream_config: DreamConfig) -> dict[str, WorkflowProfile]:
    return {
        phase: workflow for phase, workflow in dream_config.workflows.items() if workflow.enabled
    }


def build_phase_prompt(dream_config: DreamConfig, phase: str, phase_input: object) -> str:
    canonical_phase = _PHASE_ALIASES.get(phase, phase)
    phase_prompt = PHASE_PROMPTS.get(canonical_phase)
    if phase_prompt is None:
        raise ValueError(f"unknown dream workflow phase: {phase}")

    if isinstance(phase_input, str):
        rendered_input = phase_input
    else:
        rendered_input = json.dumps(phase_input, ensure_ascii=False)

    return "\n\n".join(
        [
            dream_config.general_prompt.strip(),
            phase_prompt,
            "Return a single JSON object.",
            rendered_input,
        ]
    )
