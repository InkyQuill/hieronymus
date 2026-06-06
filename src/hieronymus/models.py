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
