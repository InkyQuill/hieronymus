from __future__ import annotations

import json
import sqlite3
from datetime import UTC, datetime

from hieronymus.admin_models import (
    ActionResult,
    AdminCrystalEditPayload,
    AdminDetail,
    AdminDreamStatus,
    AdminRow,
    AdminShortTermStatus,
    AdminSnapshot,
    AdminStats,
    DreamReview,
    ProvenanceDetail,
)
from hieronymus.concepts import ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.dream_config import (
    DreamConfig,
    DreamConfigError,
    default_dream_config,
    load_dream_config,
    redacted_dream_config_payload,
)
from hieronymus.dream_locks import read_dream_cycle_state
from hieronymus.dream_providers import ProviderProfile as RuntimeProviderProfile
from hieronymus.dream_providers import resolve_provider
from hieronymus.dreaming import DreamRunRecord, DreamService
from hieronymus.llm_cache import dream_profile_cache_identity, load_model_cache
from hieronymus.memory_models import TranslationContext
from hieronymus.presentation import GREETING_ICON, TAGLINE, package_display_version
from hieronymus.provider_config import (
    ProviderCatalogError,
    default_provider_catalog,
    load_provider_catalog,
    redacted_provider_catalog_payload,
)
from hieronymus.service_manager import ServiceManager
from hieronymus.workspace import WorkspaceStore

