from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Protocol

from hieronymus.concepts import VALID_FACET_KINDS, ConceptProposalStore, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.dream_audit import DreamAuditStore
from hieronymus.dream_config import load_dream_config
from hieronymus.dream_locks import DreamCycleAlreadyRunning, dream_cycle_lock
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
from hieronymus.scoring import PASSIVE_EVENT_DELTAS, apply_score_delta
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.settings import SettingsError, load_settings
from hieronymus.workspace import WorkspaceStore, short_memory_from_row

_ALLOWED_CRYSTAL_TYPES = frozenset(
    {"lesson", "rule", "thought", "observation", "concept_note", "concept", "erudition"}
)
MALFORMED_CONFIDENCE_PENALTY = 0.2
SOURCE_CREDIBILITY_CONFIDENCE = {
    "rumor": 0.15,
    "observation": 0.35,
    "source_text": 0.7,
    "expert": 0.85,
    "user_suggestion": 0.8,
    "user_rule": 0.95,
    "thought": 0.2,
}
_MIN_NORMALIZED_CONFIDENCE = 0.05
STRENGTH_DECAY_PER_CYCLE = 0.03
CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW = 0.20
CONFIDENCE_DECAY_PER_CYCLE = 0.01
_ROLE_TYPES = {
    "user": "lesson",
    "mentor": "erudition",
    "mundane": "concept",
}
_ROLE_CONFIDENCE = {
    "user": 0.9,
    "mentor": 0.75,
    "mundane": 0.6,
}
_SENTENCE_RE = re.compile(r"[^.!?。！？]+[.!?。！？]?")


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _clamp_score(value: float) -> float:
    return min(max(value, 0.0), 1.0)


@dataclass(frozen=True)
class DreamCrystalCandidate:
    crystal_type: str
    title: str
    text: str
    strength: float
    confidence: float
    source_memory_ids: list[int] = field(default_factory=list)
    source_credibility: str = "observation"
    rule_intent: str = ""
    is_inferred: bool = False


@dataclass(frozen=True)
class DreamConceptProposal:
    series_slug: str
    source_language: str
    target_language: str
    concept_text: str
    source_form: str
    canonical_rendering: str
    approved_variants: list[str] = field(default_factory=list)
    forbidden_variants: list[str] = field(default_factory=list)
    rationale: str = ""


@dataclass(frozen=True)
class DreamOutput:
    crystals: list[DreamCrystalCandidate] = field(default_factory=list)
    concept_proposals: list[DreamConceptProposal] = field(default_factory=list)


@dataclass(frozen=True)
class DreamParseWarning:
    entry_path: str
    code: str
    message: str
    confidence_penalty: float


@dataclass(frozen=True)
class _NormalizedDreamCrystal:
    crystal_type: str
    title: str
    text: str
    strength: float
    confidence: float
    source_memory_ids: list[int]
    source_credibility: str = "observation"
    rule_intent: str = ""
    malformed_penalty: float = 0.0
    is_inferred: bool = False
    supersedes_crystal_id: int | None = None
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    concept_ids: tuple[int, ...] = ()
    concept_names: tuple[str, ...] = ()


@dataclass(frozen=True)
class _NormalizedDreamConcept:
    canonical_name: str
    description: str = ""
    tags: tuple[str, ...] = ()
    confidence_delta: float = 0.2


@dataclass(frozen=True)
class _NormalizedDreamFacet:
    concept_name: str
    value: str
    kind: str = "note"
    language_tags: tuple[str, ...] = ()
    story_scopes: tuple[str, ...] = ()
    semantic_tags: tuple[str, ...] = ()
    confidence: float = 0.2
    is_canonical: bool = False


@dataclass(frozen=True)
class _DreamSupersedeAction:
    old_crystal_id: int
    new_crystal_id: int
    reason: str = ""


@dataclass(frozen=True)
class _NormalizedDreamOutput:
    crystals: list[_NormalizedDreamCrystal] = field(default_factory=list)
    concept_proposals: list[DreamConceptProposal] = field(default_factory=list)
    concepts: list[_NormalizedDreamConcept] = field(default_factory=list)
    facets: list[_NormalizedDreamFacet] = field(default_factory=list)
    supersede_actions: list[_DreamSupersedeAction] = field(default_factory=list)
    warnings: list[DreamParseWarning] = field(default_factory=list)


@dataclass(frozen=True)
class DreamRunRecord:
    id: int
    cycle_id: int
    status: str
    provider: str = ""
    input_count: int = 0
    created_crystal_count: int = 0
    proposal_count: int = 0
    error: str = ""


class DreamProvider(Protocol):
    name: str

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput | dict[str, object]: ...


class DeterministicDreamProvider:
    name = "deterministic"

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        candidates = []
        for memory in memories:
            crystal_type = _crystal_type_for_short_memory(memory)
            if crystal_type is None:
                continue
            candidates.append(
                DreamCrystalCandidate(
                    crystal_type=crystal_type,
                    title=_title_from_kind(memory.kind),
                    text=_normalize_candidate_text(memory.text),
                    strength=0.6,
                    confidence=_confidence_for_short_memory(memory),
                    source_memory_ids=[memory.id],
                    source_credibility=memory.source_credibility,
                    rule_intent=memory.rule_intent,
                )
            )
        return DreamOutput(crystals=candidates, concept_proposals=[])


DreamBatch = list[tuple[int, TranslationContext, list[ShortTermMemoryRecord]]]
DreamBatchOutput = tuple[
    TranslationContext, int, list[ShortTermMemoryRecord], _NormalizedDreamOutput
]


