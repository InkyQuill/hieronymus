from __future__ import annotations

import fcntl
import hashlib
import json
import os
import uuid
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime, timedelta
from pathlib import Path
from threading import local
from typing import TYPE_CHECKING, Any

from hieronymus.agent_plugins.base import atomic_write_text
from hieronymus.config import HieronymusConfig

if TYPE_CHECKING:
    from hieronymus.dream_providers import ProviderProfile

CACHE_TTL = timedelta(hours=24)
_ADOPTION_LOCK_STATE = local()


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
    try:
        with _adoption_lock(config) as locked:
            if not locked:
                return CachedModels()
            return _load_model_cache_locked(config)
    except OSError:
        return CachedModels()


def _load_model_cache_locked(config: HieronymusConfig) -> CachedModels:
    path = config.llm_cache_path
    if not path.exists():
        path = _legacy_model_cache_path(config)
    if not path.exists():
        return CachedModels()
    cache, valid = _read_cache(path)
    return cache if valid else CachedModels()


def inspect_legacy_model_cache(config: HieronymusConfig) -> ModelCacheAdoption:
    try:
        with _adoption_lock(config) as locked:
            if not locked:
                return ModelCacheAdoption(status="not-needed")
            return _inspect_legacy_model_cache_locked(config)
    except OSError as error:
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(config.data_root),
            error=str(error),
        )


def _inspect_legacy_model_cache_locked(
    config: HieronymusConfig,
) -> ModelCacheAdoption:
    legacy_path = _legacy_model_cache_path(config)
    recoveries = _recovery_paths(config)
    try:
        snapshot = _read_legacy_snapshot(legacy_path)
    except OSError as error:
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(legacy_path),
            error=str(error),
        )
    if snapshot is None:
        if recoveries:
            return _inspect_recoveries(config, recoveries)
        return ModelCacheAdoption(status="not-needed")
    if recoveries:
        return _inspect_public_legacy_recoveries(snapshot, recoveries)
    if not snapshot.valid:
        return ModelCacheAdoption(status="invalid-legacy", legacy_path=str(legacy_path))
    if config.llm_cache_path.exists():
        try:
            canonical = _read_bytes(config.llm_cache_path)
        except OSError as error:
            return _recovery_pending(
                (config.llm_cache_path,), f"canonical cache could not be read: {error}"
            )
        if canonical == snapshot.content:
            return ModelCacheAdoption(status="cleanup-ready", legacy_path=str(legacy_path))
        return ModelCacheAdoption(status="conflict", legacy_path=str(legacy_path))
    return ModelCacheAdoption(status="ready", legacy_path=str(legacy_path))


def adopt_legacy_model_cache(config: HieronymusConfig) -> ModelCacheAdoption:
    try:
        with _adoption_lock(config) as locked:
            if not locked:
                return ModelCacheAdoption(status="not-needed")
            return _adopt_legacy_model_cache_locked(config)
    except OSError as error:
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(config.data_root),
            error=str(error),
        )


def _adopt_legacy_model_cache_locked(config: HieronymusConfig) -> ModelCacheAdoption:
    legacy_path = _legacy_model_cache_path(config)
    recoveries = _recovery_paths(config)
    if legacy_path.exists() and recoveries:
        try:
            snapshot = _read_legacy_snapshot(legacy_path)
        except OSError as error:
            return _recovery_pending(recoveries, f"public legacy cache could not be read: {error}")
        if snapshot is None:
            return _recovery_pending(recoveries, "public legacy cache disappeared")
        inspection = _inspect_public_legacy_recoveries(snapshot, recoveries)
        if inspection.status != "cleanup-ready":
            return inspection
        cleanup = _cleanup_private_adoption_files(config, *recoveries)
        if cleanup.status != "adopted":
            return cleanup
    try:
        claim_path = _claim_legacy_path(config, legacy_path)
    except OSError as error:
        return ModelCacheAdoption(
            status="adoption-failed",
            legacy_path=str(legacy_path),
            error=str(error),
        )
    if claim_path is None:
        return _converge_without_public_legacy(config)

    return _publish_claim(config, claim_path)


