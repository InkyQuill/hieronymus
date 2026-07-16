from __future__ import annotations

from dataclasses import replace

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import (
    DreamConfig,
    DreamConfigError,
    WorkflowProfile,
    default_dream_config,
    load_dream_config,
    redacted_dream_config_payload,
    save_dream_config,
    validate_dream_config,
)
from hieronymus.dream_providers import ProviderProfile as RuntimeProviderProfile
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
from hieronymus.provider_config import (
    ProviderCatalog,
    ProviderCatalogError,
    ProviderDefaults,
    default_provider_catalog,
    load_provider_catalog,
    redacted_provider_catalog_payload,
    save_provider_catalog,
    validate_provider_catalog,
)
from hieronymus.provider_config import (
    ProviderProfile as CatalogProviderProfile,
)
from hieronymus.release_config import (
    ReleaseConfig,
    ReleaseConfigError,
    default_release_config,
    load_release_config,
    save_release_config,
    validate_release_config,
)
from hieronymus.secrets import configured_secret_values
from hieronymus.tui_bridge.config_state import (
    field_value,
    parse_bool,
    parse_float,
    parse_positive_int,
)

REMOTE_PROVIDERS = ("openai", "gemini", "anthropic")


def _form_schema() -> dict[str, object]:
    return {
        "sections": [
            {
                "id": "dream",
                "label": "Dream",
                "description": "dream.conf",
            },
            {
                "id": "provider_catalog",
                "label": "Providers",
                "description": "provider.conf",
            },
            {
                "id": "ingest",
                "label": "Ingest",
                "description": "ingest.conf",
            },
            {
                "id": "release",
                "label": "Release",
                "description": "release.conf",
            },
        ],
        "groups": [
            {
                "id": "provider",
                "section": "provider_catalog",
                "label": "Provider",
                "description": "Connection settings for the selected provider profile.",
            },
            {
                "id": "provider_catalog",
                "section": "provider_catalog",
                "label": "Provider Catalog",
                "description": "Default provider profile and editable profile fields.",
            },
            {
                "id": "workflows",
                "section": "dream",
                "label": "Workflows",
                "description": "Dream workflow provider and model assignments.",
            },
            {
                "id": "dreaming",
                "section": "dream",
                "label": "Dreaming",
                "description": (
                    "Autostart thresholds for turning short-term memory into durable memory."
                ),
            },
            {
                "id": "ingest",
                "section": "ingest",
                "label": "Ingestion",
                "description": "Limits for short-term memory and Learn ingestion.",
            },
            {
                "id": "release",
                "section": "release",
                "label": "Updates",
                "description": "Managed install update channel.",
            },
        ],
        "fields": [
            _field(
                "provider.model",
                "provider",
                "Model",
                "text",
                section="provider_catalog",
                hint="Workflow model used by the selected provider.",
                placeholder="e.g. gpt-4.1-mini",
            ),
            _field(
                "provider.api_key",
                "provider",
                "API Key",
                "secret",
                section="provider_catalog",
                hint="Stored as plaintext in provider.conf and redacted in UI payloads.",
                placeholder="stored in provider.conf",
                redacted=True,
            ),
            _field(
                "provider.api_path",
                "provider",
                "API Path",
                "text",
                section="provider_catalog",
                hint="Base URL for OpenAI-compatible, Gemini, or Anthropic gateways.",
                placeholder="e.g. https://api.openai.com/v1",
            ),
            _field(
                "provider.timeout_seconds",
                "provider",
                "Timeout (seconds)",
                "number",
                section="provider_catalog",
                hint="Provider check and model-list timeout.",
                placeholder="e.g. 30",
                minimum=1,
            ),
            _field(
                "provider_catalog.defaults.provider",
                "provider_catalog",
                "Default Provider",
                "text",
                section="provider_catalog",
                hint="Provider profile id used when a workflow leaves provider empty.",
            ),
            _field(
                "provider_catalog.defaults.model",
                "provider_catalog",
                "Default Model",
                "text",
                section="provider_catalog",
                hint="Model used when a workflow leaves model empty.",
            ),
            _field(
                "provider_catalog.profile.name",
                "provider_catalog",
                "Profile Name",
                "text",
                section="provider_catalog",
                hint="Human-readable provider profile name.",
            ),
            _field(
                "provider_catalog.profile.type",
                "provider_catalog",
                "Profile Type",
                "choice",
                section="provider_catalog",
                hint="Runtime provider protocol used by this profile.",
                choices=["anthropic", "google", "ollama", "openai"],
            ),
            _field(
                "provider_catalog.profile.url",
                "provider_catalog",
                "Profile URL",
                "text",
                section="provider_catalog",
                hint="Provider base URL.",
            ),
            _field(
                "provider_catalog.profile.key",
                "provider_catalog",
                "Profile Key",
                "secret",
                section="provider_catalog",
                hint="Provider API key stored in provider.conf.",
                redacted=True,
            ),
            _field(
                "provider_catalog.profile.timeout_seconds",
                "provider_catalog",
                "Profile Timeout",
                "number",
                section="provider_catalog",
                hint="Provider request timeout in seconds.",
                minimum=1,
            ),
            *[
                _field(
                    f"workflows.{workflow_name}.{field_name}",
                    "workflows",
                    f"{workflow_label} {field_label}",
                    field_type,
                    section="dream",
                    hint=hint,
                )
                for workflow_name, workflow_label in (
                    ("crystallization", "Crystallization"),
                    ("reinforcement_compaction", "Reinforcement"),
                    ("relation_discovery", "Relations"),
                )
                for field_name, field_label, field_type, hint in (
                    ("provider", "Provider", "text", "Provider profile id for this workflow."),
                    ("model", "Model", "text", "Model for this workflow."),
                    ("enabled", "Enabled", "toggle", "Whether this workflow can run."),
                )
            ],
            _field(
                "dreaming.autostart_enabled",
                "dreaming",
                "Autostart Enabled",
                "toggle",
                section="dream",
                hint="Whether scheduled dreaming can run automatically.",
                choices=["yes", "no"],
                default="no",
            ),
            _field(
                "dreaming.min_interval_minutes",
                "dreaming",
                "Min Interval (minutes)",
                "number",
                section="dream",
                hint="Minimum minutes between scheduled dream cycles.",
                placeholder="e.g. 30",
                minimum=1,
            ),
            _field(
                "dreaming.new_short_term_memory_threshold",
                "dreaming",
                "New Memory Threshold",
                "number",
                section="dream",
                hint="Pending short-term memories required before scheduled dreaming runs.",
                placeholder="e.g. 25",
                minimum=1,
            ),
            _field(
                "ingest.warning_sentence_count",
                "ingest",
                "Memory Warn Sentences",
                "number",
                section="ingest",
                hint="Warn when direct short-term memory exceeds this sentence count.",
                placeholder="e.g. 6",
                default="6",
                minimum=1,
            ),
            _field(
                "ingest.rejection_sentence_count",
                "ingest",
                "Memory Reject Sentences",
                "number",
                section="ingest",
                hint="Reject direct short-term memory above this sentence count.",
                placeholder="e.g. 30",
                default="30",
                minimum=1,
            ),
            _field(
                "ingest.max_block_chars",
                "ingest",
                "Learn Block Characters",
                "number",
                section="ingest",
                hint="Maximum Learn block size before splitting.",
                placeholder="e.g. 1200",
                default="1200",
                minimum=1,
            ),
            _field(
                "release.update_channel",
                "release",
                "Update Channel",
                "choice",
                section="release",
                hint="Stable uses release tags; dev tracks the configured development target.",
                choices=["stable", "dev"],
                default="stable",
            ),
        ],
    }


