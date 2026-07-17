import tomllib
from pathlib import Path

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import (
    DreamConfigError,
    WorkflowProfile,
    default_dream_config,
    load_dream_config,
    redacted_dream_config_payload,
    save_dream_config,
)


def write_dream_config(config: HieronymusConfig, raw_config: str) -> None:
    config.config_root.mkdir(parents=True)
    config.dream_config_path.write_text(raw_config, encoding="utf-8")


def test_dream_config_paths_live_under_config_root(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert config.dream_config_path == config.config_root / "dream.conf"
    assert config.llm_cache_path == config.config_root / "llmcache.tmp"


def test_default_dream_config_matches_memory_spec(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    dream_config = load_dream_config(config)

    assert dream_config.enabled is False
    assert dream_config.schedule_interval_minutes == 30
    assert dream_config.min_pending_short_term_memories == 20
    assert dream_config.max_pending_short_term_memories == 200
    assert dream_config.max_short_term_memories_per_cycle == 50
    assert dream_config.not_enough_memories_cycle_threshold == 5
    assert tuple(dream_config.workflows) == (
        "concepts",
        "terminology_candidates",
        "rule_crystals",
        "knowledge_crystals",
        "relations",
        "reinforcement",
        "coverage_audit",
    )
    assert dream_config.workflows["knowledge_crystals"].provider == ""
    assert dream_config.max_short_term_memories_per_run == 500


def test_save_dream_config_does_not_write_provider_profiles_or_secrets(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config()

    save_dream_config(config, dream_config)

    raw = config.dream_config_path.read_text(encoding="utf-8")
    assert "api_key" not in raw
    assert "providers" not in raw
    payload = tomllib.loads(raw)
    assert "providers" not in payload
    redacted = redacted_dream_config_payload(dream_config)
    assert "providers" not in redacted


def test_load_save_dream_config_round_trips_workflows_without_providers(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile(provider="local_llm", model="local-model", enabled=True),
    )

    save_dream_config(config, dream_config)
    loaded = load_dream_config(config)

    assert loaded.workflows["knowledge_crystals"] == WorkflowProfile(
        provider="local_llm",
        model="local-model",
        enabled=True,
    )
    assert "providers" not in loaded.to_payload()


def test_load_dream_config_migrates_legacy_workflows_to_disk(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(
        config,
        """
[dreaming]
enabled = false

[workflows.crystallization]
provider = "legacy_provider"
model = "legacy-model"
enabled = true

[workflows.relation_discovery]
provider = "relations_provider"
model = "relations-model"
enabled = true

[workflows.reinforcement_compaction]
provider = "maintenance_provider"
model = "maintenance-model"
enabled = true
""",
    )

    loaded = load_dream_config(config)

    assert set(loaded.workflows) == set(default_dream_config().workflows)
    assert loaded.workflows["knowledge_crystals"].provider == "legacy_provider"
    assert loaded.workflows["relations"].model == "relations-model"
    assert loaded.workflows["coverage_audit"].provider == "maintenance_provider"
    saved = config.dream_config_path.read_text(encoding="utf-8")
    assert "[workflows.coverage_audit]" in saved
    assert "[workflows.crystallization]" not in saved


def test_load_dream_config_rejects_unknown_workflow_without_migrating(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.dream_config_path.write_text(
        "[workflows.unknown]\nprovider = 'openai'\nmodel = 'gpt-4.1-mini'\nenabled = true\n",
        encoding="utf-8",
    )

    with pytest.raises(DreamConfigError, match="workflows must contain exactly"):
        load_dream_config(config)


def test_load_dream_config_rejects_invalid_threshold_order(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(
        config,
        "[dreaming]\n"
        "enabled = true\n"
        "schedule_interval_minutes = 30\n"
        "min_pending_short_term_memories = 20\n"
        "max_pending_short_term_memories = 10\n"
        "max_short_term_memories_per_cycle = 50\n"
        "not_enough_memories_cycle_threshold = 5\n",
    )

    with pytest.raises(DreamConfigError, match="max_pending_short_term_memories"):
        load_dream_config(config)


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[dreaming]\nschedule_interval_minutes = 0\n",
            "schedule_interval_minutes must be at least 1",
        ),
        (
            "[dreaming]\nmin_pending_short_term_memories = -1\n",
            "min_pending_short_term_memories must be at least 1",
        ),
        (
            "[dreaming]\nmax_pending_short_term_memories = 0\n",
            "max_pending_short_term_memories must be at least 1",
        ),
        (
            "[dreaming]\nmax_short_term_memories_per_cycle = 0\n",
            "max_short_term_memories_per_cycle must be at least 1",
        ),
        (
            "[dreaming]\nnot_enough_memories_cycle_threshold = 0\n",
            "not_enough_memories_cycle_threshold must be at least 1",
        ),
    ],
)
def test_load_dream_config_rejects_non_positive_thresholds(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(config, raw_config)

    with pytest.raises(DreamConfigError, match=error):
        load_dream_config(config)


def test_deprecated_user_defined_provider_payload_is_ignored(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(
        config,
        '[providers.local_llm]\nendpoint = "http://localhost:11434"\n',
    )

    dream_config = load_dream_config(config)

    assert "providers" not in dream_config.to_payload()


def test_deprecated_builtin_provider_payload_is_ignored(tmp_path: Path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(
        config,
        '[providers.ollama]\nendpoint = "http://localhost:11435"\n',
    )

    dream_config = load_dream_config(config)

    assert "providers" not in dream_config.to_payload()


def test_deprecated_provider_type_is_not_validated_by_dream_config(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(
        config,
        '[providers.local_llm]\ntype = "local"\n',
    )

    dream_config = load_dream_config(config)

    assert "providers" not in dream_config.to_payload()


def test_deprecated_provider_timeout_is_not_validated_by_dream_config(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(
        config,
        "[providers.ollama]\ntimeout_seconds = 0\n",
    )

    dream_config = load_dream_config(config)

    assert "providers" not in dream_config.to_payload()


def test_workflow_provider_existence_is_validated_outside_dream_config(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile(provider="missing", model="model", enabled=True),
    )

    save_dream_config(config, dream_config)

    assert load_dream_config(config).workflows["knowledge_crystals"].provider == "missing"


def test_enabled_workflow_rejects_empty_model(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    dream_config = default_dream_config().with_workflow(
        "knowledge_crystals",
        WorkflowProfile(provider="anthropic", model="", enabled=True),
    )

    with pytest.raises(DreamConfigError, match="enabled workflow must have a model"):
        save_dream_config(config, dream_config)


@pytest.mark.parametrize(
    ("raw_config", "error"),
    [
        (
            "[dreaming]\nenabled = 'yes'\n",
            "enabled must be a boolean",
        ),
        (
            "[dreaming]\nschedule_interval_minutes = true\n",
            "schedule_interval_minutes must be an integer",
        ),
        (
            "[workflows.knowledge_crystals]\nprovider = 123\n",
            r"workflows\.knowledge_crystals\.provider must be a string",
        ),
        (
            "[workflows.knowledge_crystals]\nenabled = 'true'\n",
            r"workflows\.knowledge_crystals\.enabled must be a boolean",
        ),
    ],
)
def test_load_dream_config_rejects_toml_type_mismatches(
    tmp_path: Path,
    raw_config: str,
    error: str,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    write_dream_config(config, raw_config)

    with pytest.raises(DreamConfigError, match=error):
        load_dream_config(config)


def test_load_dream_config_migrates_removed_workflow_names(
    tmp_path: Path,
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.config_root.mkdir(parents=True)
    config.dream_config_path.write_text(
        """
[providers.openai]
type = "openai"
endpoint = "https://api.deepseek.com"
api_key = "secret"

[workflows.crystallization]
provider = "openai"
model = "deepseek-v4-flash"
enabled = true
""",
        encoding="utf-8",
    )

    loaded = load_dream_config(config)

    assert loaded.workflows["knowledge_crystals"].provider == "openai"