class DreamService:
    def __init__(
        self,
        config: HieronymusConfig,
        provider: DreamProvider,
        max_short_term_memories_per_cycle: int | None = None,
    ) -> None:
        self.config = config
        self.provider = provider
        self.max_short_term_memories_per_cycle = (
            max_short_term_memories_per_cycle
            if max_short_term_memories_per_cycle is not None
            else load_dream_config(config).max_short_term_memories_per_cycle
        )
        if self.max_short_term_memories_per_cycle < 1:
            raise ValueError("max_short_term_memories_per_cycle must be at least 1")
        self.concept_proposals = ConceptProposalStore(config)
        self.concepts = ConceptStore(config)
        self.crystals = CrystalStore(config)
        self.audit = DreamAuditStore(config)
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def select_ambient_decay_candidates(
        self,
        *,
        recalled_crystal_ids: tuple[int, ...],
        linked_crystal_ids: tuple[int, ...],
        limit: int = 5,
    ) -> tuple[int, ...]:
        linked_ids = set(linked_crystal_ids)
        unused = tuple(
            crystal_id for crystal_id in recalled_crystal_ids if crystal_id not in linked_ids
        )
        return self.crystals.low_confidence_first(unused, limit=limit)

    def decay_candidates(
        self,
        *,
        crystal_ids: tuple[int, ...],
        reason: str,
        strength_delta: float = -STRENGTH_DECAY_PER_CYCLE,
        confidence_delta: float = -CONFIDENCE_DECAY_PER_CYCLE,
        cycle_id: int = 0,
    ) -> tuple[int, ...]:
        return self._apply_score_maintenance(
            crystal_ids=crystal_ids,
            strength_delta=strength_delta,
            confidence_delta=confidence_delta,
            reason=reason,
            event_type="maintenance_decay",
            cycle_id=cycle_id,
            skip_active_rules=True,
        )

    def apply_maintenance_actions(
        self,
        payload: dict[object, object],
        *,
        cycle_id: int = 0,
    ) -> dict[str, tuple[int, ...]]:
        applied: dict[str, tuple[int, ...]] = {}
        with connect(self.config.database_path) as conn:
            try:
                reinforced_ids: list[int] = []
                for item in _list_from_payload(payload.get("reinforce")):
                    action = _normalize_score_action(item)
                    if action is None:
                        continue
                    reinforced_ids.extend(
                        self._apply_score_maintenance_with_connection(
                            conn,
                            crystal_ids=(action[0],),
                            strength_delta=action[1],
                            confidence_delta=action[2],
                            reason="maintenance reinforce",
                            event_type="maintenance_reinforce",
                            cycle_id=cycle_id,
                            skip_active_rules=False,
                        )
                    )
                if reinforced_ids:
                    applied["reinforce"] = tuple(reinforced_ids)

                decayed_ids: list[int] = []
                for item in _list_from_payload(payload.get("decay")):
                    action = _normalize_score_action(item)
                    if action is None:
                        continue
                    decayed_ids.extend(
                        self._apply_score_maintenance_with_connection(
                            conn,
                            crystal_ids=(action[0],),
                            strength_delta=action[1],
                            confidence_delta=action[2],
                            reason="maintenance decay",
                            event_type="maintenance_decay",
                            cycle_id=cycle_id,
                            skip_active_rules=True,
                        )
                    )
                if decayed_ids:
                    applied["decay"] = tuple(decayed_ids)

                combined_ids: list[int] = []
                for item in _list_from_payload(payload.get("combine")):
                    combined_id = self._apply_combine_maintenance_with_connection(
                        conn,
                        item,
                        cycle_id=cycle_id,
                    )
                    if combined_id is not None:
                        combined_ids.append(combined_id)
                if combined_ids:
                    applied["combine"] = tuple(combined_ids)

                superseded_ids: list[int] = []
                for item in _list_from_payload(payload.get("supersede")):
                    action = _normalize_supersede_action(item)
                    if action is None:
                        continue
                    self.crystals._supersede_with_connection(
                        conn,
                        old_crystal_id=action.old_crystal_id,
                        new_crystal_id=action.new_crystal_id,
                        reason=action.reason,
                        cycle_id=cycle_id,
                    )
                    superseded_ids.append(action.old_crystal_id)
                if superseded_ids:
                    applied["supersede"] = tuple(superseded_ids)
                conn.commit()
            except Exception:
                conn.rollback()
                raise

        return applied

    def run_cycle(
        self,
        *,
        owner: str = "manual",
        wait: bool = False,
        skip_when_locked: bool = False,
    ) -> DreamRunRecord:
        lock_acquired = False
        try:
            with dream_cycle_lock(self.config, owner=owner, wait=wait):
                lock_acquired = True
                return self._run_cycle_unlocked(max_batches=1)
        except DreamCycleAlreadyRunning:
            if skip_when_locked and not lock_acquired:
                return self._record_skipped_run("dream cycle already running")
            raise

    def run_all(
        self,
        *,
        owner: str = "manual",
        skip_when_locked: bool = False,
        wait: bool = False,
        ignore_minimum: bool = True,
    ) -> DreamRunRecord:
        lock_acquired = False
        try:
            with dream_cycle_lock(self.config, owner=owner, wait=wait):
                lock_acquired = True
                return self._run_cycle_unlocked(
                    max_batches=None,
                    ignore_minimum=ignore_minimum,
                )
        except DreamCycleAlreadyRunning:
            if skip_when_locked and not lock_acquired:
                return self._record_skipped_run("dream cycle already running")
            raise

    def _run_cycle_unlocked(
        self,
        *,
        max_batches: int | None,
        ignore_minimum: bool = True,
    ) -> DreamRunRecord:
        now = _now()
        with connect(self.config.database_path) as conn:
            cycle_id = self._next_cycle_id(conn)
            cursor = conn.execute(
                """
                insert into dream_runs(cycle_id, status, provider, created_at)
                values (?, 'running', ?, ?)
                """,
                (cycle_id, self.provider.name, now),
            )
            run_id = int(cursor.lastrowid)
            conn.commit()

        input_count = 0
        created_crystal_count = 0
        proposal_count = 0
        try:
            pending_count = self._pending_short_term_memory_count()
            minimum = load_dream_config(self.config).min_pending_short_term_memories
            if not ignore_minimum and pending_count < minimum:
                self._complete_run(
                    run_id=run_id,
                    input_count=0,
                    created_crystal_count=0,
                    proposal_count=0,
                )
                return DreamRunRecord(
                    id=run_id,
                    cycle_id=cycle_id,
                    status="completed",
                    provider=self.provider.name,
                    input_count=0,
                    created_crystal_count=0,
                    proposal_count=0,
                )

            processed_batches = 0
            processed_session_ids: set[int] = set()

            while max_batches is None or processed_batches < max_batches:
                groups = self._load_pending_completed_groups(
                    limit=self.max_short_term_memories_per_cycle,
                )
                if not groups:
                    break

                batch_input_count = sum(len(memories) for _sid, _context, memories in groups)
                phase_run_id = self._start_phase_run(
                    run_id=run_id,
                    phase="crystallization",
                    input_count=batch_input_count,
                )
                try:
                    raw_outputs = [
                        (
                            context,
                            session_id,
                            memories,
                            self.provider.crystallize(context, memories),
                        )
                        for session_id, context, memories in groups
                    ]
                    outputs = []
                    for context, session_id, memories, raw_output in raw_outputs:
                        allowed_memory_ids = {memory.id for memory in memories}
                        output = self._normalize_output(
                            raw_output,
                            context,
                            allowed_memory_ids,
                        )
                        self._audit_parse_warnings(run_id, phase_run_id, output.warnings)
                        self._validate_normalized_output(
                            output,
                            context,
                            allowed_memory_ids,
                        )
                        outputs.append((context, session_id, memories, output))

                    batch_created_crystals, batch_proposals = self._apply_outputs(
                        run_id=run_id,
                        cycle_id=cycle_id,
                        groups=groups,
                        outputs=outputs,
                        archive_memories=True,
                    )
                    input_count += batch_input_count
                    created_crystal_count += batch_created_crystals
                    proposal_count += batch_proposals
                    processed_session_ids.update(
                        session_id for session_id, _context, _memories in groups
                    )
                    self._complete_phase_run(
                        phase_run_id=phase_run_id,
                        output_count=batch_created_crystals + batch_proposals,
                    )
                except Exception as exc:
                    self._fail_phase_run(phase_run_id, exc)
                    raise

                processed_batches += 1

            processed_session_ids.update(self._completed_session_ids_without_pending_memories())
            with connect(self.config.database_path) as conn:
                self._apply_passive_events(conn, cycle_id)
                self._mark_activations_for_cycle(
                    conn,
                    cycle_id,
                    sorted(processed_session_ids),
                )
                self._apply_cycle_decay(conn, cycle_id)
                self._mark_fully_archived_completed_sessions(
                    conn,
                    cycle_id,
                    sorted(processed_session_ids),
                )
                self._complete_run_with_connection(
                    conn,
                    run_id=run_id,
                    input_count=input_count,
                    created_crystal_count=created_crystal_count,
                    proposal_count=proposal_count,
                )
                conn.commit()
            return DreamRunRecord(
                id=run_id,
                cycle_id=cycle_id,
                status="completed",
                provider=self.provider.name,
                input_count=input_count,
                created_crystal_count=created_crystal_count,
                proposal_count=proposal_count,
            )
        except Exception as exc:
            with connect(self.config.database_path) as conn:
                conn.execute(
                    """
                    update dream_runs
                    set status = 'failed',
                        input_count = ?,
                        created_crystal_count = ?,
                        proposal_count = ?,
                        error = ?,
                        completed_at = ?
                    where id = ?
                    """,
                    (
                        input_count,
                        created_crystal_count,
                        proposal_count,
                        self._redacted_error_message(exc),
                        _now(),
                        run_id,
                    ),
                )
                conn.commit()
            raise

    def _record_skipped_run(self, reason: str) -> DreamRunRecord:
        now = _now()
        with connect(self.config.database_path) as conn:
            try:
                conn.execute("begin immediate")
                cycle_id = self._next_skipped_cycle_id(conn)
                cursor = conn.execute(
                    """
                    insert into dream_runs(
                      cycle_id,
                      status,
                      provider,
                      error,
                      created_at,
                      completed_at
                    )
                    values (?, 'skipped', ?, ?, ?, ?)
                    """,
                    (cycle_id, self.provider.name, reason, now, now),
                )
                run_id = int(cursor.lastrowid)
                conn.commit()
            except Exception:
                conn.rollback()
                raise
        return DreamRunRecord(
            id=run_id,
            cycle_id=cycle_id,
            status="skipped",
            provider=self.provider.name,
            error=reason,
        )

    def _redacted_error_message(self, error: Exception) -> str:
        message = str(error)
        try:
            settings = load_settings(self.config)
        except SettingsError:
            return message
        return redact_configured_secret_values(message, settings)

    def _apply_score_maintenance(
        self,
        *,
        crystal_ids: tuple[int, ...],
        strength_delta: float,
        confidence_delta: float,
        reason: str,
        event_type: str,
        cycle_id: int,
        skip_active_rules: bool,
    ) -> tuple[int, ...]:
        if not crystal_ids:
            return ()
        with connect(self.config.database_path) as conn:
            try:
                applied_ids = self._apply_score_maintenance_with_connection(
                    conn,
                    crystal_ids=crystal_ids,
                    strength_delta=strength_delta,
                    confidence_delta=confidence_delta,
                    reason=reason,
                    event_type=event_type,
                    cycle_id=cycle_id,
                    skip_active_rules=skip_active_rules,
                )
                conn.commit()
            except Exception:
                conn.rollback()
                raise
        return tuple(applied_ids)

    def _apply_score_maintenance_with_connection(
        self,
        conn,
        *,
        crystal_ids: tuple[int, ...],
        strength_delta: float,
        confidence_delta: float,
        reason: str,
        event_type: str,
        cycle_id: int,
        skip_active_rules: bool,
    ) -> tuple[int, ...]:
        if not crystal_ids:
            return ()
        unique_ids = tuple(sorted(set(crystal_ids)))
        placeholders = ", ".join("?" for _ in unique_ids)
        now = _now()
        rows = conn.execute(
            f"""
            select id, crystal_type, strength, confidence, status
            from crystals
            where id in ({placeholders})
            order by id
            """,
            unique_ids,
        ).fetchall()
        applied_ids: list[int] = []
        for crystal in rows:
            if (
                skip_active_rules
                and crystal["crystal_type"] == "rule"
                and crystal["status"] == "active"
            ):
                continue
            original_strength = float(crystal["strength"])
            original_confidence = float(crystal["confidence"])
            strength, confidence, status = apply_score_delta(
                strength=original_strength,
                confidence=original_confidence,
                status=crystal["status"],
                crystal_type=crystal["crystal_type"],
                strength_delta=strength_delta,
                confidence_delta=confidence_delta,
            )
            conn.execute(
                """
                update crystals
                set strength = ?,
                    confidence = ?,
                    status = ?,
                    updated_at = ?
                where id = ?
                """,
                (strength, confidence, status, now, crystal["id"]),
            )
            actual_strength_delta = strength - original_strength
            actual_confidence_delta = confidence - original_confidence
            conn.execute(
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
                  cycle_id,
                  created_at
                )
                values (?, null, ?, 'system', ?, ?, ?, 1, ?, ?)
                """,
                (
                    crystal["id"],
                    event_type,
                    reason,
                    actual_strength_delta,
                    actual_confidence_delta,
                    cycle_id,
                    now,
                ),
            )
            applied_ids.append(int(crystal["id"]))
        return tuple(applied_ids)

    def _apply_combine_maintenance(
        self,
        item: object,
        *,
        cycle_id: int,
    ) -> int | None:
        with connect(self.config.database_path) as conn:
            try:
                combined_id = self._apply_combine_maintenance_with_connection(
                    conn,
                    item,
                    cycle_id=cycle_id,
                )
                conn.commit()
            except Exception:
                conn.rollback()
                raise
        return combined_id

    def _apply_combine_maintenance_with_connection(
        self,
        conn,
        item: object,
        *,
        cycle_id: int,
    ) -> int | None:
        if type(item) is not dict:
            return None
        payload: dict[object, object] = item
        content = _string_field(payload.get("content")).strip()
        if not content:
            content = _string_field(payload.get("text")).strip()
        if not content:
            return None

        source_ids = _clean_int_tuple(payload.get("source_crystal_ids"))
        if len(source_ids) < 2:
            raise ValueError("combine requires at least two distinct source crystal IDs")

        placeholders = ", ".join("?" for _ in source_ids)
        now = _now()
        source_rows = conn.execute(
            f"""
            select *
            from crystals
            where id in ({placeholders})
            order by id
            """,
            source_ids,
        ).fetchall()
        if len(source_rows) != len(source_ids):
            found_ids = {int(row["id"]) for row in source_rows}
            missing_ids = sorted(set(source_ids) - found_ids)
            raise KeyError(f"unknown source_crystal_ids: {missing_ids}")
        self._validate_combine_sources(source_rows)

        first = source_rows[0]
        crystal_types = {row["crystal_type"] for row in source_rows}
        strength = _clamp_score(
            sum(float(row["strength"]) for row in source_rows) / len(source_rows)
        )
        confidence = _clamp_score(
            sum(float(row["confidence"]) for row in source_rows) / len(source_rows)
        )
        context = TranslationContext(
            series_slug=first["series_slug"],
            source_language=first["source_language"],
            target_language=first["target_language"],
            task_type="translate",
            volume="",
            chapter="",
        )
        candidate = _NormalizedDreamCrystal(
            crystal_type=first["crystal_type"] if len(crystal_types) == 1 else "observation",
            title=_string_field(payload.get("title")).strip() or "Combined Memory",
            text=" ".join(content.split()),
            strength=strength,
            confidence=confidence,
            source_memory_ids=[],
            source_credibility=first["source_credibility"],
        )
        combined_id = self._insert_crystal(conn, context, candidate)
        for source_id in source_ids:
            conn.execute(
                """
                insert or ignore into crystal_links(
                  source_crystal_id,
                  target_crystal_id,
                  link_type
                )
                values (?, ?, 'combined_into')
                """,
                (source_id, combined_id),
            )
        source_evidence = (
            f"combined sources: {', '.join(str(source_id) for source_id in source_ids)}"
        )
        conn.execute(
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
              cycle_id,
              created_at
            )
            values (?, null, 'maintenance_combine', 'system', ?, 0, 0, 1, ?, ?)
            """,
            (
                combined_id,
                source_evidence,
                cycle_id,
                now,
            ),
        )
        return combined_id

    def _validate_combine_sources(self, source_rows) -> None:
        first = source_rows[0]
        for row in source_rows:
            if row["status"] not in {"active", "candidate"}:
                raise ValueError("combine source crystals must be active or candidate")
        for row in source_rows[1:]:
            for column in (
                "series_slug",
                "source_language",
                "target_language",
                "scope_type",
                "scope_key",
            ):
                if row[column] != first[column]:
                    raise ValueError(f"combine crystal {column} does not match")

    def _load_pending_completed_groups(
        self,
        *,
        limit: int,
    ) -> DreamBatch:
        selected_session_ids = self._select_pending_completed_session_batch(limit=limit)
        if not selected_session_ids:
            return []
        placeholders = ", ".join("?" for _ in selected_session_ids)
        with connect(self.config.database_path) as conn:
            memory_rows = conn.execute(
                f"""
                select
                  short_term_memories.*,
                  task_sessions.series_slug,
                  task_sessions.source_language,
                  task_sessions.target_language,
                  task_sessions.task_type,
                  task_sessions.volume,
                  task_sessions.chapter
                from short_term_memories
                join task_sessions
                  on task_sessions.id = short_term_memories.session_id
                where task_sessions.status = 'completed'
                  and task_sessions.id in ({placeholders})
                  and short_term_memories.archived_at is null
                order by task_sessions.id, short_term_memories.id
                """,
                selected_session_ids,
            ).fetchall()

        groups_by_session: dict[int, tuple[TranslationContext, list[ShortTermMemoryRecord]]] = {}
        workspace = WorkspaceStore(self.config)
        for row in memory_rows:
            session_id = int(row["session_id"])
            group = groups_by_session.get(session_id)
            if group is None:
                group = (
                    workspace.get_session(session_id).context,
                    [],
                )
                groups_by_session[session_id] = group
            group[1].append(short_memory_from_row(conn, row))

        return [
            (session_id, context, memories)
            for session_id, (context, memories) in groups_by_session.items()
        ]

    def _select_pending_completed_session_batch(self, *, limit: int) -> list[int]:
        with connect(self.config.database_path) as conn:
            session_rows = conn.execute(
                """
                select
                  task_sessions.id,
                  count(short_term_memories.id) as pending_memory_count
                from task_sessions
                join short_term_memories
                  on short_term_memories.session_id = task_sessions.id
                where task_sessions.status = 'completed'
                  and short_term_memories.archived_at is null
                group by task_sessions.id
                order by task_sessions.id
                """
            ).fetchall()

        selected_session_ids: list[int] = []
        selected_memory_count = 0
        for row in session_rows:
            session_id = int(row["id"])
            pending_memory_count = int(row["pending_memory_count"])
            if not selected_session_ids:
                selected_session_ids.append(session_id)
                selected_memory_count += pending_memory_count
                if pending_memory_count >= limit:
                    break
                continue
            if selected_memory_count + pending_memory_count > limit:
                break
            selected_session_ids.append(session_id)
            selected_memory_count += pending_memory_count
        return selected_session_ids

    def _pending_short_term_memory_count(self) -> int:
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

    def _completed_session_ids_without_pending_memories(self) -> list[int]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select task_sessions.id
                from task_sessions
                where task_sessions.status = 'completed'
                  and task_sessions.cycle_id is null
                  and not exists (
                    select 1
                    from short_term_memories
                    where short_term_memories.session_id = task_sessions.id
                      and short_term_memories.archived_at is null
                  )
                order by task_sessions.id
                """
            ).fetchall()
        return [int(row["id"]) for row in rows]

    def _start_phase_run(self, *, run_id: int, phase: str, input_count: int) -> int:
        now = _now()
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                insert into dream_phase_runs(
                  dream_run_id,
                  phase,
                  provider_profile,
                  provider_type,
                  model,
                  status,
                  input_count,
                  created_at
                )
                values (?, ?, ?, ?, ?, 'running', ?, ?)
                """,
                (
                    run_id,
                    phase,
                    self.provider.name,
                    self.provider.name,
                    self.provider.name,
                    input_count,
                    now,
                ),
            )
            phase_run_id = int(cursor.lastrowid)
            conn.commit()
        return phase_run_id

    def _complete_phase_run(self, *, phase_run_id: int, output_count: int) -> None:
        with connect(self.config.database_path) as conn:
            conn.execute(
                """
                update dream_phase_runs
                set status = 'completed',
                    output_count = ?,
                    completed_at = ?
                where id = ?
                """,
                (output_count, _now(), phase_run_id),
            )
            conn.commit()

    def _fail_phase_run(self, phase_run_id: int, error: Exception) -> None:
        with connect(self.config.database_path) as conn:
            conn.execute(
                """
                update dream_phase_runs
                set status = 'failed',
                    error = ?,
                    completed_at = ?
                where id = ?
                """,
                (self._redacted_error_message(error), _now(), phase_run_id),
            )
            conn.commit()

    def _complete_run(
        self,
        *,
        run_id: int,
        input_count: int,
        created_crystal_count: int,
        proposal_count: int,
    ) -> None:
        with connect(self.config.database_path) as conn:
            self._complete_run_with_connection(
                conn,
                run_id=run_id,
                input_count=input_count,
                created_crystal_count=created_crystal_count,
                proposal_count=proposal_count,
            )
            conn.commit()

    def _complete_run_with_connection(
        self,
        conn,
        *,
        run_id: int,
        input_count: int,
        created_crystal_count: int,
        proposal_count: int,
    ) -> None:
        conn.execute(
            """
            update dream_runs
            set status = 'completed',
                input_count = ?,
                created_crystal_count = ?,
                proposal_count = ?,
                completed_at = ?
            where id = ?
            """,
            (
                input_count,
                created_crystal_count,
                proposal_count,
                _now(),
                run_id,
            ),
        )

    def _mark_fully_archived_completed_sessions(
        self,
        conn,
        cycle_id: int,
        session_ids: list[int],
    ) -> None:
        if not session_ids:
            return
        placeholders = ", ".join("?" for _ in session_ids)
        conn.execute(
            f"""
            update task_sessions
            set status = 'dreamed',
                cycle_id = ?
            where status = 'completed'
              and id in ({placeholders})
              and not exists (
                select 1
                from short_term_memories
                where short_term_memories.session_id = task_sessions.id
                  and short_term_memories.archived_at is null
              )
            """,
            (cycle_id, *session_ids),
        )

    def _archive_memories(self, conn, groups: DreamBatch) -> None:
        memory_ids = [
            memory.id for _session_id, _context, memories in groups for memory in memories
        ]
        if not memory_ids:
            return
        placeholders = ", ".join("?" for _ in memory_ids)
        conn.execute(
            f"""
            update short_term_memories
            set archived_at = ?
            where id in ({placeholders})
            """,
            (_now(), *memory_ids),
        )

    def _apply_outputs(
        self,
        *,
        run_id: int,
        cycle_id: int,
        groups: DreamBatch,
        outputs: list[DreamBatchOutput],
        archive_memories: bool,
    ) -> tuple[int, int]:
        created_crystal_count = 0
        proposal_count = 0
        now = _now()
        with connect(self.config.database_path) as conn:
            try:
                for context, _session_id, _memories, output in outputs:
                    concept_ids_by_name: dict[str, int] = {}
                    for concept in output.concepts:
                        concept_id = self.concepts._create_or_reinforce_with_connection(
                            conn,
                            concept.canonical_name,
                            description=concept.description,
                            tags=concept.tags,
                            confidence_delta=concept.confidence_delta,
                            scope_type="global",
                            scope_key="",
                        )
                        concept_ids_by_name[concept.canonical_name.casefold()] = concept_id

                    for facet in output.facets:
                        key = facet.concept_name.casefold()
                        concept_id = concept_ids_by_name.get(key)
                        if concept_id is None:
                            concept_id = self.concepts._create_or_reinforce_with_connection(
                                conn,
                                facet.concept_name,
                                confidence_delta=0.2,
                                scope_type="global",
                                scope_key="",
                            )
                            concept_ids_by_name[key] = concept_id
                        self.concepts._add_facet_with_connection(
                            conn,
                            concept_id,
                            facet.value,
                            kind=facet.kind,
                            language_tags=facet.language_tags,
                            confidence=facet.confidence,
                            is_canonical=facet.is_canonical,
                            story_scopes=facet.story_scopes,
                            semantic_tags=facet.semantic_tags,
                            now=now,
                        )

                    for candidate in output.crystals:
                        candidate = self._resolve_candidate_concepts(
                            conn,
                            candidate,
                            concept_ids_by_name,
                        )
                        crystal_id = self._insert_crystal_for_dream(conn, context, candidate)
                        conn.execute(
                            """
                            update crystals
                            set created_cycle = ?
                            where id = ?
                            """,
                            (cycle_id, crystal_id),
                        )
                        created_crystal_count += 1
                    self._insert_concept_proposals(conn, run_id, output.concept_proposals)
                    proposal_count += len(output.concept_proposals)
                    for action in output.supersede_actions:
                        self.crystals._supersede_with_connection(
                            conn,
                            old_crystal_id=action.old_crystal_id,
                            new_crystal_id=action.new_crystal_id,
                            reason=action.reason,
                            cycle_id=cycle_id,
                        )

                if archive_memories:
                    self._archive_memories(conn, groups)
                    self._mark_fully_archived_completed_sessions(
                        conn,
                        cycle_id,
                        [session_id for session_id, _context, _memories in groups],
                    )
                conn.commit()
            except Exception:
                conn.rollback()
                raise
        return created_crystal_count, proposal_count

    def _audit_parse_warnings(
        self,
        run_id: int,
        phase_run_id: int,
        warnings: list[DreamParseWarning],
    ) -> None:
        if not warnings:
            return
        self.audit.append(
            dream_run_id=run_id,
            phase_run_id=phase_run_id,
            event_type="parse_warnings",
            severity="warning",
            summary="dream response parsed with recoverable warnings",
            payload={
                "warnings": [
                    {
                        "entry_path": warning.entry_path,
                        "code": warning.code,
                        "message": warning.message,
                        "confidence_penalty": warning.confidence_penalty,
                    }
                    for warning in warnings
                ]
            },
        )

    def _apply_passive_events(self, conn, cycle_id: int) -> None:
        now = _now()
        rows = conn.execute(
            """
            select *
            from memory_events
            where applied = 0
            order by id
            """
        ).fetchall()
        for event in rows:
            crystal_id = event["crystal_id"]
            if crystal_id is not None:
                crystal = conn.execute(
                    """
                    select strength, confidence
                    from crystals
                    where id = ?
                    """,
                    (crystal_id,),
                ).fetchone()
                if crystal is not None:
                    strength_delta = float(event["strength_delta"])
                    confidence_delta = float(event["confidence_delta"])
                    strength = _clamp_score(float(crystal["strength"]) + strength_delta)
                    confidence = _clamp_score(float(crystal["confidence"]) + confidence_delta)
                    last_reinforced_cycle = (
                        cycle_id
                        if strength_delta > 0 and event["event_type"] in PASSIVE_EVENT_DELTAS
                        else None
                    )
                    if last_reinforced_cycle is None:
                        conn.execute(
                            """
                            update crystals
                            set strength = ?,
                                confidence = ?,
                                updated_at = ?
                            where id = ?
                            """,
                            (strength, confidence, now, crystal_id),
                        )
                    else:
                        conn.execute(
                            """
                            update crystals
                            set strength = ?,
                                confidence = ?,
                                last_reinforced_cycle = ?,
                                updated_at = ?
                            where id = ?
                            """,
                            (strength, confidence, last_reinforced_cycle, now, crystal_id),
                        )
            conn.execute(
                """
                update memory_events
                set applied = 1,
                    cycle_id = ?
                where id = ?
                """,
                (cycle_id, event["id"]),
            )

    def _mark_activations_for_cycle(
        self,
        conn,
        cycle_id: int,
        session_ids: list[int],
    ) -> None:
        if not session_ids:
            return
        now = _now()
        placeholders = ", ".join("?" for _ in session_ids)
        conn.execute(
            f"""
            update crystal_activations
            set cycle_id = ?
            where session_id in ({placeholders})
            """,
            (cycle_id, *session_ids),
        )
        conn.execute(
            f"""
            update crystals
            set last_activated_cycle = ?,
                updated_at = ?
            where id in (
              select distinct crystal_id
              from crystal_activations
              where session_id in ({placeholders})
            )
            """,
            (cycle_id, now, *session_ids),
        )

    def _apply_cycle_decay(self, conn, cycle_id: int) -> None:
        now = _now()
        rows = conn.execute(
            """
            select id, crystal_type, strength, confidence, status
            from crystals
            where status in ('active', 'candidate')
              and created_cycle != ?
              and coalesce(last_activated_cycle, -1) != ?
              and coalesce(last_reinforced_cycle, -1) != ?
              and not (crystal_type = 'rule' and status = 'active')
            order by id
            """,
            (cycle_id, cycle_id, cycle_id),
        ).fetchall()
        for crystal in rows:
            original_strength = float(crystal["strength"])
            original_confidence = float(crystal["confidence"])
            strength = _clamp_score(original_strength - STRENGTH_DECAY_PER_CYCLE)
            # Confidence decay is based on strength after this cycle's strength decay.
            confidence_delta = (
                CONFIDENCE_DECAY_PER_CYCLE
                if strength < CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW
                else 0.0
            )
            strength, confidence, status = apply_score_delta(
                strength=original_strength,
                confidence=original_confidence,
                status=crystal["status"],
                crystal_type=crystal["crystal_type"],
                strength_delta=-STRENGTH_DECAY_PER_CYCLE,
                confidence_delta=-confidence_delta,
            )
            conn.execute(
                """
                update crystals
                set strength = ?,
                    confidence = ?,
                    status = ?,
                    updated_at = ?
                where id = ?
                """,
                (strength, confidence, status, now, crystal["id"]),
            )
            strength_delta = strength - original_strength
            confidence_delta = confidence - original_confidence
            if strength_delta != 0.0 or confidence_delta != 0.0:
                conn.execute(
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
                      cycle_id,
                      created_at
                    )
                    values (?, null, 'cycle_decay', 'system', 'cycle decay', ?, ?, 1, ?, ?)
                    """,
                    (
                        crystal["id"],
                        strength_delta,
                        confidence_delta,
                        cycle_id,
                        now,
                    ),
                )

    def _insert_crystal_for_dream(
        self,
        conn,
        context: TranslationContext,
        candidate: _NormalizedDreamCrystal,
    ) -> int:
        return self._insert_crystal(conn, context, candidate)

    def _insert_crystal(
        self,
        conn,
        context: TranslationContext,
        candidate: _NormalizedDreamCrystal,
    ) -> int:
        now = _now()
        tags_json = candidate.semantic_tags if candidate.semantic_tags else context.tags
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
              source_credibility,
              rule_intent,
              is_inferred,
              malformed_penalty,
              supersedes_crystal_id,
              status,
              created_at,
              updated_at
            )
            values (?, ?, ?, 'series', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
            """,
            (
                candidate.crystal_type,
                candidate.text,
                candidate.title,
                context.scope_key,
                context.series_slug,
                context.source_language,
                context.target_language,
                json.dumps(list(tags_json), ensure_ascii=False, sort_keys=True),
                candidate.strength,
                candidate.confidence,
                candidate.source_credibility,
                candidate.rule_intent,
                int(candidate.is_inferred),
                candidate.malformed_penalty,
                candidate.supersedes_crystal_id,
                now,
                now,
            ),
        )
        crystal_id = int(cursor.lastrowid)
        conn.execute(
            "insert into crystals_fts(rowid, title, text) values (?, ?, ?)",
            (crystal_id, candidate.title, candidate.text),
        )
        for memory_id in candidate.source_memory_ids:
            conn.execute(
                """
                insert into crystal_sources(crystal_id, short_term_memory_id)
                values (?, ?)
                """,
                (crystal_id, memory_id),
            )
        for story_scope in candidate.story_scopes:
            conn.execute(
                """
                insert into crystal_story_scopes(crystal_id, scope, confidence, created_at)
                values (?, ?, ?, ?)
                on conflict(crystal_id, scope) do update set
                  confidence = max(crystal_story_scopes.confidence, excluded.confidence)
                """,
                (crystal_id, story_scope, candidate.confidence, now),
            )
        for semantic_tag in candidate.semantic_tags:
            conn.execute(
                """
                insert into crystal_semantic_tags(crystal_id, tag, confidence, created_at)
                values (?, ?, ?, ?)
                on conflict(crystal_id, tag) do update set
                  confidence = max(crystal_semantic_tags.confidence, excluded.confidence)
                """,
                (crystal_id, semantic_tag, candidate.confidence, now),
            )
        for concept_id in candidate.concept_ids:
            conn.execute(
                """
                insert into crystal_concepts(
                  crystal_id,
                  concept_id,
                  link_type,
                  confidence,
                  created_at
                )
                values (?, ?, 'mentions', ?, ?)
                on conflict(crystal_id, concept_id, link_type) do update set
                  confidence = max(crystal_concepts.confidence, excluded.confidence)
                """,
                (crystal_id, concept_id, candidate.confidence, now),
            )
        return crystal_id

    def _insert_concept_proposals(
        self,
        conn,
        run_id: int,
        proposals: list[DreamConceptProposal],
    ) -> None:
        if not proposals:
            return
        for proposal in proposals:
            self.concept_proposals._create_with_connection(
                conn,
                dream_run_id=run_id,
                series_slug=proposal.series_slug,
                source_language=proposal.source_language,
                target_language=proposal.target_language,
                concept_text=proposal.concept_text,
                source_form=proposal.source_form,
                canonical_rendering=proposal.canonical_rendering,
                approved_variants=proposal.approved_variants,
                forbidden_variants=proposal.forbidden_variants,
                rationale=proposal.rationale,
            )

    def _normalize_output(
        self,
        raw_output: object,
        context: TranslationContext,
        allowed_memory_ids: set[int],
    ) -> _NormalizedDreamOutput:
        if isinstance(raw_output, DreamOutput):
            self._validate_output(raw_output, context, allowed_memory_ids)
            return _NormalizedDreamOutput(
                crystals=[
                    _NormalizedDreamCrystal(
                        crystal_type=candidate.crystal_type,
                        title=candidate.title,
                        text=candidate.text,
                        strength=candidate.strength,
                        confidence=candidate.confidence,
                        source_memory_ids=candidate.source_memory_ids,
                        source_credibility=candidate.source_credibility,
                        rule_intent=candidate.rule_intent,
                        is_inferred=candidate.is_inferred,
                    )
                    for candidate in raw_output.crystals
                ],
                concept_proposals=raw_output.concept_proposals,
            )
        if type(raw_output) is not dict:
            raise ValueError("provider output must be a DreamOutput or dict")
        return self._normalize_dict_output(raw_output, allowed_memory_ids)

    def _normalize_dict_output(
        self,
        payload: dict[object, object],
        allowed_memory_ids: set[int],
    ) -> _NormalizedDreamOutput:
        crystals: list[_NormalizedDreamCrystal] = []
        concepts: list[_NormalizedDreamConcept] = []
        facets: list[_NormalizedDreamFacet] = []
        warnings: list[DreamParseWarning] = []
        valid_concept_ids = self._valid_concept_ids()

        for index, item in enumerate(_list_from_payload(payload.get("concepts"))):
            concept = _normalize_dict_concept(item, f"concepts[{index}]", warnings)
            if concept is not None:
                concepts.append(concept)
        for index, item in enumerate(_list_from_payload(payload.get("facets"))):
            facet = _normalize_dict_facet(item, f"facets[{index}]", warnings)
            if facet is not None:
                facets.append(facet)

        for index, item in enumerate(_list_from_payload(payload.get("crystals"))):
            crystal = _normalize_dict_crystal(
                item,
                entry_path=f"crystals[{index}]",
                warnings=warnings,
                default_crystal_type="observation",
                default_source_credibility="observation",
                allowed_memory_ids=allowed_memory_ids,
                valid_concept_ids=valid_concept_ids,
            )
            if crystal is not None:
                crystals.append(crystal)
        for index, item in enumerate(_list_from_payload(payload.get("rule_crystals"))):
            crystal = _normalize_dict_crystal(
                item,
                entry_path=f"rule_crystals[{index}]",
                warnings=warnings,
                default_crystal_type="rule",
                default_source_credibility="user_rule",
                allowed_memory_ids=allowed_memory_ids,
                valid_concept_ids=valid_concept_ids,
            )
            if crystal is not None:
                crystals.append(crystal)
        for index, item in enumerate(_list_from_payload(payload.get("thoughts"))):
            crystal = _normalize_dict_crystal(
                item,
                entry_path=f"thoughts[{index}]",
                warnings=warnings,
                default_crystal_type="thought",
                default_source_credibility="thought",
                allowed_memory_ids=allowed_memory_ids,
                valid_concept_ids=valid_concept_ids,
                force_thought=True,
                force_inferred=True,
            )
            if crystal is not None:
                crystals.append(crystal)
        for index, item in enumerate(_list_from_payload(payload.get("inferred_additions"))):
            crystal = _normalize_dict_crystal(
                item,
                entry_path=f"inferred_additions[{index}]",
                warnings=warnings,
                default_crystal_type="thought",
                default_source_credibility="thought",
                allowed_memory_ids=allowed_memory_ids,
                valid_concept_ids=valid_concept_ids,
                force_thought=True,
                force_inferred=True,
            )
            if crystal is not None:
                crystals.append(crystal)

        supersede_actions = [
            action
            for action in (
                _normalize_supersede_action(item)
                for item in _list_from_payload(payload.get("supersede"))
            )
            if action is not None
        ]
        return _NormalizedDreamOutput(
            crystals=crystals,
            concepts=concepts,
            facets=facets,
            supersede_actions=supersede_actions,
            warnings=warnings,
        )

    def _valid_concept_ids(self) -> set[int]:
        with connect(self.config.database_path) as conn:
            rows = conn.execute(
                """
                select id
                from concepts
                where status not in ('archived', 'merged')
                """
            ).fetchall()
        return {int(row["id"]) for row in rows}

    def _resolve_candidate_concepts(
        self,
        conn,
        candidate: _NormalizedDreamCrystal,
        concept_ids_by_name: dict[str, int],
    ) -> _NormalizedDreamCrystal:
        concept_ids = set(candidate.concept_ids)
        for concept_name in candidate.concept_names:
            key = concept_name.casefold()
            concept_id = concept_ids_by_name.get(key)
            if concept_id is None:
                concept_id = self.concepts._create_or_reinforce_with_connection(
                    conn,
                    concept_name,
                    confidence_delta=0.2,
                    scope_type="global",
                    scope_key="",
                )
                concept_ids_by_name[key] = concept_id
            concept_ids.add(concept_id)
        if tuple(sorted(concept_ids)) == candidate.concept_ids:
            return candidate
        return _NormalizedDreamCrystal(
            crystal_type=candidate.crystal_type,
            title=candidate.title,
            text=candidate.text,
            strength=candidate.strength,
            confidence=candidate.confidence,
            source_memory_ids=candidate.source_memory_ids,
            source_credibility=candidate.source_credibility,
            rule_intent=candidate.rule_intent,
            malformed_penalty=candidate.malformed_penalty,
            is_inferred=candidate.is_inferred,
            supersedes_crystal_id=candidate.supersedes_crystal_id,
            story_scopes=candidate.story_scopes,
            semantic_tags=candidate.semantic_tags,
            concept_ids=tuple(sorted(concept_ids)),
            concept_names=candidate.concept_names,
        )

    def _next_cycle_id(self, conn) -> int:
        row = conn.execute(
            """
            select coalesce(max(cycle_id), 0) + 1
            from dream_runs
            where cycle_id > 0
            """
        ).fetchone()
        return int(row[0])

    def _next_skipped_cycle_id(self, conn) -> int:
        row = conn.execute(
            """
            select coalesce(min(cycle_id), 0) - 1
            from dream_runs
            where cycle_id < 0
            """
        ).fetchone()
        return int(row[0])

    def _validate_normalized_output(
        self,
        output: _NormalizedDreamOutput,
        context: TranslationContext,
        allowed_memory_ids: set[int],
    ) -> None:
        for candidate in output.crystals:
            if candidate.crystal_type not in _ALLOWED_CRYSTAL_TYPES:
                raise ValueError(f"unknown crystal_type: {candidate.crystal_type}")
            if not candidate.text.strip():
                raise ValueError("candidate text must not be empty")
            if not 0.0 <= candidate.strength <= 1.0:
                raise ValueError("candidate strength must be between 0 and 1")
            if not 0.0 <= candidate.confidence <= 1.0:
                raise ValueError("candidate confidence must be between 0 and 1")
            if candidate.source_memory_ids:
                unknown_ids = set(candidate.source_memory_ids) - allowed_memory_ids
                if unknown_ids:
                    raise ValueError(f"unknown source_memory_ids: {sorted(unknown_ids)}")

        for proposal in output.concept_proposals:
            if proposal.series_slug != context.series_slug:
                raise ValueError("proposal series_slug must match context")
            if proposal.source_language != context.source_language:
                raise ValueError("proposal source_language must match context")
            if proposal.target_language != context.target_language:
                raise ValueError("proposal target_language must match context")

        for facet in output.facets:
            if not facet.concept_name.strip():
                raise ValueError("facet concept_name must not be empty")
            if not facet.value.strip():
                raise ValueError("facet value must not be empty")
            if facet.kind not in VALID_FACET_KINDS:
                raise ValueError(f"unknown facet kind: {facet.kind}")
            if not 0.0 <= facet.confidence <= 1.0:
                raise ValueError("facet confidence must be between 0 and 1")

    def _validate_output(
        self,
        output: DreamOutput,
        context: TranslationContext,
        allowed_memory_ids: set[int],
    ) -> None:
        if not isinstance(output, DreamOutput):
            raise ValueError("provider output must be a DreamOutput")
        if not isinstance(output.crystals, list):
            raise ValueError("provider output crystals must be a list")
        if not isinstance(output.concept_proposals, list):
            raise ValueError("provider output concept_proposals must be a list")

        for candidate in output.crystals:
            if not isinstance(candidate, DreamCrystalCandidate):
                raise ValueError("provider crystals must contain only DreamCrystalCandidate items")
            if candidate.crystal_type not in _ALLOWED_CRYSTAL_TYPES:
                raise ValueError(f"unknown crystal_type: {candidate.crystal_type}")
            if not candidate.text.strip():
                raise ValueError("candidate text must not be empty")
            if not 0.0 <= candidate.strength <= 1.0:
                raise ValueError("candidate strength must be between 0 and 1")
            if not 0.0 <= candidate.confidence <= 1.0:
                raise ValueError("candidate confidence must be between 0 and 1")
            if not candidate.source_memory_ids:
                raise ValueError("candidate source_memory_ids must not be empty")
            unknown_ids = set(candidate.source_memory_ids) - allowed_memory_ids
            if unknown_ids:
                raise ValueError(f"unknown source_memory_ids: {sorted(unknown_ids)}")

        for proposal in output.concept_proposals:
            if not isinstance(proposal, DreamConceptProposal):
                raise ValueError(
                    "provider concept_proposals must contain only DreamConceptProposal items"
                )
            if proposal.series_slug != context.series_slug:
                raise ValueError("proposal series_slug must match context")
            if proposal.source_language != context.source_language:
                raise ValueError("proposal source_language must match context")
            if proposal.target_language != context.target_language:
                raise ValueError("proposal target_language must match context")
            if not isinstance(proposal.concept_text, str):
                raise ValueError("proposal concept_text must be a string")
            if not isinstance(proposal.source_form, str):
                raise ValueError("proposal source_form must be a string")
            if not isinstance(proposal.canonical_rendering, str):
                raise ValueError("proposal canonical_rendering must be a string")
            if not proposal.concept_text.strip():
                raise ValueError("proposal concept_text must not be empty")
            if not proposal.source_form.strip():
                raise ValueError("proposal source_form must not be empty")
            if not proposal.canonical_rendering.strip():
                raise ValueError("proposal canonical_rendering must not be empty")
            if not isinstance(proposal.approved_variants, list):
                raise ValueError("proposal approved_variants must be a list")
            if not all(isinstance(variant, str) for variant in proposal.approved_variants):
                raise ValueError("proposal approved_variants must contain only strings")
            if not isinstance(proposal.forbidden_variants, list):
                raise ValueError("proposal forbidden_variants must be a list")
            if not all(isinstance(variant, str) for variant in proposal.forbidden_variants):
                raise ValueError("proposal forbidden_variants must contain only strings")