def _field(
    key: str,
    group: str,
    label: str,
    field_type: str,
    *,
    section: str,
    hint: str,
    placeholder: str = "",
    choices: list[str] | None = None,
    default: str = "",
    minimum: int | None = None,
    redacted: bool = False,
) -> dict[str, object]:
    payload: dict[str, object] = {
        "key": key,
        "group": group,
        "section": section,
        "label": label,
        "hint": hint,
        "placeholder": placeholder,
        "type": field_type,
        "redacted": redacted,
    }
    if choices is not None:
        payload["choices"] = choices
    if default:
        payload["default"] = default
    if minimum is not None:
        payload["minimum"] = minimum
    return payload


def _required_text(payload: dict[str, object], field_name: str) -> str:
    value = payload.get(field_name)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{field_name} is required")
    return value.strip()


def _optional_text(payload: dict[str, object], field_name: str) -> str:
    value = payload.get(field_name, "")
    if not isinstance(value, str):
        raise ValueError(f"{field_name} must be text")
    return value.strip()


def _positive_timeout(payload: dict[str, object]) -> float:
    value = _required_text(payload, "timeout_seconds")
    timeout = parse_float("timeout_seconds", value)
    if timeout <= 0:
        raise ValueError("timeout_seconds must be greater than zero")
    return timeout


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
        if not params:
            self._pending_api_keys.clear()
        dream_config, dream_error = self._dream_from_params(params)
        provider_catalog, provider_error = self._provider_catalog_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config, provider_catalog)
        return self._payload(
            dream_config,
            provider_catalog,
            ingest_config,
            selected,
            release_config,
            validation_errors=_load_errors(
                dream_error,
                provider_error,
                ingest_error,
                release_error,
            ),
            detail=dream_error or provider_error or ingest_error or release_error,
        )

    def reload(self, params: dict[str, object]) -> dict[str, object]:
        return self.bootstrap(params)

    def provider_list(self, params: dict[str, object]) -> dict[str, object]:
        """Return only user-created provider profiles for the web console."""
        provider_catalog, load_error = self._provider_catalog_from_params({})
        providers = [
            self._provider_editor_payload(provider_catalog, provider_id)
            for provider_id in sorted(provider_catalog.providers)
        ]
        return {"providers": providers, "error": load_error}

    def provider_detail(self, params: dict[str, object]) -> dict[str, object]:
        """Return one profile for the provider editor modal."""
        provider_catalog, load_error = self._provider_catalog_from_params({})
        provider_id = self._require_provider_id(params.get("provider_id"))
        if provider_id not in provider_catalog.providers:
            return {"error": f"provider profile not found: {provider_id}"}
        return {
            "provider": self._provider_editor_payload(provider_catalog, provider_id),
            "error": load_error,
        }

    def save_provider(self, params: dict[str, object]) -> dict[str, object]:
        """Persist one provider profile without reading or writing dream.conf."""
        raw_provider = params.get("provider")
        if type(raw_provider) is not dict:
            raise ValueError("provider must be an object")
        provider_id = self._require_provider_id(raw_provider.get("id"))
        provider_catalog, load_error = self._provider_catalog_from_params({})
        if load_error:
            return {"error": load_error}
        try:
            existing_profile = provider_catalog.providers.get(provider_id)
            submitted_key = _optional_text(raw_provider, "key")
            profile = CatalogProviderProfile(
                name=_required_text(raw_provider, "name"),
                type=(
                    "google"
                    if _required_text(raw_provider, "type") == "gemini"
                    else _required_text(raw_provider, "type")
                ),
                url=_required_text(raw_provider, "url"),
                key=(
                    submitted_key
                    if submitted_key or existing_profile is None
                    else existing_profile.key
                ),
                timeout_seconds=_positive_timeout(raw_provider),
            )
            updated_catalog = provider_catalog.with_provider(provider_id, profile)
            validate_provider_catalog(updated_catalog)
        except (ProviderCatalogError, ValueError) as error:
            return {"error": str(error)}
        save_provider_catalog(self.config, updated_catalog)
        return {
            "provider": self._provider_editor_payload(updated_catalog, provider_id),
            "error": "",
        }

    def delete_provider(self, params: dict[str, object]) -> dict[str, object]:
        provider_id = self._require_provider_id(params.get("provider_id"))
        provider_catalog, load_error = self._provider_catalog_from_params({})
        if load_error:
            return {"error": load_error}
        if provider_id not in provider_catalog.providers:
            return {"error": f"provider profile not found: {provider_id}"}
        dream_config, dream_error = self._dream_from_params({})
        if dream_error:
            return {"error": dream_error}
        used_by = [
            name
            for name, workflow in dream_config.workflows.items()
            if workflow.provider == provider_id
        ]
        if used_by:
            return {"error": f"provider profile is used by: {', '.join(sorted(used_by))}"}
        providers = {
            name: profile
            for name, profile in provider_catalog.providers.items()
            if name != provider_id
        }
        defaults = provider_catalog.defaults
        if defaults.provider == provider_id:
            defaults = ProviderDefaults()
        updated_catalog = ProviderCatalog(providers=providers, defaults=defaults)
        save_provider_catalog(self.config, updated_catalog)
        return {"deleted": provider_id, "error": ""}

    def provider_models(self, params: dict[str, object]) -> dict[str, object]:
        provider_id = self._require_provider_id(params.get("provider_id"))
        provider_catalog, load_error = self._provider_catalog_from_params({})
        if load_error:
            return {"models": [], "source": "", "error": load_error}
        if provider_id not in provider_catalog.providers:
            return {
                "models": [],
                "source": "",
                "error": f"provider profile not found: {provider_id}",
            }
        result = self.registry.list_model_suggestions(self.config, provider_id)
        payload = _result_to_json_dict(result)
        _redact_error(payload, provider_catalog)
        return {
            "models": payload.get("models", []),
            "source": payload.get("source", ""),
            "error": payload.get("error", ""),
        }

    def check_saved_provider(self, params: dict[str, object]) -> dict[str, object]:
        provider_id = self._require_provider_id(params.get("provider_id"))
        provider_catalog, load_error = self._provider_catalog_from_params({})
        if load_error:
            return {"check": {}, "error": load_error}
        catalog_profile = provider_catalog.providers.get(provider_id)
        if catalog_profile is None:
            return {"check": {}, "error": f"provider profile not found: {provider_id}"}
        result = self.registry.check_profile_connection(
            self.config,
            provider_id,
            _runtime_provider_profile(catalog_profile),
        )
        payload = _result_to_json_dict(result)
        _redact_error(payload, provider_catalog)
        error = str(payload.get("error", ""))
        return {
            "check": {
                "ok": not error,
                "models": payload.get("models", []),
                "source": payload.get("source", ""),
                "error": error,
            },
            "error": "",
        }

    def dream_settings(self, _params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params({})
        provider_catalog, provider_error = self._provider_catalog_from_params({})
        model_cache = load_model_cache(self.config).to_payload()
        return {
            "dream": dream_config.to_payload(),
            "providers": [
                self._provider_editor_payload(provider_catalog, provider_id)
                for provider_id in sorted(provider_catalog.providers)
            ],
            "model_cache": model_cache,
            "error": dream_error or provider_error,
        }

    def save_dream_settings(self, params: dict[str, object]) -> dict[str, object]:
        raw_dream = params.get("dream")
        if type(raw_dream) is not dict:
            raise ValueError("dream must be an object")
        dream_config, load_error = self._dream_from_params({})
        if load_error:
            return {"error": load_error}
        try:
            dream_config = validate_dream_config(_dream_config_from_draft(dream_config, raw_dream))
        except DreamConfigError as error:
            return {"error": str(error)}
        save_dream_config(self.config, dream_config)
        return {"dream": dream_config.to_payload(), "error": ""}

    def ingest_settings(self, _params: dict[str, object]) -> dict[str, object]:
        ingest_config, load_error = self._ingest_from_params({})
        return {"ingest": ingest_config.to_payload(), "error": load_error}

    def save_ingest_settings(self, params: dict[str, object]) -> dict[str, object]:
        raw_ingest = params.get("ingest")
        if type(raw_ingest) is not dict:
            raise ValueError("ingest must be an object")
        ingest_config, load_error = self._ingest_from_params({})
        if load_error:
            return {"error": load_error}
        try:
            ingest_config = validate_ingest_config(
                _ingest_config_from_draft(ingest_config, raw_ingest)
            )
        except IngestConfigError as error:
            return {"error": str(error)}
        save_ingest_config(self.config, ingest_config)
        return {"ingest": ingest_config.to_payload(), "error": ""}

    def release_settings(self, _params: dict[str, object]) -> dict[str, object]:
        release_config, load_error = self._release_from_params({})
        return {"release": _release_draft(release_config), "error": load_error}

    def save_release_settings(self, params: dict[str, object]) -> dict[str, object]:
        raw_release = params.get("release")
        if type(raw_release) is not dict:
            raise ValueError("release must be an object")
        release_config, load_error = self._release_from_params({})
        if load_error:
            return {"error": load_error}
        try:
            release_config = self._release_form(raw_release, release_config)
        except ReleaseConfigError as error:
            return {"error": str(error)}
        save_release_config(self.config, release_config)
        return {"release": _release_draft(release_config), "error": ""}

    def select_provider(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        provider_catalog, provider_error = self._provider_catalog_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._require_provider_id(params.get("provider"))
        return self._payload(
            self._select_provider(dream_config, selected),
            provider_catalog,
            ingest_config,
            selected,
            release_config,
            validation_errors=_load_errors(
                dream_error,
                provider_error,
                ingest_error,
                release_error,
            ),
            detail=dream_error or provider_error or ingest_error or release_error,
        )

    def update_draft(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        provider_catalog, provider_error = self._provider_catalog_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config, provider_catalog)
        dream_config = self._select_provider(dream_config, selected)
        if errors := _canonical_draft_errors(params):
            return self._payload(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        try:
            dream_config, provider_catalog = self._apply_provider_form(
                dream_config,
                provider_catalog,
                selected,
                self._provider_form(
                    params.get("provider"),
                    dream_config,
                    provider_catalog,
                    selected,
                ),
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
            validate_provider_catalog(provider_catalog)
            validate_ingest_config(ingest_config)
        except (
            DreamConfigError,
            ProviderCatalogError,
            IngestConfigError,
            ValueError,
            ReleaseConfigError,
        ) as error:
            return self._payload(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
                validation_errors=[str(error)],
            )
        return self._payload(
            dream_config,
            provider_catalog,
            ingest_config,
            selected,
            release_config,
            validation_errors=_load_errors(
                dream_error,
                provider_error,
                ingest_error,
                release_error,
            ),
            detail=dream_error or provider_error or ingest_error or release_error,
        )

    def save(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        provider_catalog, provider_error = self._provider_catalog_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config, provider_catalog)
        if "selected_provider" in params or "provider" in params:
            dream_config = self._select_provider(dream_config, selected)
        canonical_errors = _canonical_draft_errors(params)
        validation_errors = (
            []
            if canonical_errors
            else self._validation_errors(
                params,
                dream_config,
                provider_catalog,
                ingest_config,
                release_config,
            )
        )
        load_errors = _load_errors(dream_error, provider_error, ingest_error, release_error)
        errors = [*canonical_errors, *validation_errors]
        if canonical_errors or validation_errors or not _has_complete_draft(params):
            errors = [*errors, *load_errors]
        if errors:
            return self._payload(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        save_provider_catalog(self.config, provider_catalog)
        save_dream_config(self.config, dream_config)
        save_ingest_config(self.config, ingest_config)
        save_release_config(self.config, release_config)
        return self._payload(
            dream_config,
            provider_catalog,
            ingest_config,
            selected,
            release_config,
        )

    def check_provider(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        provider_catalog, provider_error = self._provider_catalog_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config, provider_catalog)
        dream_config = self._select_provider(dream_config, selected)
        errors = _canonical_draft_errors(params)
        if not errors:
            errors = self._validation_errors(
                params,
                dream_config,
                provider_catalog,
                ingest_config,
                release_config,
            )
        errors = [*errors, *_load_errors(dream_error, provider_error, ingest_error, release_error)]
        if errors:
            return self._payload(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        profile_context = self._profile_context(dream_config, provider_catalog, selected)
        if profile_context is not None:
            profile, model = profile_context
            result = self.registry.check_profile(self.config, selected, profile, model=model)
            check_result = _result_to_json_dict(result)
            _redact_error(check_result, provider_catalog)
            suggestions = None
            if check_result.get("ok") is True and hasattr(
                self.registry,
                "list_profile_model_suggestions",
            ):
                suggestion_result = self.registry.list_profile_model_suggestions(
                    self.config,
                    selected,
                    profile,
                )
                suggestions = _result_to_json_dict(suggestion_result)
                _redact_error(suggestions, provider_catalog)
            return self._payload(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
                check_result=check_result,
                suggestions=suggestions,
                validation_errors=[],
            )
        result = self.registry.check(self.config, selected)
        check_result = _result_to_json_dict(result)
        _redact_error(check_result, provider_catalog)
        suggestions = None
        if check_result.get("ok") is True and hasattr(self.registry, "list_model_suggestions"):
            suggestion_result = self.registry.list_model_suggestions(self.config, selected)
            suggestions = _result_to_json_dict(suggestion_result)
            _redact_error(suggestions, provider_catalog)
        return self._payload(
            dream_config,
            provider_catalog,
            ingest_config,
            selected,
            release_config,
            check_result=check_result,
            suggestions=suggestions,
            validation_errors=[],
        )

    def model_suggestions(self, params: dict[str, object]) -> dict[str, object]:
        dream_config, dream_error = self._dream_from_params(params)
        provider_catalog, provider_error = self._provider_catalog_from_params(params)
        ingest_config, ingest_error = self._ingest_from_params(params)
        release_config, release_error = self._release_from_params(params)
        selected = self._selected_provider(params, dream_config, provider_catalog)
        dream_config = self._select_provider(dream_config, selected)
        errors = _canonical_draft_errors(params)
        if not errors:
            errors = self._validation_errors(
                params,
                dream_config,
                provider_catalog,
                ingest_config,
                release_config,
            )
        errors = [*errors, *_load_errors(dream_error, provider_error, ingest_error, release_error)]
        if errors:
            return self._payload(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
                validation_errors=errors,
            )
        profile_context = self._profile_context(dream_config, provider_catalog, selected)
        if profile_context is not None and hasattr(
            self.registry,
            "list_profile_model_suggestions",
        ):
            profile, _ = profile_context
            result = self.registry.list_profile_model_suggestions(self.config, selected, profile)
            suggestions = _result_to_json_dict(result)
            _redact_error(suggestions, provider_catalog)
            return self._payload(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
                suggestions=suggestions,
                validation_errors=[],
            )
        result = self.registry.list_model_suggestions(self.config, selected)
        suggestions = _result_to_json_dict(result)
        _redact_error(suggestions, provider_catalog)
        return self._payload(
            dream_config,
            provider_catalog,
            ingest_config,
            selected,
            release_config,
            suggestions=suggestions,
            validation_errors=[],
        )

    def _payload(
        self,
        dream_config: DreamConfig,
        provider_catalog: ProviderCatalog,
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
            self._validation_errors(
                {},
                dream_config,
                provider_catalog,
                ingest_config,
                release_config,
            )
            if validation_errors is None
            else validation_errors
        )
        dream_payload = redacted_dream_config_payload(dream_config)
        provider_payload = _provider_catalog_payload(provider_catalog)
        ingest_payload = ingest_config.to_payload()
        return {
            "config_paths": {
                "data_root": str(self.config.data_root),
                "config_root": str(self.config.config_root),
                "dream_config_path": str(self.config.dream_config_path),
                "provider_config_path": str(self.config.provider_config_path),
                "ingest_config_path": str(self.config.ingest_config_path),
                "release_config_path": str(self.config.release_config_path),
            },
            "dreaming": dream_payload["dreaming"],
            "provider_catalog": provider_payload,
            "providers": _compat_provider_payload(provider_catalog),
            "workflows": dream_payload["workflows"],
            "ingest": ingest_payload,
            "release": _release_payload(release_config),
            "model_cache": load_model_cache(self.config).to_payload(),
            "provider_choices": self._provider_choices(provider_catalog),
            "selected_provider": selected,
            "draft": self._draft_payload(
                dream_config,
                provider_catalog,
                ingest_config,
                release_config,
                selected,
            ),
            "form_values": self._form_values(
                dream_config,
                provider_catalog,
                ingest_config,
                selected,
                release_config,
            ),
            "form_schema": _form_schema(),
            "validation": {
                "ok": not errors,
                "errors": errors,
                "field_errors": _field_errors(errors, selected),
            },
            "check_result": check_result or {},
            "suggestions": suggestions or {},
            "detail": detail,
        }

    def _provider_choices(self, provider_catalog: ProviderCatalog) -> list[dict[str, object]]:
        registry = self.registry if hasattr(self.registry, "list") else ProviderRegistry()
        choices = [
            {
                "name": provider.name,
                "display_name": provider.display_name,
                "requires_api_key": provider.requires_api_key,
                "supports_api_path": provider.supports_base_url,
                "configured": provider.name in provider_catalog.providers,
            }
            for provider in registry.list()
            if provider.name in REMOTE_PROVIDERS
        ]
        existing = {choice["name"] for choice in choices}
        for profile_id, profile in provider_catalog.providers.items():
            if profile_id in existing:
                continue
            choices.append(
                {
                    "name": profile_id,
                    "display_name": profile.name,
                    "requires_api_key": _profile_requires_api_key(profile),
                    "supports_api_path": True,
                    "configured": True,
                    "type": profile.type,
                }
            )
        return choices

    def _provider_editor_payload(
        self,
        provider_catalog: ProviderCatalog,
        provider_id: str,
    ) -> dict[str, object]:
        profile = provider_catalog.providers.get(provider_id)
        if profile is None:
            metadata = self.registry.metadata(provider_id)
            profile = CatalogProviderProfile(
                name=metadata.display_name,
                type="google" if provider_id == "gemini" else provider_id,
            )
        model = (
            provider_catalog.defaults.model
            if provider_catalog.defaults.provider == provider_id
            else ""
        )
        return {
            "id": provider_id,
            "name": profile.name,
            "type": profile.type,
            "url": profile.url,
            "key_configured": bool(profile.key),
            "model": model,
            "timeout_seconds": profile.timeout_seconds,
        }

    def _draft_payload(
        self,
        dream_config: DreamConfig,
        provider_catalog: ProviderCatalog,
        ingest_config: IngestConfig,
        release_config: ReleaseConfig,
        selected: str,
    ) -> dict[str, object]:
        dream_payload = redacted_dream_config_payload(dream_config)
        provider_form = self._form_values(
            dream_config,
            provider_catalog,
            ingest_config,
            selected,
            release_config,
        )["provider"]
        return {
            "dream": dream_payload,
            "provider_catalog": _provider_catalog_payload(provider_catalog),
            "ingest": ingest_config.to_payload(),
            "release": _release_draft(release_config),
            "provider": provider_form,
            "dreaming": _compat_dreaming_draft(dream_config, selected),
            "providers": _compat_provider_draft(dream_config, provider_catalog, selected),
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
            dream_config = _dream_config_from_draft(dream_config, raw_dream)
        raw_workflows = draft.get("workflows")
        if type(raw_workflows) is dict:
            dream_config = _dream_config_from_draft(
                dream_config,
                {"workflows": raw_workflows},
            )
        raw_dreaming_draft = draft.get("dreaming")
        if type(raw_dreaming_draft) is dict:
            dream_config = _dream_config_from_draft(
                dream_config,
                {"dreaming": raw_dreaming_draft},
            )

        raw_dreaming = params.get("dreaming")
        if type(raw_dreaming) is dict:
            try:
                dream_config = self._apply_dreaming_form(
                    dream_config,
                    self._dreaming_form(raw_dreaming, dream_config),
                )
            except (DreamConfigError, ValueError) as error:
                return dream_config, str(error)

        raw_provider = params.get("provider")
        if type(raw_provider) is dict:
            selected = self._selected_provider(
                {key: value for key, value in params.items() if key != "provider"},
                dream_config,
                default_provider_catalog(),
            )
            workflow = dream_config.workflows.get(
                "crystallization",
                WorkflowProfile(provider=selected, model="", enabled=True),
            )
            dream_config = dream_config.with_workflow(
                "crystallization",
                replace(
                    workflow,
                    provider=selected,
                    model=str(raw_provider.get("model", workflow.model)).strip(),
                    enabled=True,
                ),
            )

        return dream_config, load_error

    def _provider_catalog_from_params(
        self,
        params: dict[str, object],
    ) -> tuple[ProviderCatalog, str]:
        try:
            provider_catalog = load_provider_catalog(self.config)
        except ProviderCatalogError as error:
            provider_catalog = default_provider_catalog()
            load_error = str(error)
        else:
            load_error = ""

        draft = params.get("draft")
        if draft is None and any(key in params for key in ("provider_catalog", "providers")):
            draft = params
        if type(draft) is dict:
            raw_provider_catalog = draft.get("provider_catalog")
            if "provider_catalog" in draft and type(raw_provider_catalog) is not dict:
                return provider_catalog, "provider_catalog must be a table"
            if type(raw_provider_catalog) is dict:
                self._clear_pending_api_keys_from_provider_draft(
                    provider_catalog,
                    raw_provider_catalog,
                )
                try:
                    provider_catalog = _provider_catalog_from_draft(
                        provider_catalog,
                        raw_provider_catalog,
                    )
                except ProviderCatalogError as error:
                    return provider_catalog, str(error)
                provider_catalog = self._apply_pending_api_keys(provider_catalog)
            elif type(draft.get("providers")) is dict:
                self._clear_pending_api_keys_from_compat_draft(provider_catalog, draft["providers"])
                try:
                    provider_catalog = _provider_catalog_from_compat_draft(
                        provider_catalog,
                        draft["providers"],
                    )
                except ProviderCatalogError as error:
                    return provider_catalog, str(error)
                provider_catalog = self._apply_pending_api_keys(provider_catalog)

        raw_provider = params.get("provider")
        explicit = params.get("selected_provider", params.get("provider_id"))
        selected = (
            self._require_provider_id(explicit)
            if explicit is not None
            else self._selected_provider(
                {key: value for key, value in params.items() if key != "provider"},
                default_dream_config(),
                provider_catalog,
            )
        )
        if type(raw_provider) is dict:
            try:
                _, provider_catalog = self._apply_provider_form(
                    default_dream_config(),
                    provider_catalog,
                    selected,
                    self._provider_form(
                        raw_provider,
                        default_dream_config(),
                        provider_catalog,
                        selected,
                    ),
                )
            except (ProviderCatalogError, ValueError) as error:
                return provider_catalog, str(error)

        return provider_catalog, load_error

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
        provider_catalog: ProviderCatalog,
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
            validate_provider_catalog(provider_catalog)
        except ProviderCatalogError as error:
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
        provider_catalog: ProviderCatalog,
    ) -> str:
        explicit = params.get("selected_provider")
        if explicit is None and type(params.get("provider")) is str:
            explicit = params.get("provider")
        if explicit is not None:
            return self._require_provider_id(explicit)
        crystallization = dream_config.workflows.get("crystallization")
        if crystallization is not None and crystallization.provider:
            return crystallization.provider
        if provider_catalog.defaults.provider:
            return provider_catalog.defaults.provider
        draft = params.get("draft")
        if type(draft) is dict:
            raw_dreaming = draft.get("dreaming")
            if type(raw_dreaming) is dict:
                provider = raw_dreaming.get("active_provider")
                if type(provider) is str and provider:
                    return provider
            raw_workflows = draft.get("workflows")
            if type(raw_workflows) is dict:
                raw_crystallization = raw_workflows.get("crystallization")
                if type(raw_crystallization) is dict:
                    provider = raw_crystallization.get("provider")
                    if type(provider) is str and provider:
                        return provider
        return "openai"

    def _require_provider_id(self, value: object) -> str:
        if type(value) is not str or not value:
            raise ValueError(f"unsupported provider profile: {value}")
        return value

    def _profile_context(
        self,
        dream_config: DreamConfig,
        provider_catalog: ProviderCatalog,
        selected: str,
    ) -> tuple[RuntimeProviderProfile, str] | None:
        catalog_profile = provider_catalog.providers.get(selected)
        profile = (
            _runtime_provider_profile(catalog_profile) if catalog_profile is not None else None
        )
        if profile is None:
            return None
        model = _model_for_profile(dream_config, selected)
        if not model and provider_catalog.defaults.provider == selected:
            model = provider_catalog.defaults.model
        return profile, model

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
        provider_catalog: ProviderCatalog,
        selected: str,
        values: dict[str, str],
    ) -> tuple[DreamConfig, ProviderCatalog]:
        provider = provider_catalog.providers.get(
            selected,
            CatalogProviderProfile(
                name=_default_profile_name(selected),
                type=_default_profile_type(selected),
                url=_default_profile_url(_default_profile_type(selected)),
            ),
        )
        endpoint = values["endpoint"].strip()
        api_key = values["api_key"].strip()
        if api_key == "***":
            api_key = provider.key
        elif api_key:
            self._pending_api_keys[selected] = api_key
        else:
            self._pending_api_keys.pop(selected, None)
        updated_provider = replace(
            provider,
            type=_default_profile_type(selected) if not provider.type else provider.type,
            url=endpoint or provider.url,
            key=api_key,
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
        return (
            dream_config.with_workflow("crystallization", updated_workflow),
            provider_catalog.with_provider(selected, updated_provider),
        )

    def _apply_pending_api_keys(self, provider_catalog: ProviderCatalog) -> ProviderCatalog:
        next_catalog = provider_catalog
        for name, api_key in self._pending_api_keys.items():
            provider = next_catalog.providers.get(name)
            if provider is not None:
                next_catalog = next_catalog.with_provider(name, replace(provider, key=api_key))
        return next_catalog

    def _clear_pending_api_keys_from_provider_draft(
        self,
        provider_catalog: ProviderCatalog,
        draft: dict[object, object],
    ) -> None:
        providers = draft.get("profiles")
        if type(providers) is not dict:
            return
        for name, raw_provider in providers.items():
            raw_api_key = raw_provider.get("key") if type(raw_provider) is dict else None
            if (
                type(name) is not str
                or type(raw_provider) is not dict
                or type(raw_api_key) is not str
            ):
                continue
            if raw_api_key == "":
                self._pending_api_keys.pop(name, None)
                continue
            profile = provider_catalog.providers.get(name)
            if (
                raw_api_key == "***"
                and name not in self._pending_api_keys
                and profile is not None
                and profile.key
            ):
                self._pending_api_keys.pop(name, None)

    def _clear_pending_api_keys_from_compat_draft(
        self,
        provider_catalog: ProviderCatalog,
        providers: dict[object, object],
    ) -> None:
        self._clear_pending_api_keys_from_provider_draft(
            provider_catalog,
            {
                "profiles": {
                    name: {"key": raw_provider.get("api_key")}
                    for name, raw_provider in providers.items()
                    if type(raw_provider) is dict
                }
            },
        )

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
        provider_catalog: ProviderCatalog,
        selected: str,
    ) -> dict[str, str]:
        provider = provider_catalog.providers.get(
            selected,
            CatalogProviderProfile(
                name=_default_profile_name(selected),
                type=_default_profile_type(selected),
                url=_default_profile_url(_default_profile_type(selected)),
            ),
        )
        values = raw if type(raw) is dict else {}
        api_path = values.get(
            "api_path",
            values.get("endpoint", values.get("base_url", provider.url)),
        )
        return {
            "model": str(values.get("model", _model_for_profile(dream_config, selected))),
            "api_key": str(values.get("api_key", provider.key)),
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
        provider_catalog: ProviderCatalog,
        ingest_config: IngestConfig,
        selected: str,
        release_config: ReleaseConfig,
    ) -> dict[str, object]:
        provider = provider_catalog.providers.get(
            selected,
            CatalogProviderProfile(
                name=_default_profile_name(selected),
                type=_default_profile_type(selected),
                url=_default_profile_url(_default_profile_type(selected)),
            ),
        )
        return {
            "provider": {
                "model": field_value(_model_for_profile(dream_config, selected)),
                "api_key": _redacted_api_key(provider),
                "api_path": field_value(provider.url),
                "timeout_seconds": field_value(provider.timeout_seconds),
            },
            "provider_catalog": {
                "defaults": provider_catalog.defaults.to_payload(),
                "profile": provider.to_payload(redact=True),
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


def _provider_catalog_payload(provider_catalog: ProviderCatalog) -> dict[str, object]:
    raw = redacted_provider_catalog_payload(provider_catalog)
    return {
        "profiles": {key: value for key, value in raw.items() if key != "defaults"},
        "defaults": raw.get("defaults", ProviderDefaults().to_payload()),
    }


def _compat_provider_payload(provider_catalog: ProviderCatalog) -> dict[str, object]:
    payload: dict[str, object] = {}
    for name in REMOTE_PROVIDERS:
        profile = provider_catalog.providers.get(
            name,
            CatalogProviderProfile(
                name=_default_profile_name(name),
                type=_default_profile_type(name),
                url=_default_profile_url(_default_profile_type(name)),
            ),
        )
        payload[name] = {
            "name": profile.name,
            "type": profile.type,
            "url": profile.url,
            "endpoint": profile.url,
            "base_url": profile.url,
            "key": _redacted_api_key(profile),
            "api_key": _redacted_api_key(profile),
            "timeout_seconds": profile.timeout_seconds,
        }
    for name, profile in provider_catalog.providers.items():
        if name in payload:
            continue
        payload[name] = _compat_provider_profile_payload(profile)
    return payload


def _compat_provider_profile_payload(profile: CatalogProviderProfile) -> dict[str, object]:
    return {
        "name": profile.name,
        "type": profile.type,
        "url": profile.url,
        "endpoint": profile.url,
        "base_url": profile.url,
        "key": _redacted_api_key(profile),
        "api_key": _redacted_api_key(profile),
        "timeout_seconds": profile.timeout_seconds,
    }


def _runtime_provider_profile(
    profile: CatalogProviderProfile | None,
) -> RuntimeProviderProfile | None:
    if profile is None:
        return None
    provider_type = "gemini" if profile.type == "google" else profile.type
    return RuntimeProviderProfile(
        type=provider_type,
        endpoint=profile.url,
        api_key=profile.key,
        timeout_seconds=profile.timeout_seconds,
    )


def _default_profile_type(profile_id: str) -> str:
    if profile_id == "gemini":
        return "google"
    if profile_id in {"anthropic", "ollama", "openai"}:
        return profile_id
    return "openai"


def _default_profile_name(profile_id: str) -> str:
    return profile_id.replace("_", " ").replace("-", " ").title()


def _default_profile_url(provider_type: str) -> str:
    if provider_type == "anthropic":
        return "https://api.anthropic.com"
    if provider_type == "google":
        return "https://generativelanguage.googleapis.com"
    if provider_type == "ollama":
        return "http://localhost:11434"
    return "https://api.openai.com/v1"


def _profile_requires_api_key(profile: CatalogProviderProfile) -> bool:
    return profile.type != "ollama"


def _redact_catalog_secret_values(text: str, provider_catalog: ProviderCatalog) -> str:
    redacted = text
    for value in sorted(configured_secret_values(provider_catalog), key=len, reverse=True):
        redacted = redacted.replace(value, "[redacted]")
    return redacted


def _result_to_json_dict(result: object) -> dict[str, object]:
    if isinstance(result, dict):
        return dict(result)
    return result.to_json_dict()


def _draft_container_errors(params: dict[str, object]) -> list[str]:
    draft = params.get("draft")
    if draft is None and any(
        key in params
        for key in ("dream", "dreaming", "provider_catalog", "providers", "ingest", "release")
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
    raw_provider_catalog = draft.get("provider_catalog")
    if "provider_catalog" in draft and type(raw_provider_catalog) is not dict:
        errors.append("provider_catalog must be a table")
    elif type(raw_provider_catalog) is dict:
        profiles = raw_provider_catalog.get("profiles")
        if "profiles" in raw_provider_catalog and type(profiles) is not dict:
            errors.append("provider_catalog.profiles must be a table")
        elif type(profiles) is dict:
            for name, raw_profile in profiles.items():
                if type(name) is str and type(raw_profile) is not dict:
                    errors.append(f"provider_catalog.profiles.{name} must be a table")
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
    required = ["dream", "ingest", "release"]
    if "provider_catalog" not in draft and "providers" not in draft:
        required.append("provider_catalog")
    missing = [key for key in required if key not in draft]
    if missing:
        if "provider_catalog" in required:
            return ["draft must include dream, provider_catalog, ingest, and release"]
        return ["draft must include dream, ingest, and release"]
    return []


def _has_complete_draft(params: dict[str, object]) -> bool:
    draft = params.get("draft")
    return (
        type(draft) is dict
        and all(key in draft for key in ("dream", "ingest", "release"))
        and ("provider_catalog" in draft or "providers" in draft)
    )


def _dream_config_from_draft(
    base: DreamConfig,
    draft: dict[object, object],
) -> DreamConfig:
    dreaming = draft.get("dreaming")
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


def _provider_catalog_from_draft(
    base: ProviderCatalog,
    draft: dict[object, object],
) -> ProviderCatalog:
    next_catalog = base
    defaults = draft.get("defaults")
    if type(defaults) is dict:
        updates = {key: defaults[key] for key in ("provider", "model") if key in defaults}
        next_catalog = replace(
            next_catalog,
            defaults=replace(next_catalog.defaults, **updates),
        )

    profiles = draft.get("profiles")
    if type(profiles) is dict:
        next_providers = dict(next_catalog.providers)
        for name, raw_profile in profiles.items():
            if type(name) is not str or type(raw_profile) is not dict:
                continue
            current = next_providers.get(
                name,
                CatalogProviderProfile(
                    name=_default_profile_name(name),
                    type=_default_profile_type(name),
                    url=_default_profile_url(_default_profile_type(name)),
                ),
            )
            updates = {
                key: raw_profile[key]
                for key in ("name", "type", "url", "timeout_seconds")
                if key in raw_profile
            }
            if "key" in raw_profile and raw_profile["key"] != "***":
                updates["key"] = raw_profile["key"]
            next_providers[name] = replace(current, **updates)
        next_catalog = replace(next_catalog, providers=next_providers)

    return next_catalog


def _provider_catalog_from_compat_draft(
    base: ProviderCatalog,
    providers: dict[object, object],
) -> ProviderCatalog:
    profiles = {}
    for name, raw_provider in providers.items():
        if type(name) is not str or type(raw_provider) is not dict:
            continue
        profiles[name] = {
            "name": raw_provider.get("name", _default_profile_name(name)),
            "type": raw_provider.get("type", _default_profile_type(name)),
            "url": raw_provider.get(
                "base_url",
                raw_provider.get(
                    "endpoint",
                    raw_provider.get(
                        "url",
                        _default_profile_url(_default_profile_type(name)),
                    ),
                ),
            ),
            "timeout_seconds": raw_provider.get("timeout_seconds", 30.0),
        }
        if "api_key" in raw_provider:
            profiles[name]["key"] = raw_provider["api_key"]
        elif "key" in raw_provider:
            profiles[name]["key"] = raw_provider["key"]
    return _provider_catalog_from_draft(base, {"profiles": profiles})


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
    provider_catalog: ProviderCatalog,
    selected: str,
) -> dict[str, dict[str, object]]:
    payload: dict[str, dict[str, object]] = {}
    names = [
        *REMOTE_PROVIDERS,
        *[name for name in provider_catalog.providers if name not in REMOTE_PROVIDERS],
    ]
    for name in names:
        profile = provider_catalog.providers.get(
            name,
            CatalogProviderProfile(
                name=_default_profile_name(name),
                type=_default_profile_type(name),
                url=_default_profile_url(_default_profile_type(name)),
            ),
        )
        payload[name] = {
            "enabled": name == selected,
            "model": _model_for_profile(dream_config, name),
            "name": profile.name,
            "type": profile.type,
            "key": _redacted_api_key(profile),
            "api_key": _redacted_api_key(profile),
            "url": profile.url,
            "endpoint": profile.url,
            "base_url": profile.url,
            "timeout_seconds": profile.timeout_seconds,
        }
    return payload


def _load_errors(*errors: str) -> list[str]:
    return [error for error in errors if error]


def _field_errors(errors: list[str], selected: str) -> dict[str, list[str]]:
    mapping: dict[str, list[str]] = {}
    for error in errors:
        field = _field_for_error(error, selected)
        if field:
            mapping.setdefault(field, []).append(error)
    return mapping


def _field_for_error(error: str, selected: str) -> str:
    selected_provider_prefix = f"providers.{selected}."
    if error.startswith(selected_provider_prefix):
        provider_field = error.removeprefix(selected_provider_prefix).split(" ", 1)[0]
        return {
            "model": "provider.model",
            "endpoint": "provider.api_path",
            "url": "provider_catalog.profile.url",
            "timeout_seconds": "provider_catalog.profile.timeout_seconds",
            "api_key": "provider_catalog.profile.key",
            "key": "provider_catalog.profile.key",
            "type": "provider_catalog.profile.type",
            "name": "provider_catalog.profile.name",
        }.get(provider_field, "")
    selected_catalog_prefix = f"provider_catalog.profiles.{selected}."
    if error.startswith(selected_catalog_prefix):
        provider_field = error.removeprefix(selected_catalog_prefix).split(" ", 1)[0]
        return {
            "url": "provider_catalog.profile.url",
            "timeout_seconds": "provider_catalog.profile.timeout_seconds",
            "key": "provider_catalog.profile.key",
            "type": "provider_catalog.profile.type",
            "name": "provider_catalog.profile.name",
        }.get(provider_field, "")
    selected_validation_prefix = f"providers.{selected}."
    if error.startswith(selected_validation_prefix):
        provider_field = error.removeprefix(selected_validation_prefix).split(" ", 1)[0]
        return {
            "url": "provider_catalog.profile.url",
            "timeout_seconds": "provider_catalog.profile.timeout_seconds",
            "key": "provider_catalog.profile.key",
            "type": "provider_catalog.profile.type",
            "name": "provider_catalog.profile.name",
        }.get(provider_field, "")
    if error.startswith("defaults.provider "):
        return "provider_catalog.defaults.provider"
    if error.startswith("defaults.model "):
        return "provider_catalog.defaults.model"
    if error == "enabled workflow must have a model: crystallization":
        return "provider.model"
    if error.startswith("autostart_enabled "):
        return "dreaming.autostart_enabled"
    if error.startswith(("schedule_interval_minutes ", "min_interval_minutes ")):
        return "dreaming.min_interval_minutes"
    if error.startswith(("min_pending_short_term_memories ", "new_short_term_memory_threshold ")):
        return "dreaming.new_short_term_memory_threshold"
    if error.startswith("short_memory.warning_sentence_count "):
        return "ingest.warning_sentence_count"
    if error.startswith("short_memory.rejection_sentence_count "):
        return "ingest.rejection_sentence_count"
    if error.startswith("learn.max_block_chars "):
        return "ingest.max_block_chars"
    if error.startswith("updates.channel "):
        return "release.update_channel"
    return ""


def _redacted_api_key(provider: CatalogProviderProfile) -> str:
    return "***" if provider.key else ""


def _release_payload(release_config: ReleaseConfig) -> dict[str, object]:
    return {
        "update_channel": release_config.update_channel,
        "update_target": release_config.update_target,
    }


def _release_draft(release_config: ReleaseConfig) -> dict[str, object]:
    return {"update_channel": release_config.update_channel}


def _redact_error(payload: dict[str, object], provider_catalog: ProviderCatalog) -> None:
    error = payload.get("error")
    if type(error) is str and error:
        payload["error"] = _redact_catalog_secret_values(error, provider_catalog)
