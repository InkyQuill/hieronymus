from __future__ import annotations

import json
import sys
from collections.abc import Callable

from hieronymus.config import HieronymusConfig
from hieronymus.dream_config import DreamConfigError, load_dream_config
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.tui_bridge.admin_api import AdminBridge
from hieronymus.tui_bridge.config_api import ConfigBridge
from hieronymus.tui_bridge.errors import error_payload
from hieronymus.tui_bridge.protocol import (
    RpcError,
    RpcRequest,
    error_response,
    parse_request,
    success_response,
)

Handler = Callable[[dict[str, object]], dict[str, object]]


def dispatch(config: HieronymusConfig, raw: dict[str, object]) -> dict[str, object]:
    normalized = _normalize_dispatch_request(raw)
    try:
        request = parse_request(json.dumps(normalized, ensure_ascii=False))
        return _dispatch_request(config, request)
    except RpcError as error:
        return error_response(_request_id(normalized), error, redact=_redactor_or_none(config))
    except Exception as error:
        return {
            "id": _request_id(normalized),
            "ok": False,
            "error": error_payload(error, redact=_redactor_or_none(config)),
        }


def run_stdio(config: HieronymusConfig) -> None:
    for line in sys.stdin:
        if not line.strip():
            continue
        response = _dispatch_line(config, line)
        sys.stdout.write(json.dumps(response, ensure_ascii=False, sort_keys=True) + "\n")
        sys.stdout.flush()


def _handlers(config: HieronymusConfig) -> dict[str, Handler]:
    admin = AdminBridge(config)
    config_bridge = ConfigBridge(config)
    return {
        "admin.bootstrap": admin.bootstrap,
        "admin.snapshot": admin.snapshot,
        "admin.filter": admin.snapshot,
        "admin.add_crystal": admin.add_crystal,
        "admin.edit_crystal": admin.edit_crystal,
        "admin.merge_crystals": admin.merge_crystals,
        "admin.split_crystal": admin.split_crystal,
        "admin.supersede_crystal": admin.supersede_crystal,
        "admin.reinforce_crystal": admin.reinforce_crystal,
        "admin.decay_crystal": admin.decay_crystal,
        "admin.deprecate_crystal": admin.deprecate_crystal,
        "admin.delete_crystal": admin.delete_crystal,
        "admin.approve_proposal": admin.approve_proposal,
        "admin.reject_proposal": admin.reject_proposal,
        "admin.provenance": admin.provenance,
        "admin.recall_reasons": admin.recall_reasons,
        "admin.run_manual_dreaming": admin.run_manual_dreaming,
        "admin.memory_contracts": admin.memory_contracts,
        "admin.config_editor": admin.config_editor,
        "admin.concept_detail": admin.concept_detail,
        "admin.add_concept": admin.add_concept,
        "admin.update_concept": admin.update_concept,
        "admin.reinforce_concept": admin.reinforce_concept,
        "admin.decay_concept": admin.decay_concept,
        "admin.rename_concept": admin.rename_concept,
        "admin.merge_concepts": admin.merge_concepts,
        "admin.archive_concept": admin.archive_concept,
        "admin.list_concept_facets": admin.list_concept_facets,
        "admin.add_concept_facet": admin.add_concept_facet,
        "admin.update_concept_facet": admin.update_concept_facet,
        "admin.set_canonical_concept_facet": admin.set_canonical_concept_facet,
        "admin.list_short_term_memories": admin.list_short_term_memories,
        "admin.remove_short_term_memory": admin.remove_short_term_memory,
        "admin.add_user_correction": admin.add_user_correction,
        "admin.dream_review": admin.dream_review,
        "config.bootstrap": config_bridge.bootstrap,
        "config.select_provider": config_bridge.select_provider,
        "config.update_draft": config_bridge.update_draft,
        "config.save": config_bridge.save,
        "config.reload": config_bridge.reload,
        "config.check_provider": config_bridge.check_provider,
        "config.model_suggestions": config_bridge.model_suggestions,
    }


def _dispatch_line(config: HieronymusConfig, line: str) -> dict[str, object]:
    try:
        request = parse_request(line)
        return _dispatch_request(config, request)
    except RpcError as error:
        return error_response(
            _request_id_from_line(line),
            error,
            redact=_redactor_or_none(config),
        )
    except Exception as error:
        return {
            "id": _request_id_from_line(line),
            "ok": False,
            "error": error_payload(error, redact=_redactor_or_none(config)),
        }


def _dispatch_request(config: HieronymusConfig, request: RpcRequest) -> dict[str, object]:
    handler = _handlers(config).get(request.method)
    if handler is None:
        error = RpcError("method_not_found", f"unknown method: {request.method}")
        return error_response(request.id, error, redact=_redactor_or_none(config))
    try:
        return success_response(request.id, handler(request.params))
    except Exception as error:
        return {
            "id": request.id,
            "ok": False,
            "error": error_payload(error, redact=_redactor_or_none(config)),
        }


def _request_id(raw: object) -> str | None:
    if type(raw) is dict and type(raw.get("id")) is str:
        return raw["id"]
    if type(raw) is dict and type(raw.get("id")) is int:
        return str(raw["id"])
    return None


def _normalize_dispatch_request(raw: dict[str, object]) -> dict[str, object]:
    request = dict(raw)
    if type(request.get("id")) is int:
        request["id"] = str(request["id"])
    return request


def _request_id_from_line(line: str) -> str | None:
    try:
        raw = json.loads(line)
    except json.JSONDecodeError:
        return None
    return _request_id(raw)


def _redactor_or_none(config: HieronymusConfig) -> Callable[[str], str] | None:
    try:
        dream_config = load_dream_config(config)
    except (DreamConfigError, OSError):
        return None
    return lambda text: redact_configured_secret_values(text, dream_config)
