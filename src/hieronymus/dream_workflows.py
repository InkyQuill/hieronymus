from __future__ import annotations

import json

from hieronymus.dream_config import DreamConfig, WorkflowProfile

CRYSTALLIZATION_PHASE = "crystallization"
RELATION_DISCOVERY_PHASE = "relation_discovery"
REINFORCEMENT_COMPACTION_PHASE = "reinforcement_compaction"


PHASE_PROMPTS = {
    CRYSTALLIZATION_PHASE: (
        "Convert short-term memories to concise long-term memory candidates. "
        "Crystals are 1-2 English sentences and must preserve Japanese or Russian "
        "only as names, translations, quoted evidence, or metadata. Correction memories "
        'saying "User told me" should become rule crystals when they express translation '
        "rules. Create concepts, facets, semantic tags, story scopes, and links when the "
        "evidence supports them. Return JSON."
    ),
    RELATION_DISCOVERY_PHASE: (
        "Inspect the affected memory set and propose additional concept links, semantic "
        "tags, story scopes, and rename candidates. Every proposal must be supported by "
        "snapshot evidence from the supplied snapshot. Return JSON."
    ),
    REINFORCEMENT_COMPACTION_PHASE: (
        "Decide what to reinforce, combine, supersede, or decay. Active rule crystals do "
        "not decay, but they may be superseded or combined when newer evidence supports "
        "a clearer rule. Return JSON maintenance actions."
    ),
}


def resolve_enabled_workflows(dream_config: DreamConfig) -> dict[str, WorkflowProfile]:
    return {
        phase: workflow for phase, workflow in dream_config.workflows.items() if workflow.enabled
    }


def build_phase_prompt(dream_config: DreamConfig, phase: str, phase_input: object) -> str:
    phase_prompt = PHASE_PROMPTS.get(phase)
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
