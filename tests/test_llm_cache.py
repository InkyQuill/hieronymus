from __future__ import annotations

import json
import os
import stat
import threading
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

import hieronymus.llm_cache as llm_cache_module
from hieronymus.config import HieronymusConfig
from hieronymus.llm_cache import (
    CachedModels,
    ModelCacheEntry,
    adopt_legacy_model_cache,
    inspect_legacy_model_cache,
    load_model_cache,
    save_model_cache,
)


def _cache() -> CachedModels:
    return CachedModels().with_entry(
        ModelCacheEntry(
            provider="openai",
            models=("gpt-4.1", "gpt-4.1-mini"),
            fetched_at="2026-06-09T12:00:00+00:00",
            error="",
        )
    )


def _legacy_cache_path(config: HieronymusConfig) -> Path:
    return config.data_root / ("llmcache" + ".tmp")


def test_model_cache_round_trips_provider_models_through_json_cache(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    cache = _cache()

    save_model_cache(config, cache)
    loaded = load_model_cache(config)

    assert config.llm_cache_path == config.data_root / "llm-cache.json"
    assert loaded == cache


def test_load_model_cache_reads_legacy_cache_without_adopting_it(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")

    loaded = load_model_cache(config)

    assert loaded == _cache()
    assert not config.llm_cache_path.exists()
    assert legacy_path.exists()


def test_load_model_cache_of_missing_root_is_empty_and_non_mutating(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert load_model_cache(config) == CachedModels()
    assert not config.data_root.exists()


def test_adopt_legacy_model_cache_publishes_without_overwriting(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "adopted"
    assert load_model_cache(config) == _cache()
    assert config.llm_cache_path.exists()
    assert not legacy_path.exists()


def test_adopt_legacy_model_cache_fsyncs_parent_directory(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
    fsynced_directories = 0
    fsynced_files = 0

    def record_fsync(descriptor: int) -> None:
        nonlocal fsynced_directories, fsynced_files
        mode = os.fstat(descriptor).st_mode
        if stat.S_ISDIR(mode):
            fsynced_directories += 1
        elif stat.S_ISREG(mode):
            fsynced_files += 1
        else:
            pytest.fail("cache adoption fsynced an unexpected descriptor type")

    monkeypatch.setattr(os, "fsync", record_fsync)

    assert adopt_legacy_model_cache(config).status == "adopted"
    assert fsynced_files == 1
    assert fsynced_directories == 2


def test_load_model_cache_leaves_new_cache_unchanged(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_model_cache(config, _cache())
    original = config.llm_cache_path.read_bytes()

    assert load_model_cache(config) == _cache()
    assert config.llm_cache_path.read_bytes() == original


def test_load_model_cache_prefers_new_cache_when_both_exist(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    old_cache = _cache()
    new_cache = CachedModels().with_entry(
        ModelCacheEntry(
            provider="gemini",
            models=("gemini-2.5-flash",),
            fetched_at="2026-06-09T12:00:00+00:00",
        )
    )
    save_model_cache(config, new_cache)
    legacy_path = _legacy_cache_path(config)
    legacy_path.write_text(json.dumps(old_cache.to_payload()), encoding="utf-8")

    assert load_model_cache(config) == new_cache
    assert legacy_path.exists()


def test_load_model_cache_does_not_adopt_malformed_legacy_cache(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text("{not json", encoding="utf-8")

    assert load_model_cache(config) == CachedModels()
    assert legacy_path.exists()
    assert not config.llm_cache_path.exists()


def test_adoption_uses_stable_bytes_when_legacy_path_is_replaced(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)
    replacement = b'{"providers": {}}'
    original_rename = os.rename

    def replacing_rename(source, target):
        result = original_rename(source, target)
        if Path(source) == legacy_path:
            replacement_path = legacy_path.with_name("replacement-cache")
            replacement_path.write_bytes(replacement)
            replacement_path.replace(legacy_path)
        return result

    monkeypatch.setattr(os, "rename", replacing_rename)

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "adopted"
    assert config.llm_cache_path.read_bytes() == original
    assert legacy_path.read_bytes() == replacement

    retry = adopt_legacy_model_cache(config)
    assert retry.status == "recovery-pending"
    assert ".legacy" in retry.error
    assert config.llm_cache_path.read_bytes() == original
    assert legacy_path.read_bytes() == replacement


def test_adoption_exclusive_create_never_clobbers_racing_canonical(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
    newer = b'{"providers": {}}'
    original_link = os.link

    def racing_link(source, target):
        if Path(target) == config.llm_cache_path:
            config.llm_cache_path.write_bytes(newer)
        return original_link(source, target)

    monkeypatch.setattr(os, "link", racing_link)

    assert adopt_legacy_model_cache(config).status == "conflict"
    assert config.llm_cache_path.read_bytes() == newer
    assert legacy_path.exists()


def test_repeated_conflicts_never_leak_owned_canonical_temps(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    different = b'{"providers": {}}'
    config.llm_cache_path.write_bytes(different)

    for _ in range(3):
        _legacy_cache_path(config).write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
        assert adopt_legacy_model_cache(config).status == "conflict"
        assert not tuple(config.data_root.glob("*.canonical"))
        assert _legacy_cache_path(config).exists()

    assert config.llm_cache_path.read_bytes() == different


def test_observer_waits_for_publisher_and_never_cleans_live_adoption(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)
    original_link = os.link
    publication_ready = threading.Event()
    allow_publication = threading.Event()
    observer_started = threading.Event()

    def synchronized_link(source, target):
        if Path(target) == config.llm_cache_path:
            publication_ready.set()
            assert allow_publication.wait(timeout=2)
        return original_link(source, target)

    monkeypatch.setattr(os, "link", synchronized_link)

    def inspect_after_start():
        observer_started.set()
        return inspect_legacy_model_cache(config)

    with ThreadPoolExecutor(max_workers=2) as executor:
        publisher = executor.submit(adopt_legacy_model_cache, config)
        assert publication_ready.wait(timeout=2)
        observer = executor.submit(inspect_after_start)
        assert observer_started.wait(timeout=2)
        with pytest.raises(TimeoutError):
            observer.result(timeout=0.2)
        allow_publication.set()
        publisher_result = publisher.result(timeout=2)
        observer_result = observer.result(timeout=2)

    assert publisher_result.status == "adopted"
    assert observer_result.status == "not-needed"
    assert config.llm_cache_path.read_bytes() == original
    assert adopt_legacy_model_cache(config).status == "adopted"
    assert not legacy_path.exists()


def test_reader_waits_for_publication_and_returns_canonical_cache(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
    original_link = os.link
    publication_ready = threading.Event()
    allow_publication = threading.Event()
    reader_started = threading.Event()

    def synchronized_link(source, target):
        if Path(target) == config.llm_cache_path:
            publication_ready.set()
            assert allow_publication.wait(timeout=2)
        return original_link(source, target)

    monkeypatch.setattr(os, "link", synchronized_link)

    def load_after_start():
        reader_started.set()
        return load_model_cache(config)

    with ThreadPoolExecutor(max_workers=2) as executor:
        publisher = executor.submit(adopt_legacy_model_cache, config)
        assert publication_ready.wait(timeout=2)
        reader = executor.submit(load_after_start)
        assert reader_started.wait(timeout=2)
        with pytest.raises(TimeoutError):
            reader.result(timeout=0.2)
        allow_publication.set()

        assert publisher.result(timeout=2).status == "adopted"
        assert reader.result(timeout=2) == _cache()


def test_model_cache_read_reuses_held_adoption_lock_without_deadlock(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    save_model_cache(config, _cache())

    def nested_read():
        with llm_cache_module._adoption_lock(config) as locked:
            assert locked
            return load_model_cache(config)

    with ThreadPoolExecutor(max_workers=1) as executor:
        result = executor.submit(nested_read).result(timeout=2)

    assert result == _cache()


def test_inspection_locks_existing_directory_without_mutating_it(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    before = tuple(config.data_root.iterdir())
    lock_operations: list[int] = []

    def record_flock(_descriptor: int, operation: int) -> None:
        lock_operations.append(operation)

    assert hasattr(llm_cache_module, "fcntl")
    monkeypatch.setattr(llm_cache_module.fcntl, "flock", record_flock)

    assert inspect_legacy_model_cache(config).status == "not-needed"
    assert tuple(config.data_root.iterdir()) == before
    assert len(lock_operations) == 2


def test_inspection_of_missing_data_root_creates_nothing(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")

    assert inspect_legacy_model_cache(config).status == "not-needed"
    assert not config.data_root.exists()


def test_crash_after_claim_is_reported_and_recovered_on_retry(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)

    monkeypatch.setattr(
        "hieronymus.llm_cache._read_legacy_snapshot",
        lambda _path: (_ for _ in ()).throw(KeyboardInterrupt()),
    )
    with pytest.raises(KeyboardInterrupt):
        adopt_legacy_model_cache(config)
    monkeypatch.undo()

    inspection = inspect_legacy_model_cache(config)
    assert inspection.status == "recovery-ready"
    assert inspection.legacy_path
    assert Path(inspection.legacy_path).exists()

    assert adopt_legacy_model_cache(config).status == "adopted"
    assert config.llm_cache_path.read_bytes() == original
    assert not tuple(config.data_root.glob(".llm-cache-adoption-*"))


def test_crash_after_private_temp_is_reported_and_recovered_on_retry(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)
    original_link = os.link

    def interrupt_publication(source, destination, *args, **kwargs):
        if Path(source).suffix == ".canonical":
            raise KeyboardInterrupt()
        return original_link(source, destination, *args, **kwargs)

    monkeypatch.setattr(os, "link", interrupt_publication)
    with pytest.raises(KeyboardInterrupt):
        adopt_legacy_model_cache(config)
    monkeypatch.undo()

    inspection = inspect_legacy_model_cache(config)
    assert inspection.status == "recovery-ready"
    assert tuple(config.data_root.glob("*.legacy"))
    assert tuple(config.data_root.glob("*.canonical"))

    assert adopt_legacy_model_cache(config).status == "adopted"
    assert config.llm_cache_path.read_bytes() == original
    assert not tuple(config.data_root.glob(".llm-cache-adoption-*"))


def test_write_interruption_closes_private_temp_descriptor(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
    original_open = os.open
    private_descriptors: list[int] = []

    def record_private_open(path, flags, mode=0o777):
        descriptor = original_open(path, flags, mode)
        if Path(path).suffix == ".canonical":
            private_descriptors.append(descriptor)
        return descriptor

    monkeypatch.setattr(os, "open", record_private_open)
    monkeypatch.setattr(
        "hieronymus.llm_cache._write_all",
        lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()),
    )

    with pytest.raises(KeyboardInterrupt):
        adopt_legacy_model_cache(config)

    assert private_descriptors
    with pytest.raises(OSError):
        os.fstat(private_descriptors[-1])


def test_retry_converges_identical_public_legacy_and_private_claim(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    different = b'{"providers": {}}'
    legacy_path.write_bytes(original)
    config.llm_cache_path.write_bytes(different)

    monkeypatch.setattr(
        "hieronymus.llm_cache._fsync_directory",
        lambda *_: (_ for _ in ()).throw(OSError("restore fsync")),
    )
    assert adopt_legacy_model_cache(config).status == "recovery-pending"
    monkeypatch.undo()

    assert legacy_path.read_bytes() == original
    assert tuple(config.data_root.glob("*.legacy"))
    assert inspect_legacy_model_cache(config).status == "cleanup-ready"

    retry = adopt_legacy_model_cache(config)
    assert retry.status == "conflict"
    assert legacy_path.read_bytes() == original
    assert config.llm_cache_path.read_bytes() == different
    assert not tuple(config.data_root.glob(".llm-cache-adoption-*"))


def test_recovery_read_errors_preserve_every_artifact(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    canonical = config.llm_cache_path
    recovery = config.data_root / ".llm-cache-adoption-unreadable.canonical"
    canonical.write_bytes(b'{"providers": {}}')
    recovery.write_bytes(b'{"providers": {}}')
    original_read_bytes = Path.read_bytes

    def fail_recovery_reads(path: Path) -> bytes:
        if path in {canonical, recovery}:
            raise OSError("unreadable")
        return original_read_bytes(path)

    monkeypatch.setattr(Path, "read_bytes", fail_recovery_reads)

    inspection = inspect_legacy_model_cache(config)
    adoption = adopt_legacy_model_cache(config)

    assert inspection.status == "recovery-pending"
    assert adoption.status == "recovery-pending"
    assert canonical.exists()
    assert recovery.exists()


def test_ambiguous_orphan_claims_are_reported_and_preserved(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    first = config.data_root / ".llm-cache-adoption-first.legacy"
    second = config.data_root / ".llm-cache-adoption-second.legacy"
    first.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
    second.write_bytes(b'{"providers": {}}')

    inspection = inspect_legacy_model_cache(config)

    assert inspection.status == "recovery-pending"
    assert str(first) in inspection.error
    assert str(second) in inspection.error
    assert adopt_legacy_model_cache(config).status == "recovery-pending"
    assert first.exists()
    assert second.exists()
    assert not config.llm_cache_path.exists()


def test_public_legacy_does_not_hide_orphan_recovery_files(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    legacy_path = _legacy_cache_path(config)
    recovery_path = config.data_root / ".llm-cache-adoption-orphan.legacy"
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
    recovery_path.write_bytes(b'{"providers": {}}')

    inspection = inspect_legacy_model_cache(config)

    assert inspection.status == "recovery-pending"
    assert str(recovery_path) in inspection.error
    assert legacy_path.exists()
    assert recovery_path.exists()


@pytest.mark.parametrize(
    "failure",
    ["open", "read", "write", "file-fsync"],
)
def test_failed_publication_removes_partial_canonical_and_preserves_legacy(
    tmp_path, monkeypatch, failure
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)

    if failure == "open":
        original_open = os.open

        def fail_legacy_open(path, flags, mode=0o777):
            if Path(path).suffix == ".legacy" and flags == os.O_RDONLY:
                raise OSError("open")
            return original_open(path, flags, mode)

        monkeypatch.setattr(os, "open", fail_legacy_open)
    elif failure == "read":
        monkeypatch.setattr(os, "read", lambda *_: (_ for _ in ()).throw(OSError("read")))
    elif failure == "write":
        monkeypatch.setattr(os, "write", lambda *_: (_ for _ in ()).throw(OSError("write")))
    elif failure == "file-fsync":
        original_fsync = os.fsync

        def fail_file_fsync(descriptor):
            if stat.S_ISREG(os.fstat(descriptor).st_mode):
                raise OSError("file fsync")
            original_fsync(descriptor)

        monkeypatch.setattr(os, "fsync", fail_file_fsync)
    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "adoption-failed"
    assert legacy_path.read_bytes() == original
    assert not config.llm_cache_path.exists()


def test_publication_fsync_failure_leaves_valid_canonical_for_retry(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)

    monkeypatch.setattr(
        "hieronymus.llm_cache._fsync_directory",
        lambda *_: (_ for _ in ()).throw(OSError("publication fsync")),
    )

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "publication-sync-pending"
    assert config.llm_cache_path.read_bytes() == original

    monkeypatch.undo()
    fsync_calls = 0

    def record_retry_fsync(_path):
        nonlocal fsync_calls
        fsync_calls += 1

    monkeypatch.setattr("hieronymus.llm_cache._fsync_directory", record_retry_fsync)

    retry = adopt_legacy_model_cache(config)

    assert retry.status == "adopted"
    assert fsync_calls >= 1


def test_failed_publisher_does_not_remove_replacement_canonical(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    legacy_path.write_text(json.dumps(_cache().to_payload()), encoding="utf-8")
    replacement = b'{"providers": {}}'

    def replace_then_fail(_descriptor, _content):
        config.llm_cache_path.write_bytes(replacement)
        raise OSError("publisher lost ownership")

    monkeypatch.setattr("hieronymus.llm_cache._write_all", replace_then_fail)

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "adoption-failed"
    assert config.llm_cache_path.read_bytes() == replacement
    assert legacy_path.exists()


def test_unlink_failure_is_reported_as_cleanup_pending(tmp_path, monkeypatch) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)
    original_unlink = os.unlink

    def fail_legacy_unlink(path, *args, **kwargs):
        if Path(path).suffix == ".legacy":
            raise OSError("unlink")
        return original_unlink(path, *args, **kwargs)

    monkeypatch.setattr(os, "unlink", fail_legacy_unlink)

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "cleanup-pending"
    assert config.llm_cache_path.read_bytes() == original
    assert not legacy_path.exists()

    monkeypatch.undo()
    assert adopt_legacy_model_cache(config).status == "adopted"
    assert not legacy_path.exists()


def test_cleanup_fsync_failure_is_not_reported_as_publication_failure(
    tmp_path, monkeypatch
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)
    calls = 0

    def fail_second_fsync(_path):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise OSError("cleanup fsync")

    monkeypatch.setattr("hieronymus.llm_cache._fsync_directory", fail_second_fsync)

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "cleanup-sync-pending"
    assert config.llm_cache_path.read_bytes() == original
    assert not legacy_path.exists()

    monkeypatch.undo()
    fsync_calls = 0

    def record_retry_fsync(_path):
        nonlocal fsync_calls
        fsync_calls += 1

    monkeypatch.setattr("hieronymus.llm_cache._fsync_directory", record_retry_fsync)

    retry = adopt_legacy_model_cache(config)

    assert retry.status == "adopted"
    assert fsync_calls >= 1


def test_writer_recreating_public_legacy_during_cleanup_is_never_deleted(
    tmp_path, monkeypatch
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    legacy_path.write_bytes(original)
    replacement = b'{"providers": {}}'
    fsync_calls = 0

    def recreate_legacy_on_publication_sync(_path):
        nonlocal fsync_calls
        fsync_calls += 1
        if fsync_calls == 1:
            replacement_path = legacy_path.with_name("writer-replacement")
            replacement_path.write_bytes(replacement)
            replacement_path.replace(legacy_path)

    monkeypatch.setattr(
        "hieronymus.llm_cache._fsync_directory",
        recreate_legacy_on_publication_sync,
    )

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "adopted"
    assert config.llm_cache_path.read_bytes() == original
    assert legacy_path.read_bytes() == replacement


def test_retry_with_identical_canonical_finishes_legacy_cleanup(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    _legacy_cache_path(config).write_bytes(original)
    config.llm_cache_path.write_bytes(original)

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "adopted"
    assert not _legacy_cache_path(config).exists()
    assert config.llm_cache_path.read_bytes() == original


def test_retry_with_different_canonical_remains_conflict(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    original = json.dumps(_cache().to_payload()).encode()
    _legacy_cache_path(config).write_bytes(original)
    different = b'{"providers": {}}'
    config.llm_cache_path.write_bytes(different)

    assert adopt_legacy_model_cache(config).status == "conflict"
    assert _legacy_cache_path(config).read_bytes() == original
    assert config.llm_cache_path.read_bytes() == different


@pytest.mark.parametrize(
    "malformed_entry",
    [
        {
            "provider": "openai",
            "models": 123,
            "fetched_at": "2026-06-09T12:00:00+00:00",
            "error": "",
        },
        {
            "provider": "openai",
            "models": ["gpt-4.1", 123],
            "fetched_at": "2026-06-09T12:00:00+00:00",
            "error": "",
        },
        {
            "provider": "openai",
            "models": ["gpt-4.1"],
            "fetched_at": 123,
            "error": "",
        },
        {
            "provider": "openai",
            "models": ["gpt-4.1"],
            "fetched_at": "2026-06-09T12:00:00+00:00",
            "error": 123,
        },
    ],
)
def test_adopt_legacy_model_cache_rejects_malformed_provider_schema(
    tmp_path, malformed_entry
) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    legacy_path = _legacy_cache_path(config)
    legacy_path.parent.mkdir(parents=True)
    malformed = {"providers": {"openai": malformed_entry}}
    original = json.dumps(malformed).encode()
    legacy_path.write_bytes(original)

    adoption = adopt_legacy_model_cache(config)

    assert adoption.status == "invalid-legacy"
    assert legacy_path.read_bytes() == original
    assert not config.llm_cache_path.exists()


def test_model_cache_entry_is_stale_after_24_hours_exactly() -> None:
    fetched_at = datetime(2026, 6, 8, 12, 0, 0, tzinfo=UTC)
    entry = ModelCacheEntry(
        provider="anthropic",
        models=("claude-3-5-haiku-latest",),
        fetched_at=fetched_at.isoformat(),
    )

    assert entry.is_stale(fetched_at + timedelta(hours=24)) is True


def test_model_cache_entry_is_not_stale_before_24_hours() -> None:
    fetched_at = datetime(2026, 6, 8, 12, 0, 0, tzinfo=UTC)
    entry = ModelCacheEntry(
        provider="gemini",
        models=("gemini-2.5-flash",),
        fetched_at=fetched_at.isoformat(),
    )

    assert entry.is_stale(fetched_at + timedelta(hours=24) - timedelta(microseconds=1)) is False


def test_model_cache_entry_with_future_fetched_at_is_stale() -> None:
    now = datetime(2026, 6, 9, 12, 0, 0, tzinfo=UTC)
    entry = ModelCacheEntry(
        provider="openai",
        models=("gpt-4.1-mini",),
        fetched_at=(now + timedelta(seconds=1)).isoformat(),
    )

    assert entry.is_stale(now) is True


def test_load_model_cache_tolerates_invalid_json(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text("{not json", encoding="utf-8")

    assert load_model_cache(config) == CachedModels()


def test_load_model_cache_normalizes_provider_to_map_key(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text(
        "{"
        '"providers": {'
        '"openai": {'
        '"provider": "gemini",'
        '"models": ["gpt-4.1-mini"],'
        '"fetched_at": "2026-06-09T12:00:00+00:00",'
        '"error": ""'
        "}"
        "}"
        "}",
        encoding="utf-8",
    )

    assert load_model_cache(config).providers["openai"].provider == "openai"


def test_load_model_cache_skips_bad_datetime_entries(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text(
        "{"
        '"providers": {'
        '"openai": {'
        '"provider": "openai",'
        '"models": ["gpt-4.1-mini"],'
        '"fetched_at": "not-a-date",'
        '"error": "model suggestions unavailable"'
        "}"
        "}"
        "}",
        encoding="utf-8",
    )

    assert load_model_cache(config) == CachedModels()


def test_load_model_cache_skips_success_entries_without_valid_models(tmp_path) -> None:
    config = HieronymusConfig(data_root=tmp_path / "hieronymus")
    config.data_root.mkdir(parents=True)
    config.llm_cache_path.write_text(
        "{"
        '"providers": {'
        '"openai": {'
        '"provider": "openai",'
        '"models": [1, null],'
        '"fetched_at": "2026-06-09T12:00:00+00:00",'
        '"error": ""'
        "}"
        "}"
        "}",
        encoding="utf-8",
    )

    assert load_model_cache(config) == CachedModels()
