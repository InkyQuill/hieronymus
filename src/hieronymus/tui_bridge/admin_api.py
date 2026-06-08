from __future__ import annotations

from hieronymus.admin import ADMIN_VIEWS, AdminStore
from hieronymus.admin_models import ActionResult, AdminDetail, AdminRow, AdminSnapshot
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.service_manager import ServiceManager
from hieronymus.tui_bridge.protocol import dataclass_to_json

DEFAULT_VIEW = "Crystals"
FilterValue = str | tuple[str, ...]
SUPPORTED_FILTERS = frozenset(
    {
        "status",
        "kind",
        "type",
        "series_slug",
        "language_pair",
        "confidence",
        "strength",
        "cycle",
        "tags",
    }
)


class AdminBridge:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        self.store = AdminStore(config)

    def bootstrap(self, params: dict[str, object]) -> dict[str, object]:
        view = _optional_string(params.get("view"), "view") or DEFAULT_VIEW
        selected_id = params.get("selected_id", params.get("id"))
        filters = _filters(params.get("filters"))
        return {
            "views": list(ADMIN_VIEWS),
            "default_view": DEFAULT_VIEW,
            "stats": dataclass_to_json(self.store.stats()),
            "service": dataclass_to_json(ServiceManager(self.config).status()),
            "snapshot": dataclass_to_json(self._snapshot(view, selected_id, filters)),
        }

    def snapshot(self, params: dict[str, object]) -> dict[str, object]:
        view = _optional_string(params.get("view"), "view") or DEFAULT_VIEW
        selected_id = params.get("selected_id", params.get("id"))
        filters = _filters(params.get("filters"))
        return self._snapshot_payload(view=view, selected_id=selected_id, filters=filters)

    def add_crystal(self, params: dict[str, object]) -> dict[str, object]:
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        crystal_id = self.store.add_crystal(
            series_slug=_required_string(params.get("series_slug"), "series_slug"),
            source_language=_required_string(params.get("source_language"), "source_language"),
            target_language=_required_string(params.get("target_language"), "target_language"),
            crystal_type=_crystal_type(params),
            title=_required_string(params.get("title"), "title"),
            text=_required_string(params.get("text"), "text"),
            tags=_string_tuple(params.get("tags"), "tags"),
        )
        result = ActionResult("crystal", crystal_id, "add", "Crystal added")
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=crystal_id,
        )

    def edit_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        result = self.store.edit_crystal(
            crystal_id,
            title=_required_string(params.get("title"), "title"),
            text=_required_string(params.get("text"), "text"),
        )
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=crystal_id,
        )

    def merge_crystals(self, params: dict[str, object]) -> dict[str, object]:
        crystal_ids = _required_int_list(params.get("ids"), "ids")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        merged_id = self.store.merge_crystals(
            crystal_ids,
            title=_required_string(params.get("title"), "title"),
            text=_required_string(params.get("text"), "text"),
        )
        result = ActionResult("crystal", merged_id, "merge", "Crystals merged")
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=merged_id,
        )

    def split_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        new_ids = self.store.split_crystal(crystal_id, parts=_split_parts(params))
        selected_id = new_ids[0] if new_ids else crystal_id
        result = ActionResult("crystal", selected_id, "split", "Crystal split")
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=selected_id,
        )

    def supersede_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        result = self.store.supersede_crystal(
            crystal_id,
            replacement_id=_required_int(params.get("replacement_id"), "replacement_id"),
            evidence=_evidence(params, default="Superseded from admin bridge"),
        )
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=crystal_id,
        )

    def reinforce_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        result = self.store.reinforce_crystal(
            crystal_id,
            evidence=_evidence(params, default="Reinforced from admin bridge"),
        )
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=crystal_id,
        )

    def decay_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        result = self.store.decay_crystal(
            crystal_id,
            evidence=_evidence(params, default="Decayed from admin bridge"),
        )
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=crystal_id,
        )

    def deprecate_crystal(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        result = self.store.deprecate_crystal(
            crystal_id,
            evidence=_evidence(params, default="Deprecated from admin bridge"),
        )
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=crystal_id,
        )

    def delete_crystal(self, params: dict[str, object]) -> dict[str, object]:
        if params.get("confirmed") is not True:
            raise ValueError("delete requires confirmation")
        crystal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view=_crystal_mutation_view(params))
        result = self.store.delete_crystal(
            crystal_id,
            evidence=_evidence(params, default="Deleted from admin bridge"),
        )
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=crystal_id,
        )

    def approve_proposal(self, params: dict[str, object]) -> dict[str, object]:
        proposal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view="Proposals")
        term_id = self.store.approve_proposal(proposal_id)
        result = ActionResult("strict_term", term_id, "approve", "Proposal approved")
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=proposal_id,
        )

    def reject_proposal(self, params: dict[str, object]) -> dict[str, object]:
        proposal_id = _required_int(params.get("id"), "id")
        view, filters = _refresh_context(params, default_view="Proposals")
        result = self.store.reject_proposal(
            proposal_id,
            evidence=_evidence(params, default="Rejected from admin bridge"),
        )
        return self._mutation_payload(
            result,
            params,
            view=view,
            filters=filters,
            selected_id=proposal_id,
        )

    def provenance(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(_aliased_param(params, "crystal_id", "id"), "crystal_id")
        return {
            "provenance": dataclass_to_json(self.store.provenance_for_crystal(crystal_id)),
            "stats": dataclass_to_json(self.store.stats()),
        }

    def recall_reasons(self, params: dict[str, object]) -> dict[str, object]:
        crystal_id = _required_int(_aliased_param(params, "crystal_id", "id"), "crystal_id")
        return {
            "reasons": dataclass_to_json(self.store.recall_reasons_for_crystal(crystal_id)),
            "stats": dataclass_to_json(self.store.stats()),
        }

    def run_manual_dreaming(self, params: dict[str, object]) -> dict[str, object]:
        view, filters = _refresh_context(params, default_view="Dream Runs")
        run = self.store.run_manual_dreaming()
        return self._mutation_payload(
            dataclass_to_json(run),
            params,
            view=view,
            filters=filters,
            selected_id=run.id,
        )

    def dream_review(self, params: dict[str, object]) -> dict[str, object]:
        run_id = _required_int(_aliased_param(params, "run_id", "id"), "run_id")
        return {
            "review": dataclass_to_json(self.store.dream_review(run_id)),
            "stats": dataclass_to_json(self.store.stats()),
            "snapshot": dataclass_to_json(self._snapshot("Dream Runs", run_id, {})),
        }

    def _mutation_payload(
        self,
        result: object,
        params: dict[str, object],
        *,
        view: str,
        filters: dict[str, FilterValue] | None = None,
        selected_id: int | str | None,
    ) -> dict[str, object]:
        filters = _filters(params.get("filters")) if filters is None else filters
        return {
            "result": dataclass_to_json(result),
            "stats": dataclass_to_json(self.store.stats()),
            "snapshot": dataclass_to_json(self._snapshot(view, selected_id, filters)),
        }

    def _snapshot_payload(
        self,
        *,
        view: str,
        selected_id: object,
        filters: dict[str, FilterValue],
    ) -> dict[str, object]:
        return {
            "stats": dataclass_to_json(self.store.stats()),
            "snapshot": dataclass_to_json(self._snapshot(view, selected_id, filters)),
        }

    def _snapshot(
        self,
        view: str,
        selected_id: object,
        filters: dict[str, FilterValue],
    ) -> AdminSnapshot:
        _validate_view_filters(view, filters)
        snapshot = self._base_snapshot(view, selected_id, filters)
        rows = _filter_rows(snapshot.rows, filters)
        selected = _select_row(rows, selected_id)
        detail = self._detail_for_filtered_row(view, selected) if selected else None
        if detail is None:
            detail = AdminDetail(title=view, subtitle="No rows", body="")
        return AdminSnapshot(
            view=snapshot.view,
            rows=rows,
            selected=selected,
            detail=detail,
            filters=_filter_labels(filters),
        )

    def _base_snapshot(
        self,
        view: str,
        selected_id: object,
        filters: dict[str, FilterValue],
    ) -> AdminSnapshot:
        if view not in {"Crystals", "Lessons"} or not filters:
            return self.store.snapshot(view, selected_id=selected_id)
        rows = self.store.list_crystals(
            series_slug=_string_filter(filters, "series_slug") or None,
            crystal_type=_crystal_filter_type(view, filters),
            status=_string_filter(filters, "status") or None,
            tags=_tuple_filter(filters, "tags"),
        )
        selected = _select_row(rows, selected_id)
        detail = self._detail_for_filtered_row(view, selected) if selected else None
        if detail is None:
            detail = AdminDetail(title=view, subtitle="No rows", body="")
        return AdminSnapshot(view=view, rows=rows, selected=selected, detail=detail)

    def _detail_for_filtered_row(
        self,
        view: str,
        selected: AdminRow | None,
    ) -> AdminDetail | None:
        if selected is None:
            return None
        if view not in {"Crystals", "Lessons"}:
            return self.store.snapshot(view, selected_id=selected.id).detail
        crystal = CrystalStore(self.config).get(_required_int(selected.id, "selected.id"))
        return AdminDetail(
            title=selected.label,
            subtitle=f"{selected.kind} / {selected.status}",
            body=crystal.text,
            fields=(
                ("Series", selected.scope),
                ("Language", selected.language_pair),
                ("Quality", selected.quality_label),
            ),
        )


def _required_int(value: object, name: str) -> int:
    if type(value) is not int:
        raise ValueError(f"{name} must be an integer")
    return value


def _required_int_list(value: object, name: str) -> list[int]:
    if type(value) is not list or not value:
        raise ValueError(f"{name} must be a non-empty list of integers")
    return [_required_int(item, f"{name} item") for item in value]


def _required_string(value: object, name: str) -> str:
    if type(value) is not str:
        raise ValueError(f"{name} must be a string")
    return value


def _aliased_param(params: dict[str, object], primary: str, alias: str) -> object:
    value = params.get(primary)
    if value is None:
        value = params.get(alias)
    return value


def _optional_string(value: object, name: str) -> str | None:
    if value is None:
        return None
    return _required_string(value, name)


def _string_tuple(value: object, name: str) -> tuple[str, ...]:
    if value is None:
        return ()
    if type(value) is str:
        return tuple(item.strip() for item in value.split(",") if item.strip())
    if type(value) not in {list, tuple}:
        raise ValueError(f"{name} must be a list of strings")
    return tuple(_required_string(item, f"{name} item") for item in value)


def _crystal_type(params: dict[str, object]) -> str:
    value = params.get("crystal_type")
    if value is None:
        value = params.get("type")
    return _required_string(value, "type")


def _crystal_mutation_view(params: dict[str, object]) -> str:
    return _optional_string(params.get("view"), "view") or DEFAULT_VIEW


def _refresh_context(
    params: dict[str, object],
    *,
    default_view: str,
) -> tuple[str, dict[str, FilterValue]]:
    view = _optional_string(params.get("view"), "view") or default_view
    filters = _filters(params.get("filters"))
    _validate_view_filters(view, filters)
    return view, filters


def _evidence(params: dict[str, object], *, default: str) -> str:
    evidence = params.get("evidence", default)
    if evidence is None or (type(evidence) is str and not evidence.strip()):
        return default
    return _required_string(evidence, "evidence")


def _filters(value: object) -> dict[str, FilterValue]:
    if value is None:
        return {}
    if type(value) is not dict:
        raise ValueError("filters must be an object")
    result: dict[str, FilterValue] = {}
    for raw_key, raw in value.items():
        key = _required_string(raw_key, "filter key")
        if key not in SUPPORTED_FILTERS:
            raise ValueError(f"unsupported admin filter: {key}")
        if raw is None or raw == "":
            continue
        if key == "tags":
            tags = _string_tuple(raw, f"filters.{key}")
            if tags:
                result[key] = tags
            continue
        result[key] = _required_string(raw, f"filters.{key}")
    return result


def _filter_rows(rows: list[AdminRow], filters: dict[str, FilterValue]) -> list[AdminRow]:
    result = rows
    if status := _string_filter(filters, "status"):
        result = [row for row in result if row.status == status]
    if kind := _string_filter(filters, "kind"):
        result = [row for row in result if row.kind == kind]
    if row_type := _string_filter(filters, "type"):
        result = [row for row in result if row.kind == row_type]
    if series_slug := _string_filter(filters, "series_slug"):
        result = [row for row in result if row.scope == series_slug]
    if language_pair := _string_filter(filters, "language_pair"):
        result = [row for row in result if row.language_pair == language_pair]
    if cycle := _string_filter(filters, "cycle"):
        result = [row for row in result if row.label == f"Cycle {cycle}"]
    if tags := filters.get("tags"):
        required_tags = set(tags)
        result = [row for row in result if required_tags.issubset(row.tags)]
    return result


def _select_row(rows: list[AdminRow], selected_id: object) -> AdminRow | None:
    if not rows:
        return None
    if selected_id is None:
        return rows[0]
    normalized_id = str(selected_id)
    return next((row for row in rows if str(row.id) == normalized_id), rows[0])


def _filter_labels(filters: dict[str, FilterValue]) -> list[str]:
    labels = []
    for key in (
        "status",
        "kind",
        "type",
        "series_slug",
        "language_pair",
        "confidence",
        "strength",
        "cycle",
        "tags",
    ):
        if key not in filters:
            continue
        value = filters[key]
        if isinstance(value, tuple):
            labels.append(f"{key}={','.join(value)}")
            continue
        labels.append(f"{key}={value}")
    return labels


def _string_filter(filters: dict[str, FilterValue], key: str) -> str:
    value = filters.get(key, "")
    return value if isinstance(value, str) else ""


def _tuple_filter(filters: dict[str, FilterValue], key: str) -> tuple[str, ...]:
    value = filters.get(key, ())
    return value if isinstance(value, tuple) else ()


def _validate_view_filters(view: str, filters: dict[str, FilterValue]) -> None:
    if view not in ADMIN_VIEWS:
        raise ValueError(f"unsupported admin view: {view}")
    if not filters:
        return
    if view not in {"Crystals", "Lessons"}:
        key = next(iter(filters))
        raise ValueError(f"unsupported admin filter for {view}: {key}")
    safe_filters = {"status", "kind", "series_slug", "tags"}
    if view == "Crystals":
        safe_filters = safe_filters | {"type"}
    for key in filters:
        if key not in safe_filters:
            raise ValueError(f"unsupported admin filter for {view}: {key}")


def _crystal_filter_type(view: str, filters: dict[str, FilterValue]) -> str | None:
    if view == "Lessons":
        return "lesson"
    return _string_filter(filters, "type") or _string_filter(filters, "kind") or None


def _split_parts(params: dict[str, object]) -> list[tuple[str, str] | dict[str, str]]:
    value = params.get("parts")
    if value is None:
        return [
            (
                _required_string(params.get("part_one_title"), "part_one_title"),
                _required_string(params.get("part_one_text"), "part_one_text"),
            ),
            (
                _required_string(params.get("part_two_title"), "part_two_title"),
                _required_string(params.get("part_two_text"), "part_two_text"),
            ),
        ]
    if type(value) is not list:
        raise ValueError("parts must be a list")
    parts: list[tuple[str, str] | dict[str, str]] = []
    for index, item in enumerate(value):
        if type(item) is dict:
            parts.append(
                {
                    "title": _required_string(item.get("title"), f"parts[{index}].title"),
                    "text": _required_string(item.get("text"), f"parts[{index}].text"),
                }
            )
            continue
        if type(item) in {list, tuple} and len(item) == 2:
            parts.append(
                (
                    _required_string(item[0], f"parts[{index}][0]"),
                    _required_string(item[1], f"parts[{index}][1]"),
                )
            )
            continue
        raise ValueError("parts items must be title/text pairs or objects")
    return parts
