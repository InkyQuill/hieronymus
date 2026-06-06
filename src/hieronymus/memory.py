from __future__ import annotations

from hieronymus.config import HieronymusConfig
from hieronymus.crystals import CrystalStore
from hieronymus.memory_models import TranslationContext
from hieronymus.models import MemoryEntry


class MemoryStore:
    def __init__(self, config: HieronymusConfig, context: TranslationContext) -> None:
        self.config = config
        self.context = context
        self._crystals = CrystalStore(config)

    def add(self, *, kind: str, text: str, source_ref: str = "", importance: int = 3) -> int:
        if not kind.strip():
            raise ValueError("kind must not be empty")
        if not text.strip():
            raise ValueError("text must not be empty")

        strength = min(max(importance / 5, 0.0), 1.0)
        return self._crystals.add_crystal(
            self.context,
            crystal_type="erudition",
            text=text,
            title=kind,
            strength=strength,
            confidence=0.5,
        )

    def search(self, query: str, *, limit: int = 5) -> list[MemoryEntry]:
        return [
            MemoryEntry(
                id=crystal.id,
                kind=crystal.title or crystal.crystal_type,
                text=crystal.text,
                importance=round(crystal.strength * 5),
                source_ref="",
            )
            for crystal in self._crystals.search(self.context, query, limit=limit)
        ]
