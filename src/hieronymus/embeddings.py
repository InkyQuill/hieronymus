from __future__ import annotations

import math
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from hieronymus.config import HieronymusConfig
from hieronymus.semantic_config import SemanticConfig, validate_semantic_config

type Vector = list[float]


class EmbeddingError(RuntimeError):
    """Base class for embedding provider failures."""


class EmbeddingInputError(EmbeddingError):
    """Raised when text passed to a provider is invalid."""


class EmbeddingBatchSizeError(EmbeddingInputError):
    """Raised when a document batch exceeds its configured bound."""


class EmbeddingUnavailableError(EmbeddingError):
    """Raised when an embedding model cannot be loaded or executed."""


class EmbeddingResultError(EmbeddingError):
    """Raised when an embedding provider returns an invalid result."""


class UnsupportedEmbeddingProviderError(EmbeddingError):
    """Raised when no factory is registered for a configured provider."""


class UnsupportedEmbeddingRevisionError(EmbeddingError):
    """Raised when a provider cannot enforce the configured model revision."""


@dataclass(frozen=True)
class EmbeddingIdentity:
    provider: str
    model: str
    revision: str | None
    dimensions: int


@dataclass(frozen=True)
class EmbeddingProgress:
    phase: str
    completed: int
    total: int


type ProgressCallback = Callable[[EmbeddingProgress], None]


class EmbeddingProvider(Protocol):
    @property
    def identity(self) -> EmbeddingIdentity: ...

    @property
    def max_batch_size(self) -> int: ...

    def embed_documents(
        self,
        documents: Sequence[str],
        *,
        progress: ProgressCallback | None = None,
    ) -> list[Vector]: ...

    def embed_query(
        self,
        query: str,
        *,
        progress: ProgressCallback | None = None,
    ) -> Vector: ...


class FastEmbedProvider:
    """Lazy, object-scoped adapter around FastEmbed's local text model."""

    def __init__(self, config: SemanticConfig, *, cache_root: Path) -> None:
        self._config = validate_semantic_config(config)
        if self._config.revision is not None:
            raise UnsupportedEmbeddingRevisionError(
                "the local FastEmbed provider does not support revisions; "
                "remove semantic.revision or choose a provider that can enforce it"
            )
        self._cache_root = cache_root
        self._model: Any | None = None

    @property
    def identity(self) -> EmbeddingIdentity:
        return EmbeddingIdentity(
            provider=self._config.provider,
            model=self._config.model,
            revision=self._config.revision,
            dimensions=self._config.dimensions,
        )

    @property
    def max_batch_size(self) -> int:
        return self._config.batch_size

    def embed_documents(
        self,
        documents: Sequence[str],
        *,
        progress: ProgressCallback | None = None,
    ) -> list[Vector]:
        document_list = list(documents)
        if not document_list:
            return []
        if len(document_list) > self.max_batch_size:
            raise EmbeddingBatchSizeError(
                f"document embedding accepts at most {self.max_batch_size} texts per batch"
            )
        if any(type(document) is not str or not document.strip() for document in document_list):
            raise EmbeddingInputError("documents must contain non-blank strings")
        try:
            raw_vectors = list(
                self._get_model(progress=progress, total=len(document_list)).passage_embed(
                    document_list,
                    batch_size=self.max_batch_size,
                )
            )
        except EmbeddingError:
            raise
        except Exception as error:
            raise EmbeddingUnavailableError("local document embedding failed") from error
        vectors = self._validated_vectors(raw_vectors, expected_count=len(document_list))
        if progress is not None:
            progress(
                EmbeddingProgress(
                    phase="embedding",
                    completed=len(vectors),
                    total=len(document_list),
                )
            )
        return vectors

    def embed_query(
        self,
        query: str,
        *,
        progress: ProgressCallback | None = None,
    ) -> Vector:
        if type(query) is not str or not query.strip():
            raise EmbeddingInputError("query must not be blank")
        try:
            raw_vectors = list(self._get_model(progress=progress, total=1).query_embed(query))
        except EmbeddingError:
            raise
        except Exception as error:
            raise EmbeddingUnavailableError("local query embedding failed") from error
        vector = self._validated_vectors(raw_vectors, expected_count=1)[0]
        if progress is not None:
            progress(EmbeddingProgress(phase="embedding", completed=1, total=1))
        return vector

    def _get_model(self, *, progress: ProgressCallback | None, total: int) -> Any:
        if self._model is not None:
            return self._model
        if progress is not None:
            progress(EmbeddingProgress(phase="model-loading", completed=0, total=total))
        try:
            from fastembed import TextEmbedding

            model = TextEmbedding(
                model_name=self._config.model,
                cache_dir=str(self._cache_root),
                lazy_load=True,
            )
        except Exception as error:
            raise EmbeddingUnavailableError("local embedding model is unavailable") from error
        self._model = model
        return model

    def _validated_vectors(
        self, raw_vectors: Iterable[object], *, expected_count: int
    ) -> list[Vector]:
        vectors = [self._validated_vector(vector) for vector in raw_vectors]
        if len(vectors) != expected_count:
            raise EmbeddingResultError(
                f"embedding provider returned {len(vectors)} vectors for {expected_count} texts"
            )
        return vectors

    def _validated_vector(self, raw_vector: object) -> Vector:
        try:
            vector = [float(value) for value in raw_vector]  # type: ignore[union-attr]
        except (TypeError, ValueError) as error:
            raise EmbeddingResultError(
                "embedding provider returned a non-numeric vector"
            ) from error
        if len(vector) != self.identity.dimensions:
            raise EmbeddingResultError(
                f"expected {self.identity.dimensions} dimensions, received {len(vector)}"
            )
        for index, value in enumerate(vector):
            if not math.isfinite(value):
                raise EmbeddingResultError(
                    f"embedding provider returned a non-finite component at index {index}"
                )
        return vector


type ProviderFactory = Callable[[SemanticConfig, HieronymusConfig], EmbeddingProvider]


class EmbeddingProviderRegistry:
    def __init__(self) -> None:
        self._factories: dict[str, ProviderFactory] = {}

    def register(self, name: str, factory: ProviderFactory) -> None:
        clean_name = name.strip().casefold()
        if not clean_name:
            raise ValueError("embedding provider name must not be blank")
        self._factories[clean_name] = factory

    def create(
        self, semantic_config: SemanticConfig, config: HieronymusConfig
    ) -> EmbeddingProvider:
        name = semantic_config.provider.casefold()
        factory = self._factories.get(name)
        if factory is None:
            available = ", ".join(sorted(self._factories)) or "none"
            raise UnsupportedEmbeddingProviderError(
                f"embedding provider {semantic_config.provider!r} is not registered; "
                f"available providers: {available}"
            )
        return factory(semantic_config, config)


def default_embedding_provider_registry() -> EmbeddingProviderRegistry:
    registry = EmbeddingProviderRegistry()
    registry.register(
        "local",
        lambda semantic, config: FastEmbedProvider(
            semantic,
            cache_root=config.embedding_cache_root,
        ),
    )
    return registry


def create_embedding_provider(
    semantic_config: SemanticConfig,
    config: HieronymusConfig,
    *,
    registry: EmbeddingProviderRegistry | None = None,
) -> EmbeddingProvider | None:
    semantic_config = validate_semantic_config(semantic_config)
    if semantic_config.fts_only:
        return None
    return (registry or default_embedding_provider_registry()).create(semantic_config, config)
