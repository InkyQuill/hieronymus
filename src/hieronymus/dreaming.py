from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.db import apply_migration, connect
from hieronymus.memory_models import ShortTermMemoryRecord, TranslationContext

_ALLOWED_CRYSTAL_TYPES = frozenset({"lesson", "concept", "erudition"})
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


class DreamService:
    def __init__(self, config: HieronymusConfig, provider: DreamProvider) -> None:
        self.config = config
        self.provider = provider
        self.crystals = CrystalStore(config)
        with connect(self.config.database_path) as conn:
            apply_migration(conn, "global.sql")

    def run_cycle(self) -> DreamRunRecord:
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
            groups = self._load_completed_groups()
            outputs = [
                (context, session_id, memories, self.provider.crystallize(context, memories))
                for session_id, context, memories in groups
            ]
            for _context, _session_id, memories, output in outputs:
                self._validate_output(output, {memory.id for memory in memories})

            created_crystal_count = 0
            proposal_count = 0
            for context, _session_id, _memories, output in outputs:
                for candidate in output.crystals:
                    self.crystals.add_crystal(
                        context,
                        crystal_type=candidate.crystal_type,
                        title=candidate.title,
                        text=candidate.text,
                        strength=candidate.strength,
                        confidence=candidate.confidence,
                        source_memory_ids=candidate.source_memory_ids,
                    )
                    created_crystal_count += 1
                proposal_count += self._insert_concept_proposals(run_id, output.concept_proposals)

            with connect(self.config.database_path) as conn:
                session_ids = [session_id for session_id, _context, _memories in groups]
                for session_id in session_ids:
                    conn.execute(
                        """
                        update task_sessions
                        set status = 'dreamed',
                            cycle_id = ?
                        where id = ?
                        """,
                        (cycle_id, session_id),
                    )
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
                        sum(len(memories) for _sid, _context, memories in groups),
                        created_crystal_count,
                        proposal_count,
                        _now(),
                        run_id,
                    ),
                )
                conn.commit()
            return DreamRunRecord(
                id=run_id,
                cycle_id=cycle_id,
                status="completed",
                input_count=sum(len(memories) for _sid, _context, memories in groups),
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
                    (str(exc), _now(), run_id),
                )
                conn.commit()
            raise

    def _load_completed_groups(
        self,
    ) -> list[tuple[int, TranslationContext, list[ShortTermMemoryRecord]]]:
        with connect(self.config.database_path) as conn:
            session_rows = conn.execute(
                """
                select *
                from task_sessions
                where status = 'completed'
                  and cycle_id is null
                order by id
                """
            ).fetchall()
            groups = []
            for session in session_rows:
                memory_rows = conn.execute(
                    """
                    select *
                    from short_term_memories
                    where session_id = ?
                      and archived_at is null
                    order by id
                    """,
                    (session["id"],),
                ).fetchall()
                memories = [
                    ShortTermMemoryRecord(
                        id=row["id"],
                        session_id=row["session_id"],
                        source_role=row["source_role"],
                        kind=row["kind"],
                        text=row["text"],
                        source_ref=row["source_ref"],
                        metadata=json.loads(row["metadata_json"]),
                    )
                    for row in memory_rows
                ]
                groups.append(
                    (
                        int(session["id"]),
                        TranslationContext(
                            series_slug=session["series_slug"],
                            source_language=session["source_language"],
                            target_language=session["target_language"],
                            task_type=session["task_type"],
                            volume=session["volume"],
                            chapter=session["chapter"],
                        ),
                        memories,
                    )
                )
        return groups

    def _insert_concept_proposals(
        self,
        run_id: int,
        proposals: list[DreamConceptProposal],
    ) -> int:
        if not proposals:
            return 0
        now = _now()
        with connect(self.config.database_path) as conn:
            for proposal in proposals:
                conn.execute(
                    """
                    insert into strict_concept_proposals(
                      dream_run_id,
                      series_slug,
                      source_language,
                      target_language,
                      concept_text,
                      source_form,
                      canonical_rendering,
                      approved_variants_json,
                      forbidden_variants_json,
                      rationale,
                      status,
                      created_at,
                      updated_at
                    )
                    values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)
                    """,
                    (
                        run_id,
                        proposal.series_slug,
                        proposal.source_language,
                        proposal.target_language,
                        proposal.concept_text,
                        proposal.source_form,
                        proposal.canonical_rendering,
                        json.dumps(proposal.approved_variants, ensure_ascii=False),
                        json.dumps(proposal.forbidden_variants, ensure_ascii=False),
                        proposal.rationale,
                        now,
                        now,
                    ),
                )
            conn.commit()
        return len(proposals)

    def _next_cycle_id(self, conn) -> int:
        row = conn.execute("select coalesce(max(cycle_id), 0) + 1 from dream_runs").fetchone()
        return int(row[0])

    def _validate_output(self, output: DreamOutput, allowed_memory_ids: set[int]) -> None:
        if not isinstance(output, DreamOutput):
            raise ValueError("provider output must be a DreamOutput")

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
            if not proposal.concept_text.strip():
                raise ValueError("proposal concept_text must not be empty")
            if not proposal.source_form.strip():
                raise ValueError("proposal source_form must not be empty")
            if not proposal.canonical_rendering.strip():
                raise ValueError("proposal canonical_rendering must not be empty")


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