ADMIN_VIEWS = (
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Memory",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Dream Audits",
    "Audit Log",
)
ADMIN_VIEW_KEYS = (
    "concepts",
    "renderings",
    "crystals",
    "lessons",
    "short_term_memory",
    "short_term_sessions",
    "dream_runs",
    "proposals",
    "dream_audits",
    "audit_log",
)
ADMIN_VIEW_LABELS = dict(zip(ADMIN_VIEW_KEYS, ADMIN_VIEWS, strict=True))
ADMIN_LABEL_VIEW_KEYS = {label: key for key, label in ADMIN_VIEW_LABELS.items()}
ADMIN_COMMANDS = (
    {
        "id": "add_memory",
        "label": "Add Memory",
        "hint": "Create a new crystal in the current memory view.",
        "key": "a",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": False,
    },
    {
        "id": "edit_memory",
        "label": "Edit Memory",
        "hint": "Edit the selected crystal or lesson text.",
        "key": "e",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "delete_selected",
        "label": "Delete Selected",
        "hint": "Delete or archive the selected row after confirmation.",
        "key": "d",
        "group": "Memory",
        "views": ("Concepts", "Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "merge_selected",
        "label": "Merge Selected",
        "hint": "Merge the selected concept or crystal into another item.",
        "key": "m",
        "group": "Memory",
        "views": ("Concepts", "Crystals"),
        "requires_selection": True,
    },
    {
        "id": "split_crystal",
        "label": "Split Crystal",
        "hint": "Split the selected crystal or lesson into two memories.",
        "key": "s",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "reinforce_crystal",
        "label": "Reinforce Crystal",
        "hint": "Increase strength/confidence for the selected crystal or lesson.",
        "key": "+",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "decay_crystal",
        "label": "Decay Crystal",
        "hint": "Decrease strength/confidence for the selected crystal or lesson.",
        "key": "-",
        "group": "Memory",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "approve_proposal",
        "label": "Approve Proposal",
        "hint": "Approve the selected compatibility proposal.",
        "key": "a",
        "group": "Proposals",
        "views": ("Proposals",),
        "requires_selection": True,
    },
    {
        "id": "reject_proposal",
        "label": "Reject Proposal",
        "hint": "Reject the selected compatibility proposal.",
        "key": "x",
        "group": "Proposals",
        "views": ("Proposals",),
        "requires_selection": True,
    },
    {
        "id": "inspect_provenance",
        "label": "Inspect Provenance",
        "hint": "Load provenance for the selected crystal or lesson.",
        "key": "p",
        "group": "Inspect",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "inspect_recall_reasons",
        "label": "Inspect Recall Reasons",
        "hint": "Load recall reason data for the selected crystal or lesson.",
        "key": "r",
        "group": "Inspect",
        "views": ("Crystals", "Lessons"),
        "requires_selection": True,
    },
    {
        "id": "run_manual_dreaming",
        "label": "Run Manual Dreaming",
        "hint": "Run dreaming manually and select the resulting dream run.",
        "key": "D",
        "group": "Dreaming",
        "views": ("Dream Runs",),
        "requires_selection": False,
    },
    {
        "id": "review_dream_output",
        "label": "Review Dream Output",
        "hint": "Load the review payload for the selected dream run.",
        "key": "enter",
        "group": "Dreaming",
        "views": ("Dream Runs",),
        "requires_selection": True,
    },
)
_ADMIN_IMMEDIATE_EVENT_DELTAS = {
    "confirmed_by_user": (0.15, 0.20),
    "contradicted_by_user": (-0.20, -0.25),
    "deleted_by_user": (-0.50, -0.35),
}


def admin_view_key(view: str) -> str:
    return ADMIN_LABEL_VIEW_KEYS.get(view, view)


def admin_view_label(view: str) -> str:
    return ADMIN_VIEW_LABELS.get(view, view)


def admin_view_options() -> list[dict[str, str]]:
    return [
        {"key": key, "label": label}
        for key, label in zip(ADMIN_VIEW_KEYS, ADMIN_VIEWS, strict=True)
    ]


def admin_command_options() -> list[dict[str, object]]:
    return [
        {
            "id": str(command["id"]),
            "label": str(command["label"]),
            "hint": str(command["hint"]),
            "key": str(command["key"]),
            "group": str(command["group"]),
            "views": list(command["views"]),
            "requires_selection": bool(command["requires_selection"]),
        }
        for command in ADMIN_COMMANDS
    ]


def _safe_dream_config(config: HieronymusConfig) -> tuple[DreamConfig, str]:
    try:
        return load_dream_config(config), ""
    except DreamConfigError as error:
        return default_dream_config(), str(error)


def _safe_provider_catalog(config: HieronymusConfig):
    try:
        return load_provider_catalog(config), ""
    except (OSError, ProviderCatalogError) as error:
        return default_provider_catalog(), str(error)


def _proposal_match_tags(proposal: sqlite3.Row, semantic_tags: tuple[str, ...]) -> tuple[str, ...]:
    values: list[str] = list(semantic_tags)
    for token in str(proposal["rationale"]).replace(",", " ").replace(".", " ").split():
        clean = token.strip().casefold()
        if clean:
            values.append(clean)
    result: list[str] = []
    seen: set[str] = set()
    for value in values:
        if value not in seen:
            result.append(value)
            seen.add(value)
    return tuple(result)


def _tag_score(candidate_tags: tuple[str, ...], wanted_tags: tuple[str, ...]) -> int:
    return len(set(candidate_tags).intersection(wanted_tags))


_ARCHIVE_STRENGTH_THRESHOLD = 0.05


class AdminStore:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def status_payload(self) -> dict[str, object]:
        return {
            "tui": "available",
            "header": self.header_status_payload(),
            "views": list(ADMIN_VIEWS),
            "view_keys": list(ADMIN_VIEW_KEYS),
            "view_labels": dict(ADMIN_VIEW_LABELS),
            "view_options": admin_view_options(),
            "counts": self.stats().as_dict(),
            "service": ServiceManager(self.config).status(),
            **self.dashboard_status_payload(),
        }

    def header_status_payload(self) -> dict[str, object]:
        return {
            "product": "Hieronymus",
            "version": package_display_version(),
            "tagline": TAGLINE,
            "logo": {
                "text": GREETING_ICON,
                "name": "feather",
                "alt": "Hieronymus feather logo",
            },
        }

    def dashboard_status_payload(self) -> dict[str, object]:
        dream_config, dream_config_error = _safe_dream_config(self.config)
        pending_count = self.pending_completed_short_term_memory_count()
        drain_progress = self._dream_drain_progress(pending_count)
        dream_status = self._dream_status(dream_config).as_dict()
        if dream_config_error:
            dream_status["reason"] = dream_config_error
        return {
            "short_term_status": self._short_term_status(
                dream_config,
                pending_count,
                drain_progress=drain_progress,
            ).as_dict(),
            "dream_status": dream_status,
            "dream_config_error": dream_config_error,
        }

    def pending_completed_short_term_memory_count(self) -> int:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                """
                select count(*)
                from short_term_memories
                join task_sessions
                  on task_sessions.id = short_term_memories.session_id
                where task_sessions.status = 'completed'
                  and short_term_memories.archived_at is null
                """
            ).fetchone()
        return int(row[0])

    def _short_term_status(
        self,
        dream_config: DreamConfig,
        pending_count: int,
        *,
        drain_progress: dict[str, int | bool | float],
    ) -> AdminShortTermStatus:
        return AdminShortTermStatus(
            pending_count=pending_count,
            min_pending_short_term_memories=dream_config.min_pending_short_term_memories,
            max_pending_short_term_memories=dream_config.max_pending_short_term_memories,
            urgent=pending_count >= dream_config.max_pending_short_term_memories,
            drain_in_progress=bool(drain_progress["in_progress"]),
            drain_completed=int(drain_progress["completed"]),
            drain_remaining=int(drain_progress["remaining"]),
            drain_total=int(drain_progress["total"]),
            drain_progress=float(drain_progress["progress"]),
        )

    def _dream_status(self, dream_config: DreamConfig) -> AdminDreamStatus:
        active_cycle = read_dream_cycle_state(self.config)
        with connect(self.config.database_path) as conn:
            running_phase = conn.execute(
                """
                select
                  p.*,
                  r.id as dream_run_id,
                  r.cycle_id as dream_cycle_id
                from dream_phase_runs as p
                join dream_runs as r
                  on r.id = p.dream_run_id
                where p.status = 'running'
                order by p.id desc
                limit 1
                """
            ).fetchone()
            run = None
            if running_phase is None:
                run = conn.execute(
                    """
                    select *
                    from dream_runs
                    where status = 'running'
                    order by id desc
                    limit 1
                    """
                ).fetchone()
            phase = running_phase
            if run is not None:
                phase = conn.execute(
                    """
                    select *
                    from dream_phase_runs
                    where dream_run_id = ?
                    order by case status when 'running' then 0 else 1 end, id desc
                    limit 1
                    """,
                    (run["id"],),
                ).fetchone()

        run_id = None
        cycle_id = None
        if running_phase is not None:
            run_id = int(running_phase["dream_run_id"])
            cycle_id = int(running_phase["dream_cycle_id"])
        elif run is not None:
            run_id = int(run["id"])
            cycle_id = int(run["cycle_id"])

        if active_cycle is None and run_id is None:
            state = "IDLE" if dream_config.enabled else "DISABLED"
            return AdminDreamStatus(state=state, current_phase="", progress=0.0)

        current_phase = "starting" if phase is None else phase["phase"]
        return AdminDreamStatus(
            state="WORKING",
            current_phase=current_phase,
            progress=self._phase_progress(current_phase),
            run_id=run_id,
            cycle_id=cycle_id,
            owner="" if active_cycle is None else active_cycle.owner,
            started_at="" if active_cycle is None else active_cycle.started_at,
        )

    def _phase_progress(self, current_phase: str) -> float:
        if current_phase == "starting":
            return 0.0
        if current_phase == "maintenance":
            return 0.9
        drain = self._dream_drain_progress(self.pending_completed_short_term_memory_count())
        if int(drain["total"]) > 0:
            return float(drain["progress"])
        return 0.5

    def _dream_drain_progress(self, pending_count: int) -> dict[str, int | bool | float]:
        with connect(self.config.database_path) as conn:
            running_phase = conn.execute(
                """
                select dream_run_id
                from dream_phase_runs
                where status = 'running'
                order by id desc
                limit 1
                """
            ).fetchone()
            run_id = None
            if running_phase is not None:
                run_id = int(running_phase["dream_run_id"])
            else:
                run = conn.execute(
                    """
                    select id
                    from dream_runs
                    where status = 'running'
                    order by id desc
                    limit 1
                    """
                ).fetchone()
                if run is not None:
                    run_id = int(run["id"])
            if run_id is None and read_dream_cycle_state(self.config) is None:
                return {
                    "in_progress": False,
                    "completed": 0,
                    "remaining": pending_count,
                    "total": pending_count,
                    "progress": 0.0,
                }
            completed_row = conn.execute(
                """
                select coalesce(sum(input_count), 0) as input_count
                from dream_phase_runs
                where dream_run_id = ?
                  and status = 'completed'
                """,
                (-1 if run_id is None else run_id,),
            ).fetchone()
        completed = int(completed_row["input_count"]) if completed_row is not None else 0
        total = completed + pending_count
        progress = 1.0 if total == 0 else completed / total
        return {
            "in_progress": True,
            "completed": completed,
            "remaining": pending_count,
            "total": total,
            "progress": round(progress, 4),
        }

    def provenance_for_crystal(self, crystal_id: int) -> ProvenanceDetail:
        with connect(self.config.database_path) as conn:
            crystal = self._get_crystal(conn, crystal_id)
            rows = conn.execute(
                """
                select
                  m.id,
                  m.session_id,
                  m.source_role,
                  m.kind,
                  m.text,
                  m.source_ref
                from crystal_sources as s
                join short_term_memories as m
                  on m.id = s.short_term_memory_id
                where s.crystal_id = ?
                order by m.id
                """,
                (crystal_id,),
            ).fetchall()
        return ProvenanceDetail(
            title=crystal["title"] or _excerpt(crystal["text"]),
            sources=[
                {
                    "id": str(row["id"]),
                    "session_id": str(row["session_id"]),
                    "source_role": row["source_role"],
                    "kind": row["kind"],
                    "text": row["text"],
                    "source_ref": row["source_ref"],
                }
                for row in rows
            ],
        )

    def recall_reasons_for_crystal(self, crystal_id: int) -> list[dict[str, str]]:
        with connect(self.config.database_path) as conn:
            self._get_crystal(conn, crystal_id)
            rows = conn.execute(
                """
                select session_id, recall_query, rank, score, reason
                from crystal_activations
                where crystal_id = ?
                order by id desc
                """,
                (crystal_id,),
            ).fetchall()
        return [
            {
                "session_id": str(row["session_id"]),
                "query": row["recall_query"],
                "rank": str(row["rank"]),
                "score": f"{float(row['score']):.3f}",
                "reason": row["reason"],
            }
            for row in rows
        ]

    def crystal_edit_payload(self, crystal_id: int) -> AdminCrystalEditPayload:
        with connect(self.config.database_path) as conn:
            row = self._get_crystal(conn, crystal_id)
        return AdminCrystalEditPayload(title=row["title"], text=row["text"])

    def add_crystal(
        self,
        *,
        series_slug: str,
        source_language: str,
        target_language: str,
        crystal_type: str,
        title: str,
        text: str,
        tags: tuple[str, ...] = (),
    ) -> int:
        context = TranslationContext(
            series_slug=series_slug,
            source_language=source_language,
            target_language=target_language,
            task_type="admin",
            tags=tags,
        )
        crystal_id = CrystalStore(self.config).add_crystal(
            context,
            crystal_type=crystal_type,
            title=title,
            text=text,
        )
        self._audit("add", "crystal", crystal_id, note="Added from admin TUI")
        return crystal_id

    def list_concepts(self) -> list[dict[str, object]]:
        return [
            self._concept_payload(record) for record in ConceptStore(self.config).list_concepts()
        ]

    def concept_detail(self, concept_id: int) -> dict[str, object]:
        concepts = ConceptStore(self.config)
        concept = concepts.get(concept_id)
        return {
            **self._concept_payload(concept),
            "facets": [self._facet_payload(facet) for facet in concepts.list_facets(concept_id)],
        }

    def add_concept(
        self,
        *,
        canonical_name: str,
        description: str = "",
        status: str = "candidate",
        confidence: float = 0.2,
        scope_type: str = "global",
        scope_key: str = "",
        semantic_tags: tuple[str, ...] = (),
    ) -> ActionResult:
        record = ConceptStore(self.config).create_concept(
            canonical_name,
            description=description,
            status=status,
            confidence=confidence,
            scope_type=scope_type,
            scope_key=scope_key,
            semantic_tags=semantic_tags,
        )
        self._audit("add", "concept", record.id, note="Added from admin contract")
        return ActionResult("concept", record.id, "add", "Concept added")

    def update_concept(
        self,
        concept_id: int,
        *,
        description: str | None = None,
        status: str | None = None,
        confidence: float | None = None,
    ) -> ActionResult:
        ConceptStore(self.config).update_concept(
            concept_id,
            description=description,
            status=status,
            confidence=confidence,
        )
        self._audit("edit", "concept", concept_id, note="Edited from admin contract")
        return ActionResult("concept", concept_id, "edit", "Concept edited")

    def reinforce_concept(self, concept_id: int, *, evidence: str) -> ActionResult:
        concepts = ConceptStore(self.config)
        record = concepts.get(concept_id)
        concepts.update_concept(concept_id, confidence=_clamp_score(record.confidence + 0.15))
        self._audit("reinforce", "concept", concept_id, note=evidence)
        return ActionResult("concept", concept_id, "reinforce", "Concept reinforced")

    def decay_concept(self, concept_id: int, *, evidence: str) -> ActionResult:
        concepts = ConceptStore(self.config)
        record = concepts.get(concept_id)
        concepts.update_concept(concept_id, confidence=_clamp_score(record.confidence - 0.15))
        self._audit("decay", "concept", concept_id, note=evidence)
        return ActionResult("concept", concept_id, "decay", "Concept decayed")

    def rename_concept(self, concept_id: int, *, canonical_name: str) -> ActionResult:
        ConceptStore(self.config).rename_concept(concept_id, canonical_name)
        self._audit("rename", "concept", concept_id, note="Renamed from admin contract")
        return ActionResult("concept", concept_id, "rename", "Concept renamed")

    def merge_concepts(
        self,
        source_concept_id: int,
        target_concept_id: int,
        *,
        reason: str,
    ) -> ActionResult:
        ConceptStore(self.config).merge_concepts(source_concept_id, target_concept_id, reason)
        self._audit("merge", "concept", source_concept_id, note=reason)
        return ActionResult("concept", source_concept_id, "merge", "Concept merged")

    def archive_concept(self, concept_id: int, *, reason: str) -> ActionResult:
        ConceptStore(self.config).archive_concept(concept_id, reason)
        self._audit("archive", "concept", concept_id, note=reason)
        return ActionResult("concept", concept_id, "archive", "Concept archived")

    def list_concept_facets(self, concept_id: int) -> list[dict[str, object]]:
        return [
            self._facet_payload(facet)
            for facet in ConceptStore(self.config).list_facets(concept_id)
        ]

    def add_concept_facet(
        self,
        concept_id: int,
        *,
        value: str,
        language: str = "",
        language_tags: tuple[str, ...] = (),
        kind: str | None = None,
        facet_type: str | None = None,
        confidence: float = 0.2,
        source_crystal_id: int | None = None,
        is_canonical: bool = False,
        story_scopes: tuple[str, ...] = (),
        semantic_tags: tuple[str, ...] = (),
    ) -> ActionResult:
        facet = ConceptStore(self.config).add_facet(
            concept_id,
            value,
            language=language,
            language_tags=language_tags,
            kind=kind,
            facet_type=facet_type,
            confidence=confidence,
            source_crystal_id=source_crystal_id,
            is_canonical=is_canonical,
            story_scopes=story_scopes,
            semantic_tags=semantic_tags,
        )
        self._audit("add", "concept_facet", facet.id, note="Added from admin contract")
        return ActionResult("concept_facet", facet.id, "add", "Concept facet added")

    def update_concept_facet(
        self,
        facet_id: int,
        *,
        value: str | None = None,
        language: str | None = None,
        language_tags: tuple[str, ...] | None = None,
        kind: str | None = None,
        facet_type: str | None = None,
        confidence: float | None = None,
        source_crystal_id: int | None = None,
        is_canonical: bool | None = None,
        story_scopes: tuple[str, ...] | None = None,
        semantic_tags: tuple[str, ...] | None = None,
    ) -> ActionResult:
        facet = ConceptStore(self.config).update_facet(
            facet_id,
            value=value,
            language=language,
            language_tags=language_tags,
            kind=kind,
            facet_type=facet_type,
            confidence=confidence,
            source_crystal_id=source_crystal_id,
            is_canonical=is_canonical,
            story_scopes=story_scopes,
            semantic_tags=semantic_tags,
        )
        self._audit("edit", "concept_facet", facet.id, note="Edited from admin contract")
        return ActionResult("concept_facet", facet.id, "edit", "Concept facet edited")

    def set_canonical_concept_facet(self, concept_id: int, facet_id: int) -> ActionResult:
        ConceptStore(self.config).set_canonical_facet(concept_id, facet_id)
        self._audit("canonical", "concept_facet", facet_id, note="Set canonical from admin")
        return ActionResult("concept_facet", facet_id, "canonical", "Canonical facet set")

    def list_short_term_memories(self, *, limit: int = 200) -> list[dict[str, object]]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select
                  m.*,
                  s.series_slug,
                  s.source_language,
                  s.target_language,
                  s.status as session_status
                from short_term_memories as m
                join task_sessions as s
                  on s.id = m.session_id
                where m.archived_at is null
                order by m.id desc
                limit ?
                """,
                (max(limit, 1),),
            ).fetchall()
        return [
            {
                "id": int(row["id"]),
                "session_id": int(row["session_id"]),
                "source_role": row["source_role"],
                "kind": row["kind"],
                "text": row["text"],
                "source_ref": row["source_ref"],
                "series_slug": row["series_slug"],
                "language_pair": _language_pair(row),
                "session_status": row["session_status"],
                "created_at": row["created_at"],
            }
            for row in rows
        ]

    def remove_short_term_memory(self, memory_id: int, *, reason: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                update short_term_memories
                set archived_at = ?
                where id = ?
                  and archived_at is null
                """,
                (self._now(), memory_id),
            )
            if cursor.rowcount == 0:
                raise KeyError(f"unknown active short-term memory: {memory_id}")
            self._audit_with_connection(
                conn,
                "remove",
                "short_term_memory",
                memory_id,
                note=reason,
            )
            conn.commit()
        return ActionResult("short_term_memory", memory_id, "remove", "Short-term memory removed")

    def close_session(self, session_id: int) -> ActionResult:
        WorkspaceStore(self.config).complete_session(session_id)
        return ActionResult("task_session", session_id, "complete", "Session closed")

    def add_user_correction(
        self,
        *,
        session_id: int,
        text: str,
        source_ref: str = "admin:user-correction",
        language_tags: tuple[str, ...] = (),
        story_scopes: tuple[str, ...] = (),
        semantic_tags: tuple[str, ...] = (),
        rule_intent: str = "correction",
    ) -> ActionResult:
        memory_id = WorkspaceStore(self.config).add_short_term_memory(
            session_id,
            source_role="user",
            kind="correction",
            text=text,
            source_ref=source_ref,
            metadata={"admin_command": "user_correction"},
            language_tags=language_tags,
            story_scopes=story_scopes,
            semantic_tags=semantic_tags,
            source_credibility="user_rule",
            rule_intent=rule_intent,
        )
        self._audit("add", "short_term_memory", memory_id, note="User correction from admin")
        return ActionResult(
            "short_term_memory",
            memory_id,
            "add_user_correction",
            "User correction recorded",
        )

    def run_manual_dreaming(self) -> DreamRunRecord:
        run = DreamService(self.config, resolve_provider(self.config)).run_all(
            owner="admin",
            ignore_minimum=True,
        )
        self._audit(
            "run",
            "dream",
            run.id,
            note=f"Manual dream run {run.cycle_id} with provider {run.provider}",
        )
        return run

    def dream_review(self, run_id: int) -> DreamReview:
        with connect(self.config.database_path) as conn:
            run = self._get_dream_run(conn, run_id)
            cycle_id = int(run["cycle_id"])
            source_sessions = [
                int(row["id"])
                for row in conn.execute(
                    """
                    select id
                    from task_sessions
                    where cycle_id = ?
                    order by id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]
            consumed_memories = [
                row["text"]
                for row in conn.execute(
                    """
                    select m.text
                    from short_term_memories as m
                    join task_sessions as s
                      on s.id = m.session_id
                    where s.cycle_id = ?
                    order by m.id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]
            created_crystals = [
                row["text"]
                for row in conn.execute(
                    """
                    select text
                    from crystals
                    where created_cycle = ?
                    order by id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]
            strict_proposals = [
                row["concept_text"]
                for row in conn.execute(
                    """
                    select concept_text
                    from strict_concept_proposals
                    where dream_run_id = ?
                    order by id
                    """,
                    (run_id,),
                ).fetchall()
            ]
            decayed_crystals = [
                row["label"]
                for row in conn.execute(
                    """
                    select coalesce(nullif(c.title, ''), c.text, m.crystal_id) as label
                    from memory_events as m
                    left join crystals as c
                      on c.id = m.crystal_id
                    where m.cycle_id = ?
                      and m.applied = 1
                      and m.event_type = 'cycle_decay'
                      and (m.strength_delta < 0 or m.confidence_delta < 0)
                    order by m.id
                    """,
                    (cycle_id,),
                ).fetchall()
            ]
            passes = [
                {
                    "name": row["phase"],
                    "status": row["status"],
                    "input_count": int(row["input_count"]),
                    "output_count": int(row["output_count"]),
                    "covered_count": (
                        int(row["output_count"]) if row["phase"] == "coverage_audit" else 0
                    ),
                }
                for row in conn.execute(
                    """
                    select phase, status, input_count, output_count
                    from dream_phase_runs
                    where dream_run_id = ?
                    order by id
                    """,
                    (run_id,),
                ).fetchall()
            ]

        failed_outputs = [run["error"]] if run["status"] == "failed" and run["error"] else []
        return DreamReview(
            run_id=run_id,
            source_sessions=source_sessions,
            consumed_memories=consumed_memories,
            created_crystals=created_crystals,
            updated_crystals=[],
            decayed_crystals=decayed_crystals,
            strict_proposals=strict_proposals,
            failed_outputs=failed_outputs,
            validation_errors=[],
            passes=passes,
        )

    def config_editor_payload(self) -> dict[str, object]:
        dream_config, dream_config_error = _safe_dream_config(self.config)
        provider_catalog, provider_config_error = _safe_provider_catalog(self.config)
        cache = load_model_cache(self.config)
        warnings: list[dict[str, str]] = []
        if provider_config_error:
            warnings.append(
                {
                    "code": "provider_config_invalid",
                    "message": provider_config_error,
                }
            )
        for workflow_name, workflow in dream_config.workflows.items():
            provider = provider_catalog.providers.get(workflow.provider)
            if provider is None:
                warnings.append(
                    {
                        "workflow": workflow_name,
                        "provider": workflow.provider,
                        "code": "provider_missing",
                        "message": "workflow provider is not configured",
                    }
                )
                continue
            entry = cache.providers.get(workflow.provider)
            runtime_provider = RuntimeProviderProfile(
                type="gemini" if provider.type == "google" else provider.type,
                endpoint=provider.url,
                api_key=provider.key,
                timeout_seconds=provider.timeout_seconds,
            )
            expected_identity = dream_profile_cache_identity(workflow.provider, runtime_provider)
            if entry is None:
                warnings.append(
                    {
                        "workflow": workflow_name,
                        "provider": workflow.provider,
                        "code": "model_cache_missing",
                        "message": "model cache has not been fetched for provider",
                    }
                )
                continue
            if entry.identity and entry.identity != expected_identity:
                warnings.append(
                    {
                        "workflow": workflow_name,
                        "provider": workflow.provider,
                        "code": "model_cache_identity_mismatch",
                        "message": "model cache was fetched for different provider settings",
                    }
                )
            if entry.is_stale():
                warnings.append(
                    {
                        "workflow": workflow_name,
                        "provider": workflow.provider,
                        "code": "model_cache_stale",
                        "message": "model cache is stale",
                    }
                )
            if workflow.model and workflow.model not in entry.models:
                warnings.append(
                    {
                        "workflow": workflow_name,
                        "provider": workflow.provider,
                        "code": "workflow_model_not_cached",
                        "message": "workflow model is not present in cached model list",
                    }
                )
            if entry.error:
                warnings.append(
                    {
                        "workflow": workflow_name,
                        "provider": workflow.provider,
                        "code": "model_cache_error",
                        "message": entry.error,
                    }
                )
        return {
            "config": redacted_dream_config_payload(dream_config),
            "config_error": dream_config_error,
            "provider_config_error": provider_config_error,
            "providers": redacted_provider_catalog_payload(provider_catalog),
            "workflows": redacted_dream_config_payload(dream_config)["workflows"],
            "prompts": {"general": dream_config.general_prompt},
            "thresholds": {
                "min_pending_short_term_memories": (dream_config.min_pending_short_term_memories),
                "max_pending_short_term_memories": (dream_config.max_pending_short_term_memories),
                "max_short_term_memories_per_cycle": (
                    dream_config.max_short_term_memories_per_cycle
                ),
                "not_enough_memories_cycle_threshold": (
                    dream_config.not_enough_memories_cycle_threshold
                ),
                "max_changed_crystals_per_cycle": dream_config.max_changed_crystals_per_cycle,
                "max_related_concepts_per_cycle": dream_config.max_related_concepts_per_cycle,
                "max_related_crystals_per_concept": (dream_config.max_related_crystals_per_concept),
                "max_total_affected_crystals": dream_config.max_total_affected_crystals,
            },
            "model_cache": cache.to_payload(),
            "model_cache_warnings": warnings,
        }

    def reinforce_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            self._record_immediate_feedback(
                conn,
                crystal_id,
                "confirmed_by_user",
                evidence=evidence,
            )
            self._audit_with_connection(conn, "reinforce", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "reinforce", "Crystal reinforced")

    def decay_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            self._record_immediate_feedback(
                conn,
                crystal_id,
                "contradicted_by_user",
                evidence=evidence,
            )
            self._audit_with_connection(conn, "decay", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "decay", "Crystal decayed")

    def edit_crystal(self, crystal_id: int, *, title: str, text: str) -> ActionResult:
        if not text.strip():
            raise ValueError("text must not be empty")
        now = self._now()
        with connect(self.config.database_path) as conn:
            before = self._get_crystal(conn, crystal_id)
            conn.execute(
                """
                update crystals
                set title = ?,
                    text = ?,
                    updated_at = ?
                where id = ?
                """,
                (title, text, now, crystal_id),
            )
            self._replace_crystal_fts(
                conn,
                crystal_id,
                old_title=before["title"],
                old_text=before["text"],
                title=title,
                text=text,
            )
            after = self._get_crystal(conn, crystal_id)
            self._audit_with_connection(
                conn,
                "edit",
                "crystal",
                crystal_id,
                before_json=self._row_json(before),
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult("crystal", crystal_id, "edit", "Crystal edited")

    def deprecate_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(conn, "deprecate", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "deprecate", "Crystal deprecated")

    def delete_crystal(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            before_json = self._row_json(self._get_crystal(conn, crystal_id))
            self._record_immediate_feedback(
                conn,
                crystal_id,
                "deleted_by_user",
                evidence=evidence,
            )
            conn.execute(
                """
                update crystals
                set status = 'archived',
                    strength = 0,
                    confidence = 0,
                    updated_at = ?
                where id = ?
                """,
                (self._now(), crystal_id),
            )
            after = self._get_crystal(conn, crystal_id)
            self._audit_with_connection(
                conn,
                "delete",
                "crystal",
                crystal_id,
                note=evidence,
                before_json=before_json,
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult("crystal", crystal_id, "delete", "Crystal deleted")

    def supersede_crystal(
        self,
        crystal_id: int,
        *,
        replacement_id: int,
        evidence: str,
    ) -> ActionResult:
        if crystal_id == replacement_id:
            raise ValueError("crystal cannot supersede itself")
        with connect(self.config.database_path) as conn:
            source = self._get_crystal(conn, crystal_id)
            replacement = self._get_crystal(conn, replacement_id)
            self._validate_active_crystals([source, replacement])
            self._validate_crystal_contexts([source, replacement])
            conn.execute(
                """
                insert or ignore into crystal_links(
                  source_crystal_id,
                  target_crystal_id,
                  link_type
                )
                values (?, ?, 'supersedes')
                """,
                (replacement_id, crystal_id),
            )
            self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(conn, "supersede", "crystal", crystal_id, note=evidence)
            conn.commit()
        return ActionResult("crystal", crystal_id, "supersede", "Crystal superseded")

    def merge_crystals(self, crystal_ids: list[int], *, title: str, text: str) -> int:
        if len(set(crystal_ids)) != len(crystal_ids):
            raise ValueError("crystal IDs must identify distinct crystals")
        if len(crystal_ids) < 2:
            raise ValueError("at least two distinct crystals are required")
        if not text.strip():
            raise ValueError("text must not be empty")
        now = self._now()
        with connect(self.config.database_path) as conn:
            rows = [self._get_crystal(conn, crystal_id) for crystal_id in crystal_ids]
            self._validate_active_crystals(rows)
            self._validate_crystal_contexts(rows)
            first = rows[0]
            cursor = conn.execute(
                """
                insert into crystals(
                  crystal_type,
                  text,
                  title,
                  scope_type,
                  scope_key,
                  series_slug,
                  source_language,
                  target_language,
                  tags_json,
                  strength,
                  confidence,
                  status,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
                """,
                (
                    first["crystal_type"],
                    text,
                    title,
                    first["scope_type"],
                    first["scope_key"],
                    first["series_slug"],
                    first["source_language"],
                    first["target_language"],
                    first["tags_json"],
                    max(float(row["strength"]) for row in rows),
                    max(float(row["confidence"]) for row in rows),
                    now,
                    now,
                ),
            )
            merged_id = int(cursor.lastrowid)
            conn.execute(
                "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                (merged_id, title, text),
            )
            for crystal_id in crystal_ids:
                conn.execute(
                    """
                    insert or ignore into crystal_links(
                      source_crystal_id,
                      target_crystal_id,
                      link_type
                    )
                    values (?, ?, 'merged_from')
                    """,
                    (merged_id, crystal_id),
                )
                self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(
                conn,
                "merge",
                "crystal",
                merged_id,
                before_json=json.dumps(crystal_ids),
                after_json=self._row_json(self._get_crystal(conn, merged_id)),
            )
            conn.commit()
        return merged_id

    def split_crystal(
        self,
        crystal_id: int,
        *,
        parts: list[tuple[str, str] | dict[str, str]],
    ) -> list[int]:
        if len(parts) < 2:
            raise ValueError("at least two parts are required")
        now = self._now()
        new_ids: list[int] = []
        with connect(self.config.database_path) as conn:
            source = self._get_crystal(conn, crystal_id)
            self._validate_active_crystals([source])
            for part in parts:
                title, text = self._split_part_title_text(part)
                if not isinstance(title, str):
                    raise ValueError("part title must be a string")
                if not isinstance(text, str):
                    raise ValueError("part text must be a string")
                if not text.strip():
                    raise ValueError("part text must not be empty")
                cursor = conn.execute(
                    """
                    insert into crystals(
                      crystal_type,
                      text,
                      title,
                      scope_type,
                      scope_key,
                      series_slug,
                      source_language,
                      target_language,
                      tags_json,
                      strength,
                      confidence,
                      status,
                      created_at,
                      updated_at
                    )
                    values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
                    """,
                    (
                        source["crystal_type"],
                        text,
                        title,
                        source["scope_type"],
                        source["scope_key"],
                        source["series_slug"],
                        source["source_language"],
                        source["target_language"],
                        source["tags_json"],
                        source["strength"],
                        source["confidence"],
                        now,
                        now,
                    ),
                )
                new_id = int(cursor.lastrowid)
                new_ids.append(new_id)
                conn.execute(
                    "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                    (new_id, title, text),
                )
                conn.execute(
                    """
                    insert or ignore into crystal_links(
                      source_crystal_id,
                      target_crystal_id,
                      link_type
                    )
                    values (?, ?, 'split_from')
                    """,
                    (new_id, crystal_id),
                )
            self._set_crystal_status(conn, crystal_id, "archived")
            self._audit_with_connection(
                conn,
                "split",
                "crystal",
                crystal_id,
                before_json=self._row_json(source),
                after_json=json.dumps(new_ids),
            )
            conn.commit()
        return new_ids

    def promote_local_lesson(self, crystal_id: int, *, evidence: str) -> int:
        now = self._now()
        with connect(self.config.database_path) as conn:
            source = self._get_crystal(conn, crystal_id)
            self._validate_promotable_lesson(source)
            cursor = conn.execute(
                """
                insert into crystals(
                  crystal_type,
                  text,
                  title,
                  scope_type,
                  scope_key,
                  series_slug,
                  source_language,
                  target_language,
                  tags_json,
                  strength,
                  confidence,
                  status,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, 'global', 'global', '', ?, ?, ?, ?, ?, 'candidate', ?, ?)
                """,
                (
                    source["crystal_type"],
                    source["text"],
                    source["title"],
                    source["source_language"],
                    source["target_language"],
                    source["tags_json"],
                    source["strength"],
                    source["confidence"],
                    now,
                    now,
                ),
            )
            promoted_id = int(cursor.lastrowid)
            conn.execute(
                "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
                (promoted_id, source["title"], source["text"]),
            )
            self._audit_with_connection(
                conn,
                "promote",
                "crystal",
                promoted_id,
                note=evidence,
                before_json=self._row_json(source),
                after_json=self._row_json(self._get_crystal(conn, promoted_id)),
            )
            conn.commit()
        return promoted_id

    def activate_global_lesson(self, crystal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            before = self._get_crystal(conn, crystal_id)
            self._validate_activatable_global_lesson(before)
            self._set_crystal_status(conn, crystal_id, "active")
            after = self._get_crystal(conn, crystal_id)
            self._audit_with_connection(
                conn,
                "activate",
                "crystal",
                crystal_id,
                note=evidence,
                before_json=self._row_json(before),
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult("crystal", crystal_id, "activate", "Global lesson activated")

    def approve_proposal(self, proposal_id: int) -> int:
        now = self._now()
        with connect(self.config.database_path) as conn:
            proposal = self._get_proposal(conn, proposal_id)
            if proposal["status"] != "pending":
                raise ValueError("proposal must be pending")
            concept_id = self._approve_advisory_concept_proposal(conn, proposal, now=now)
            conn.execute(
                """
                update strict_concept_proposals
                set status = 'approved',
                    updated_at = ?
                where id = ?
                """,
                (now, proposal_id),
            )
            self._audit_with_connection(
                conn,
                "approve",
                "strict_concept_proposal",
                proposal_id,
                before_json=self._row_json(proposal),
                after_json=json.dumps({"concept_id": concept_id}, sort_keys=True),
            )
            conn.commit()
        return concept_id

    def reject_proposal(self, proposal_id: int, *, evidence: str) -> ActionResult:
        with connect(self.config.database_path) as conn:
            before = self._get_proposal(conn, proposal_id)
            if before["status"] != "pending":
                raise ValueError("proposal must be pending")
            conn.execute(
                """
                update strict_concept_proposals
                set status = 'rejected',
                    updated_at = ?
                where id = ?
                """,
                (self._now(), proposal_id),
            )
            after = self._get_proposal(conn, proposal_id)
            self._audit_with_connection(
                conn,
                "reject",
                "strict_concept_proposal",
                proposal_id,
                note=evidence,
                before_json=self._row_json(before),
                after_json=self._row_json(after),
            )
            conn.commit()
        return ActionResult(
            "strict_concept_proposal",
            proposal_id,
            "reject",
            "Proposal rejected",
        )

    def stats(self) -> AdminStats:
        with connect(self.config.database_path) as conn:
            return AdminStats(
                series=self._count(conn, "series"),
                crystals=self._count(conn, "crystals"),
                lessons=self._count(
                    conn,
                    "crystals",
                    "where crystal_type = 'lesson'",
                ),
                short_term_memories=self._count(
                    conn,
                    "short_term_memories",
                    "where archived_at is null",
                ),
                sessions=self._count(conn, "task_sessions"),
                dream_runs=self._count(conn, "dream_runs"),
                pending_proposals=self._count(
                    conn,
                    "strict_concept_proposals",
                    "where status = 'pending'",
                ),
                audit_events=self._count_audit_events(conn),
            )

    def list_crystals(
        self,
        *,
        series_slug: str | None = None,
        crystal_type: str | None = None,
        status: str | None = None,
        tags: tuple[str, ...] = (),
        limit: int = 200,
    ) -> list[AdminRow]:
        clauses = []
        params: list[object] = []
        if series_slug is not None:
            clauses.append("series_slug = ?")
            params.append(series_slug)
        if crystal_type is not None:
            clauses.append("crystal_type = ?")
            params.append(crystal_type)
        if status is not None:
            clauses.append("status = ?")
            params.append(status)

        where_sql = f"where {' and '.join(clauses)}" if clauses else ""
        bounded_limit = max(limit, 1)
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                f"""
                select *
                from crystals
                {where_sql}
                order by id
                """,
                params,
            ).fetchall()

        required_tags = set(tags)
        admin_rows = []
        for row in rows:
            row_tags = _json_tuple(row["tags_json"])
            if required_tags and not required_tags.issubset(row_tags):
                continue
            admin_rows.append(self._crystal_row(row, row_tags))
            if len(admin_rows) >= bounded_limit:
                break
        return admin_rows

    def snapshot(self, view: str, selected_id: int | str | None = None) -> AdminSnapshot:
        view = admin_view_label(view)
        if view not in ADMIN_VIEWS:
            raise ValueError(f"unknown admin view: {view}")

        rows = self._rows_for_view(view)
        selected = self._select_row(rows, selected_id)
        detail = self._detail_for_view(view, selected)
        return AdminSnapshot(
            view=view,
            rows=rows,
            selected=selected,
            detail=detail,
            filters=[],
        )

    def _rows_for_view(self, view: str) -> list[AdminRow]:
        if view == "Concepts":
            return self._list_concept_rows()
        if view == "Renderings":
            return self._list_strict_terms(label_column="canonical_translation")
        if view == "Crystals":
            return self.list_crystals()
        if view == "Lessons":
            return self.list_crystals(crystal_type="lesson")
        if view == "Short-Term Memory":
            return self._list_short_term_memory_rows()
        if view == "Short-Term Sessions":
            return self._list_sessions()
        if view == "Dream Runs":
            return self._list_dream_runs()
        if view == "Dream Audits":
            return self._list_dream_audits()
        if view == "Proposals":
            return self._list_proposals()
        if view == "Audit Log":
            return self._list_audit_log()
        raise ValueError(f"unknown admin view: {view}")

    def _list_concept_rows(self) -> list[AdminRow]:
        return [
            AdminRow(
                id=record.id,
                kind=record.scope_type,
                label=record.canonical_name,
                status=record.status,
                scope=record.scope_key or record.scope_type,
                language_pair="",
                quality_label=f"{_percent(record.confidence)} conf",
                tags=record.tags,
            )
            for record in ConceptStore(self.config).list_concepts()
        ][:200]

    def _list_strict_terms(self, *, label_column: str) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from strict_terms
                order by id
                limit 200
                """
            ).fetchall()
            result = []
            for row in rows:
                tag_rows = conn.execute(
                    """
                    select tag
                    from strict_term_tags
                    where term_id = ?
                    order by tag
                    """,
                    (row["id"],),
                ).fetchall()
                result.append(
                    AdminRow(
                        id=int(row["id"]),
                        kind=row["category"],
                        label=row[label_column],
                        status=row["status"],
                        scope=row["series_slug"],
                        language_pair=_language_pair(row),
                        tags=tuple(tag["tag"] for tag in tag_rows),
                    )
                )
        return result

    def _list_sessions(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from task_sessions
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind=row["task_type"],
                label=_session_label(row),
                status=row["status"],
                scope=row["series_slug"],
                language_pair=_language_pair(row),
            )
            for row in rows
        ]

    def _list_short_term_memory_rows(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select
                  m.*, s.series_slug, s.source_language, s.target_language,
                  s.status as session_status
                from short_term_memories as m
                join task_sessions as s on s.id = m.session_id
                where m.archived_at is null
                order by m.id desc
                limit 500
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind=row["kind"],
                label=_excerpt(row["text"]),
                status=row["session_status"],
                scope=f"session:{row['session_id']}",
                language_pair=_language_pair(row),
                quality_label=row["source_role"],
            )
            for row in rows
        ]

    def _list_dream_runs(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from dream_runs
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind=row["provider"],
                label=f"Cycle {row['cycle_id']}",
                status=row["status"],
                scope="global",
                language_pair="",
                quality_label=(
                    f"{row['created_crystal_count']} crystals / {row['proposal_count']} proposals"
                ),
            )
            for row in rows
        ]

    def _list_dream_audits(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from dream_audit_entries
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind="dream audit",
                label=f"{row['event_type']}: {row['summary']}",
                status=row["severity"],
                scope=f"dream:{row['dream_run_id']}",
                language_pair="",
                quality_label=row["created_at"],
            )
            for row in rows
        ]

    def _list_proposals(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select *
                from strict_concept_proposals
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind="strict concept",
                label=row["concept_text"],
                status=row["status"],
                scope=row["series_slug"],
                language_pair=_language_pair(row),
                quality_label=row["canonical_rendering"],
            )
            for row in rows
        ]

    def _list_audit_log(self) -> list[AdminRow]:
        with connect(self.config.database_path) as conn:
            audit_table = self._audit_log_table(conn)
            if audit_table != "memory_events":
                rows = conn.execute(
                    f"""
                    select *
                    from {audit_table}
                    order by id desc
                    limit 200
                    """
                ).fetchall()
                return [
                    AdminRow(
                        id=int(row["id"]),
                        kind=_row_value(row, "action", _row_value(row, "event_type", "event")),
                        label=_row_value(row, "note", _row_value(row, "message", "")),
                        status=_row_value(row, "entity_type", _row_value(row, "status", "")),
                        scope=_row_value(
                            row, "entity_id", _row_value(row, "series_slug", "global")
                        ),
                        language_pair="",
                        quality_label=_row_value(row, "created_at", ""),
                    )
                    for row in rows
                ]
            rows = conn.execute(
                """
                select *
                from memory_events
                order by id desc
                limit 200
                """
            ).fetchall()
        return [
            AdminRow(
                id=int(row["id"]),
                kind=row["event_type"],
                label=row["evidence"] or row["source_role"],
                status="applied" if row["applied"] else "pending",
                scope=f"session:{row['session_id']}" if row["session_id"] is not None else "global",
                language_pair="",
                quality_label=_event_quality(row),
            )
            for row in rows
        ]

    def _crystal_row(self, row: sqlite3.Row, tags: tuple[str, ...]) -> AdminRow:
        return AdminRow(
            id=int(row["id"]),
            kind=row["crystal_type"],
            label=row["title"] or _excerpt(row["text"]),
            status=row["status"],
            scope=row["series_slug"] or row["scope_key"] or row["scope_type"],
            language_pair=_language_pair(row),
            quality_label=(f"{_percent(row['confidence'])} conf / {_percent(row['strength'])} str"),
            tags=tags,
        )

    def _concept_payload(self, record) -> dict[str, object]:
        return {
            "id": record.id,
            "canonical_name": record.canonical_name,
            "description": record.description,
            "status": record.status,
            "confidence": record.confidence,
            "scope_type": record.scope_type,
            "scope_key": record.scope_key,
            "tags": list(record.tags),
            "merged_into_concept_id": record.merged_into_concept_id,
        }

    def _facet_payload(self, record) -> dict[str, object]:
        return {
            "id": record.id,
            "concept_id": record.concept_id,
            "language": record.language,
            "kind": record.kind,
            "facet_type": record.facet_type,
            "value": record.value,
            "confidence": record.confidence,
            "source_crystal_id": record.source_crystal_id,
            "language_tags": list(record.language_tags),
            "story_scopes": list(record.story_scopes),
            "semantic_tags": list(record.semantic_tags),
            "is_canonical": record.is_canonical,
        }

    def _select_row(
        self,
        rows: list[AdminRow],
        selected_id: int | str | None,
    ) -> AdminRow | None:
        if not rows:
            return None
        if selected_id is None:
            return rows[0]
        normalized_id = str(selected_id)
        return next((row for row in rows if str(row.id) == normalized_id), rows[0])

    def _detail_for_view(self, view: str, selected: AdminRow | None) -> AdminDetail:
        if selected is None:
            return AdminDetail(title=view, subtitle="No rows", body="")
        if view in {"Crystals", "Lessons"}:
            return self._crystal_detail(int(selected.id))
        if view == "Concepts":
            return self._concept_detail(int(selected.id))
        if view == "Renderings":
            return self._strict_term_detail(int(selected.id))
        if view == "Short-Term Memory":
            return self._short_term_memory_detail(int(selected.id))
        if view == "Short-Term Sessions":
            return self._session_detail(int(selected.id))
        if view == "Dream Runs":
            return self._dream_run_detail(int(selected.id))
        if view == "Dream Audits":
            return self._dream_audit_detail(int(selected.id))
        if view == "Proposals":
            return self._proposal_detail(int(selected.id))
        if view == "Audit Log":
            return AdminDetail(
                title=selected.label or selected.kind,
                subtitle=selected.status,
                body=selected.quality_label,
                fields=_row_fields(selected),
            )
        raise ValueError(f"unknown admin view: {view}")

    def _crystal_detail(self, crystal_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing crystal", subtitle="", body="")
        label = row["title"] or _excerpt(row["text"])
        body = f"{label}\n\n{row['text']}" if label else row["text"]
        return AdminDetail(
            title=label,
            subtitle=f"{row['crystal_type']} / {row['status']}",
            body=body,
            fields=(
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
                (
                    "Quality",
                    f"{_percent(row['confidence'])} conf / {_percent(row['strength'])} str",
                ),
            ),
        )

    def _concept_detail(self, concept_id: int) -> AdminDetail:
        detail = self.concept_detail(concept_id)
        facets = detail["facets"]
        facet_lines = [
            f"{facet['kind']} [{','.join(facet['language_tags'])}]: {facet['value']}"
            for facet in facets
        ]
        return AdminDetail(
            title=str(detail["canonical_name"]),
            subtitle=f"{detail['scope_type']} / {detail['status']}",
            body="\n".join(facet_lines) or str(detail["description"]),
            fields=(
                ("Description", str(detail["description"])),
                ("Scope", str(detail["scope_key"] or detail["scope_type"])),
                ("Confidence", _percent(float(detail["confidence"]))),
                ("Facets", str(len(facets))),
            ),
        )

    def _strict_term_detail(self, term_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from strict_terms where id = ?", (term_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing term", subtitle="", body="")
        return AdminDetail(
            title=row["source_text"],
            subtitle=f"{row['category']} / {row['status']}",
            body=row["notes"],
            fields=(
                ("Rendering", row["canonical_translation"]),
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
            ),
        )

    def _session_detail(self, session_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from task_sessions where id = ?", (session_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing session", subtitle="", body="")
        return AdminDetail(
            title=_session_label(row),
            subtitle=f"{row['task_type']} / {row['status']}",
            body="",
            fields=(
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
                ("Cycle", str(row["cycle_id"] or "")),
            ),
        )

    def _short_term_memory_detail(self, memory_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                """
                select
                  m.*, s.series_slug, s.source_language, s.target_language,
                  s.status as session_status
                from short_term_memories as m
                join task_sessions as s on s.id = m.session_id
                where m.id = ? and m.archived_at is null
                """,
                (memory_id,),
            ).fetchone()
        if row is None:
            return AdminDetail(title="Missing short-term memory", subtitle="", body="")
        return AdminDetail(
            title=_excerpt(row["text"]),
            subtitle=f"{row['kind']} / session {row['session_status']}",
            body=row["text"],
            fields=(
                ("Session", str(row["session_id"])),
                ("Source role", row["source_role"]),
                ("Source reference", row["source_ref"]),
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
                ("Created", row["created_at"]),
            ),
        )

    def _dream_run_detail(self, run_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute("select * from dream_runs where id = ?", (run_id,)).fetchone()
        if row is None:
            return AdminDetail(title="Missing dream run", subtitle="", body="")
        return AdminDetail(
            title=f"Cycle {row['cycle_id']}",
            subtitle=f"{row['provider']} / {row['status']}",
            body=row["error"],
            fields=(
                ("Inputs", str(row["input_count"])),
                ("Crystals", str(row["created_crystal_count"])),
                ("Proposals", str(row["proposal_count"])),
            ),
        )

    def _dream_audit_detail(self, audit_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from dream_audit_entries where id = ?",
                (audit_id,),
            ).fetchone()
        if row is None:
            return AdminDetail(title="Missing dream audit", subtitle="", body="")
        try:
            payload = json.loads(row["payload_json"])
        except json.JSONDecodeError:
            payload = {"_invalid_json": row["payload_json"]}
        except Exception as error:
            payload = {
                "_invalid_json": row["payload_json"],
                "_error": str(error),
            }
        return AdminDetail(
            title=f"{row['event_type']}: {row['summary']}",
            subtitle=row["severity"],
            body=json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True),
            fields=(
                ("Dream run", str(row["dream_run_id"])),
                ("Phase run", "" if row["phase_run_id"] is None else str(row["phase_run_id"])),
                ("Severity", row["severity"]),
                ("Created", row["created_at"]),
            ),
        )

    def _proposal_detail(self, proposal_id: int) -> AdminDetail:
        with connect(self.config.database_path) as conn:
            row = conn.execute(
                "select * from strict_concept_proposals where id = ?",
                (proposal_id,),
            ).fetchone()
        if row is None:
            return AdminDetail(title="Missing proposal", subtitle="", body="")
        return AdminDetail(
            title=row["concept_text"],
            subtitle=f"strict concept / {row['status']}",
            body=row["rationale"],
            fields=(
                ("Source form", row["source_form"]),
                ("Rendering", row["canonical_rendering"]),
                ("Series", row["series_slug"]),
                ("Language", _language_pair(row)),
            ),
        )

    def _set_crystal_status(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        status: str,
    ) -> None:
        cursor = conn.execute(
            """
            update crystals
            set status = ?,
                updated_at = ?
            where id = ?
            """,
            (status, self._now(), crystal_id),
        )
        if cursor.rowcount == 0:
            raise KeyError(f"unknown crystal: {crystal_id}")

    def _record_immediate_feedback(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        event_type: str,
        *,
        evidence: str,
    ) -> int:
        strength_delta, confidence_delta = _ADMIN_IMMEDIATE_EVENT_DELTAS[event_type]
        now = self._now()
        crystal = self._get_crystal(conn, crystal_id)
        cursor = conn.execute(
            """
            insert into memory_events(
              crystal_id,
              session_id,
              event_type,
              source_role,
              evidence,
              strength_delta,
              confidence_delta,
              applied,
              created_at
            )
            values (?, null, ?, 'user', ?, ?, ?, 1, ?)
            """,
            (
                crystal_id,
                event_type,
                evidence,
                strength_delta,
                confidence_delta,
                now,
            ),
        )
        strength = _clamp_score(float(crystal["strength"]) + strength_delta)
        confidence = _clamp_score(float(crystal["confidence"]) + confidence_delta)
        status = crystal["status"]
        if event_type == "deleted_by_user" and strength < _ARCHIVE_STRENGTH_THRESHOLD:
            status = "archived"
        conn.execute(
            """
            update crystals
            set strength = ?,
                confidence = ?,
                status = ?,
                updated_at = ?
            where id = ?
            """,
            (strength, confidence, status, now, crystal_id),
        )
        return int(cursor.lastrowid)

    def _validate_crystal_contexts(self, rows: list[sqlite3.Row]) -> None:
        if len(rows) < 2:
            return
        first = rows[0]
        for row in rows[1:]:
            for column in (
                "series_slug",
                "source_language",
                "target_language",
                "crystal_type",
                "scope_type",
                "scope_key",
            ):
                if row[column] != first[column]:
                    raise ValueError(f"crystal {column} does not match")

    def _validate_active_crystals(self, rows: list[sqlite3.Row]) -> None:
        for row in rows:
            if row["status"] not in {"active", "candidate"}:
                raise ValueError("crystals must be active or candidate")

    def _validate_promotable_lesson(self, row: sqlite3.Row) -> None:
        if row["crystal_type"] != "lesson":
            raise ValueError("source crystal must be a lesson")
        if row["scope_type"] != "series":
            raise ValueError("source lesson must be series-scoped")
        if row["status"] in {"archived", "rejected"}:
            raise ValueError("source lesson must not be archived or rejected")

    def _validate_activatable_global_lesson(self, row: sqlite3.Row) -> None:
        if row["crystal_type"] != "lesson":
            raise ValueError("crystal must be a lesson")
        if row["scope_type"] != "global":
            raise ValueError("lesson must be global-scoped")
        if row["status"] != "candidate":
            raise ValueError("global lesson must be candidate")

    def _split_part_title_text(
        self,
        part: tuple[str, str] | dict[str, str],
    ) -> tuple[object, object]:
        if isinstance(part, tuple):
            if len(part) != 2:
                raise ValueError("part tuple must contain title and text")
            return part
        if isinstance(part, dict):
            return part.get("title", ""), part.get("text", "")
        raise ValueError("part must be a title/text tuple or dict")

    def _audit(
        self,
        action: str,
        entity_type: str,
        entity_id: int | str,
        *,
        note: str = "",
        before_json: str = "{}",
        after_json: str = "{}",
    ) -> None:
        with connect(self.config.database_path) as conn:
            self._audit_with_connection(
                conn,
                action,
                entity_type,
                entity_id,
                note=note,
                before_json=before_json,
                after_json=after_json,
            )
            conn.commit()

    def _audit_with_connection(
        self,
        conn: sqlite3.Connection,
        action: str,
        entity_type: str,
        entity_id: int | str,
        *,
        note: str = "",
        before_json: str = "{}",
        after_json: str = "{}",
    ) -> None:
        conn.execute(
            """
            insert into audit_log(
              action,
              entity_type,
              entity_id,
              note,
              before_json,
              after_json,
              created_at
            )
            values (?, ?, ?, ?, ?, ?, ?)
            """,
            (
                action,
                entity_type,
                str(entity_id),
                note,
                before_json,
                after_json,
                self._now(),
            ),
        )

    def _row_json(self, row: sqlite3.Row) -> str:
        return json.dumps(
            {key: row[key] for key in row.keys()},
            ensure_ascii=False,
            sort_keys=True,
        )

    def _get_crystal(self, conn: sqlite3.Connection, crystal_id: int) -> sqlite3.Row:
        row = conn.execute("select * from crystals where id = ?", (crystal_id,)).fetchone()
        if row is None:
            raise KeyError(f"unknown crystal: {crystal_id}")
        return row

    def _get_proposal(self, conn: sqlite3.Connection, proposal_id: int) -> sqlite3.Row:
        row = conn.execute(
            "select * from strict_concept_proposals where id = ?",
            (proposal_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"unknown concept proposal: {proposal_id}")
        return row

    def _get_dream_run(self, conn: sqlite3.Connection, run_id: int) -> sqlite3.Row:
        row = conn.execute("select * from dream_runs where id = ?", (run_id,)).fetchone()
        if row is None:
            raise KeyError(f"unknown dream run: {run_id}")
        return row

    def _replace_crystal_fts(
        self,
        conn: sqlite3.Connection,
        crystal_id: int,
        *,
        old_title: str,
        old_text: str,
        title: str,
        text: str,
    ) -> None:
        conn.execute(
            "insert into crystals_fts(crystals_fts, rowid, title, text) values ('delete', ?, ?, ?)",
            (crystal_id, old_title, old_text),
        )
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
            (crystal_id, title, text),
        )

    def _approve_advisory_concept_proposal(
        self,
        conn: sqlite3.Connection,
        proposal: sqlite3.Row,
        *,
        now: str,
    ) -> int:
        source_form = proposal["source_form"].strip() or proposal["concept_text"]
        concept_id = self._ensure_concept_for_proposal(
            conn,
            proposal,
            source_form=source_form,
            semantic_tags=("concept-proposal",),
            now=now,
        )
        renderings = [
            proposal["canonical_rendering"],
            *json.loads(proposal["approved_variants_json"]),
            *json.loads(proposal["forbidden_variants_json"]),
        ]
        for rendering in renderings:
            if not isinstance(rendering, str) or not rendering.strip():
                continue
            self._ensure_concept_facet(
                conn,
                concept_id=concept_id,
                value=rendering.strip(),
                facet_type="rendering",
                language_tag=proposal["target_language"],
                is_canonical=False,
                now=now,
            )
        return concept_id

    def _ensure_concept_for_proposal(
        self,
        conn: sqlite3.Connection,
        proposal: sqlite3.Row,
        *,
        source_form: str,
        semantic_tags: tuple[str, ...],
        now: str,
    ) -> int:
        scope_key = f"series:{proposal['series_slug']}"
        concept_id = self._matching_concept_id_for_proposal(
            conn,
            proposal,
            semantic_tags=semantic_tags,
            scope_key=scope_key,
        )
        if concept_id is None:
            cursor = conn.execute(
                """
                insert into concepts(
                  canonical_name,
                  description,
                  scope_type,
                  scope_key,
                  status,
                  confidence,
                  created_at,
                  updated_at
                )
                values (?, ?, 'series', ?, 'established', 0.95, ?, ?)
                """,
                (
                    proposal["concept_text"],
                    proposal["rationale"],
                    scope_key,
                    now,
                    now,
                ),
            )
            concept_id = int(cursor.lastrowid)

        for tag in semantic_tags:
            conn.execute(
                """
                insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                values (?, ?, 0.95, ?)
                on conflict(concept_id, tag) do update set
                  confidence = max(concept_semantic_tags.confidence, excluded.confidence)
                """,
                (concept_id, tag, now),
            )
        self._ensure_concept_facet(
            conn,
            concept_id=concept_id,
            value=source_form,
            facet_type="name",
            language_tag=proposal["source_language"],
            is_canonical=True,
            now=now,
        )
        self._ensure_concept_facet(
            conn,
            concept_id=concept_id,
            value=proposal["canonical_rendering"],
            facet_type="rendering",
            language_tag=proposal["target_language"],
            is_canonical=False,
            now=now,
        )
        return concept_id

    def _matching_concept_id_for_proposal(
        self,
        conn: sqlite3.Connection,
        proposal: sqlite3.Row,
        *,
        semantic_tags: tuple[str, ...],
        scope_key: str,
    ) -> int | None:
        candidate_rows = conn.execute(
            """
            select id
            from concepts
            where canonical_name = ?
              and scope_type = 'series'
              and scope_key = ?
              and status not in ('archived', 'merged')
            order by id
            """,
            (proposal["concept_text"], scope_key),
        ).fetchall()
        if not candidate_rows:
            return None
        if len(candidate_rows) == 1:
            return int(candidate_rows[0]["id"])

        wanted_tags = _proposal_match_tags(proposal, semantic_tags)
        scored = [
            (
                _tag_score(
                    self._concept_semantic_tags(conn, int(row["id"])),
                    wanted_tags,
                ),
                int(row["id"]),
            )
            for row in candidate_rows
        ]
        best_score = max(score for score, _ in scored)
        if best_score <= 0:
            return None
        best_ids = [concept_id for score, concept_id in scored if score == best_score]
        if len(best_ids) != 1:
            return None
        return best_ids[0]

    def _concept_semantic_tags(
        self,
        conn: sqlite3.Connection,
        concept_id: int,
    ) -> tuple[str, ...]:
        rows = conn.execute(
            """
            select tag
            from concept_semantic_tags
            where concept_id = ?
            order by tag
            """,
            (concept_id,),
        ).fetchall()
        return tuple(row["tag"] for row in rows)

    def _ensure_concept_facet(
        self,
        conn: sqlite3.Connection,
        *,
        concept_id: int,
        value: str,
        facet_type: str,
        language_tag: str,
        is_canonical: bool,
        now: str,
    ) -> None:
        existing = conn.execute(
            """
            select id, is_canonical, confidence
            from concept_facets
            where concept_id = ?
              and value = ?
              and facet_type = ?
              and superseded_at is null
            order by id
            limit 1
            """,
            (concept_id, value, facet_type),
        ).fetchone()
        if existing is None:
            cursor = conn.execute(
                """
                insert into concept_facets(
                  concept_id,
                  language,
                  facet_type,
                  value,
                  source_crystal_id,
                  confidence,
                  is_canonical,
                  created_at,
                  updated_at
                )
                values (?, ?, ?, ?, null, 0.95, ?, ?, ?)
                """,
                (
                    concept_id,
                    language_tag,
                    facet_type,
                    value,
                    int(is_canonical),
                    now,
                    now,
                ),
            )
            facet_id = int(cursor.lastrowid)
        else:
            facet_id = int(existing["id"])
            current_is_canonical = bool(existing["is_canonical"])
            current_confidence = float(existing["confidence"])
            next_is_canonical = current_is_canonical or is_canonical
            next_confidence = max(current_confidence, 0.95)
            if next_is_canonical != current_is_canonical or next_confidence > current_confidence:
                conn.execute(
                    """
                    update concept_facets
                    set is_canonical = ?,
                        confidence = ?,
                        updated_at = ?
                    where id = ?
                    """,
                    (int(next_is_canonical), next_confidence, now, facet_id),
                )

        if language_tag:
            conn.execute(
                """
                insert or ignore into concept_facet_language_tags(facet_id, language_tag)
                values (?, ?)
                """,
                (facet_id, language_tag.casefold()),
            )

    def _insert_alias(
        self,
        conn: sqlite3.Connection,
        term_id: int,
        *,
        language: str,
        text: str,
        kind: str,
    ) -> None:
        if not text.strip():
            return
        conn.execute(
            """
            insert into strict_term_aliases(term_id, language, text, kind, case_sensitive)
            values (?, ?, ?, ?, 1)
            """,
            (term_id, language, text, kind),
        )

    def _now(self) -> str:
        return datetime.now(UTC).isoformat()

    def _count(
        self,
        conn: sqlite3.Connection,
        table: str,
        where_sql: str = "",
    ) -> int:
        if not self._table_exists(conn, table):
            return 0
        row = conn.execute(f"select count(*) from {table} {where_sql}").fetchone()
        return int(row[0])

    def _count_audit_events(self, conn: sqlite3.Connection) -> int:
        return self._count(conn, self._audit_log_table(conn))

    def _audit_log_table(self, conn: sqlite3.Connection) -> str:
        for table in ("audit_log", "audit_events", "memory_events"):
            if self._table_exists(conn, table):
                return table
        return "memory_events"

    def _table_exists(self, conn: sqlite3.Connection, table: str) -> bool:
        row = conn.execute(
            """
            select 1
            from sqlite_master
            where type = 'table'
              and name = ?
            """,
            (table,),
        ).fetchone()
        return row is not None


def _json_tuple(value: str) -> tuple[str, ...]:
    loaded = json.loads(value)
    if not isinstance(loaded, list):
        return ()
    return tuple(str(item) for item in loaded)


def _clamp_score(value: float) -> float:
    return min(max(value, 0.0), 1.0)


def _language_pair(row: sqlite3.Row) -> str:
    return f"{row['source_language']} -> {row['target_language']}"


def _percent(value: object) -> str:
    return f"{round(float(value) * 100):.0f}%"


def _excerpt(text: str, *, limit: int = 80) -> str:
    normalized = " ".join(text.split())
    if len(normalized) <= limit:
        return normalized
    return f"{normalized[: limit - 1]}..."


def _session_label(row: sqlite3.Row) -> str:
    parts = [row["series_slug"]]
    if row["volume"]:
        parts.append(f"v{row['volume']}")
    if row["chapter"]:
        parts.append(f"ch{row['chapter']}")
    return " / ".join(parts)


def _row_value(row: sqlite3.Row, key: str, default: str) -> str:
    if key not in row.keys():
        return default
    value = row[key]
    return default if value is None else str(value)


def _row_fields(row: AdminRow) -> tuple[tuple[str, str], ...]:
    return (
        ("Kind", row.kind),
        ("Scope", row.scope),
        ("Language", row.language_pair),
        ("Quality", row.quality_label),
    )


def _event_quality(row: sqlite3.Row) -> str:
    return f"{row['strength_delta']:+.2f} str / {row['confidence_delta']:+.2f} conf"
