from __future__ import annotations

import re
from dataclasses import dataclass

from hieronymus.config import HieronymusConfig
from hieronymus.ingest_config import load_ingest_config
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
    if max_chars < 1:
        raise ValueError("max_chars must be positive")

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

            if len(sentence) > max_chars:
                if current:
                    block_texts.append(current)
                    current = ""
                block_texts.extend(_split_oversized_text(sentence, max_chars))
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


def _split_oversized_text(text: str, max_chars: int) -> list[str]:
    chunks: list[str] = []
    current = ""

    for word in text.split():
        if len(word) > max_chars:
            if current:
                chunks.append(current)
                current = ""
            chunks.extend(
                word[index : index + max_chars] for index in range(0, len(word), max_chars)
            )
            continue

        candidate = word if not current else f"{current} {word}"
        if len(candidate) > max_chars:
            chunks.append(current)
            current = word
        else:
            current = candidate

    if current:
        chunks.append(current)

    if chunks:
        return chunks

    return [text[index : index + max_chars] for index in range(0, len(text), max_chars)]


def extract_candidate_terms(text: str) -> list[str]:
    """Return lightweight heuristic read findings for initial agent ingestion."""
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
        learn_limits = load_ingest_config(self.config).learn
        blocks = split_learning_blocks(text, max_chars=learn_limits.max_block_chars)
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
        workspace = WorkspaceStore(self.config)
        session = workspace.get_session(session_id)
        if session.status != "active":
            raise ValueError("read ingestion requires an active session")

        candidate_terms = extract_candidate_terms(text)
        findings = [f"candidate_term:{term}" for term in candidate_terms]
        stored_memory_ids: list[int] = []

        if store_observation and findings:
            stored_memory_ids.append(
                workspace.add_short_term_memory(
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
