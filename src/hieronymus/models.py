from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ContractTerm:
    id: int
    category: str
    source_text: str
    canonical_translation: str
    forbidden_variants: list[str]
    tags: list[str]
    notes: str


@dataclass(frozen=True)
class ValidationFinding:
    term_id: int
    kind: str
    severity: str
    expected: str
    observed: str
    message: str


@dataclass(frozen=True)
class MemoryEntry:
    id: int
    kind: str
    text: str
    importance: int
    source_ref: str
