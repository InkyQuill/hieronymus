from __future__ import annotations

from dataclasses import replace

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import (
    DreamConfig,
    DreamConfigError,
    ProviderProfile,
    default_dream_config,
    load_dream_config,
    redacted_dream_config_payload,
)
from hieronymus.dream_providers import ProviderRegistry
from hieronymus.ingest_config import (
    IngestConfigError,
    default_ingest_config,
    load_ingest_config,
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
from hieronymus.settings import (
    DreamingSettings,
    HieronymusSettings,
    ProviderSettings,
    SettingsError,
    load_settings,
    save_settings,
)
from hieronymus.tui_bridge.config_state import (
    apply_dreaming_form,
    apply_provider_form,
    field_value,
    validate_draft,
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

    def bootstrap(self, params: dict[str, object]) -> dict[str, object]:
        settings = self._settings_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, settings)
        return self._payload(
            self._select_provider(settings, selected),
            selected,
            release_config,
            validation_errors=[release_error] if release_error else None,
            detail=release_error,
        )

    def reload(self, params: dict[str, object]) -> dict[str, object]:
        return self.bootstrap(params)

    def select_provider(self, params: dict[str, object]) -> dict[str, object]:
        settings = self._settings_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._require_remote_provider(params.get("provider"))
        return self._payload(
            self._select_provider(settings, selected),
            selected,
            release_config,
            validation_errors=[release_error] if release_error else None,
            detail=release_error,
        )

    def update_draft(self, params: dict[str, object]) -> dict[str, object]:
        settings = self._settings_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, settings)
        settings = self._select_provider(settings, selected)
        try:
            settings = apply_provider_form(
                settings,
                selected,
                self._provider_form(params.get("provider"), settings.providers[selected]),
            )
            settings = apply_dreaming_form(
                settings,
                self._dreaming_form(params.get("dreaming"), settings.dreaming, selected),
            )
            release_config = self._release_form(params.get("release"), release_config)
            settings = self._select_provider(settings, selected)
        except (SettingsError, ReleaseConfigError) as error:
            return self._payload(settings, selected, release_config, validation_errors=[str(error)])
        return self._payload(
            settings,
            selected,
            release_config,
            validation_errors=[release_error] if release_error else None,
            detail=release_error,
        )

    def save(self, params: dict[str, object]) -> dict[str, object]:
        settings = self._settings_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, settings)
        if "selected_provider" in params or "provider" in params:
            settings = self._select_provider(settings, selected)
        errors = self._validation_errors(params, settings, release_config)
        if release_error:
            errors = [*errors, release_error]
        if errors:
            return self._payload(settings, selected, release_config, validation_errors=errors)
        save_settings(self.config, settings)
        save_release_config(self.config, release_config)
        return self._payload(settings, selected, release_config)

    def check_provider(self, params: dict[str, object]) -> dict[str, object]:
        settings = self._settings_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, settings)
        settings = self._select_provider(settings, selected)
        profile_context = self._dream_profile_context(selected)
        if profile_context is not None:
            if errors := _draft_container_errors(params):
                return self._payload(settings, selected, release_config, validation_errors=errors)
            profile, model = profile_context
            result = self.registry.check_profile(self.config, selected, profile, model=model)
            check_result = _result_to_json_dict(result)
            _redact_error(check_result, self.config)
            suggestions = None
            if check_result.get("ok") is True:
                suggestion_result = self.registry.list_profile_model_suggestions(
                    self.config,
                    selected,
                    profile,
                )
                suggestions = _result_to_json_dict(suggestion_result)
                _redact_error(suggestions, self.config)
            return self._payload(
                settings,
                selected,
                release_config,
                check_result=check_result,
                suggestions=suggestions,
                validation_errors=[release_error] if release_error else None,
                detail=release_error,
            )
        if errors := self._validation_errors(params, settings, release_config):
            return self._payload(settings, selected, release_config, validation_errors=errors)
        result = self.registry.check(self.config, selected, settings=settings)
        check_result = _result_to_json_dict(result)
        _redact_error(check_result, self.config)
        suggestions = None
        if check_result.get("ok") is True and hasattr(self.registry, "list_model_suggestions"):
            suggestion_result = self.registry.list_model_suggestions(
                self.config,
                selected,
                settings=settings,
            )
            suggestions = _result_to_json_dict(suggestion_result)
            _redact_error(suggestions, self.config)
        return self._payload(
            settings,
            selected,
            release_config,
            check_result=check_result,
            suggestions=suggestions,
            validation_errors=[release_error] if release_error else None,
            detail=release_error,
        )

    def model_suggestions(self, params: dict[str, object]) -> dict[str, object]:
        settings = self._settings_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, settings)
        settings = self._select_provider(settings, selected)
        profile_context = self._dream_profile_context(selected)
        if profile_context is not None:
            if errors := _draft_container_errors(params):
                return self._payload(settings, selected, release_config, validation_errors=errors)
            profile, _ = profile_context
            result = self.registry.list_profile_model_suggestions(self.config, selected, profile)
            suggestions = _result_to_json_dict(result)
            _redact_error(suggestions, self.config)
            return self._payload(
                settings,
                selected,
                release_config,
                suggestions=suggestions,
                validation_errors=[release_error] if release_error else None,
                detail=release_error,
            )
        if errors := self._validation_errors(params, settings, release_config):
            return self._payload(settings, selected, release_config, validation_errors=errors)
        result = self.registry.list_model_suggestions(self.config, selected, settings=settings)
        suggestions = _result_to_json_dict(result)
        _redact_error(suggestions, self.config)
        return self._payload(
            settings,
            selected,
            release_config,
            suggestions=suggestions,
            validation_errors=[release_error] if release_error else None,
            detail=release_error,
        )

    def _payload(
        self,
        settings: HieronymusSettings,
        selected: str,
        release_config: ReleaseConfig,
        *,
        validation_errors: list[str] | None = None,
        check_result: dict[str, object] | None = None,
        suggestions: dict[str, object] | None = None,
        detail: str = "",
    ) -> dict[str, object]:
        errors = validate_draft(settings) if validation_errors is None else validation_errors
        dream_config, dream_error = _safe_dream_config_payload(self.config)
        if dream_error:
            errors = [*errors, dream_error]
            detail = detail or dream_error
        ingest_config, ingest_error = _safe_ingest_config_payload(self.config)
        if ingest_error:
            errors = [*errors, ingest_error]
            detail = detail or ingest_error
        return {
            "config_paths": {
                "data_root": str(self.config.data_root),
                "config_root": str(self.config.config_root),
                "settings_path": str(self.config.settings_path),
                "ingest_config_path": str(self.config.ingest_config_path),
                "release_config_path": str(self.config.release_config_path),
            },
            "dreaming": dream_config["dreaming"],
            "providers": dream_config["providers"],
            "workflows": dream_config["workflows"],
            "ingest": ingest_config,
            "release": _release_payload(release_config),
            "model_cache": load_model_cache(self.config).to_payload(),
            "provider_choices": self._provider_choices(),
            "selected_provider": selected,
            "draft": {**settings.to_json_dict(), "release": _release_draft(release_config)},
            "form_values": self._form_values(settings, selected, release_config),
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

    def _settings_from_params(self, params: dict[str, object]) -> HieronymusSettings:
        draft = params.get("draft")
        if draft is None and any(key in params for key in ("dreaming", "providers")):
            draft = params
        if type(draft) is dict and draft:
            return self._settings_from_draft(draft)
        return load_settings(self.config)

    def _settings_from_draft(self, draft: dict[object, object]) -> HieronymusSettings:
        base = load_settings(self.config)
        providers = dict(base.providers)
        raw_providers = draft.get("providers")
        if type(raw_providers) is dict:
            for name, raw_provider in raw_providers.items():
                if type(name) is not str or type(raw_provider) is not dict:
                    continue
                current = providers.get(name, ProviderSettings())
                providers[name] = replace(
                    current,
                    **{
                        key: raw_provider[key]
                        for key in (
                            "enabled",
                            "model",
                            "api_key_env",
                            "base_url",
                            "timeout_seconds",
                        )
                        if key in raw_provider
                    },
                )

        dreaming = base.dreaming
        raw_dreaming = draft.get("dreaming")
        if type(raw_dreaming) is dict:
            dreaming = replace(
                dreaming,
                **{
                    key: raw_dreaming[key]
                    for key in (
                        "active_provider",
                        "autostart_enabled",
                        "min_interval_minutes",
                        "new_short_term_memory_threshold",
                        "max_cycles_per_autostart",
                    )
                    if key in raw_dreaming
                },
            )
        return HieronymusSettings(dreaming=dreaming, providers=providers)

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
        settings: HieronymusSettings,
        release_config: ReleaseConfig,
    ) -> list[str]:
        errors = _draft_container_errors(params)
        if errors:
            return errors
        try:
            validate_release_config(release_config)
        except ReleaseConfigError as error:
            return [str(error)]
        return validate_draft(settings)

    def _selected_provider(
        self,
        params: dict[str, object],
        settings: HieronymusSettings,
    ) -> str:
        explicit = params.get("selected_provider", params.get("provider"))
        if explicit is not None:
            return self._require_remote_provider(explicit)
        if settings.dreaming.active_provider in REMOTE_PROVIDERS:
            return settings.dreaming.active_provider
        return "openai"

    def _require_remote_provider(self, value: object) -> str:
        if type(value) is not str or value not in REMOTE_PROVIDERS:
            raise ValueError(f"unsupported remote provider: {value}")
        return value

    def _dream_profile_context(self, selected: str) -> tuple[ProviderProfile, str] | None:
        if not self.config.dream_config_path.exists():
            return None
        try:
            dream_config = load_dream_config(self.config)
        except DreamConfigError:
            return None
        profile = dream_config.providers.get(selected)
        if profile is None:
            return None
        return profile, _model_for_profile(dream_config, selected)

    def _select_provider(
        self,
        settings: HieronymusSettings,
        selected: str,
    ) -> HieronymusSettings:
        providers = dict(settings.providers)
        for name, provider in list(providers.items()):
            providers[name] = replace(provider, enabled=name == selected)
        dreaming = replace(settings.dreaming, active_provider=selected)
        return HieronymusSettings(dreaming=dreaming, providers=providers)

    def _provider_form(
        self,
        raw: object,
        provider: ProviderSettings,
    ) -> dict[str, str]:
        values = raw if type(raw) is dict else {}
        api_path = values.get("api_path", values.get("base_url", provider.base_url))
        return {
            "enabled": "yes",
            "model": str(values.get("model", provider.model)),
            "api_key_env": str(values.get("api_key_env", provider.api_key_env)),
            "base_url": "" if api_path is None else str(api_path),
            "timeout_seconds": str(values.get("timeout_seconds", provider.timeout_seconds)),
        }

    def _dreaming_form(
        self,
        raw: object,
        dreaming: DreamingSettings,
        selected: str,
    ) -> dict[str, str]:
        values = raw if type(raw) is dict else {}
        return {
            "active_provider": selected,
            "autostart_enabled": str(
                values.get("autostart_enabled", field_value(dreaming.autostart_enabled))
            ),
            "min_interval_minutes": str(
                values.get("min_interval_minutes", field_value(dreaming.min_interval_minutes))
            ),
            "new_short_term_memory_threshold": str(
                values.get(
                    "new_short_term_memory_threshold",
                    field_value(dreaming.new_short_term_memory_threshold),
                )
            ),
            "max_cycles_per_autostart": str(
                values.get(
                    "max_cycles_per_autostart",
                    field_value(dreaming.max_cycles_per_autostart),
                )
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
        settings: HieronymusSettings,
        selected: str,
        release_config: ReleaseConfig,
    ) -> dict[str, object]:
        provider = settings.providers[selected]
        return {
            "provider": {
                "enabled": field_value(provider.enabled),
                "model": field_value(provider.model),
                "api_key_env": field_value(provider.api_key_env),
                "api_path": field_value(provider.base_url),
                "timeout_seconds": field_value(provider.timeout_seconds),
            },
            "dreaming": {
                "active_provider": field_value(settings.dreaming.active_provider),
                "autostart_enabled": field_value(settings.dreaming.autostart_enabled),
                "min_interval_minutes": field_value(settings.dreaming.min_interval_minutes),
                "new_short_term_memory_threshold": field_value(
                    settings.dreaming.new_short_term_memory_threshold
                ),
                "max_cycles_per_autostart": field_value(settings.dreaming.max_cycles_per_autostart),
            },
            "release": {
                "update_channel": release_config.update_channel,
            },
        }


def _safe_dream_config_payload(config: HieronymusConfig) -> tuple[dict[str, object], str]:
    try:
        return redacted_dream_config_payload(load_dream_config(config)), ""
    except DreamConfigError as error:
        return redacted_dream_config_payload(default_dream_config()), str(error)


def _safe_ingest_config_payload(config: HieronymusConfig) -> tuple[dict[str, object], str]:
    try:
        return load_ingest_config(config).to_payload(), ""
    except IngestConfigError as error:
        return default_ingest_config().to_payload(), str(error)


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
    if draft is None and any(key in params for key in ("dreaming", "providers", "release")):
        draft = params
    if type(draft) is not dict:
        return []

    errors: list[str] = []
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
    raw_release = draft.get("release")
    if "release" in draft and type(raw_release) is not dict:
        errors.append("release must be a table")
    return errors


def _release_payload(release_config: ReleaseConfig) -> dict[str, object]:
    return {
        "update_channel": release_config.update_channel,
        "update_target": release_config.update_target,
    }


def _release_draft(release_config: ReleaseConfig) -> dict[str, object]:
    return {"update_channel": release_config.update_channel}


def _redact_error(payload: dict[str, object], config: HieronymusConfig) -> None:
    error = payload.get("error")
    if type(error) is str and error:
        try:
            dream_config = load_dream_config(config)
        except (DreamConfigError, OSError):
            return
        payload["error"] = redact_configured_secret_values(error, dream_config)
