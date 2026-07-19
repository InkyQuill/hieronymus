from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field, replace
from typing import Protocol

from hieronymus.concepts import ConceptProposalStore, ConceptStore
from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import connect, ensure_schema
from hieronymus.dream_audit import DreamAuditStore
from hieronymus.dream_config import load_dream_config
from hieronymus.dream_locks import DreamCycleAlreadyRunning, dream_cycle_lock
from hieronymus.dream_maintenance import DreamMaintenance
from hieronymus.dream_persistence import insert_concept_proposals, insert_crystal
from hieronymus.dream_validation import (
    DreamConceptProposal,
    DreamCrystalCandidate,
    DreamOutput,
    DreamParseWarning,
    _clean_int_tuple,
    _confidence_for_short_memory,
    _crystal_type_for_short_memory,
    _list_from_payload,
    _normalize_candidate_text,
    _normalize_score_action,
    _normalize_supersede_action,
    _recover_crystal_text,
    _string_field,
    _title_from_kind,
    normalize_dict_output,
    normalize_output,
    validate_normalized_output,
    validate_output,
)
from hieronymus.dream_validation import (
    NormalizedDreamCrystal as _NormalizedDreamCrystal,
)
from hieronymus.dream_validation import (
    NormalizedDreamOutput as _NormalizedDreamOutput,
)
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext
from hieronymus.provider_config import load_provider_catalog
from hieronymus.scoring import apply_score_delta
from hieronymus.secrets import redact_configured_secret_values
from hieronymus.values import clamp_score as _clamp_score
from hieronymus.values import utc_now as _now
from hieronymus.workspace import WorkspaceStore, short_memory_from_row

__all__ = (
    "DeterministicDreamProvider",
    "DreamConceptProposal",
    "DreamCrystalCandidate",
    "DreamOutput",
    "DreamRunRecord",
    "DreamService",
    "_recover_crystal_text",
)

STRENGTH_DECAY_PER_CYCLE = 0.03
CONFIDENCE_DECAY_AFTER_STRENGTH_BELOW = 0.20
CONFIDENCE_DECAY_PER_CYCLE = 0.01


def _trigger_type_from_owner(owner: str) -> str:
    if owner in {"manual", "admin"}:
        return "manual"
    if owner == "scheduler":
        return "scheduled"
    return owner


def _selected_memory_ids(groups: DreamBatch) -> tuple[int, ...]:
    return tuple(memory.id for _session_id, _context, memories in groups for memory in memories)


def _unique_ints(values: tuple[int, ...]) -> tuple[int, ...]:
    seen: set[int] = set()
    unique: list[int] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        unique.append(value)
    return tuple(unique)


def _summary_changed_crystal_ids(summaries: list[_DreamApplySummary]) -> tuple[int, ...]:
    return _unique_ints(
        tuple(
            crystal_id
            for summary in summaries
            for crystal_id in (
                *summary.created_crystal_ids,
                *summary.superseded_crystal_ids,
                *summary.reinforced_crystal_ids,
                *summary.decayed_crystal_ids,
            )
        )
    )


def _output_rejected_entries(outputs: list[DreamBatchOutput]) -> list[dict[str, object]]:
    return [entry for *_rest, output in outputs for entry in output.rejected_entries]


def _output_skipped_candidates(outputs: list[DreamBatchOutput]) -> list[dict[str, object]]:
    return [item for *_rest, output in outputs for item in output.skipped_candidates]


def _summary_has_activity(summary: _DreamApplySummary) -> bool:
    return any(
        (
            summary.created_crystal_ids,
            summary.created_concept_ids,
            summary.created_facet_ids,
            summary.created_links,
            summary.superseded_crystal_ids,
            summary.reinforced_crystal_ids,
            summary.decayed_crystal_ids,
            summary.rejected_entries,
            summary.skipped_candidates,
        )
    )


def _summary_output_count(summary: _DreamApplySummary) -> int:
    return sum(
        len(values)
        for values in (
            summary.created_crystal_ids,
            summary.created_concept_ids,
            summary.created_facet_ids,
            summary.created_links,
            summary.superseded_crystal_ids,
            summary.reinforced_crystal_ids,
            summary.decayed_crystal_ids,
        )
    )


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


@dataclass(frozen=True)
class _DreamApplySummary:
    created_crystal_ids: tuple[int, ...] = ()
    created_concept_ids: tuple[int, ...] = ()
    created_facet_ids: tuple[int, ...] = ()
    created_links: tuple[dict[str, int | str], ...] = ()
    superseded_crystal_ids: tuple[int, ...] = ()
    reinforced_crystal_ids: tuple[int, ...] = ()
    decayed_crystal_ids: tuple[int, ...] = ()
    rejected_entries: tuple[dict[str, object], ...] = ()
    skipped_candidates: tuple[dict[str, object], ...] = ()
    searched_related_candidates: dict[str, object] = field(default_factory=dict)
    affected_memory_set: dict[str, object] = field(default_factory=dict)


