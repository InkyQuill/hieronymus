from __future__ import annotations

from dataclasses import replace

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import (
    DreamConfig,
    DreamConfigError,
    ProviderProfile,
    WorkflowProfile,
    default_dream_config,
    load_dream_config,
    redacted_dream_config_payload,
    save_dream_config,
    validate_dream_config,
)
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.ingest_config import (
    IngestConfig,
    IngestConfigError,
    default_ingest_config,
    load_ingest_config,
    save_ingest_config,
    validate_ingest_config,
)
from hieronymus.llm_cache import load_model_cache
from hieronymus.release_config import (
    ReleaseConfig,
    ReleaseConfigError,
    default_release_config,
    load_release_config,
    save_release_config,
    validate_release_config,
)
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.tui_bridge.config_state import (
    field_value,
    parse_bool,
    parse_float,
    parse_positive_int,
)

REMOTE_PROVIDERS = ("openai", "gemini", "anthropic")


class ConfigBridge:
    def __init__(
        self,
        config: HieronymusConfig,
        *,
        registry: ProviderRegistry | None = None,
    ) -> None:
        self.config = config
        self.registry = registry or ProviderRegistry()
        self._pending_api_keys: dict[str, str] = {}

    def bootstrap(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config)
        return self._payload(
            dream_config,
            ingest_config,
            selected,
            release_config,
            validation_errors=_load_errors(dream_error, ingest_error, release_error),
            detail=dream_error or ingest_error or release_error,
        )

    def reload(self, params: dict[str, object]) -> dict[str, object]:
        return self.bootstrap(params)

    def select_provider(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._require_remote_provider(params.get("provider"))
        return self._payload(
            self._select_provider(dream_config, selected),
            ingest_config,
            selected,
            release_config,
            validation_errors=_load_errors(dream_error, ingest_error, release_error),
            detail=dream_error or ingest_error or release_error,
        )

    def update_draft(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config)
        dream_config = self._select_provider(dream_config, selected)
        if errors := _canonical_draft_errors(params):
            return self._payload(
                dream_config,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        try:
            dream_config = self._apply_provider_form(
                dream_config,
                selected,
                self._provider_form(params.get("provider"), dream_config, selected),
            )
            dream_config = self._apply_dreaming_form(
                dream_config,
                self._dreaming_form(params.get("dreaming"), dream_config),
            )
            ingest_config = self._apply_ingest_form(
                ingest_config,
                self._ingest_form(params.get("ingest"), ingest_config),
            )
            release_config = self._release_form(params.get("release"), release_config)
            dream_config = self._select_provider(dream_config, selected)
            validate_dream_config(dream_config)
            validate_ingest_config(ingest_config)
        except (DreamConfigError, IngestConfigError, ValueError, ReleaseConfigError) as error:
            return self._payload(
                dream_config,
                ingest_config,
                selected,
                release_config,
                validation_errors=[str(error)],
            )
        return self._payload(
            dream_config,
            ingest_config,
            selected,
            release_config,
            validation_errors=_load_errors(dream_error, ingest_error, release_error),
            detail=dream_error or ingest_error or release_error,
        )

    def save(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config)
        if "selected_provider" in params or "provider" in params:
            dream_config = self._select_provider(dream_config, selected)
        canonical_errors = _canonical_draft_errors(params)
        validation_errors = (
            []
            if canonical_errors
            else self._validation_errors(params, dream_config, ingest_config, release_config)
        )
        load_errors = _load_errors(dream_error, ingest_error, release_error)
        errors = [*canonical_errors, *validation_errors]
        if canonical_errors or validation_errors or not _has_complete_draft(params):
            errors = [*errors, *load_errors]
        if errors:
            return self._payload(
                dream_config,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        save_dream_config(self.config, dream_config)
        save_ingest_config(self.config, ingest_config)
        save_release_config(self.config, release_config)
        return self._payload(dream_config, ingest_config, selected, release_config)

    def check_provider(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config)
        dream_config = self._select_provider(dream_config, selected)
        errors = _canonical_draft_errors(params)
        if not errors:
            errors = self._validation_errors(params, dream_config, ingest_config, release_config)
        errors = [*errors, *_load_errors(dream_error, ingest_error, release_error)]
        if errors:
            return self._payload(
                dream_config,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        profile_context = self._dream_profile_context(dream_config, selected)
        if profile_context is not None:
            profile, model = profile_context
            result = self.registry.check_profile(self.config, selected, profile, model=model)
            check_result = _result_to_json_dict(result)
            _redact_error(check_result, dream_config)
            suggestions = None
            if check_result.get("ok") is True:
                suggestion_result = self.registry.list_profile_model_suggestions(
                    self.config,
                    selected,
                    profile,
                )
                suggestions = _result_to_json_dict(suggestion_result)
                _redact_error(suggestions, dream_config)
            return self._payload(
                dream_config,
                ingest_config,
                selected,
                release_config,
                check_result=check_result,
                suggestions=suggestions,
                validation_errors=[],
            )
        result = self.registry.check(self.config, selected)
        check_result = _result_to_json_dict(result)
        _redact_error(check_result, dream_config)
        suggestions = None
        if check_result.get("ok") is True and hasattr(self.registry, "list_model_suggestions"):
            suggestion_result = self.registry.list_model_suggestions(self.config, selected)
            suggestions = _result_to_json_dict(suggestion_result)
            _redact_error(suggestions, dream_config)
        return self._payload(
            dream_config,
            ingest_config,
            selected,
            release_config,
            check_result=check_result,
            suggestions=suggestions,
            validation_errors=[],
        )

    def model_suggestions(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config)
        dream_config = self._select_provider(dream_config, selected)
        errors = _canonical_draft_errors(params)
        if not errors:
            errors = self._validation_errors(params, dream_config, ingest_config, release_config)
        errors = [*errors, *_load_errors(dream_error, ingest_error, release_error)]
        if errors:
            return self._payload(
                dream_config,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        profile_context = self._dream_profile_context(dream_config, selected)
        if profile_context is not None and hasattr(
            self.registry,
            "list_profile_model_suggestions",
        ):
            profile, _ = profile_context
            result = self.registry.list_profile_model_suggestions(self.config, selected, profile)
            suggestions = _result_to_json_dict(result)
            _redact_error(suggestions, dream_config)
            return self._payload(
                dream_config,
                ingest_config,
                selected,
                release_config,
                suggestions=suggestions,
                validation_errors=[],
            )
        result = self.registry.list_model_suggestions(self.config, selected)
        suggestions = _result_to_json_dict(result)
        _redact_error(suggestions, dream_config)
        return self._payload(
            dream_config,
            ingest_config,
            selected,
            release_config,
            suggestions=suggestions,
            validation_errors=[],
        )

    def _payload(
        self,
        dream_config: DreamConfig,
        ingest_config: IngestConfig,
        selected: str,
        release_config: ReleaseConfig,
        *,
        validation_errors: list[str] | None = None,
        check_result: dict[str, object] | None = None,
        suggestions: dict[str, object] | None = None,
        detail: str = "",
    ) -> dict[str, object]:
        errors = (
            self._validation_errors({}, dream_config, ingest_config, release_config)
            if validation_errors is None
            else validation_errors
        )
        dream_payload = redacted_dream_config_payload(dream_config)
        ingest_payload = ingest_config.to_payload()
        return {
            "config_paths": {
                "data_root": str(self.config.data_root),
                "config_root": str(self.config.config_root),
                "dream_config_path": str(self.config.dream_config_path),
                "ingest_config_path": str(self.config.ingest_config_path),
                "release_config_path": str(self.config.release_config_path),
            },
            "dreaming": dream_payload["dreaming"],
            "providers": dream_payload["providers"],
            "workflows": dream_payload["workflows"],
            "ingest": ingest_payload,
            "release": _release_payload(release_config),
            "model_cache": load_model_cache(self.config).to_payload(),
            "provider_choices": self._provider_choices(),
            "selected_provider": selected,
            "draft": self._draft_payload(dream_config, ingest_config, release_config, selected),
            "form_values": self._form_values(
                dream_config,
                ingest_config,
                selected,
                release_config,
            ),
            "validation": {"ok": not errors, "errors": errors},
            "check_result": check_result or {},
            "suggestions": suggestions or {},
            "detail": detail,
        }

    def _provider_choices(self) -> list[dict[str, object]]:
        registry = self.registry if hasattr(self.registry, "list") else ProviderRegistry()
        return [
            {
                "name": provider.name,
                "display_name": provider.display_name,
                "requires_api_key": provider.requires_api_key,
                "supports_api_path": provider.supports_base_url,
            }
            for provider in registry.list()
            if provider.name in REMOTE_PROVIDERS
        ]

    def _draft_payload(
        self,
        dream_config: DreamConfig,
        ingest_config: IngestConfig,
        release_config: ReleaseConfig,
        selected: str,
    ) -> dict[str, object]:
        dream_payload = redacted_dream_config_payload(dream_config)
        provider_form = self._form_values(
            dream_config,
            ingest_config,
            selected,
            release_config,
        )["provider"]
        return {
            "dream": dream_payload,
            "ingest": ingest_config.to_payload(),
            "release": _release_draft(release_config),
            "provider": provider_form,
            "dreaming": _compat_dreaming_draft(dream_config, selected),
            "providers": _compat_provider_draft(dream_config, selected),
            "workflows": dream_payload["workflows"],
        }

    def _dream_from_params(self, params: dict[str, object]) -> tuple[DreamConfig, str]:
        try:
            dream_config = load_dream_config(self.config)
        except DreamConfigError as error:
            dream_config = default_dream_config()
            load_error = str(error)
        else:
            load_error = ""

        draft = params.get("draft")
        if draft is None and any(
            key in params for key in ("dream", "dreaming", "providers", "provider")
        ):
            draft = params
        if type(draft) is not dict or not draft:
            return dream_config, load_error

        raw_dream = draft.get("dream")
        if type(raw_dream) is dict:
            self._clear_pending_api_keys_from_draft(dream_config, raw_dream)
            dream_config = _dream_config_from_draft(dream_config, raw_dream)
            dream_config = self._apply_pending_api_keys(dream_config)

        raw_provider = params.get("provider")
        selected = self._selected_provider(params, dream_config)
        if type(raw_provider) is dict:
            try:
                dream_config = self._apply_provider_form(
                    dream_config,
                    selected,
                    self._provider_form(raw_provider, dream_config, selected),
                )
            except (DreamConfigError, ValueError) as error:
                return dream_config, str(error)

        raw_dreaming = params.get("dreaming")
        if type(raw_dreaming) is dict:
            try:
                dream_config = self._apply_dreaming_form(
                    dream_config,
                    self._dreaming_form(raw_dreaming, dream_config),
                )
            except (DreamConfigError, ValueError) as error:
                return dream_config, str(error)

        return dream_config, load_error

    def _ingest_from_params(self, params: dict[str, object]) -> tuple[IngestConfig, str]:
        try:
            ingest_config = load_ingest_config(self.config)
        except IngestConfigError as error:
            ingest_config = default_ingest_config()
            load_error = str(error)
        else:
            load_error = ""

        draft = params.get("draft")
        if draft is None and "ingest" in params:
            draft = params
        if type(draft) is not dict:
            return ingest_config, load_error

        raw_ingest = draft.get("ingest")
        if type(raw_ingest) is dict:
            ingest_config = _ingest_config_from_draft(ingest_config, raw_ingest)
        return ingest_config, load_error

    def _release_from_params(
        self,
        params: dict[str, object],
    ) -> tuple[ReleaseConfig, str]:
        try:
            release_config = load_release_config(self.config)
        except ReleaseConfigError as error:
            release_config = default_release_config()
            load_error = str(error)
        else:
            load_error = ""

        draft = params.get("draft")
        if draft is None and "release" in params:
            draft = params
        if type(draft) is not dict:
            return release_config, load_error

        raw_release = draft.get("release")
        if type(raw_release) is not dict:
            return release_config, load_error
        if "update_channel" in raw_release:
            release_config = release_config.with_update_channel(raw_release["update_channel"])
        return release_config, load_error

    def _validation_errors(
        self,
        params: dict[str, object],
        dream_config: DreamConfig,
        ingest_config: IngestConfig,
        release_config: ReleaseConfig,
    ) -> list[str]:
        errors = _draft_container_errors(params)
        if errors:
            return errors
        try:
            validate_dream_config(dream_config)
        except DreamConfigError as error:
            return [str(error)]
        try:
            validate_ingest_config(ingest_config)
        except IngestConfigError as error:
            return [str(error)]
        try:
            validate_release_config(release_config)
        except ReleaseConfigError as error:
            return [str(error)]
        return []

    def _selected_provider(
        self,
        params: dict[str, object],
        dream_config: DreamConfig,
    ) -> str:
        explicit = params.get("selected_provider", params.get("provider"))
        if explicit is not None:
            return self._require_remote_provider(explicit)
        crystallization = dream_config.workflows.get("crystallization")
        if crystallization is not None and crystallization.provider in REMOTE_PROVIDERS:
            return crystallization.provider
        draft = params.get("draft")
        if type(draft) is dict:
            raw_dreaming = draft.get("dreaming")
            if type(raw_dreaming) is dict:
                provider = raw_dreaming.get("active_provider")
                if provider in REMOTE_PROVIDERS:
                    return provider
            raw_workflows = draft.get("workflows")
            if type(raw_workflows) is dict:
                raw_crystallization = raw_workflows.get("crystallization")
                if type(raw_crystallization) is dict:
                    provider = raw_crystallization.get("provider")
                    if provider in REMOTE_PROVIDERS:
                        return provider
        return "openai"

    def _require_remote_provider(self, value: object) -> str:
        if type(value) is not str or value not in REMOTE_PROVIDERS:
            raise ValueError(f"unsupported remote provider: {value}")
        return value

    def _dream_profile_context(
        self,
        dream_config: DreamConfig,
        selected: str,
    ) -> tuple[ProviderProfile, str] | None:
        profile = dream_config.providers.get(selected)
        if profile is None:
            return None
        return profile, _model_for_profile(dream_config, selected)

    def _select_provider(
        self,
        dream_config: DreamConfig,
        selected: str,
    ) -> DreamConfig:
        workflow = dream_config.workflows.get(
            "crystallization",
            WorkflowProfile(provider=selected, model="", enabled=True),
        )
        return dream_config.with_workflow(
            "crystallization",
            replace(workflow, provider=selected, enabled=True),
        )

    def _apply_provider_form(
        self,
        dream_config: DreamConfig,
        selected: str,
        values: dict[str, str],
    ) -> DreamConfig:
        provider = dream_config.providers.get(selected, ProviderProfile(type=selected))
        endpoint = values["endpoint"].strip()
        api_key = values["api_key"].strip()
        if api_key == "***":
            api_key = provider.api_key
        elif api_key:
            self._pending_api_keys[selected] = api_key
        else:
            self._pending_api_keys.pop(selected, None)
        updated_provider = replace(
            provider,
            type=selected,
            endpoint=endpoint,
            api_key=api_key,
            timeout_seconds=parse_float(
                f"providers.{selected}.timeout_seconds",
                values["timeout_seconds"],
            ),
        )
        workflow = dream_config.workflows.get(
            "crystallization",
            WorkflowProfile(provider=selected, model="", enabled=True),
        )
        updated_workflow = replace(
            workflow,
            provider=selected,
            model=values["model"].strip(),
            enabled=True,
        )
        return dream_config.with_provider(selected, updated_provider).with_workflow(
            "crystallization",
            updated_workflow,
        )

    def _apply_pending_api_keys(self, dream_config: DreamConfig) -> DreamConfig:
        next_config = dream_config
        for name, api_key in self._pending_api_keys.items():
            provider = next_config.providers.get(name)
            if provider is not None:
                next_config = next_config.with_provider(name, replace(provider, api_key=api_key))
        return next_config

    def _clear_pending_api_keys_from_draft(
        self,
        dream_config: DreamConfig,
        draft: dict[object, object],
    ) -> None:
        providers = draft.get("providers")
        if type(providers) is not dict:
            return
        for name, raw_provider in providers.items():
            raw_api_key = raw_provider.get("api_key") if type(raw_provider) is dict else None
            if (
                type(name) is not str
                or type(raw_provider) is not dict
                or type(raw_api_key) is not str
            ):
                continue
            if raw_api_key == "":
                self._pending_api_keys.pop(name, None)
                continue
            profile = dream_config.providers.get(name)
            if raw_api_key.strip("*") == "" and profile is not None and profile.api_key:
                self._pending_api_keys.pop(name, None)

    def _apply_dreaming_form(
        self,
        dream_config: DreamConfig,
        values: dict[str, str],
    ) -> DreamConfig:
        return replace(
            dream_config,
            enabled=parse_bool("autostart_enabled", values["autostart_enabled"]),
            schedule_interval_minutes=parse_positive_int(
                "min_interval_minutes",
                values["min_interval_minutes"],
            ),
            min_pending_short_term_memories=parse_positive_int(
                "new_short_term_memory_threshold",
                values["new_short_term_memory_threshold"],
            ),
        )

    def _apply_ingest_form(
        self,
        ingest_config: IngestConfig,
        values: dict[str, str],
    ) -> IngestConfig:
        short_memory = replace(
            ingest_config.short_memory,
            warning_sentence_count=parse_positive_int(
                "short_memory.warning_sentence_count",
                values["warning_sentence_count"],
            ),
            rejection_sentence_count=parse_positive_int(
                "short_memory.rejection_sentence_count",
                values["rejection_sentence_count"],
            ),
        )
        learn = replace(
            ingest_config.learn,
            max_block_chars=parse_positive_int(
                "learn.max_block_chars",
                values["max_block_chars"],
            ),
        )
        return ingest_config.with_short_memory(short_memory).with_learn(learn)

    def _provider_form(
        self,
        raw: object,
        dream_config: DreamConfig,
        selected: str,
    ) -> dict[str, str]:
        provider = dream_config.providers.get(selected, ProviderProfile(type=selected))
        values = raw if type(raw) is dict else {}
        api_path = values.get(
            "api_path",
            values.get("endpoint", values.get("base_url", provider.endpoint)),
        )
        return {
            "model": str(values.get("model", _model_for_profile(dream_config, selected))),
            "api_key": str(values.get("api_key", provider.api_key)),
            "endpoint": "" if api_path is None else str(api_path),
            "timeout_seconds": str(values.get("timeout_seconds", provider.timeout_seconds)),
        }

    def _dreaming_form(
        self,
        raw: object,
        dream_config: DreamConfig,
    ) -> dict[str, str]:
        values = raw if type(raw) is dict else {}
        return {
            "autostart_enabled": str(
                values.get("autostart_enabled", field_value(dream_config.enabled))
            ),
            "min_interval_minutes": str(
                values.get(
                    "min_interval_minutes",
                    field_value(dream_config.schedule_interval_minutes),
                )
            ),
            "new_short_term_memory_threshold": str(
                values.get(
                    "new_short_term_memory_threshold",
                    field_value(dream_config.min_pending_short_term_memories),
                )
            ),
            "max_cycles_per_autostart": str(
                values.get(
                    "max_cycles_per_autostart",
                    "1",
                )
            ),
        }

    def _ingest_form(self, raw: object, ingest_config: IngestConfig) -> dict[str, str]:
        values = raw if type(raw) is dict else {}
        short_memory = ingest_config.short_memory
        learn = ingest_config.learn
        return {
            "warning_sentence_count": str(
                values.get(
                    "warning_sentence_count",
                    field_value(short_memory.warning_sentence_count),
                )
            ),
            "rejection_sentence_count": str(
                values.get(
                    "rejection_sentence_count",
                    field_value(short_memory.rejection_sentence_count),
                )
            ),
            "max_block_chars": str(
                values.get("max_block_chars", field_value(learn.max_block_chars))
            ),
        }

    def _release_form(self, raw: object, release_config: ReleaseConfig) -> ReleaseConfig:
        values = raw if type(raw) is dict else {}
        return validate_release_config(
            release_config.with_update_channel(
                str(values.get("update_channel", release_config.update_channel))
            )
        )

    def _form_values(
        self,
        dream_config: DreamConfig,
        ingest_config: IngestConfig,
        selected: str,
        release_config: ReleaseConfig,
    ) -> dict[str, object]:
        provider = dream_config.providers.get(selected, ProviderProfile(type=selected))
        return {
            "provider": {
                "model": field_value(_model_for_profile(dream_config, selected)),
                "api_key": _redacted_api_key(provider),
                "api_path": field_value(provider.endpoint),
                "timeout_seconds": field_value(provider.timeout_seconds),
            },
            "dreaming": {
                "active_provider": selected,
                "autostart_enabled": field_value(dream_config.enabled),
                "min_interval_minutes": field_value(dream_config.schedule_interval_minutes),
                "new_short_term_memory_threshold": field_value(
                    dream_config.min_pending_short_term_memories
                ),
            },
            "ingest": {
                "warning_sentence_count": field_value(
                    ingest_config.short_memory.warning_sentence_count
                ),
                "rejection_sentence_count": field_value(
                    ingest_config.short_memory.rejection_sentence_count
                ),
                "max_block_chars": field_value(ingest_config.learn.max_block_chars),
            },
            "release": {
                "update_channel": release_config.update_channel,
            },
        }


def _model_for_profile(dream_config: DreamConfig, profile_name: str) -> str:
    for workflow in dream_config.workflows.values():
        if workflow.enabled and workflow.provider == profile_name and workflow.model.strip():
            return workflow.model
    for workflow in dream_config.workflows.values():
        if workflow.provider == profile_name and workflow.model.strip():
            return workflow.model
    return ""


def _result_to_json_dict(result: object) -> dict[str, object]:
    if isinstance(result, dict):
        return dict(result)
    return result.to_json_dict()


def _draft_container_errors(params: dict[str, object]) -> list[str]:
    draft = params.get("draft")
    if draft is None and any(
        key in params for key in ("dream", "dreaming", "providers", "ingest", "release")
    ):
        draft = params
    if type(draft) is not dict:
        return []

    errors: list[str] = []
    raw_dream = draft.get("dream")
    if "dream" in draft and type(raw_dream) is not dict:
        errors.append("dream must be a table")

    raw_dreaming = draft.get("dreaming")
    if "dreaming" in draft and type(raw_dreaming) is not dict:
        errors.append("dreaming must be a table")

    raw_providers = draft.get("providers")
    if "providers" in draft and type(raw_providers) is not dict:
        errors.append("providers must be a table")
    elif type(raw_providers) is dict:
        for name, raw_provider in raw_providers.items():
            if type(name) is str and type(raw_provider) is not dict:
                errors.append(f"providers.{name} must be a table")
    raw_ingest = draft.get("ingest")
    if "ingest" in draft and type(raw_ingest) is not dict:
        errors.append("ingest must be a table")
    raw_release = draft.get("release")
    if "release" in draft and type(raw_release) is not dict:
        errors.append("release must be a table")
    return errors


def _canonical_draft_errors(params: dict[str, object]) -> list[str]:
    draft = params.get("draft")
    if type(draft) is not dict or not draft:
        return []
    missing = [key for key in ("dream", "ingest", "release") if key not in draft]
    if missing:
        return ["draft must include dream, ingest, and release"]
    return []


def _has_complete_draft(params: dict[str, object]) -> bool:
    draft = params.get("draft")
    return type(draft) is dict and all(key in draft for key in ("dream", "ingest", "release"))


def _dream_config_from_draft(
    base: DreamConfig,
    draft: dict[object, object],
) -> DreamConfig:
    dreaming = draft.get("dreaming")
    providers = draft.get("providers")
    workflows = draft.get("workflows")
    next_config = base

    if type(dreaming) is dict:
        updates = {
            key: dreaming[key]
            for key in (
                "enabled",
                "schedule_interval_minutes",
                "min_pending_short_term_memories",
                "max_pending_short_term_memories",
                "max_short_term_memories_per_cycle",
                "not_enough_memories_cycle_threshold",
                "max_changed_crystals_per_cycle",
                "max_related_concepts_per_cycle",
                "max_related_crystals_per_concept",
                "max_total_affected_crystals",
                "general_prompt",
            )
            if key in dreaming
        }
        next_config = replace(next_config, **updates)

    if type(providers) is dict:
        next_providers = dict(next_config.providers)
        for name, raw_provider in providers.items():
            if type(name) is not str or type(raw_provider) is not dict:
                continue
            current = next_providers.get(name, ProviderProfile(type=name))
            updates = {
                key: raw_provider[key]
                for key in ("type", "endpoint", "timeout_seconds")
                if key in raw_provider
            }
            if "api_key" in raw_provider and raw_provider["api_key"] != "***":
                updates["api_key"] = raw_provider["api_key"]
            next_providers[name] = replace(current, **updates)
        next_config = replace(next_config, providers=next_providers)

    if type(workflows) is dict:
        next_workflows = dict(next_config.workflows)
        for name, raw_workflow in workflows.items():
            if type(name) is not str or type(raw_workflow) is not dict:
                continue
            current = next_workflows.get(name, WorkflowProfile(provider="", model=""))
            updates = {
                key: raw_workflow[key]
                for key in ("provider", "model", "enabled")
                if key in raw_workflow
            }
            next_workflows[name] = replace(current, **updates)
        next_config = replace(next_config, workflows=next_workflows)

    return next_config


def _ingest_config_from_draft(
    base: IngestConfig,
    draft: dict[object, object],
) -> IngestConfig:
    short_memory_payload = draft.get("short_memory")
    learn_payload = draft.get("learn")
    short_memory = base.short_memory
    learn = base.learn

    if type(short_memory_payload) is dict:
        short_memory = replace(
            short_memory,
            **{
                key: short_memory_payload[key]
                for key in (
                    "warning_sentence_count",
                    "rejection_sentence_count",
                    "warning_symbol_count",
                    "rejection_symbol_count",
                )
                if key in short_memory_payload
            },
        )
    if type(learn_payload) is dict:
        learn = replace(
            learn,
            **{key: learn_payload[key] for key in ("max_block_chars",) if key in learn_payload},
        )

    flat_values = {
        key: draft[key]
        for key in ("warning_sentence_count", "rejection_sentence_count")
        if key in draft
    }
    if flat_values:
        short_memory = replace(short_memory, **flat_values)
    if "max_block_chars" in draft:
        learn = replace(learn, max_block_chars=draft["max_block_chars"])
    return base.with_short_memory(short_memory).with_learn(learn)


def _compat_dreaming_draft(dream_config: DreamConfig, selected: str) -> dict[str, object]:
    return {
        "active_provider": selected,
        "autostart_enabled": dream_config.enabled,
        "min_interval_minutes": dream_config.schedule_interval_minutes,
        "new_short_term_memory_threshold": dream_config.min_pending_short_term_memories,
    }


def _compat_provider_draft(
    dream_config: DreamConfig,
    selected: str,
) -> dict[str, dict[str, object]]:
    payload: dict[str, dict[str, object]] = {}
    for name in REMOTE_PROVIDERS:
        profile = dream_config.providers.get(name, ProviderProfile(type=name))
        payload[name] = {
            "enabled": name == selected,
            "model": _model_for_profile(dream_config, name),
            "api_key": _redacted_api_key(profile),
            "base_url": profile.endpoint,
            "timeout_seconds": profile.timeout_seconds,
        }
    return payload


def _load_errors(*errors: str) -> list[str]:
    return [error for error in errors if error]


def _redacted_api_key(provider: ProviderProfile) -> str:
    return "***" if provider.api_key else ""


def _release_payload(release_config: ReleaseConfig) -> dict[str, object]:
    return {
        "update_channel": release_config.update_channel,
        "update_target": release_config.update_target,
    }


def _release_draft(release_config: ReleaseConfig) -> dict[str, object]:
    return {"update_channel": release_config.update_channel}


def _redact_error(payload: dict[str, object], dream_config: DreamConfig) -> None:
    error = payload.get("error")
    if type(error) is str and error:
        payload["error"] = redact_configured_secret_values(error, dream_config)
