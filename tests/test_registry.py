import pytest

from hieronymus import registry as registry_module
from hieronymus.registry import Registry


def test_create_series_initializes_registry_and_series_database(config):
    registry = Registry(config)

    series = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    assert series.slug == "only-sense-online"
    assert config.registry_path.exists()
    assert (config.series_dir / "only-sense-online.sqlite").exists()
    assert registry.get_series("only-sense-online").title == "Only Sense Online"


@pytest.mark.parametrize("slug", ["../../escape", "bad/slug"])
def test_create_series_rejects_filename_unsafe_slugs(config, slug):
    registry = Registry(config)

    with pytest.raises(ValueError, match="invalid series slug"):
        registry.create_series(
            slug=slug,
            title="Unsafe",
            source_language="ja",
            target_language="en",
        )

    assert not (config.data_root.parent / "escape.sqlite").exists()
    assert not (config.series_dir / "bad" / "slug.sqlite").exists()
    assert list(config.series_dir.rglob("*.sqlite")) == []


def test_create_series_does_not_record_series_when_series_migration_fails(config, monkeypatch):
    registry = Registry(config)
    original_apply_migration = registry_module.apply_migration

    def fail_series_migration(conn, name):
        if name == "series.sql":
            raise RuntimeError("series migration failed")
        original_apply_migration(conn, name)

    monkeypatch.setattr(registry_module, "apply_migration", fail_series_migration)

    with pytest.raises(RuntimeError, match="series migration failed"):
        registry.create_series(
            slug="migration-failure",
            title="Migration Failure",
            source_language="ja",
            target_language="en",
        )

    with pytest.raises(KeyError, match="unknown series: migration-failure"):
        registry.get_series("migration-failure")
