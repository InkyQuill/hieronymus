from __future__ import annotations

import os
import shutil
import sqlite3
import subprocess
import tomllib
from dataclasses import asdict, dataclass
from pathlib import Path

from hieronymus.agent_plugins import available_plugins
from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import (
    DreamConfig,
    DreamConfigError,
    _dream_config_from_payload,
    default_dream_config,
    load_dream_config,
)
from hieronymus.llm_cache import (
    dream_profile_cache_identity,
    load_model_cache,
    model_cache_identity,
)
from hieronymus.memory_migration import MemoryGraphMigrator
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.service_manager import ServiceManager
from hieronymus.settings import SettingsError, load_settings


@dataclass(frozen=True)
class DoctorFinding:
    level: str
    code: str
    message: str
    autofixed: bool = False

    def to_json_dict(self) -> dict[str, object]:
        return asdict(self)


DoctorReport = dict[str, list[DoctorFinding]]


class Doctor:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config

    def run(self, autofix: bool = False) -> DoctorReport:
        report: DoctorReport = {"autofixed": [], "warnings": [], "errors": []}

        self._check_config_root(report, autofix=autofix)
        self._check_database(report)
        self._check_memory_graph_migration(report)
        self._check_daemon(report)
        self._check_ink_runtime(report)
        self._check_settings_and_providers(report)
        self._check_dream_config_readiness(report)
        self._check_llm_model_cache(report)
        self._check_agent_plugins(report)

        return report

    def _check_config_root(self, report: DoctorReport, *, autofix: bool) -> None:
        config_root = self.config.config_root
        if not config_root.exists():
            if autofix:
                config_root.mkdir(parents=True, exist_ok=True)
                report["autofixed"].append(
                    DoctorFinding(
                        level="info",
                        code="config-root-created",
                        message=f"Config root created: {config_root}",
                        autofixed=True,
                    )
                )
                return
            report["warnings"].append(
                DoctorFinding(
                    level="warning",
                    code="config-root-missing",
                    message=f"Config root does not exist: {config_root}",
                )
            )
            return

        if not config_root.is_dir():
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="config-root-not-directory",
                    message=f"Config root is not a directory: {config_root}",
                )
            )

    def _check_database(self, report: DoctorReport) -> None:
        database_path = self.config.database_path
        if not database_path.exists():
            return

        try:
            with sqlite3.connect(database_path) as connection:
                connection.execute("pragma schema_version").fetchone()
        except sqlite3.DatabaseError:
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="database-unreadable",
                    message=f"Database file is unreadable: {database_path}",
                )
            )

    def _check_memory_graph_migration(self, report: DoctorReport) -> None:
        database_path = self.config.database_path
        if not database_path.exists():
            return

        try:
            migration_report = MemoryGraphMigrator.inspect(self.config)
        except sqlite3.DatabaseError:
            return

        pending = {key: count for key, count in migration_report.pending.items() if count > 0}
        if not pending:
            return
        pending_text = ", ".join(f"{key}: {count}" for key, count in sorted(pending.items()))
        report["warnings"].append(
            DoctorFinding(
                level="warning",
                code="memory-graph-migration-pending",
                message=f"Legacy memory graph migration has pending dry-run work: {pending_text}",
            )
        )

    def _check_daemon(self, report: DoctorReport) -> None:
        status = ServiceManager(self.config).status()
        if status.get("running") is True:
            report["autofixed"].append(
                DoctorFinding(
                    level="info",
                    code="daemon-running",
                    message="Hieronymus daemon is reachable.",
                )
            )
            return

        report["warnings"].append(
            DoctorFinding(
                level="warning",
                code="daemon-not-running",
                message="Hieronymus daemon is not running.",
            )
        )

    def _check_ink_runtime(self, report: DoctorReport) -> None:
        node_path = shutil.which("node")
        if not node_path:
            report["warnings"].append(
                DoctorFinding(
                    level="warning",
                    code="node-runtime-missing",
                    message=(
                        "Node.js is not available; install Node.js >=22 to run the Hieronymus terminal user interface (TUI)."
                    ),
                )
            )
        else:
            node_major = _node_major_version(node_path)
            if node_major is not None and node_major >= 22:
                report["autofixed"].append(
                    DoctorFinding(
                        level="info",
                        code="node-runtime-available",
                        message="Node.js runtime is available for the Hieronymus TUI.",
                    )
                )
            elif node_major is None:
                report["warnings"].append(
                    DoctorFinding(
                        level="warning",
                        code="node-runtime-unusable",
                        message=(
                            "Node.js version could not be checked; install Node.js >=22 "
                            "to run the Hieronymus TUI."
                        ),
                    )
                )
            else:
                report["warnings"].append(
                    DoctorFinding(
                        level="warning",
                        code="node-runtime-too-old",
                        message=(
                            "Node.js >=22 is required for the Hieronymus TUI; install or activate "
                            "a supported Node.js runtime."
                        ),
                    )
                )

        if shutil.which("pnpm"):
            report["autofixed"].append(
                DoctorFinding(
                    level="info",
                    code="pnpm-available",
                    message="pnpm is available for frontend development and builds.",
                )
            )
        elif _needs_pnpm_for_frontend_dev():
            report["warnings"].append(
                DoctorFinding(
                    level="warning",
                    code="pnpm-missing",
                    message=(
                        "pnpm is not available; install pnpm to develop or build the Ink frontend."
                    ),
                )
            )
        else:
            report["autofixed"].append(
                DoctorFinding(
                    level="info",
                    code="pnpm-missing",
                    message=(
                        "pnpm is not installed; it is only needed for frontend development/builds."
                    ),
                )
            )

    def _check_agent_plugins(self, report: DoctorReport) -> None:
        for plugin in available_plugins():
            availability = plugin.availability(self.config)
            if not availability.available or availability.installed:
                continue
            report["warnings"].append(
                DoctorFinding(
                    level="warning",
                    code="agent-plugin-available",
                    message=(
                        f"{availability.display_name} is available but Hieronymus is not installed"
                    ),
                )
            )

    def _check_settings_and_providers(self, report: DoctorReport) -> None:
        try:
            settings = load_settings(self.config)
        except SettingsError as error:
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="settings-invalid",
                    message=str(error),
                )
            )
            return

        active_name = settings.dreaming.active_provider
        active = settings.providers[active_name]

        def safe(message: str) -> str:
            return redact_configured_secret_values(message, settings)

        if not active.enabled:
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="active-provider-disabled",
                    message=safe(f"Active dream provider is disabled: {active_name}"),
                )
            )
            return

        if (
            active_name != "deterministic"
            and active.api_key_env
            and not os.environ.get(active.api_key_env)
        ):
            report["errors"].append(
                DoctorFinding(
                    level="error",
                    code="provider-env-missing",
                    message=safe(
                        "Missing environment variable for active dream provider: "
                        f"{active.api_key_env}"
                    ),
                )
            )
            return

        report["autofixed"].append(
            DoctorFinding(
                level="info",
                code="provider-configured",
                message=safe(f"Active dream provider is configured: {active_name}"),
            )
        )

    def _check_dream_config_readiness(self, report: DoctorReport) -> None:
        dream_config, config_finding = _load_dream_config_for_readiness(self.config)
        if config_finding is not None:
            report[f"{config_finding.level}s"].append(config_finding)
        if dream_config is None:
            return

        if not dream_config.enabled:
            report["warnings"].append(
                DoctorFinding(
                    level="warning",
                    code="dreaming_disabled",
                    message="Dreaming is disabled",
                )
            )
            return

        self._check_enabled_dream_workflow_profiles(report, dream_config)
        self._check_dream_model_cache_readiness(report, dream_config)

    def _check_enabled_dream_workflow_profiles(
        self,
        report: DoctorReport,
        dream_config: DreamConfig,
    ) -> None:
        missing_profile_reported = False
        missing_model_reported = False
        providers_with_missing_key_reported: set[str] = set()
        for workflow in dream_config.workflows.values():
            if not workflow.enabled:
                continue
            provider = dream_config.providers.get(workflow.provider)
            if provider is None:
                if not missing_profile_reported:
                    report["errors"].append(
                        DoctorFinding(
                            level="error",
                            code="dream_provider_profile_missing",
                            message="Referenced provider profile is missing",
                        )
                    )
                    missing_profile_reported = True
                continue
            if not workflow.model.strip() and not missing_model_reported:
                report["errors"].append(
                    DoctorFinding(
                        level="error",
                        code="dream_model_not_set",
                        message="Model not set for workflow",
                    )
                )
                missing_model_reported = True
            if (
                provider.type != "ollama"
                and not provider.api_key.strip()
                and workflow.provider not in providers_with_missing_key_reported
            ):
                report["errors"].append(
                    DoctorFinding(
                        level="error",
                        code="dream_api_key_missing",
                        message="API key missing for provider profile",
                    )
                )
                providers_with_missing_key_reported.add(workflow.provider)

    def _check_dream_model_cache_readiness(
        self,
        report: DoctorReport,
        dream_config: DreamConfig,
    ) -> None:
        cache = load_model_cache(self.config)
        provider_identities = {
            name: dream_profile_cache_identity(name, provider)
            for name, provider in dream_config.providers.items()
        }
        reported_cache_errors: set[tuple[str, str]] = set()
        reported_missing_models: set[tuple[str, str]] = set()

        for workflow in dream_config.workflows.values():
            if not workflow.enabled:
                continue
            entry = cache.providers.get(workflow.provider)
            if entry is None or entry.is_stale():
                continue
            if entry.identity != provider_identities.get(workflow.provider):
                continue
            if entry.error:
                if _is_403_error(entry.error):
                    key = (workflow.provider, "dream_api_key_rejected")
                    if key not in reported_cache_errors:
                        report["errors"].append(
                            DoctorFinding(
                                level="error",
                                code="dream_api_key_rejected",
                                message="API key rejected with 403",
                            )
                        )
                        reported_cache_errors.add(key)
                    continue

                key = (workflow.provider, "dream_provider_unreachable")
                if key not in reported_cache_errors:
                    report["warnings"].append(
                        DoctorFinding(
                            level="warning",
                            code="dream_provider_unreachable",
                            message="Provider in use cannot be reached",
                        )
                    )
                    reported_cache_errors.add(key)
                continue

            model = workflow.model.strip()
            key = (workflow.provider, model)
            if (
                model
                and entry.models
                and model not in entry.models
                and key not in reported_missing_models
            ):
                report["warnings"].append(
                    DoctorFinding(
                        level="warning",
                        code="dream_model_missing",
                        message="Configured model was not found in provider cache",
                    )
                )
                reported_missing_models.add(key)

    def _check_llm_model_cache(self, report: DoctorReport) -> None:
        identities = self._current_llm_model_cache_identities()
        for entry in load_model_cache(self.config).providers.values():
            if not entry.error or entry.is_stale():
                continue
            valid_identities = identities.get(entry.provider)
            if valid_identities is None or entry.identity not in valid_identities:
                continue
            report["warnings"].append(
                DoctorFinding(
                    level="warning",
                    code="llm-model-cache-refresh-failed",
                    message=(f"Model cache refresh failed for provider profile: {entry.provider}"),
                )
            )

    def _current_llm_model_cache_identities(self) -> dict[str, tuple[str, ...]]:
        identities = {"anthropic": ("", model_cache_identity("anthropic"))}
        try:
            settings = load_settings(self.config)
        except SettingsError:
            return identities

        for provider_name in ("openai", "gemini"):
            provider = settings.providers.get(provider_name)
            if provider is None:
                continue
            identities[provider_name] = (model_cache_identity(provider_name, provider),)
        return identities


