from __future__ import annotations

import hashlib
import json
import os
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import TYPE_CHECKING, Any

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig

if TYPE_CHECKING:
    from hieronymus.dream_providers import ProviderProfile

CACHE_TTL = timedelta(hours=24)


@dataclass(frozen=True)
class ModelCacheAdoption:
    status: str
    legacy_path: str = ""
    error: str = ""


@dataclass(frozen=True)
class _LegacyCacheSnapshot:
    path: Path
    content: bytes
    device: int
    inode: int
    valid: bool


@dataclass(frozen=True)
class ModelCacheEntry:
    provider: str
    models: tuple[str, ...]
    fetched_at: str
    error: str = ""
    identity: str = ""

    def is_stale(self, now: datetime | None = None) -> bool:
        try:
            fetched_at = _parse_datetime(self.fetched_at)
        except ValueError:
            return True
        current = now or datetime.now(UTC)
        if current.tzinfo is None:
            current = current.replace(tzinfo=UTC)
        if fetched_at > current:
            return True
        return current - fetched_at >= CACHE_TTL

    def to_payload(self) -> dict[str, object]:
        return {
            "provider": self.provider,
            "models": list(self.models),
            "fetched_at": self.fetched_at,
            "error": self.error,
            "identity": self.identity,
        }


@dataclass(frozen=True)
class CachedModels:
    providers: dict[str, ModelCacheEntry] = field(default_factory=dict)

    def with_entry(self, entry: ModelCacheEntry) -> CachedModels:
        return replace(self, providers={**self.providers, entry.provider: entry})

    def to_payload(self) -> dict[str, object]:
        return {
            "providers": {
                provider: entry.to_payload() for provider, entry in self.providers.items()
            }
        }


def load_model_cache(config: HieronymusConfig) -> CachedModels:
    path = config.llm_cache_path
    if not path.exists():
        path = _legacy_model_cache_path(config)
    if not path.exists():
        return CachedModels()
    cache, valid = _read_cache(path)
    return cache if valid else CachedModels()


def inspect_legacy_model_cache(config: HieronymusConfig) -> ModelCacheAdoption:
    legacy_path = _legacy_model_cache_path(config)
    try:
        snapshot = _read_legacy_snapshot(legacy_path)
    except OSError as error:
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(legacy_path),
            error=str(error),
        )
    if snapshot is None:
        return ModelCacheAdoption(status="not-needed")
    if not snapshot.valid:
        return ModelCacheAdoption(status="invalid-legacy", legacy_path=str(legacy_path))
    if config.llm_cache_path.exists():
        if _read_bytes(config.llm_cache_path) == snapshot.content:
            return ModelCacheAdoption(status="cleanup-ready", legacy_path=str(legacy_path))
        return ModelCacheAdoption(status="conflict", legacy_path=str(legacy_path))
    return ModelCacheAdoption(status="ready", legacy_path=str(legacy_path))


def adopt_legacy_model_cache(config: HieronymusConfig) -> ModelCacheAdoption:
    legacy_path = _legacy_model_cache_path(config)
    try:
        snapshot = _read_legacy_snapshot(legacy_path)
    except OSError as error:
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(legacy_path),
            error=str(error),
        )
    if snapshot is None:
        return ModelCacheAdoption(status="not-needed")
    if not snapshot.valid:
        return ModelCacheAdoption(status="invalid-legacy", legacy_path=str(legacy_path))

    if config.llm_cache_path.exists():
        return _finish_existing_publication(config, snapshot)

    descriptor: int | None = None
    try:
        descriptor = os.open(
            config.llm_cache_path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL,
            0o600,
        )
    except FileExistsError:
        return _finish_existing_publication(config, snapshot)
    except OSError as error:
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(legacy_path),
            error=str(error),
        )

    publication_metadata = os.fstat(descriptor)
    publication_identity = (publication_metadata.st_dev, publication_metadata.st_ino)
    try:
        _write_all(descriptor, snapshot.content)
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = None
        _fsync_directory(config.data_root)
    except OSError as error:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
        _remove_partial_publication(
            config.llm_cache_path,
            config.data_root,
            publication_identity,
        )
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(legacy_path),
            error=str(error),
        )
    return _cleanup_legacy_snapshot(config, snapshot)