def _normalize_candidate_text(text: str) -> str:
    chunks = [match.group(0).strip() for match in _SENTENCE_RE.finditer(text)]
    chunks = [chunk for chunk in chunks if chunk]
    if not chunks:
        return " ".join(text.split())
    return " ".join(chunks[:3])


def _normalize_dict_crystal(
    item: object,
    *,
    entry_path: str,
    warnings: list[DreamParseWarning],
    default_crystal_type: str,
    default_source_credibility: str,
    allowed_memory_ids: set[int],
    valid_concept_ids: set[int],
    force_thought: bool = False,
    force_inferred: bool = False,
) -> _NormalizedDreamCrystal | None:
    payload: dict[object, object]
    if isinstance(item, str):
        payload = {"text": item}
    elif type(item) is dict:
        payload = item
    else:
        return None

    text, penalty = _recover_crystal_text(payload)
    if penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_content_field",
            "used fallback content field",
            penalty,
        )
    crystal_type, kind_penalty = _recover_crystal_type(payload, default_crystal_type)
    if force_thought:
        crystal_type = "thought"
    penalty += kind_penalty
    if kind_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_type",
            "used fallback crystal type",
            kind_penalty,
        )
    source_credibility, credibility_penalty = _recover_source_credibility(
        payload,
        default_source_credibility,
    )
    if force_thought:
        source_credibility = "thought"
    penalty += credibility_penalty
    if credibility_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_source_credibility",
            "used fallback source credibility",
            credibility_penalty,
        )
    penalty += max(_numeric_field(payload.get("malformed_penalty"), default=0.0), 0.0)

    is_inferred = force_inferred or _bool_field(payload.get("is_inferred"), default=False)
    if is_inferred and not force_thought:
        crystal_type = "thought"
        source_credibility = "thought"
    story_scopes, story_scope_penalty = _recover_crystal_string_tuple(
        payload,
        entry_path,
        warnings,
        "malformed_crystal_story_scopes",
        "ignored malformed crystal story scope metadata",
        "story_scopes",
    )
    semantic_tags, semantic_tag_penalty = _recover_crystal_string_tuple(
        payload,
        entry_path,
        warnings,
        "malformed_crystal_semantic_tags",
        "ignored malformed crystal semantic tag metadata",
        "semantic_tags",
        "tags",
    )
    concept_ids, concept_id_penalty = _recover_crystal_int_tuple(
        payload,
        entry_path,
        warnings,
        "malformed_crystal_concept_ids",
        "ignored malformed crystal concept id metadata",
        "concept_ids",
        "concept_id",
        valid_ids=valid_concept_ids,
    )
    concept_names, concept_name_penalty = _concept_names_from_payload(payload, entry_path, warnings)
    penalty += (
        story_scope_penalty + semantic_tag_penalty + concept_id_penalty + concept_name_penalty
    )
    confidence = _normalized_confidence(payload, source_credibility, penalty)
    title = _string_field(payload.get("title")).strip() or _title_from_kind(crystal_type)
    source_memory_ids = _source_memory_ids(payload, allowed_memory_ids)
    if source_memory_ids is None:
        return None
    return _NormalizedDreamCrystal(
        crystal_type=crystal_type,
        title=title,
        text=text,
        strength=_clamp_score(_numeric_field(payload.get("strength"), default=0.5)),
        confidence=confidence,
        source_memory_ids=source_memory_ids,
        source_credibility=source_credibility,
        rule_intent=_string_field(payload.get("rule_intent")),
        malformed_penalty=penalty,
        is_inferred=is_inferred,
        supersedes_crystal_id=_optional_int(payload.get("supersedes_crystal_id")),
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        concept_ids=concept_ids,
        concept_names=concept_names,
    )


