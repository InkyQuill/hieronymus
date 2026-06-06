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
