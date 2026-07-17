from __future__ import annotations

import json

from hieronymus.dream_config import DreamConfig, WorkflowProfile
from hieronymus.provider_config import ProviderCatalog, ProviderCatalogError

DREAM_PASS_NAMES = (
    "concepts",
    "terminology_candidates",
    "rule_crystals",
    "knowledge_crystals",
    "relations",
    "reinforcement",
    "coverage_audit",
)

_ENGLISH_MEMORY_PROSE = (
    "Use English memory prose by default. Japanese, Russian, or other languages may "
    "appear only as terms, names, renderings, quotes, or metadata. Long-term crystals "
    "must be 1-2 sentences. Short-term memories must be 1-6 sentences."
)


PHASE_PROMPTS = {
    "concepts": (
        f"{_ENGLISH_MEMORY_PROSE} Extract every supported concept and its advisory facets. "
        "Do not create translation rules. Every item must list source_memory_ids. Return JSON."
    ),
    "terminology_candidates": (
        f"{_ENGLISH_MEMORY_PROSE} Extract advisory terminology candidates and source evidence. "
        "They must not impose strict validation. Return JSON."
    ),
    "rule_crystals": (
        f"{_ENGLISH_MEMORY_PROSE} Discover deterministic translation rules only when "
        "explicit user-rule evidence supports them. Every rule must list source_memory_ids. "
        "Return JSON."
    ),
    "knowledge_crystals": (
        f"{_ENGLISH_MEMORY_PROSE} Extract factual, narrative, stylistic, character, world, "
        "and analytical knowledge as concise crystals with source_memory_ids. Return JSON."
    ),
    "relations": (
        f"{_ENGLISH_MEMORY_PROSE} Discover supported relations between concepts and crystals "
        "with source_memory_ids. "
        "Return JSON."
    ),
    "reinforcement": (
        f"{_ENGLISH_MEMORY_PROSE} Identify referenced long-term memory to reinforce, with "
        "source_memory_ids. Return reinforce as objects with crystal_id, strength_delta, and "
        "confidence_delta. Return JSON."
    ),
    "coverage_audit": (
        f"{_ENGLISH_MEMORY_PROSE} Account for every selected short-term memory ID. "
        "Return covered_memory_ids and "
        "source_memory_ids for every audit item. Return JSON."
    ),
}


def resolve_enabled_workflows(dream_config: DreamConfig) -> dict[str, WorkflowProfile]:
    return {
        phase: workflow for phase, workflow in dream_config.workflows.items() if workflow.enabled
    }


def resolve_effective_workflow(
    dream_config: DreamConfig,
    provider_catalog: ProviderCatalog,
    workflow_name: str,
) -> WorkflowProfile:
    workflow = dream_config.workflows.get(workflow_name)
    if workflow is None:
        raise ProviderCatalogError(f"workflow is missing: {workflow_name}")
    if not workflow.enabled:
        return workflow

    provider = workflow.provider.strip() or provider_catalog.defaults.provider.strip()
    model = workflow.model.strip() or provider_catalog.defaults.model.strip()
    if not provider:
        raise ProviderCatalogError(f"enabled workflow must have a provider: {workflow_name}")
    if not model:
        raise ProviderCatalogError(f"enabled workflow must have a model: {workflow_name}")
    if provider != "deterministic" and provider not in provider_catalog.providers:
        raise ProviderCatalogError(f"provider profile missing: {provider}")
    return WorkflowProfile(provider=provider, model=model, enabled=True)


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
