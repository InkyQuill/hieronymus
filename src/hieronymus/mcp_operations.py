from __future__ import annotations

from collections.abc import Callable
from typing import TypeAlias

from hieronymus.config import HieronymusConfig
from hieronymus.service_discovery import discover_local_service

McpPayload: TypeAlias = dict[str, object] | list[dict[str, object]]
McpOperation: TypeAlias = Callable[[HieronymusConfig, dict[str, object]], McpPayload]

def _status(config: HieronymusConfig, _: dict[str, object]) -> dict[str, object]:
    return {
        "service": discover_local_service(config),
        "data_root": str(config.data_root),
        "database_path": str(config.database_path),
    }


MCP_OPERATION_HANDLERS: dict[str, McpOperation] = {"status": _status}


def _daemon_operation(
    operation: str,
) -> McpOperation:
    def invoke(config: HieronymusConfig, params: dict[str, object]) -> McpPayload:
        from hieronymus.mcp_server import invoke_daemon_operation

        return invoke_daemon_operation(config, operation, params)

    return invoke


for _operation in (
    "series_create", "series_init", "series_list", "series_set_language_tags",
    "concept_list", "concept_get", "concept_create", "concept_update", "concept_archive",
    "concept_merge", "concept_rename", "concept_facet_add", "concept_facet_update",
    "concept_facet_list", "concept_facet_set_canonical", "concept_semantic_tags_set",
    "crystal_link_concept", "crystal_story_scopes_set", "crystal_semantic_tags_set",
    "rule_crystals_list", "rule_crystal_archive", "rule_crystal_validate",
    "termbase_contract", "termbase_validate", "termbase_propose", "termbase_approve",
    "memory_search", "rag_import", "rag_search", "memory_add", "session_start",
    "session_complete", "short_term_add", "recall", "feedback", "dream",
    "concept_proposals_list",
):
    MCP_OPERATION_HANDLERS[_operation] = _daemon_operation(_operation)
