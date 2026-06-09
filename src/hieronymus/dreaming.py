from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Protocol

from hieronymus.concepts import ConceptProposalStore
from hieronymus.config import HieronymusConfig
from hieronymus.db import apply_migration, connect
from hieronymus.dream_config import load_dream_config
from hieronymus.dream_locks import DreamCycleAlreadyRunning, dream_cycle_lock
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
from hieronymus.scoring import PASSIVE_EVENT_DELTAS
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.settings import SettingsError, load_settings

_ALLOWED_CRYSTAL_TYPES = frozenset({"lesson", "concept", "erudition"})
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
    ) -> DreamOutput: ...


class DeterministicDreamProvider:
    name = "deterministic"

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput:
        candidates = []
        for memory in memories:
            crystal_type = _ROLE_TYPES.get(memory.source_role)
            if crystal_type is None:
                continue
            candidates.append(
                DreamCrystalCandidate(
                    crystal_type=crystal_type,
                    title=_title_from_kind(memory.kind),
                    text=_normalize_candidate_text(memory.text),
                    strength=0.6,
                    confidence=_ROLE_CONFIDENCE[memory.source_role],
                    source_memory_ids=[memory.id],
                )
            )
        return DreamOutput(crystals=candidates, concept_proposals=[])


DreamBatch = list[tuple[int, TranslationContext, list[ShortTermMemoryRecord]]]
DreamBatchOutput = tuple[TranslationContext, int, list[ShortTermMemoryRecord], DreamOutput]


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
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

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

            input_count = 0
            created_crystal_count = 0
            proposal_count = 0
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
                    outputs = [
                        (
                            context,
                            session_id,
                            memories,
                            self.provider.crystallize(context, memories),
                        )
                        for session_id, context, memories in groups
                    ]
                    for context, _session_id, memories, output in outputs:
                        self._validate_output(output, context, {memory.id for memory in memories})

                    batch_created_crystals, batch_proposals = self._apply_outputs(
                        run_id=run_id,
                        cycle_id=cycle_id,
                        groups=groups,
                        outputs=outputs,
                        archive_memories=True,
                    )
                    self._complete_phase_run(
                        phase_run_id=phase_run_id,
                        output_count=batch_created_crystals + batch_proposals,
                    )
                except Exception as exc:
                    self._fail_phase_run(phase_run_id, exc)
                    raise

                input_count += batch_input_count
                created_crystal_count += batch_created_crystals
                proposal_count += batch_proposals
                processed_session_ids.update(
                    session_id for session_id, _context, _memories in groups
                )
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
                        error = ?,
                        completed_at = ?
                    where id = ?
                    """,
                    (self._redacted_error_message(exc), _now(), run_id),
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

    def _load_pending_completed_groups(
        self,
        *,
        limit: int,
    ) -> DreamBatch:
        with connect(self.config.database_path) as conn:
            memory_rows = conn.execute(
                """
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
                  and short_term_memories.archived_at is null
                order by task_sessions.id, short_term_memories.id
                limit ?
                """,
                (limit,),
            ).fetchall()

        groups_by_session: dict[int, tuple[TranslationContext, list[ShortTermMemoryRecord]]] = {}
        for row in memory_rows:
            session_id = int(row["session_id"])
            group = groups_by_session.get(session_id)
            if group is None:
                group = (
                    TranslationContext(
                        series_slug=row["series_slug"],
                        source_language=row["source_language"],
                        target_language=row["target_language"],
                        task_type=row["task_type"],
                        volume=row["volume"],
                        chapter=row["chapter"],
                    ),
                    [],
                )
                groups_by_session[session_id] = group
            group[1].append(
                ShortTermMemoryRecord(
                    id=row["id"],
                    session_id=row["session_id"],
                    source_role=row["source_role"],
                    kind=row["kind"],
                    text=row["text"],
                    source_ref=row["source_ref"],
                    metadata=json.loads(row["metadata_json"]),
                )
            )

        return [
            (session_id, context, memories)
            for session_id, (context, memories) in groups_by_session.items()
        ]

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
        with connect(self.config.database_path) as conn:
            try:
                for context, _session_id, _memories, output in outputs:
                    for candidate in output.crystals:
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

                if archive_memories:
                    self._archive_memories(conn, groups)
                conn.commit()
            except Exception:
                conn.rollback()
                raise
        return created_crystal_count, proposal_count

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
            select id, strength, confidence
            from crystals
            where status in ('active', 'candidate')
              and created_cycle != ?
              and coalesce(last_activated_cycle, -1) != ?
              and coalesce(last_reinforced_cycle, -1) != ?
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
            confidence = _clamp_score(original_confidence - confidence_delta)
            conn.execute(
                """
                update crystals
                set strength = ?,
                    confidence = ?,
                    updated_at = ?
                where id = ?
                """,
                (strength, confidence, now, crystal["id"]),
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
        candidate: DreamCrystalCandidate,
    ) -> int:
        return self._insert_crystal(conn, context, candidate)

    def _insert_crystal(
        self,
        conn,
        context: TranslationContext,
        candidate: DreamCrystalCandidate,
    ) -> int:
        now = _now()
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
            values (?, ?, ?, 'series', ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)
            """,
            (
                candidate.crystal_type,
                candidate.text,
                candidate.title,
                context.scope_key,
                context.series_slug,
                context.source_language,
                context.target_language,
                json.dumps(list(context.tags), ensure_ascii=False, sort_keys=True),
                candidate.strength,
                candidate.confidence,
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


def _title_from_kind(kind: str) -> str:
    words = re.findall(r"[A-Za-z0-9А-Яа-яЁё]+", kind.replace("-", " "))
    if not words:
        return ""
    return " ".join(word[:1].upper() + word[1:] for word in words[:4])