def _publish_claim(config: HieronymusConfig, claim_path: Path) -> ModelCacheAdoption:
    legacy_path = _legacy_model_cache_path(config)

    try:
        snapshot = _read_legacy_snapshot(claim_path)
    except OSError as error:
        return _prepublication_failure(config, claim_path, error=str(error))
    if snapshot is None:
        return ModelCacheAdoption(status="adoption-failed", error="legacy claim disappeared")
    if not snapshot.valid:
        restored = _restore_claim(config, claim_path)
        return ModelCacheAdoption(
            status="invalid-legacy" if restored else "recovery-pending",
            legacy_path=str(legacy_path),
            error="legacy cache is malformed",
        )

    temp_path = _unique_adoption_path(config, "canonical")
    descriptor: int | None = None
    try:
        descriptor = os.open(temp_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        _write_all(descriptor, snapshot.content)
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = None
    except OSError as error:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
        return _prepublication_failure(
            config,
            claim_path,
            error=str(error),
            temp_path=temp_path,
        )

    try:
        os.link(temp_path, config.llm_cache_path)
    except FileExistsError:
        try:
            canonical = _read_bytes(config.llm_cache_path)
        except OSError as error:
            return _recovery_pending(
                (claim_path, temp_path), f"canonical cache could not be read: {error}"
            )
        if canonical != snapshot.content:
            try:
                os.unlink(temp_path)
            except OSError as error:
                return _recovery_pending(
                    (claim_path, temp_path),
                    f"conflicting canonical cache; temporary cleanup failed: {error}",
                )
            if _restore_claim(config, claim_path):
                return ModelCacheAdoption(status="conflict", legacy_path=str(legacy_path))
            return _recovery_pending(
                (claim_path,), "conflicting canonical cache; legacy claim could not be restored"
            )
    except OSError as error:
        return _prepublication_failure(
            config,
            claim_path,
            error=str(error),
            temp_path=temp_path,
        )

    try:
        _fsync_directory(config.data_root)
    except OSError as error:
        return ModelCacheAdoption(
            status="publication-sync-pending",
            legacy_path=str(claim_path),
            error=str(error),
        )
    return _cleanup_private_adoption_files(config, claim_path, temp_path)


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


def _unique_adoption_path(config: HieronymusConfig, kind: str) -> Path:
    return config.data_root / f".llm-cache-adoption-{uuid.uuid4().hex}.{kind}"


@contextmanager
def _adoption_lock(config: HieronymusConfig) -> Iterator[bool]:
    try:
        descriptor = os.open(config.data_root, os.O_RDONLY | os.O_DIRECTORY)
    except FileNotFoundError:
        yield False
        return
    try:
        metadata = os.fstat(descriptor)
    except OSError:
        os.close(descriptor)
        raise
    identity = (metadata.st_dev, metadata.st_ino)
    held: dict[tuple[int, int], int] = getattr(_ADOPTION_LOCK_STATE, "held", {})
    if identity in held:
        held[identity] += 1
        try:
            yield True
        finally:
            held[identity] -= 1
            os.close(descriptor)
        return
    locked = False
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        locked = True
        held[identity] = 1
        _ADOPTION_LOCK_STATE.held = held
        yield True
    finally:
        try:
            held.pop(identity, None)
            if locked:
                fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)


def _recovery_paths(config: HieronymusConfig) -> tuple[Path, ...]:
    return tuple(sorted(config.data_root.glob(".llm-cache-adoption-*")))


def _recovery_pending(paths: tuple[Path, ...], reason: str) -> ModelCacheAdoption:
    rendered = ", ".join(str(path) for path in paths)
    return ModelCacheAdoption(
        status="recovery-pending",
        legacy_path=str(paths[0]) if paths else "",
        error=f"{reason}; recovery files: {rendered}" if rendered else reason,
    )


def _inspect_recoveries(
    config: HieronymusConfig,
    recoveries: tuple[Path, ...],
) -> ModelCacheAdoption:
    try:
        canonical = _read_bytes(config.llm_cache_path)
    except OSError as error:
        return _recovery_pending(recoveries, f"canonical cache could not be read: {error}")
    if canonical is not None:
        try:
            matches = all(_read_bytes(path) == canonical for path in recoveries)
        except OSError as error:
            return _recovery_pending(recoveries, f"recovery file could not be read: {error}")
        if matches:
            return ModelCacheAdoption(
                status="cleanup-ready",
                legacy_path=", ".join(str(path) for path in recoveries),
            )
        return _recovery_pending(recoveries, "recovery files do not all match the canonical cache")

    claims = tuple(path for path in recoveries if path.suffix == ".legacy")
    if len(claims) == 1:
        try:
            snapshot = _read_legacy_snapshot(claims[0])
        except OSError as error:
            return _recovery_pending(recoveries, f"legacy claim could not be read: {error}")
        if snapshot is not None and snapshot.valid:
            return ModelCacheAdoption(status="recovery-ready", legacy_path=str(claims[0]))
    return _recovery_pending(recoveries, "recovery files are ambiguous or incomplete")


