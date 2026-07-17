from __future__ import annotations

import pytest

from hieronymus.dream_config import WorkflowProfile, default_dream_config
from hieronymus.dream_workflows import (
    DREAM_PASS_NAMES,
    PHASE_PROMPTS,
    build_phase_prompt,
    resolve_effective_workflow,
    resolve_enabled_workflows,
)
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderCatalogError,
    ProviderDefaults,
    ProviderProfile,
)


def test_default_workflows_have_exactly_seven_separate_pass_prompts() -> None:
    assert tuple(PHASE_PROMPTS) == DREAM_PASS_NAMES
    assert "advisory" in PHASE_PROMPTS["concepts"]
    assert "deterministic translation rules" in PHASE_PROMPTS["rule_crystals"]
    assert "Account for every" in PHASE_PROMPTS["coverage_audit"]
    assert len(set(PHASE_PROMPTS.values())) == 7
    for prompt in PHASE_PROMPTS.values():
        assert "Use English memory prose by default." in prompt
        assert "Long-term crystals must be 1-2 sentences." in prompt
        assert "Short-term memories must be 1-6 sentences." in prompt


def test_default_workflows_require_explicit_provider_configuration() -> None:
    resolved = resolve_enabled_workflows(default_dream_config())

    assert resolved == {}


def test_phase_prompt_includes_general_prompt_format_constraint_and_input() -> None:
    prompt = build_phase_prompt(
        default_dream_config(),
        "knowledge_crystals",
        '{"pending_short_term_memory_ids": [1, 2]}',
    )

    assert "Use English as the primary searchable memory language." in prompt
    assert "Return a single JSON object." in prompt
    assert '{"pending_short_term_memory_ids": [1, 2]}' in prompt


def test_unknown_phase_raises() -> None:
    with pytest.raises(ValueError, match="unknown dream workflow phase: missing"):
        build_phase_prompt(default_dream_config(), "missing", "{}")


def test_effective_workflow_uses_provider_catalog_defaults() -> None:
    dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile(
            provider="",
            model="",
            enabled=True,
        ),
    )
    catalog = ProviderCatalog(
        providers={
            "openai": ProviderProfile(
                name="OpenAI",
                type="openai",
                url="https://api.openai.com/v1",
                key="secret-openai",
            )
        },
        defaults=ProviderDefaults(provider="openai", model="gpt-4.1-mini"),
    )

    resolved = resolve_effective_workflow(dream_config, catalog, "knowledge_crystals")

    assert resolved.provider == "openai"
    assert resolved.model == "gpt-4.1-mini"


def test_effective_workflow_requires_enabled_provider() -> None:
    dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile(
            provider="",
            model="model",
            enabled=True,
        ),
    )

    with pytest.raises(
        ProviderCatalogError,
        match="enabled workflow must have a provider: knowledge_crystals",
    ):
        resolve_effective_workflow(dream_config, ProviderCatalog(), "knowledge_crystals")


def test_effective_workflow_requires_enabled_model() -> None:
    dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile(
            provider="openai",
            model="",
            enabled=True,
        ),
    )
    catalog = ProviderCatalog(
        providers={
            "openai": ProviderProfile(
                name="OpenAI",
                type="openai",
                url="https://api.openai.com/v1",
                key="secret-openai",
            )
        },
    )

    with pytest.raises(
        ProviderCatalogError,
        match="enabled workflow must have a model: knowledge_crystals",
    ):
        resolve_effective_workflow(dream_config, catalog, "knowledge_crystals")


def test_effective_workflow_requires_catalog_profile() -> None:
    configured = default_dream_config().with_workflow(
        "knowledge_crystals", WorkflowProfile(provider="anthropic", model="claude", enabled=True)
    )
    with pytest.raises(ProviderCatalogError, match="provider profile missing: anthropic"):
        resolve_effective_workflow(
            configured,
            ProviderCatalog(),
            "knowledge_crystals",
        )