def _recover_crystal_text(payload: dict[object, object]) -> tuple[str, float]:
    for key in ("content", "text"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return (" ".join(value.split()), 0.0)
    value = payload.get("body")
    if isinstance(value, str) and value.strip():
        return (" ".join(value.split()), MALFORMED_CONFIDENCE_PENALTY)
    raise ValueError("dream candidate content is required")


def _recover_crystal_type(
    payload: dict[object, object],
    default_crystal_type: str,
) -> tuple[str, float]:
    penalty = 0.0
    if "crystal_type" in payload:
        value = payload["crystal_type"]
    elif "type" in payload:
        value = payload["type"]
    elif "kind" in payload:
        value = payload["kind"]
    else:
        return (default_crystal_type, 0.0)

    if not isinstance(value, str):
        return ("observation", penalty + MALFORMED_CONFIDENCE_PENALTY)

    crystal_type = value.strip().casefold().replace("-", "_")
    if crystal_type == "rule_crystal":
        return ("rule", penalty + MALFORMED_CONFIDENCE_PENALTY)
    if crystal_type in _ALLOWED_CRYSTAL_TYPES:
        return (crystal_type, penalty)
    return ("observation", penalty + MALFORMED_CONFIDENCE_PENALTY)


def _recover_source_credibility(
    payload: dict[object, object],
    default_source_credibility: str,
) -> tuple[str, float]:
    value = payload.get("source_credibility", default_source_credibility)
    if isinstance(value, str) and value.strip():
        return (value.strip(), 0.0)
    return (default_source_credibility, MALFORMED_CONFIDENCE_PENALTY)


def _normalized_confidence(
    payload: dict[object, object],
    source_credibility: str,
    penalty: float,
) -> float:
    if source_credibility in SOURCE_CREDIBILITY_CONFIDENCE:
        base = SOURCE_CREDIBILITY_CONFIDENCE[source_credibility]
    else:
        base = _numeric_field(
            payload.get("confidence"), default=SOURCE_CREDIBILITY_CONFIDENCE["observation"]
        )
    return min(max(base - penalty, _MIN_NORMALIZED_CONFIDENCE), 1.0)


def _normalize_dict_concept(
    item: object,
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> _NormalizedDreamConcept | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    penalty = 0.0
    name = _string_field(payload.get("canonical_name")).strip()
    if name == "":
        name = _string_field(payload.get("name")).strip()
    if name == "":
        name = _string_field(payload.get("label")).strip()
        if name:
            penalty += MALFORMED_CONFIDENCE_PENALTY
            _append_parse_warning(
                warnings,
                entry_path,
                "malformed_concept_name",
                "used fallback concept label",
                MALFORMED_CONFIDENCE_PENALTY,
            )
    if name == "":
        raise ValueError(f"{entry_path}.canonical_name is required")
    confidence = _numeric_field(payload.get("confidence"), default=0.2)
    confidence_delta = min(max(confidence - penalty, _MIN_NORMALIZED_CONFIDENCE), 1.0)
    return _NormalizedDreamConcept(
        canonical_name=name,
        description=_string_field(payload.get("description")),
        tags=_clean_string_tuple(payload.get("tags"), payload.get("semantic_tags")),
        confidence_delta=confidence_delta,
    )


def _normalize_dict_facet(
    item: object,
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> _NormalizedDreamFacet | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    value, content_penalty = _recover_facet_value(payload)
    if value is None:
        raise ValueError(f"{entry_path}.value is required")
    if content_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_value",
            "used fallback facet value field",
            content_penalty,
        )

    concept_name = _facet_concept_name_from_payload(payload, entry_path, warnings)
    if concept_name == "":
        raise ValueError(f"{entry_path}.concept_name is required")

    kind, kind_penalty = _recover_facet_kind(payload)
    language_tags, language_penalty = _recover_facet_string_tuple(
        payload,
        "language_tags",
        "language",
    )
    story_scopes, story_scope_penalty = _recover_facet_string_tuple(
        payload,
        "story_scopes",
        "story_scope",
    )
    semantic_tags, semantic_tag_penalty = _recover_facet_string_tuple(
        payload,
        "semantic_tags",
        "tags",
    )
    is_canonical, canonical_penalty = _recover_facet_canonical(payload)
    metadata_penalty = (
        kind_penalty
        + content_penalty
        + language_penalty
        + story_scope_penalty
        + semantic_tag_penalty
        + canonical_penalty
    )
    if kind_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_kind",
            "used fallback facet kind",
            kind_penalty,
        )
    if language_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_language_tags",
            "ignored malformed facet language metadata",
            language_penalty,
        )
    if story_scope_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_story_scopes",
            "ignored malformed facet story scope metadata",
            story_scope_penalty,
        )
    if semantic_tag_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_semantic_tags",
            "ignored malformed facet semantic tag metadata",
            semantic_tag_penalty,
        )
    if canonical_penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_canonical",
            "parsed non-boolean canonical metadata",
            canonical_penalty,
        )
    confidence = _numeric_field(payload.get("confidence"), default=0.2)
    return _NormalizedDreamFacet(
        concept_name=concept_name,
        value=value,
        kind=kind,
        language_tags=language_tags,
        story_scopes=story_scopes,
        semantic_tags=semantic_tags,
        confidence=min(max(confidence - metadata_penalty, _MIN_NORMALIZED_CONFIDENCE), 1.0),
        is_canonical=is_canonical,
    )


