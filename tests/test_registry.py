import pytest

from hieronymus import registry as registry_module
from hieronymus.registry import Registry


def test_create_series_initializes_global_database(config):
    registry = Registry(config)

    series = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    assert series.slug == "only-sense-online"
    assert not hasattr(series, "database_path")
    assert config.database_path.exists()
    assert registry.get_series("only-sense-online").title == "Only Sense Online"
    assert registry.list_series() == [series]


def test_create_series_upserts_existing_slug(config):
    registry = Registry(config)
    registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    updated = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online Rebuild",
        source_language="jp",
        target_language="ru",
    )

    assert updated.title == "Only Sense Online Rebuild"
    assert updated.source_language == "jp"
    assert updated.target_language == "ru"
    assert registry.get_series("only-sense-online") == updated
    assert registry.list_series() == [updated]


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
    assert list(config.data_root.rglob("*.sqlite")) == [config.database_path]
    with pytest.raises(KeyError, match=f"unknown series: {slug}"):
        registry.get_series(slug)


def test_registry_raises_when_global_migration_fails(config, monkeypatch):
    original_apply_migration = registry_module.apply_migration

    def fail_global_migration(conn, name):
        if name == "global.sql":
            raise RuntimeError("global migration failed")
        original_apply_migration(conn, name)

    monkeypatch.setattr(registry_module, "apply_migration", fail_global_migration)

    with pytest.raises(RuntimeError, match="global migration failed"):
        Registry(config)