class DreamProvider(Protocol):
    name: str

    def crystallize(
        self,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> DreamOutput | dict[str, object]: ...


class EvidenceDreamProvider(Protocol):
    name: str

    def run_pass(
        self,
        pass_name: str,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> dict[str, object]: ...


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

    def run_pass(
        self,
        pass_name: str,
        context: TranslationContext,
        memories: list[ShortTermMemoryRecord],
    ) -> dict[str, object]:
        if pass_name == "coverage_audit":
            return {"covered_memory_ids": [memory.id for memory in memories]}
        if pass_name != "knowledge_crystals":
            return {}
        output = self.crystallize(context, memories)
        return {
            "crystals": [
                {
                    "crystal_type": candidate.crystal_type,
                    "title": candidate.title,
                    "text": candidate.text,
                    "strength": candidate.strength,
                    "confidence": candidate.confidence,
                    "source_memory_ids": candidate.source_memory_ids,
                    "source_credibility": candidate.source_credibility,
                    "rule_intent": candidate.rule_intent,
                }
                for candidate in output.crystals
            ]
        }


DreamBatch = list[tuple[int, TranslationContext, list[ShortTermMemoryRecord]]]
DreamBatchOutput = tuple[
    TranslationContext, int, list[ShortTermMemoryRecord], _NormalizedDreamOutput
]
DreamEventSink = Callable[[str, dict[str, object]], None]


class DreamService:
    def __init__(
        self,
        config: HieronymusConfig,
        provider: DreamProvider,
        max_short_term_memories_per_cycle: int | None = None,
        event_sink: DreamEventSink | None = None,
    ) -> None:
        self.config = config
        self.dream_config = load_dream_config(config)
        self.provider = provider
        self._event_sink = event_sink
        self._phase_event_payloads: dict[int, dict[str, object]] = {}
        self.max_short_term_memories_per_cycle = (
            max_short_term_memories_per_cycle
            if max_short_term_memories_per_cycle is not None
            else self.dream_config.max_short_term_memories_per_cycle
        )
        if self.max_short_term_memories_per_cycle < 1:
            raise ValueError("max_short_term_memories_per_cycle must be at least 1")
        self.concept_proposals = ConceptProposalStore(config)
        self.concepts = ConceptStore(config)
        self.crystals = CrystalStore(config)
        self.audit = DreamAuditStore(config)
        with connect(self.config.database_path) as conn:
            ensure_schema(conn)

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
        trigger_type: str | None = None,
    ) -> DreamRunRecord:
        lock_acquired = False
        try:
            with dream_cycle_lock(self.config, owner=owner, wait=wait):
                lock_acquired = True
                return self._run_cycle_unlocked(
                    max_batches=1,
                    trigger_type=trigger_type or _trigger_type_from_owner(owner),
                )
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
        trigger_type: str | None = None,
    ) -> DreamRunRecord:
        lock_acquired = False
        try:
            with dream_cycle_lock(self.config, owner=owner, wait=wait):
                lock_acquired = True
                return self._run_cycle_unlocked(
                    max_batches=None,
                    ignore_minimum=ignore_minimum,
                    trigger_type=trigger_type or _trigger_type_from_owner(owner),
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
        trigger_type: str,
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
        batch_apply_summaries: list[_DreamApplySummary] = []
        maintenance_phase_run_id: int | None = None
        try:
            if hasattr(self.provider, "run_pass"):
                return self._run_evidence_pass_cycle(
                    run_id=run_id,
                    cycle_id=cycle_id,
                    trigger_type=trigger_type,
                    ignore_minimum=ignore_minimum,
                )
            pending_count = self._pending_short_term_memory_count()
            minimum = self.dream_config.min_pending_short_term_memories
            threshold_state = self._threshold_state(
                pending_count=pending_count,
                ignore_minimum=ignore_minimum,
                trigger_type=trigger_type,
            )
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
                selected_memory_ids = _selected_memory_ids(groups)
                phase_run_id = self._start_phase_run(
                    run_id=run_id,
                    cycle_id=cycle_id,
                    phase="crystallization",
                    input_count=batch_input_count,
                )
                try:
                    self._audit_provider_request(
                        run_id=run_id,
                        phase_run_id=phase_run_id,
                        trigger_type=trigger_type,
                        threshold_state=threshold_state,
                        selected_memory_ids=selected_memory_ids,
                        groups=groups,
                    )
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
                        self._audit_parse_warnings(
                            run_id,
                            phase_run_id,
                            output.warnings,
                            trigger_type=trigger_type,
                            threshold_state=threshold_state,
                            selected_memory_ids=selected_memory_ids,
                        )
                        self._validate_normalized_output(
                            output,
                            context,
                            allowed_memory_ids,
                        )
                        outputs.append((context, session_id, memories, output))

                    apply_summary = self._apply_outputs(
                        run_id=run_id,
                        cycle_id=cycle_id,
                        groups=groups,
                        outputs=outputs,
                        archive_memories=True,
                    )
                    batch_created_crystals = len(apply_summary.created_crystal_ids)
                    batch_proposals = sum(
                        len(output.concept_proposals) for *_rest, output in outputs
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
                    self._audit_provider_response(
                        run_id=run_id,
                        phase_run_id=phase_run_id,
                        trigger_type=trigger_type,
                        threshold_state=threshold_state,
                        selected_memory_ids=selected_memory_ids,
                        outputs=outputs,
                    )
                    batch_apply_summaries.append(apply_summary)
                    self._audit_phase_completed(
                        run_id=run_id,
                        phase_run_id=phase_run_id,
                        trigger_type=trigger_type,
                        threshold_state=threshold_state,
                        selected_memory_ids=selected_memory_ids,
                        groups=groups,
                        outputs=outputs,
                        apply_summary=apply_summary,
                    )
                except Exception as exc:
                    self._fail_phase_run(phase_run_id, exc)
                    raise

                processed_batches += 1

            processed_session_ids.update(self._completed_session_ids_without_pending_memories())
            with connect(self.config.database_path) as conn:
                changed_ids_before_maintenance = _summary_changed_crystal_ids(batch_apply_summaries)
                remaining_changed_crystals = max(
                    0,
                    self.dream_config.max_changed_crystals_per_cycle
                    - len(changed_ids_before_maintenance),
                )
                if self._has_maintenance_candidates(conn, cycle_id):
                    maintenance_phase_run_id = self._start_phase_run(
                        run_id=run_id,
                        cycle_id=cycle_id,
                        phase="maintenance",
                        input_count=0,
                    )
                passive_summary = self._apply_passive_events(
                    conn,
                    cycle_id,
                    max_changed_crystals=remaining_changed_crystals,
                )
                remaining_changed_crystals = max(
                    0,
                    remaining_changed_crystals
                    - len(_summary_changed_crystal_ids([passive_summary])),
                )
                self._mark_activations_for_cycle(
                    conn,
                    cycle_id,
                    sorted(processed_session_ids),
                )
                decay_summary = self._apply_cycle_decay(
                    conn,
                    cycle_id,
                    max_changed_crystals=remaining_changed_crystals,
                )
                maintenance_summary = self._merge_apply_summaries(
                    conn,
                    passive_summary,
                    decay_summary,
                )
                self._mark_fully_archived_completed_sessions(
                    conn,
                    cycle_id,
                    sorted(processed_session_ids),
                )
                conn.commit()
            if maintenance_phase_run_id is not None:
                self._complete_phase_run(
                    phase_run_id=maintenance_phase_run_id,
                    output_count=_summary_output_count(maintenance_summary),
                )
                if _summary_has_activity(maintenance_summary):
                    self._audit_phase_completed(
                        run_id=run_id,
                        phase_run_id=maintenance_phase_run_id,
                        trigger_type=trigger_type,
                        threshold_state=threshold_state,
                        selected_memory_ids=(),
                        groups=[],
                        outputs=[],
                        apply_summary=maintenance_summary,
                        phase_name="maintenance",
                    )
            self._complete_run(
                run_id=run_id,
                input_count=input_count,
                created_crystal_count=created_crystal_count,
                proposal_count=proposal_count,
            )
            maintenance_phase_run_id = None
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
            if maintenance_phase_run_id is not None:
                with connect(self.config.database_path) as conn:
                    phase = conn.execute(
                        "select status from dream_phase_runs where id = ?",
                        (maintenance_phase_run_id,),
                    ).fetchone()
                if phase is not None and phase["status"] == "running":
                    self._fail_phase_run(maintenance_phase_run_id, exc)
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

    def _run_evidence_pass_cycle(
        self,
        *,
        run_id: int,
        cycle_id: int,
        trigger_type: str,
        ignore_minimum: bool,
    ) -> DreamRunRecord:
        from hieronymus.dream_workflows import DREAM_PASS_NAMES

        pending_count = self._pending_short_term_memory_count()
        if not ignore_minimum and pending_count < self.dream_config.min_pending_short_term_memories:
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
            )

        groups = self._load_pending_completed_groups(
            limit=self.dream_config.max_short_term_memories_per_run
        )
        if not groups:
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
            )
        selected_memory_ids = _selected_memory_ids(groups)
        allowed_memory_ids = set(selected_memory_ids)
        selection_context = groups[0][1]
        selected_memories = [
            memory for _session_id, _context, memories in groups for memory in memories
        ]
        staged_outputs: list[DreamBatchOutput] = []
        covered_memory_ids: set[int] = set()
        phase_run_ids: list[int] = []
        try:
            for pass_name in DREAM_PASS_NAMES:
                phase_run_id = self._start_phase_run(
                    run_id=run_id,
                    cycle_id=cycle_id,
                    phase=pass_name,
                    input_count=len(selected_memory_ids),
                )
                phase_run_ids.append(phase_run_id)
                pass_output_count = 0
                raw_output = self.provider.run_pass(pass_name, selection_context, selected_memories)
                if type(raw_output) is not dict:
                    raise ValueError(f"{pass_name} output must be an object")
                if pass_name == "coverage_audit":
                    covered = self._coverage_ids(raw_output, allowed_memory_ids)
                    covered_memory_ids.update(covered)
                    pass_output_count = len(covered)
                else:
                    output = self._normalize_dict_output(raw_output, allowed_memory_ids)
                    self._validate_normalized_output(
                        output,
                        selection_context,
                        allowed_memory_ids,
                    )
                    self._validate_pass_output(pass_name, output)
                    staged_outputs.append(
                        (selection_context, groups[0][0], selected_memories, output)
                    )
                    pass_output_count = _normalized_output_count(output)
                self._complete_phase_run(phase_run_id=phase_run_id, output_count=pass_output_count)
            missing = allowed_memory_ids - covered_memory_ids
            if missing:
                raise ValueError(
                    "coverage_incomplete: "
                    + ", ".join(str(memory_id) for memory_id in sorted(missing))
                )
            staged_record_count = sum(
                _normalized_output_count(output) for *_prefix, output in staged_outputs
            )
            if staged_record_count > self.dream_config.max_long_term_records_affected_per_run:
                raise ValueError("dream run exceeds max_long_term_records_affected_per_run")
            apply_summary = self._apply_outputs(
                run_id=run_id,
                cycle_id=cycle_id,
                groups=groups,
                outputs=_deduplicate_staged_outputs(staged_outputs),
                archive_memories=True,
            )
            processed_session_ids = [session_id for session_id, _context, _memories in groups]
            with connect(self.config.database_path) as conn:
                passive_summary = self._apply_passive_events(
                    conn,
                    cycle_id,
                    max_changed_crystals=self.dream_config.max_changed_crystals_per_cycle,
                )
                remaining = max(
                    0,
                    self.dream_config.max_changed_crystals_per_cycle
                    - len(_summary_changed_crystal_ids([passive_summary])),
                )
                self._apply_cycle_decay(
                    conn,
                    cycle_id,
                    max_changed_crystals=remaining,
                )
                self._mark_activations_for_cycle(conn, cycle_id, processed_session_ids)
                self._mark_fully_archived_completed_sessions(conn, cycle_id, processed_session_ids)
                conn.commit()
        except Exception as error:
            for phase_run_id in phase_run_ids:
                self._fail_phase_run(phase_run_id, error)
            raise
        created_crystal_count = len(apply_summary.created_crystal_ids)
        proposal_count = sum(len(output.concept_proposals) for *_prefix, output in staged_outputs)
        self._complete_run(
            run_id=run_id,
            input_count=len(selected_memory_ids),
            created_crystal_count=created_crystal_count,
            proposal_count=proposal_count,
        )
        return DreamRunRecord(
            id=run_id,
            cycle_id=cycle_id,
            status="completed",
            provider=self.provider.name,
            input_count=len(selected_memory_ids),
            created_crystal_count=created_crystal_count,
            proposal_count=proposal_count,
        )

    def _coverage_ids(self, payload: dict[str, object], allowed_memory_ids: set[int]) -> set[int]:
        raw_ids = payload.get("covered_memory_ids")
        if not isinstance(raw_ids, list) or not all(type(item) is int for item in raw_ids):
            raise ValueError("coverage_audit output requires covered_memory_ids")
        covered = set(raw_ids)
        unknown = covered - allowed_memory_ids
        if unknown:
            raise ValueError("coverage_audit references unknown source_memory_ids")
        return covered

    def _validate_pass_output(self, pass_name: str, output: _NormalizedDreamOutput) -> None:
        record_count = _normalized_output_count(output)
        limit = self.dream_config.workflows[pass_name].max_records_per_pass
        if pass_name == "relations":
            limit = min(limit, self.dream_config.max_relation_records_per_pass)
        if record_count > limit:
            raise ValueError(f"{pass_name} output exceeds max_records_per_pass")

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
            return redact_configured_secret_values(message, load_provider_catalog(self.config))
        except Exception:
            return message

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
        if limit < 1:
            return []
        groups_by_session: dict[int, tuple[TranslationContext, list[ShortTermMemoryRecord]]] = {}
        workspace = WorkspaceStore(self.config)
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

    def _start_phase_run(self, *, run_id: int, cycle_id: int, phase: str, input_count: int) -> int:
        now = _now()
        provider_profile = self._provider_profile()
        provider_type = self.provider.name
        model = self._provider_model()
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
                    provider_profile,
                    provider_type,
                    model,
                    input_count,
                    now,
                ),
            )
            phase_run_id = int(cursor.lastrowid)
            conn.commit()
        payload: dict[str, object] = {
            "run_id": run_id,
            "cycle_id": cycle_id,
            "phase_run_id": phase_run_id,
            "phase": phase,
            "status": "running",
            "input_count": input_count,
        }
        self._phase_event_payloads[phase_run_id] = payload
        self._publish_event("dream_phase_progress", payload)
        return phase_run_id

    def _complete_phase_run(self, *, phase_run_id: int, output_count: int) -> None:
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                update dream_phase_runs
                set status = 'completed',
                    output_count = ?,
                    completed_at = ?
                where id = ? and status = 'running'
                """,
                (output_count, _now(), phase_run_id),
            )
            conn.commit()
        if cursor.rowcount:
            self._publish_phase_transition(
                phase_run_id,
                status="completed",
                terminal=True,
                output_count=output_count,
            )

    def _fail_phase_run(self, phase_run_id: int, error: Exception) -> None:
        redacted_error = self._redacted_error_message(error)
        with connect(self.config.database_path) as conn:
            cursor = conn.execute(
                """
                update dream_phase_runs
                set status = 'failed',
                    error = ?,
                    completed_at = ?
                where id = ? and status = 'running'
                """,
                (redacted_error, _now(), phase_run_id),
            )
            conn.commit()
        if cursor.rowcount:
            self._publish_phase_transition(
                phase_run_id,
                status="failed",
                terminal=True,
                error=self._safe_event_error_message(error),
            )

    def _publish_phase_transition(
        self,
        phase_run_id: int,
        *,
        status: str,
        terminal: bool = False,
        **details: object,
    ) -> None:
        started = (
            self._phase_event_payloads.pop(phase_run_id, None)
            if terminal
            else self._phase_event_payloads.get(phase_run_id)
        )
        if started is None:
            return
        self._publish_event(
            "dream_phase_progress",
            {**started, "status": status, **details},
        )

    def _publish_event(self, event_type: str, payload: dict[str, object]) -> None:
        if self._event_sink is None:
            return
        try:
            self._event_sink(event_type, payload)
        except Exception:
            return

    def _safe_event_error_message(self, error: Exception) -> str:
        try:
            return redact_configured_secret_values(str(error), load_provider_catalog(self.config))
        except Exception:
            return "dreaming failed"

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
    ) -> _DreamApplySummary:
        created_crystal_ids: list[int] = []
        created_concept_ids: list[int] = []
        created_facet_ids: list[int] = []
        created_links: list[dict[str, int | str]] = []
        superseded_crystal_ids: list[int] = []
        reinforced_crystal_ids: list[int] = []
        reinforced_action_ids: set[int] = set()
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
                        created_concept_ids.append(concept_id)

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
                        facet_id = self.concepts._add_facet_with_connection(
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
                        created_facet_ids.append(facet_id)

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
                        created_crystal_ids.append(crystal_id)
                        created_links.extend(
                            {
                                "crystal_id": crystal_id,
                                "concept_id": concept_id,
                                "link_type": "mentions",
                            }
                            for concept_id in candidate.concept_ids
                        )
                    self._insert_concept_proposals(conn, run_id, output.concept_proposals)
                    for crystal_id, strength_delta, confidence_delta in output.reinforce_actions:
                        if crystal_id in reinforced_action_ids:
                            continue
                        reinforced_action_ids.add(crystal_id)
                        reinforced_crystal_ids.extend(
                            self._apply_score_maintenance_with_connection(
                                conn,
                                crystal_ids=(crystal_id,),
                                strength_delta=strength_delta,
                                confidence_delta=confidence_delta,
                                reason="dream reinforcement",
                                event_type="dream_reinforce",
                                cycle_id=cycle_id,
                                skip_active_rules=False,
                            )
                        )
                    for action in output.supersede_actions:
                        self.crystals._supersede_with_connection(
                            conn,
                            old_crystal_id=action.old_crystal_id,
                            new_crystal_id=action.new_crystal_id,
                            reason=action.reason,
                            cycle_id=cycle_id,
                        )
                        superseded_crystal_ids.append(action.old_crystal_id)

                related_candidates = self._searched_related_candidates(
                    conn,
                    created_concept_ids=tuple(created_concept_ids),
                )
                affected_memory_set = self._affected_memory_set(
                    changed_crystal_ids=tuple([*created_crystal_ids, *superseded_crystal_ids]),
                    related_candidates=related_candidates,
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
        return _DreamApplySummary(
            created_crystal_ids=tuple(created_crystal_ids),
            created_concept_ids=tuple(created_concept_ids),
            created_facet_ids=tuple(created_facet_ids),
            created_links=tuple(created_links),
            superseded_crystal_ids=tuple(superseded_crystal_ids),
            reinforced_crystal_ids=tuple(reinforced_crystal_ids),
            rejected_entries=tuple(_output_rejected_entries(outputs)),
            skipped_candidates=tuple(_output_skipped_candidates(outputs)),
            searched_related_candidates=related_candidates,
            affected_memory_set=affected_memory_set,
        )

    def _merge_apply_summaries(
        self,
        conn,
        *summaries: _DreamApplySummary,
    ) -> _DreamApplySummary:
        created_concept_ids = tuple(
            concept_id for summary in summaries for concept_id in summary.created_concept_ids
        )
        related_candidates = self._searched_related_candidates(
            conn,
            created_concept_ids=created_concept_ids,
        )
        changed_crystal_ids = _summary_changed_crystal_ids(list(summaries))
        return _DreamApplySummary(
            created_crystal_ids=_unique_ints(
                tuple(
                    crystal_id
                    for summary in summaries
                    for crystal_id in summary.created_crystal_ids
                )
            ),
            created_concept_ids=_unique_ints(created_concept_ids),
            created_facet_ids=_unique_ints(
                tuple(facet_id for summary in summaries for facet_id in summary.created_facet_ids)
            ),
            created_links=tuple(link for summary in summaries for link in summary.created_links),
            superseded_crystal_ids=_unique_ints(
                tuple(
                    crystal_id
                    for summary in summaries
                    for crystal_id in summary.superseded_crystal_ids
                )
            ),
            reinforced_crystal_ids=_unique_ints(
                tuple(
                    crystal_id
                    for summary in summaries
                    for crystal_id in summary.reinforced_crystal_ids
                )
            ),
            decayed_crystal_ids=_unique_ints(
                tuple(
                    crystal_id
                    for summary in summaries
                    for crystal_id in summary.decayed_crystal_ids
                )
            ),
            rejected_entries=tuple(
                entry for summary in summaries for entry in summary.rejected_entries
            ),
            skipped_candidates=tuple(
                item for summary in summaries for item in summary.skipped_candidates
            ),
            searched_related_candidates=related_candidates,
            affected_memory_set=self._affected_memory_set(
                changed_crystal_ids=changed_crystal_ids,
                related_candidates=related_candidates,
            ),
        )

    def _searched_related_candidates(
        self,
        conn,
        *,
        created_concept_ids: tuple[int, ...],
    ) -> dict[str, object]:
        concept_ids = _unique_ints(created_concept_ids)[
            : self.dream_config.max_related_concepts_per_cycle
        ]
        crystals_by_concept: list[dict[str, object]] = []
        for concept_id in concept_ids:
            rows = conn.execute(
                """
                select crystal_id
                from crystal_concepts
                where concept_id = ?
                order by crystal_id
                limit ?
                """,
                (concept_id, self.dream_config.max_related_crystals_per_concept),
            ).fetchall()
            crystals_by_concept.append(
                {
                    "concept_id": concept_id,
                    "crystal_ids": [int(row["crystal_id"]) for row in rows],
                }
            )
        return {
            "concept_ids": list(concept_ids),
            "crystals_by_concept": crystals_by_concept,
            "caps": {
                "max_related_concepts_per_cycle": (
                    self.dream_config.max_related_concepts_per_cycle
                ),
                "max_related_crystals_per_concept": (
                    self.dream_config.max_related_crystals_per_concept
                ),
            },
        }

    def _affected_memory_set(
        self,
        *,
        changed_crystal_ids: tuple[int, ...],
        related_candidates: dict[str, object],
    ) -> dict[str, object]:
        changed_ids = _unique_ints(changed_crystal_ids)[
            : self.dream_config.max_changed_crystals_per_cycle
        ]
        related_ids: list[int] = []
        crystals_by_concept = related_candidates.get("crystals_by_concept")
        if isinstance(crystals_by_concept, list):
            for item in crystals_by_concept:
                if not isinstance(item, dict):
                    continue
                crystal_ids = item.get("crystal_ids")
                if not isinstance(crystal_ids, list):
                    continue
                related_ids.extend(
                    crystal_id for crystal_id in crystal_ids if type(crystal_id) is int
                )
        related_ids = [
            crystal_id
            for crystal_id in _unique_ints(tuple(related_ids))
            if crystal_id not in changed_ids
        ]

        affected_ids: list[int] = []
        for crystal_id in (*changed_ids, *related_ids):
            if len(affected_ids) >= self.dream_config.max_total_affected_crystals:
                break
            affected_ids.append(crystal_id)
        capped_changed_ids = [
            crystal_id for crystal_id in changed_ids if crystal_id in set(affected_ids)
        ]
        capped_related_ids = [
            crystal_id for crystal_id in related_ids if crystal_id in set(affected_ids)
        ]
        return {
            "changed_crystal_ids": capped_changed_ids,
            "related_crystal_ids": capped_related_ids,
            "all_crystal_ids": affected_ids,
            "total_crystal_count": len(affected_ids),
            "caps": {
                "max_changed_crystals_per_cycle": (
                    self.dream_config.max_changed_crystals_per_cycle
                ),
                "max_total_affected_crystals": self.dream_config.max_total_affected_crystals,
            },
        }

    def _audit_provider_request(
        self,
        *,
        run_id: int,
        phase_run_id: int,
        trigger_type: str,
        threshold_state: dict[str, object],
        selected_memory_ids: tuple[int, ...],
        groups: DreamBatch,
    ) -> None:
        self.audit.append(
            dream_run_id=run_id,
            phase_run_id=phase_run_id,
            event_type="provider_request",
            severity="info",
            summary="sent crystallization request",
            payload={
                "trigger_type": trigger_type,
                "threshold_state": threshold_state,
                "selected_short_term_memory_ids": list(selected_memory_ids),
                "phase_name": "crystallization",
                "prompt_version": self._prompt_version("crystallization"),
                "provider_profile": self._provider_profile(),
                "model": self._provider_model(),
                "request_summary": self._request_summary(groups),
            },
        )

    def _audit_provider_response(
        self,
        *,
        run_id: int,
        phase_run_id: int,
        trigger_type: str,
        threshold_state: dict[str, object],
        selected_memory_ids: tuple[int, ...],
        outputs: list[DreamBatchOutput],
    ) -> None:
        self.audit.append(
            dream_run_id=run_id,
            phase_run_id=phase_run_id,
            event_type="provider_response",
            severity="info",
            summary="received crystallization response",
            payload={
                "trigger_type": trigger_type,
                "threshold_state": threshold_state,
                "selected_short_term_memory_ids": list(selected_memory_ids),
                "phase_name": "crystallization",
                "prompt_version": self._prompt_version("crystallization"),
                "provider_profile": self._provider_profile(),
                "model": self._provider_model(),
                "response_summary": self._response_summary(outputs),
                "parse_warnings": self._parse_warning_payload(outputs),
            },
        )

    def _audit_phase_completed(
        self,
        *,
        run_id: int,
        phase_run_id: int,
        trigger_type: str,
        threshold_state: dict[str, object],
        selected_memory_ids: tuple[int, ...],
        groups: DreamBatch,
        outputs: list[DreamBatchOutput],
        apply_summary: _DreamApplySummary,
        phase_name: str = "crystallization",
    ) -> None:
        self.audit.append(
            dream_run_id=run_id,
            phase_run_id=phase_run_id,
            event_type="phase_completed",
            severity="info",
            summary=f"completed {phase_name} phase",
            payload={
                "trigger_type": trigger_type,
                "threshold_state": threshold_state,
                "selected_short_term_memory_ids": list(selected_memory_ids),
                "phase_name": phase_name,
                "prompt_version": self._prompt_version(phase_name),
                "provider_profile": self._provider_profile(),
                "model": self._provider_model(),
                "request_summary": self._request_summary(groups),
                "response_summary": self._response_summary(outputs),
                "parse_warnings": self._parse_warning_payload(outputs),
                "accepted_entries": self._accepted_entries(outputs),
                "rejected_entries": list(apply_summary.rejected_entries),
                "confidence_penalties": self._confidence_penalties(outputs),
                "created_crystals": list(apply_summary.created_crystal_ids),
                "created_concepts": list(apply_summary.created_concept_ids),
                "created_facets": list(apply_summary.created_facet_ids),
                "created_links": list(apply_summary.created_links),
                "superseded_crystals": list(apply_summary.superseded_crystal_ids),
                "reinforced_crystals": list(apply_summary.reinforced_crystal_ids),
                "decayed_crystals": list(apply_summary.decayed_crystal_ids),
                "searched_related_candidates": apply_summary.searched_related_candidates,
                "affected_memory_set": apply_summary.affected_memory_set,
                "skipped_candidates": list(apply_summary.skipped_candidates),
            },
        )

    def _audit_parse_warnings(
        self,
        run_id: int,
        phase_run_id: int,
        warnings: list[DreamParseWarning],
        *,
        trigger_type: str = "",
        threshold_state: dict[str, object] | None = None,
        selected_memory_ids: tuple[int, ...] = (),
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
                "trigger_type": trigger_type,
                "threshold_state": threshold_state or {},
                "selected_short_term_memory_ids": list(selected_memory_ids),
                "phase_name": "crystallization",
                "prompt_version": self._prompt_version("crystallization"),
                "provider_profile": self._provider_profile(),
                "model": self._provider_model(),
                "warnings": [
                    {
                        "entry_path": warning.entry_path,
                        "code": warning.code,
                        "message": warning.message,
                        "confidence_penalty": warning.confidence_penalty,
                    }
                    for warning in warnings
                ],
            },
        )

    def _threshold_state(
        self,
        *,
        pending_count: int,
        ignore_minimum: bool,
        trigger_type: str,
    ) -> dict[str, object]:
        minimum = self.dream_config.min_pending_short_term_memories
        maximum = self.dream_config.max_pending_short_term_memories
        return {
            "pending_short_term_memories": pending_count,
            "min_pending_short_term_memories": minimum,
            "max_pending_short_term_memories": maximum,
            "max_short_term_memories_per_cycle": self.max_short_term_memories_per_cycle,
            "minimum_met": pending_count >= minimum,
            "urgent_threshold_met": pending_count >= maximum,
            "ignore_minimum": ignore_minimum,
            "stale_cycle_override": trigger_type == "backlog_escape",
            "not_enough_memories_cycle_threshold": (
                self.dream_config.not_enough_memories_cycle_threshold
            ),
        }

    def _request_summary(self, groups: DreamBatch) -> dict[str, object]:
        return {
            "memory_count": sum(len(memories) for _session_id, _context, memories in groups),
            "session_ids": [session_id for session_id, _context, _memories in groups],
            "context_count": len(groups),
            "batch_cap": self.max_short_term_memories_per_cycle,
        }

    def _response_summary(self, outputs: list[DreamBatchOutput]) -> dict[str, object]:
        accepted = self._accepted_entries(outputs)
        return {
            "crystal_count": accepted["crystals"],
            "concept_count": accepted["concepts"],
            "facet_count": accepted["facets"],
            "concept_proposal_count": accepted["concept_proposals"],
            "supersede_action_count": accepted["supersede_actions"],
            "parse_warning_count": len(self._parse_warning_payload(outputs)),
        }

    def _accepted_entries(self, outputs: list[DreamBatchOutput]) -> dict[str, int]:
        return {
            "crystals": sum(len(output.crystals) for *_rest, output in outputs),
            "concepts": sum(len(output.concepts) for *_rest, output in outputs),
            "facets": sum(len(output.facets) for *_rest, output in outputs),
            "concept_proposals": sum(len(output.concept_proposals) for *_rest, output in outputs),
            "supersede_actions": sum(len(output.supersede_actions) for *_rest, output in outputs),
        }

    def _parse_warning_payload(
        self,
        outputs: list[DreamBatchOutput],
    ) -> list[dict[str, object]]:
        return [
            {
                "entry_path": warning.entry_path,
                "code": warning.code,
                "message": warning.message,
                "confidence_penalty": warning.confidence_penalty,
            }
            for *_rest, output in outputs
            for warning in output.warnings
        ]

    def _confidence_penalties(
        self,
        outputs: list[DreamBatchOutput],
    ) -> list[dict[str, object]]:
        return [
            {
                "entry_path": warning.entry_path,
                "code": warning.code,
                "confidence_penalty": warning.confidence_penalty,
            }
            for *_rest, output in outputs
            for warning in output.warnings
            if warning.confidence_penalty > 0
        ]

    def _provider_profile(self) -> str:
        profile_name = getattr(self.provider, "profile_name", "")
        if isinstance(profile_name, str) and profile_name.strip():
            return profile_name.strip()
        return self.provider.name

    def _provider_model(self) -> str:
        settings = getattr(self.provider, "settings", None)
        model = getattr(settings, "model", "")
        if isinstance(model, str) and model.strip():
            return model.strip()
        return self.provider.name

    def _prompt_version(self, phase: str) -> str:
        return f"{phase}:v1"

    # Compatibility seams retained for callers that instrument dream orchestration. The
    # implementation and transaction ownership live in DreamMaintenance.
    def _has_maintenance_candidates(self, conn, cycle_id: int) -> bool:
        return DreamMaintenance(conn).has_candidates(cycle_id)

    def _apply_passive_events(
        self,
        conn,
        cycle_id: int,
        *,
        max_changed_crystals: int,
    ):
        return DreamMaintenance(conn).apply_passive_events(
            cycle_id,
            max_changed_crystals=max_changed_crystals,
        )

    def _mark_activations_for_cycle(
        self,
        conn,
        cycle_id: int,
        session_ids: list[int],
    ) -> None:
        DreamMaintenance(conn).mark_activations_for_cycle(cycle_id, session_ids)

    def _apply_cycle_decay(
        self,
        conn,
        cycle_id: int,
        *,
        max_changed_crystals: int,
    ):
        return DreamMaintenance(conn).apply_cycle_decay(
            cycle_id,
            max_changed_crystals=max_changed_crystals,
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
        return insert_crystal(conn, context, candidate)

    def _insert_concept_proposals(
        self,
        conn,
        run_id: int,
        proposals: list[DreamConceptProposal],
    ) -> None:
        insert_concept_proposals(conn, self.concept_proposals, run_id, proposals)

    def _normalize_output(
        self,
        raw_output: object,
        context: TranslationContext,
        allowed_memory_ids: set[int],
    ) -> _NormalizedDreamOutput:
        return normalize_output(
            raw_output,
            context,
            allowed_memory_ids,
            self._valid_concept_ids(),
        )

    def _normalize_dict_output(
        self,
        payload: dict[object, object],
        allowed_memory_ids: set[int],
    ) -> _NormalizedDreamOutput:
        return normalize_dict_output(
            payload,
            allowed_memory_ids,
            self._valid_concept_ids(),
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
        validate_normalized_output(output, context, allowed_memory_ids)

    def _validate_output(
        self,
        output: DreamOutput,
        context: TranslationContext,
        allowed_memory_ids: set[int],
    ) -> None:
        validate_output(output, context, allowed_memory_ids)


def _normalized_output_count(output: _NormalizedDreamOutput) -> int:
    return (
        len(output.crystals)
        + len(output.concept_proposals)
        + len(output.concepts)
        + len(output.facets)
        + len(output.supersede_actions)
        + len(output.reinforce_actions)
    )


def _deduplicate_staged_outputs(outputs: list[DreamBatchOutput]) -> list[DreamBatchOutput]:
    seen_crystals: set[tuple[str, str, str]] = set()
    seen_concepts: set[str] = set()
    result: list[DreamBatchOutput] = []
    for context, session_id, memories, output in outputs:
        crystals = []
        for crystal in output.crystals:
            key = (crystal.crystal_type, crystal.title.casefold(), crystal.text.casefold())
            if key not in seen_crystals:
                seen_crystals.add(key)
                crystals.append(crystal)
        concepts = []
        for concept in output.concepts:
            key = concept.canonical_name.casefold()
            if key not in seen_concepts:
                seen_concepts.add(key)
                concepts.append(concept)
        result.append(
            (context, session_id, memories, replace(output, crystals=crystals, concepts=concepts))
        )
    return result