def _recover_facet_value(payload: dict[object, object]) -> tuple[str | None, float]:
    for key in ("content", "value", "text"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return (" ".join(value.split()), 0.0)
    value = payload.get("body")
    if isinstance(value, str) and value.strip():
        return (" ".join(value.split()), MALFORMED_CONFIDENCE_PENALTY)
    return (None, 0.0)


def _recover_facet_kind(payload: dict[object, object]) -> tuple[str, float]:
    if "kind" not in payload and "facet_type" not in payload:
        return ("note", 0.0)
    value = payload.get("kind", payload.get("facet_type"))
    if not isinstance(value, str) or not value.strip():
        return ("note", MALFORMED_CONFIDENCE_PENALTY)
    clean_kind = value.strip().casefold().replace("-", "_")
    if clean_kind in VALID_FACET_KINDS:
        return (clean_kind, 0.0)
    if clean_kind in {"alias", "former_label"}:
        return ("name", MALFORMED_CONFIDENCE_PENALTY)
    return ("note", MALFORMED_CONFIDENCE_PENALTY)


def _recover_facet_string_tuple(
    payload: dict[object, object],
    *keys: str,
) -> tuple[tuple[str, ...], float]:
    values: list[str] = []
    penalty = 0.0
    for key in keys:
        if key not in payload:
            continue
        value = payload[key]
        if isinstance(value, str):
            if value.strip():
                values.append(value)
            else:
                penalty += MALFORMED_CONFIDENCE_PENALTY
            continue
        if type(value) is list:
            for item in value:
                if isinstance(item, str) and item.strip():
                    values.append(item)
                else:
                    penalty += MALFORMED_CONFIDENCE_PENALTY
            continue
        penalty += MALFORMED_CONFIDENCE_PENALTY
    return (_clean_text_tuple(tuple(values)), penalty)


def _recover_facet_canonical(payload: dict[object, object]) -> tuple[bool, float]:
    if "is_canonical" in payload:
        return _parse_facet_canonical(payload["is_canonical"])
    if "canonical" in payload:
        return _parse_facet_canonical(payload["canonical"])
    return (False, 0.0)


def _parse_facet_canonical(value: object) -> tuple[bool, float]:
    if isinstance(value, bool):
        return (value, 0.0)
    if isinstance(value, str):
        clean_value = value.strip().casefold()
        if clean_value in {"true", "yes", "1"}:
            return (True, MALFORMED_CONFIDENCE_PENALTY)
        if clean_value in {"false", "no", "0"}:
            return (False, MALFORMED_CONFIDENCE_PENALTY)
        return (False, MALFORMED_CONFIDENCE_PENALTY)
    if isinstance(value, int) and value in {0, 1}:
        return (bool(value), MALFORMED_CONFIDENCE_PENALTY)
    return (False, MALFORMED_CONFIDENCE_PENALTY)


def _facet_concept_name_from_payload(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> str:
    for key in ("concept_name", "concept_label", "canonical_name"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    concept = payload.get("concept")
    if isinstance(concept, str) and concept.strip():
        return concept.strip()
    if type(concept) is dict:
        normalized = _normalize_dict_concept(concept, f"{entry_path}.concept", warnings)
        if normalized is not None:
            return normalized.canonical_name
    return ""


def _normalize_supersede_action(item: object) -> _DreamSupersedeAction | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    old_crystal_id = _optional_int(payload.get("old_crystal_id"))
    new_crystal_id = _optional_int(payload.get("new_crystal_id"))
    if old_crystal_id is None or new_crystal_id is None:
        return None
    return _DreamSupersedeAction(
        old_crystal_id=old_crystal_id,
        new_crystal_id=new_crystal_id,
        reason=_string_field(payload.get("reason")),
    )


def _normalize_score_action(item: object) -> tuple[int, float, float] | None:
    if type(item) is not dict:
        return None
    payload: dict[object, object] = item
    crystal_id = _optional_int(payload.get("crystal_id"))
    if crystal_id is None:
        return None
    return (
        crystal_id,
        _numeric_field(payload.get("strength_delta"), default=0.0),
        _numeric_field(payload.get("confidence_delta"), default=0.0),
    )


def _list_from_payload(value: object) -> list[object]:
    if value is None:
        return []
    if type(value) is list:
        return value
    return [value]


def _string_field(value: object) -> str:
    return value if isinstance(value, str) else ""


def _numeric_field(value: object, *, default: float) -> float:
    if isinstance(value, bool):
        return default
    if isinstance(value, int | float):
        return float(value)
    return default


def _bool_field(value: object, *, default: bool) -> bool:
    if isinstance(value, bool):
        return value
    return default


def _append_parse_warning(
    warnings: list[DreamParseWarning],
    entry_path: str,
    code: str,
    message: str,
    confidence_penalty: float,
) -> None:
    warnings.append(
        DreamParseWarning(
            entry_path=entry_path,
            code=code,
            message=message,
            confidence_penalty=confidence_penalty,
        )
    )


def _optional_int(value: object) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value.strip())
        except ValueError:
            return None
    return None


def _source_memory_ids(
    payload: dict[object, object],
    allowed_memory_ids: set[int],
) -> list[int] | None:
    has_source_field = "source_memory_ids" in payload or "source_memory_id" in payload
    clean_ids = [
        memory_id
        for memory_id in _clean_int_tuple(
            payload.get("source_memory_ids"), payload.get("source_memory_id")
        )
        if memory_id in allowed_memory_ids
    ]
    if clean_ids:
        return clean_ids
    if has_source_field:
        return None
    return sorted(allowed_memory_ids)


def _recover_crystal_string_tuple(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
    code: str,
    message: str,
    *keys: str,
) -> tuple[tuple[str, ...], float]:
    values, penalty = _recover_facet_string_tuple(payload, *keys)
    if penalty:
        _append_parse_warning(warnings, entry_path, code, message, penalty)
    return (values, penalty)


def _recover_crystal_int_tuple(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
    code: str,
    message: str,
    *keys: str,
    valid_ids: set[int] | None = None,
) -> tuple[tuple[int, ...], float]:
    integers: list[int] = []
    penalty = 0.0
    for key in keys:
        if key not in payload:
            continue
        value = payload[key]
        if isinstance(value, bool):
            penalty += MALFORMED_CONFIDENCE_PENALTY
        elif isinstance(value, int):
            integers.append(value)
        elif type(value) is list:
            for item in value:
                if isinstance(item, bool):
                    penalty += MALFORMED_CONFIDENCE_PENALTY
                elif isinstance(item, int):
                    integers.append(item)
                else:
                    penalty += MALFORMED_CONFIDENCE_PENALTY
        else:
            penalty += MALFORMED_CONFIDENCE_PENALTY
    if penalty:
        _append_parse_warning(warnings, entry_path, code, message, penalty)
    clean_ids = tuple(sorted(set(integers)))
    if valid_ids is None:
        return (clean_ids, penalty)
    valid = tuple(concept_id for concept_id in clean_ids if concept_id in valid_ids)
    invalid_count = len(clean_ids) - len(valid)
    if invalid_count:
        invalid_penalty = MALFORMED_CONFIDENCE_PENALTY * invalid_count
        _append_parse_warning(
            warnings,
            entry_path,
            "invalid_crystal_concept_ids",
            "ignored unknown or inactive crystal concept ids",
            invalid_penalty,
        )
        penalty += invalid_penalty
    return (valid, penalty)


def _concept_names_from_payload(
    payload: dict[object, object],
    entry_path: str,
    warnings: list[DreamParseWarning],
) -> tuple[tuple[str, ...], float]:
    values: list[object] = []
    for key in ("concept_names", "concepts"):
        value = payload.get(key)
        if type(value) is list:
            values.extend(value)
        elif value is not None:
            values.append(value)
    name = payload.get("concept_name")
    if name is not None:
        values.append(name)

    names: list[str] = []
    penalty = 0.0
    for value in values:
        if isinstance(value, str):
            if value.strip():
                names.append(value)
            else:
                penalty += MALFORMED_CONFIDENCE_PENALTY
        elif type(value) is dict:
            name, name_penalty = _recover_optional_concept_name(value)
            penalty += name_penalty
            if name:
                names.append(name)
        else:
            penalty += MALFORMED_CONFIDENCE_PENALTY
    if penalty:
        _append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_concept_metadata",
            "ignored malformed crystal concept metadata",
            penalty,
        )
    return (_clean_text_tuple(tuple(names)), penalty)


def _recover_optional_concept_name(payload: dict[object, object]) -> tuple[str, float]:
    name = _string_field(payload.get("canonical_name")).strip()
    if name:
        return (name, 0.0)
    name = _string_field(payload.get("name")).strip()
    if name:
        return (name, 0.0)
    name = _string_field(payload.get("label")).strip()
    if name:
        return (name, MALFORMED_CONFIDENCE_PENALTY)
    return ("", MALFORMED_CONFIDENCE_PENALTY)


def _clean_string_tuple(*values: object) -> tuple[str, ...]:
    strings: list[str] = []
    for value in values:
        if isinstance(value, str):
            strings.append(value)
        elif type(value) is list:
            strings.extend(item for item in value if isinstance(item, str))
    return _clean_text_tuple(tuple(strings))


def _clean_int_tuple(*values: object) -> tuple[int, ...]:
    integers: list[int] = []
    for value in values:
        if isinstance(value, bool):
            continue
        if isinstance(value, int):
            integers.append(value)
        elif type(value) is list:
            integers.extend(
                item for item in value if isinstance(item, int) and not isinstance(item, bool)
            )
    return tuple(sorted(set(integers)))


def _clean_text_tuple(values: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(sorted({value.strip() for value in values if value.strip()}))


def _title_from_kind(kind: str) -> str:
    words = re.findall(r"[A-Za-z0-9А-Яа-яЁё]+", kind.replace("-", " "))
    if not words:
        return ""
    return " ".join(word[:1].upper() + word[1:] for word in words[:4])


def _crystal_type_for_short_memory(memory: ShortTermMemoryRecord) -> str | None:
    if memory.source_credibility == "user_rule" or memory.rule_intent.strip():
        return "rule"
    return _ROLE_TYPES.get(memory.source_role)


def _confidence_for_short_memory(memory: ShortTermMemoryRecord) -> float:
    if memory.source_credibility in SOURCE_CREDIBILITY_CONFIDENCE:
        return SOURCE_CREDIBILITY_CONFIDENCE[memory.source_credibility]
    return _ROLE_CONFIDENCE[memory.source_role]
