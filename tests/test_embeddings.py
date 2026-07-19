from __future__ import annotations

import math
import sys
from collections.abc import Iterable
from types import SimpleNamespace

import pytest

from hieronymus.config import HieronymusConfig
from hieronymus.embeddings import (
    EmbeddingBatchSizeError,
    EmbeddingIdentity,
    EmbeddingInputError,
    EmbeddingProgress,
    EmbeddingResultError,
    EmbeddingUnavailableError,
    FastEmbedProvider,
    UnsupportedEmbeddingProviderError,
    UnsupportedEmbeddingRevisionError,
    create_embedding_provider,
)
from hieronymus.semantic_config import SemanticConfig


class StubTextEmbedding:
    constructions = 0

    def __init__(self, **kwargs: object) -> None:
        type(self).constructions += 1
        self.kwargs = kwargs

    def passage_embed(self, texts: Iterable[str], **kwargs: object) -> Iterable[list[float]]:
        del kwargs
        return ([float(len(text)), 1.0, 2.0] for text in texts)

    def query_embed(self, query: str, **kwargs: object) -> Iterable[list[float]]:
        del kwargs
        return iter(([float(len(query)), 1.0, 2.0],))


def _semantic(**changes: object) -> SemanticConfig:
    values: dict[str, object] = {
        "enabled": True,
        "provider": "local",
        "model": "test/model",
        "revision": None,
        "dimensions": 3,
        "batch_size": 2,
        "candidate_multiplier": 4,
        "fusion_constant": 60,
    }
    values.update(changes)
    return SemanticConfig(**values)  # type: ignore[arg-type]


def _provider(tmp_path, monkeypatch: pytest.MonkeyPatch, **changes: object) -> FastEmbedProvider:
    StubTextEmbedding.constructions = 0
    monkeypatch.setitem(sys.modules, "fastembed", SimpleNamespace(TextEmbedding=StubTextEmbedding))
    return FastEmbedProvider(
        _semantic(**changes),
        cache_root=HieronymusConfig(data_root=tmp_path).embedding_cache_root,
    )


def test_fastembed_provider_exposes_stable_identity_without_loading_model(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    provider = _provider(tmp_path, monkeypatch)

    assert provider.identity == EmbeddingIdentity(
        provider="local", model="test/model", revision=None, dimensions=3
    )
    assert StubTextEmbedding.constructions == 0


def test_fastembed_provider_constructs_and_caches_model_on_first_embedding(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    provider = _provider(tmp_path, monkeypatch)

    assert provider.embed_query("query") == [5.0, 1.0, 2.0]
    assert provider.embed_query("again") == [5.0, 1.0, 2.0]
    assert StubTextEmbedding.constructions == 1


def test_empty_document_batch_does_not_load_model(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    provider = _provider(tmp_path, monkeypatch)

    assert provider.embed_documents([]) == []
    assert StubTextEmbedding.constructions == 0


def test_document_batch_is_bounded_before_model_load(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    provider = _provider(tmp_path, monkeypatch, batch_size=2)

    with pytest.raises(EmbeddingBatchSizeError, match="at most 2"):
        provider.embed_documents(["one", "two", "three"])
    assert StubTextEmbedding.constructions == 0


def test_document_embedding_reports_progress(tmp_path, monkeypatch: pytest.MonkeyPatch) -> None:
    provider = _provider(tmp_path, monkeypatch)
    events: list[EmbeddingProgress] = []

    vectors = provider.embed_documents(["one", "three"], progress=events.append)

    assert vectors == [[3.0, 1.0, 2.0], [5.0, 1.0, 2.0]]
    assert events == [
        EmbeddingProgress(phase="model-loading", completed=0, total=2),
        EmbeddingProgress(phase="embedding", completed=2, total=2),
    ]


def test_query_embedding_exposes_model_loading_progress(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    provider = _provider(tmp_path, monkeypatch)
    events: list[EmbeddingProgress] = []

    provider.embed_query("query", progress=events.append)

    assert events == [
        EmbeddingProgress(phase="model-loading", completed=0, total=1),
        EmbeddingProgress(phase="embedding", completed=1, total=1),
    ]


@pytest.mark.parametrize("query", ["", "   "])
def test_query_embedding_rejects_blank_input_before_model_load(
    tmp_path, monkeypatch: pytest.MonkeyPatch, query: str
) -> None:
    provider = _provider(tmp_path, monkeypatch)

    with pytest.raises(EmbeddingInputError, match="query must not be blank"):
        provider.embed_query(query)
    assert StubTextEmbedding.constructions == 0


def test_provider_rejects_wrong_vector_dimensions(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    provider = _provider(tmp_path, monkeypatch, dimensions=4)

    with pytest.raises(EmbeddingResultError, match="expected 4 dimensions, received 3"):
        provider.embed_query("query")


def test_local_provider_rejects_unenforced_revision_without_loading_model(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(UnsupportedEmbeddingRevisionError, match="does not support revisions"):
        _provider(tmp_path, monkeypatch, revision="main")
    assert StubTextEmbedding.constructions == 0


@pytest.mark.parametrize("component", [math.nan, math.inf, -math.inf])
def test_provider_rejects_non_finite_vector_components(
    tmp_path, monkeypatch: pytest.MonkeyPatch, component: float
) -> None:
    provider = _provider(tmp_path, monkeypatch)

    class NonFiniteModel:
        def query_embed(self, _query: str) -> Iterable[list[float]]:
            return iter(([component, 1.0, 2.0],))

    provider._model = NonFiniteModel()

    with pytest.raises(EmbeddingResultError, match="non-finite component at index 0"):
        provider.embed_query("query")


def test_provider_wraps_model_construction_failure(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class BrokenTextEmbedding:
        def __init__(self, **kwargs: object) -> None:
            del kwargs
            raise OSError("offline")

    monkeypatch.setitem(
        sys.modules, "fastembed", SimpleNamespace(TextEmbedding=BrokenTextEmbedding)
    )
    provider = FastEmbedProvider(_semantic(), cache_root=tmp_path)

    with pytest.raises(EmbeddingUnavailableError, match="local embedding model is unavailable"):
        provider.embed_query("query")


def test_registry_returns_precise_error_for_unregistered_remote_provider(tmp_path) -> None:
    with pytest.raises(
        UnsupportedEmbeddingProviderError,
        match="embedding provider 'remote' is not registered; available providers: local",
    ):
        create_embedding_provider(
            _semantic(provider="remote"), HieronymusConfig(data_root=tmp_path)
        )


def test_disabled_semantic_mode_creates_no_embedding_provider(tmp_path) -> None:
    assert (
        create_embedding_provider(_semantic(enabled=False), HieronymusConfig(data_root=tmp_path))
        is None
    )
