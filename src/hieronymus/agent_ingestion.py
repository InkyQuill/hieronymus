from __future__ import annotations

import re
from dataclasses import dataclass

from hieronymus.config import HieronymusConfig
from hieronymus.workspace import WorkspaceStore

TERM_PATTERN = re.compile(r"\b[A-Z][A-Za-z0-9'_-]{2,}\b")
SENTENCE_BOUNDARY_PATTERN = re.compile(r"(?<=[.!?。！？])\s+")


@dataclass(frozen=True)
class LearningBlock:
    index: int
    text: str


@dataclass(frozen=True)
class LearnResult:
    session_id: int
    block_count: int
    memory_ids: list[int]


@dataclass(frozen=True)
class ReadResult:
    session_id: int
    candidate_terms: list[str]
    findings: list[str]
    stored_memory_ids: list[int]


def split_learning_blocks(text: str, max_chars: int = 1200) -> list[LearningBlock]:
    paragraphs = [part.strip() for part in re.split(r"\n\s*\n", text) if part.strip()]
    block_texts: list[str] = []

    for paragraph in paragraphs:
        if len(paragraph) <= max_chars:
            block_texts.append(paragraph)
            continue

        current = ""
        for sentence in SENTENCE_BOUNDARY_PATTERN.split(paragraph):
            sentence = sentence.strip()
            if not sentence:
                continue

            candidate = sentence if not current else f"{current} {sentence}"
            if len(candidate) > max_chars and current:
                block_texts.append(current)
                current = sentence
            else:
                current = candidate

        if current:
            block_texts.append(current)

    return [
        LearningBlock(index=index, text=block_text)
        for index, block_text in enumerate(block_texts, start=1)
    ]


def extract_candidate_terms(text: str) -> list[str]:
    terms: list[str] = []
    seen: set[str] = set()
    for match in TERM_PATTERN.finditer(text):
        term = match.group(0)
        if term in seen:
            continue
        seen.add(term)
        terms.append(term)
    return terms


class IngestionService:
    def __init__(self, config: HieronymusConfig) -> None:
        self.config = config

    def learn(
        self,
        *,
        session_id: int,
        text: str,
        source_role: str,
        source_ref: str = "",
        kind: str = "learned_block",
    ) -> LearnResult:
        workspace = WorkspaceStore(self.config)
        blocks = split_learning_blocks(text)
        memory_ids: list[int] = []

        for block in blocks:
            memory_ids.append(
                workspace.add_short_term_memory(
                    session_id=session_id,
                    source_role=source_role,
                    kind=kind,
                    text=block.text,
                    source_ref=source_ref,
                    metadata={
                        "ingestion_mode": "learn",
                        "block_index": block.index,
                        "block_count": len(blocks),
                    },
                )
            )

        return LearnResult(
            session_id=session_id,
            block_count=len(blocks),
            memory_ids=memory_ids,
        )

    def read(
        self,
        *,
        session_id: int,
        text: str,
        source_ref: str = "",
        store_observation: bool = False,
    ) -> ReadResult:
        candidate_terms = extract_candidate_terms(text)
        findings = [f"candidate_term:{term}" for term in candidate_terms]
        stored_memory_ids: list[int] = []

        if store_observation and findings:
            stored_memory_ids.append(
                WorkspaceStore(self.config).add_short_term_memory(
                    session_id=session_id,
                    source_role="mundane",
                    kind="read_observation",
                    text="\n".join(findings),
                    source_ref=source_ref,
                    metadata={
                        "ingestion_mode": "read",
                        "candidate_terms": candidate_terms,
                    },
                )
            )

        return ReadResult(
            session_id=session_id,
            candidate_terms=candidate_terms,
            findings=findings,
            stored_memory_ids=stored_memory_ids,
        )