def _inspect_public_legacy_recoveries(
    snapshot: _LegacyCacheSnapshot,
    recoveries: tuple[Path, ...],
) -> ModelCacheAdoption:
    claims = tuple(path for path in recoveries if path.suffix == ".legacy")
    if len(claims) != 1:
        return _recovery_pending(recoveries, "public legacy cache and recovery files coexist")
    try:
        claim = _read_legacy_snapshot(claims[0])
    except OSError as error:
        return _recovery_pending(recoveries, f"legacy claim could not be read: {error}")
    if claim is None or claim.content != snapshot.content:
        return _recovery_pending(recoveries, "public legacy cache and recovery claim differ")
    return ModelCacheAdoption(
        status="cleanup-ready",
        legacy_path=", ".join(str(path) for path in recoveries),
    )


def _claim_legacy_path(config: HieronymusConfig, legacy_path: Path) -> Path | None:
    claim_path = _unique_adoption_path(config, "legacy")
    try:
        os.rename(legacy_path, claim_path)
    except FileNotFoundError:
        return None
    return claim_path


def _restore_claim(config: HieronymusConfig, claim_path: Path) -> bool:
    legacy_path = _legacy_model_cache_path(config)
    try:
        os.link(claim_path, legacy_path)
    except FileExistsError:
        return False
    except OSError:
        return False
    try:
        _fsync_directory(config.data_root)
        os.unlink(claim_path)
        _fsync_directory(config.data_root)
    except OSError:
        return False
    return True


def _prepublication_failure(
    config: HieronymusConfig,
    claim_path: Path,
    *,
    error: str,
    temp_path: Path | None = None,
) -> ModelCacheAdoption:
    cleanup_error = ""
    if temp_path is not None:
        try:
            os.unlink(temp_path)
        except FileNotFoundError:
            pass
        except OSError as cleanup_failure:
            cleanup_error = str(cleanup_failure)
    if cleanup_error:
        return _recovery_pending(
            (claim_path, temp_path),
            f"{error}; temporary cleanup failed: {cleanup_error}",
        )
    restored = _restore_claim(config, claim_path)
    recovery = str(claim_path) if not restored else ""
    detail = error if not recovery else f"{error}; recovery files: {recovery}"
    return ModelCacheAdoption(
        status="adoption-failed" if restored else "recovery-pending",
        legacy_path=str(_legacy_model_cache_path(config)),
        error=detail,
    )


def _cleanup_private_adoption_files(
    config: HieronymusConfig,
    *paths: Path,
) -> ModelCacheAdoption:
    for path in paths:
        try:
            os.unlink(path)
        except FileNotFoundError:
            continue
        except OSError as error:
            return ModelCacheAdoption(
                status="cleanup-pending",
                legacy_path=str(path),
                error=str(error),
            )
    try:
        _fsync_directory(config.data_root)
    except OSError as error:
        return ModelCacheAdoption(
            status="cleanup-sync-pending",
            error=str(error),
        )
    return ModelCacheAdoption(status="adopted")


def _converge_without_public_legacy(config: HieronymusConfig) -> ModelCacheAdoption:
    recoveries = _recovery_paths(config)
    if not config.llm_cache_path.exists():
        inspection = _inspect_recoveries(config, recoveries) if recoveries else None
        if inspection is None:
            return ModelCacheAdoption(status="not-needed")
        if inspection.status == "recovery-ready":
            claim_path = Path(inspection.legacy_path)
            temporaries = tuple(path for path in recoveries if path != claim_path)
            if temporaries:
                cleanup = _cleanup_private_adoption_files(config, *temporaries)
                if cleanup.status != "adopted":
                    return cleanup
            return _publish_claim(config, claim_path)
        return inspection
    try:
        _fsync_directory(config.data_root)
    except OSError as error:
        return ModelCacheAdoption(status="publication-sync-pending", error=str(error))

    try:
        canonical = _read_bytes(config.llm_cache_path)
        unmatched = tuple(path for path in recoveries if _read_bytes(path) != canonical)
    except OSError as error:
        return _recovery_pending(recoveries, f"cache recovery file could not be read: {error}")
    if unmatched:
        return _recovery_pending(recoveries, "recovery files do not all match the canonical cache")
    if recoveries:
        cleanup = _cleanup_private_adoption_files(config, *recoveries)
        if cleanup.status != "adopted":
            return cleanup
    return ModelCacheAdoption(status="adopted")


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
    except FileNotFoundError:
        return None


def _write_all(descriptor: int, content: bytes) -> None:
    written = 0
    while written < len(content):
        count = os.write(descriptor, content[written:])
        if count <= 0:
            raise OSError("model cache publication write made no progress")
        written += count


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
