from __future__ import annotations

from hieronymus.db import connect
from hieronymus.registry import Registry


def test_create_series_accepts_language_tags_without_translation_direction(config):
    registry = Registry(config)

    series = registry.create_series(
        slug="book-of-friends",
        title="Book of Friends",
        language_tags=["ja", "en", "ru"],
    )

    assert series.slug == "book-of-friends"
    assert series.source_language == ""
    assert series.target_language == ""
    assert series.language_tags == ("en", "ja", "ru")
    assert registry.get_series("book-of-friends") == series


def test_legacy_series_languages_seed_language_tags(config):
    registry = Registry(config)

    series = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="JA",
        target_language=" en ",
    )

    assert series.source_language == "JA"
    assert series.target_language == " en "
    assert series.language_tags == ("en", "ja")

    with connect(config.database_path) as conn:
        rows = conn.execute(
            """
            select language_tag
            from series_language_tags
            where series_id = ?
            order by language_tag
            """,
            (series.id,),
        ).fetchall()

    assert [row["language_tag"] for row in rows] == ["en", "ja"]


def test_explicit_empty_language_tags_do_not_seed_from_legacy_languages(config):
    registry = Registry(config)

    series = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
        language_tags=[],
    )

    assert series.source_language == "ja"
    assert series.target_language == "en"
    assert series.language_tags == ()
    assert registry.get_series("only-sense-online").language_tags == ()


def test_set_series_language_tags_does_not_change_compatibility_fields(config):
    registry = Registry(config)
    series = registry.create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )

    registry.set_series_language_tags(series.id, ["RU", " en ", "ru"])

    updated = registry.get_series("only-sense-online")
    assert updated.source_language == "ja"
    assert updated.target_language == "en"
    assert updated.language_tags == ("en", "ru")


def test_mcp_series_list_returns_language_tags_without_direction_fields(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    created = mcp_server.hieronymus_series_create(
        slug="book-of-friends",
        title="Book of Friends",
        language_tags=["ja", "en", "ru"],
    )
    series_id = created["id"]

    assert created == {
        "id": series_id,
        "slug": "book-of-friends",
        "title": "Book of Friends",
        "source_language": "",
        "target_language": "",
        "language_tags": ["en", "ja", "ru"],
    }
    assert mcp_server.hieronymus_series_list() == [created]

    mcp_server.hieronymus_series_set_language_tags(series_id, ["fr", "EN"])

    assert mcp_server.hieronymus_series_list() == [
        {
            "id": series_id,
            "slug": "book-of-friends",
            "title": "Book of Friends",
            "source_language": "",
            "target_language": "",
            "language_tags": ["en", "fr"],
        }
    ]


def test_mcp_omitted_language_tags_seed_from_legacy_languages(monkeypatch, tmp_path):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    created = mcp_server.hieronymus_series_create(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="JA",
        target_language=" en ",
    )
    series_id = created["id"]

    assert created == {
        "id": series_id,
        "slug": "only-sense-online",
        "title": "Only Sense Online",
        "source_language": "JA",
        "target_language": " en ",
        "language_tags": ["en", "ja"],
    }
    assert mcp_server.hieronymus_series_list() == [created]


def test_mcp_explicit_empty_language_tags_do_not_seed_from_legacy_languages(
    monkeypatch,
    tmp_path,
):
    monkeypatch.setenv("HIERONYMUS_DATA_ROOT", str(tmp_path / "hieronymus"))

    from hieronymus import mcp_server

    created = mcp_server.hieronymus_series_create(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
        language_tags=[],
    )
    series_id = created["id"]

    assert created == {
        "id": series_id,
        "slug": "only-sense-online",
        "title": "Only Sense Online",
        "source_language": "ja",
        "target_language": "en",
        "language_tags": [],
    }
    assert mcp_server.hieronymus_series_list() == [created]