def save_model_cache(config: HieronymusConfig, cache: CachedModels) -> None:
    atomic_write_text(
        config.llm_cache_path,
        json.dumps(
            cache.to_payload(),
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )


def model_cache_identity(name: str) -> str:
    payload = {"provider": name}
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


def dream_profile_cache_identity(name: str, provider: ProviderProfile) -> str:
    payload = {
        "provider": name,
        "type": provider.type,
        "endpoint": provider.endpoint.rstrip("/"),
        "api_key_sha256": _secret_fingerprint(provider.api_key),
    }
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


def _secret_fingerprint(value: str) -> str:
    if not value:
        return ""
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _cache_from_payload(payload: Any) -> CachedModels:
    if type(payload) is not dict:
        return CachedModels()
    providers = payload.get("providers")
    if type(providers) is not dict:
        return CachedModels()
    entries = {}
    for provider, raw_entry in providers.items():
        if type(provider) is not str or type(raw_entry) is not dict:
            continue
        entry = _entry_from_payload(provider, raw_entry)
        if entry is not None:
            entries[provider] = entry
    return CachedModels(providers=entries)


def _legacy_model_cache_path(config: HieronymusConfig) -> Path:
    return config.data_root / ("llm" + "cache.tmp")


def _read_legacy_snapshot(path: Path) -> _LegacyCacheSnapshot | None:
    try:
        descriptor = os.open(path, os.O_RDONLY)
    except FileNotFoundError:
        return None
    try:
        metadata = os.fstat(descriptor)
        chunks = []
        while chunk := os.read(descriptor, 64 * 1024):
            chunks.append(chunk)
    finally:
        os.close(descriptor)
    content = b"".join(chunks)
    return _LegacyCacheSnapshot(
        path=path,
        content=content,
        device=metadata.st_dev,
        inode=metadata.st_ino,
        valid=_valid_complete_cache_bytes(content),
    )


def _valid_complete_cache_bytes(content: bytes) -> bool:
    try:
        payload = json.loads(content.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return False
    if type(payload) is not dict or type(payload.get("providers")) is not dict:
        return False
    cache = _cache_from_payload(payload)
    return _is_complete_cache_payload(payload, cache)


def _read_bytes(path: Path) -> bytes | None:
    try:
        return path.read_bytes()
    except OSError:
        return None


def _finish_existing_publication(
    config: HieronymusConfig,
    snapshot: _LegacyCacheSnapshot,
) -> ModelCacheAdoption:
    if _read_bytes(config.llm_cache_path) != snapshot.content:
        return ModelCacheAdoption(status="conflict", legacy_path=str(snapshot.path))
    return _cleanup_legacy_snapshot(config, snapshot)


def _cleanup_legacy_snapshot(
    config: HieronymusConfig,
    snapshot: _LegacyCacheSnapshot,
) -> ModelCacheAdoption:
    try:
        current = snapshot.path.stat(follow_symlinks=False)
    except FileNotFoundError:
        return ModelCacheAdoption(status="adopted", legacy_path=str(snapshot.path))
    except OSError as error:
        return ModelCacheAdoption(
            status="published-cleanup-pending",
            legacy_path=str(snapshot.path),
            error=str(error),
        )
    if (current.st_dev, current.st_ino) != (snapshot.device, snapshot.inode):
        return ModelCacheAdoption(
            status="published-cleanup-pending",
            legacy_path=str(snapshot.path),
            error="legacy cache path changed during adoption",
        )
    if _read_bytes(snapshot.path) != snapshot.content:
        return ModelCacheAdoption(
            status="published-cleanup-pending",
            legacy_path=str(snapshot.path),
            error="legacy cache content changed during adoption",
        )
    try:
        os.unlink(snapshot.path)
        _fsync_directory(config.data_root)
    except OSError as error:
        return ModelCacheAdoption(
            status="published-cleanup-pending",
            legacy_path=str(snapshot.path),
            error=str(error),
        )
    return ModelCacheAdoption(status="adopted", legacy_path=str(snapshot.path))


def _write_all(descriptor: int, content: bytes) -> None:
    written = 0
    while written < len(content):
        count = os.write(descriptor, content[written:])
        if count <= 0:
            raise OSError("model cache publication write made no progress")
        written += count


def _remove_partial_publication(
    path: Path,
    parent: Path,
    expected_identity: tuple[int, int],
) -> None:
    try:
        current = path.stat(follow_symlinks=False)
    except OSError:
        return
    if (current.st_dev, current.st_ino) != expected_identity:
        return
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
    except OSError:
        return
    try:
        _fsync_directory(parent)
    except OSError:
        pass


def _read_cache(path: Path, *, require_complete: bool = False) -> tuple[CachedModels, bool]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return CachedModels(), False
    if type(payload) is not dict or type(payload.get("providers")) is not dict:
        return CachedModels(), False
    cache = _cache_from_payload(payload)
    if require_complete and not _is_complete_cache_payload(payload, cache):
        return CachedModels(), False
    return cache, True


def _is_complete_cache_payload(payload: dict[str, Any], cache: CachedModels) -> bool:
    providers = payload["providers"]
    if len(cache.providers) != len(providers):
        return False
    for provider, raw_entry in providers.items():
        if type(provider) is not str or not provider or type(raw_entry) is not dict:
            return False
        raw_provider = raw_entry.get("provider", provider)
        raw_models = raw_entry.get("models")
        if type(raw_provider) is not str or type(raw_models) is not list:
            return False
        if any(type(model) is not str for model in raw_models):
            return False
    return True


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _entry_from_payload(provider: str, payload: dict[str, Any]) -> ModelCacheEntry | None:
    raw_models = payload.get("models")
    raw_fetched_at = payload.get("fetched_at")
    raw_error = payload.get("error", "")
    raw_identity = payload.get("identity", "")
    if (
        type(raw_models) is not list
        or type(raw_fetched_at) is not str
        or type(raw_error) is not str
        or type(raw_identity) is not str
    ):
        return None
    try:
        _parse_datetime(raw_fetched_at)
    except ValueError:
        return None
    models = tuple(model for model in raw_models if type(model) is str)
    if not models and not raw_error:
        return None
    return ModelCacheEntry(
        provider=provider,
        models=models,
        fetched_at=raw_fetched_at,
        error=raw_error,
        identity=raw_identity,
    )


def _parse_datetime(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return parsed