def _node_major_version(node_path: str) -> int | None:
    try:
        result = subprocess.run(
            [node_path, "--version"],
            capture_output=True,
            text=True,
            check=False,
            timeout=2,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None

    if result.returncode != 0:
        return None

    version = result.stdout.strip()
    if version.startswith("v"):
        version = version[1:]
    major, _, _ = version.partition(".")
    try:
        return int(major)
    except ValueError:
        return None


def _needs_pnpm_for_frontend_dev() -> bool:
    package_root = Path(__file__).resolve().parent
    bundled_entrypoint = package_root / "frontend" / "dist" / "main.js"
    source_frontend_manifest = package_root.parents[1] / "frontend" / "package.json"
    return source_frontend_manifest.exists() and not bundled_entrypoint.exists()


def _load_dream_config_for_readiness(
    config: HieronymusConfig,
) -> tuple[DreamConfig | None, DoctorFinding | None]:
    try:
        return load_dream_config(config), None
    except DreamConfigError as error:
        try:
            dream_config = _load_dream_config_without_final_validation(config)
        except DreamConfigError:
            return None, _dream_conf_invalid_finding()
        if _is_dream_readiness_validation_error(error):
            return dream_config, None
        return dream_config, _dream_conf_invalid_finding()


def _load_dream_config_without_final_validation(config: HieronymusConfig) -> DreamConfig:
    if not config.dream_config_path.exists():
        return default_dream_config()
    try:
        payload = tomllib.loads(config.dream_config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise DreamConfigError(f"dream.conf is not valid TOML: {error}") from error
    return _dream_config_from_payload(payload)


def _dream_conf_invalid_finding() -> DoctorFinding:
    return DoctorFinding(
        level="error",
        code="dream_conf_invalid",
        message="dream.conf invalid",
    )


def _is_dream_readiness_validation_error(error: DreamConfigError) -> bool:
    message = str(error)
    return (
        "referenced provider profile is missing" in message
        or "enabled workflow must have a model" in message
    )


def _is_403_error(error: str) -> bool:
    normalized = error.lower()
    return "403" in normalized or "forbidden" in normalized


def report_to_json(report: DoctorReport) -> dict[str, list[dict[str, object]]]:
    return {
        section: [finding.to_json_dict() for finding in findings]
        for section, findings in report.items()
    }
