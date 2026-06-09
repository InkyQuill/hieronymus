import pytest

from hieronymus.crystals import CrystalStore
from hieronymus.db import connect
from hieronymus.memory_models import TranslationContext
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def _context_for_series(series, *, target_language: str | None = None) -> TranslationContext:
    return TranslationContext(
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=target_language or series.target_language,
        task_type="translation",
    )


def _create_termbase(config, *, target_language: str | None = None) -> Termbase:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    return Termbase(config, _context_for_series(series, target_language=target_language))


def _termbase_for_series(config, slug: str, title: str) -> Termbase:
    series = Registry(config).create_series(
        slug=slug,
        title=title,
        source_language="ja",
        target_language="en",
    )
    return Termbase(config, _context_for_series(series))


def _propose_sense_name(termbase: Termbase) -> int:
    return termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
        tags=["sense"],
        notes="OSO Sense name.",
    )


def _add_rule_crystal(
    termbase: Termbase,
    text: str,
    *,
    semantic_tags: tuple[str, ...] = (),
    status: str = "active",
) -> int:
    return CrystalStore(termbase.config).add_crystal(
        termbase.context,
        crystal_type="rule",
        text=text,
        source_credibility="user_rule",
        confidence=0.95,
        strength=0.8,
        semantic_tags=semantic_tags,
        status=status,
    )


def test_contract_returns_terms_found_in_raw_text(config):
    termbase = _create_termbase(config)
    crystal_id = _add_rule_crystal(
        termbase,
        "攻撃力上昇 is translated as ATK Up, not Attack Increase.",
        semantic_tags=("sense",),
    )

    contract = termbase.contract("ユンは攻撃力上昇を取るべきだと言われた。")

    assert contract[0].id == crystal_id
    assert contract[0].category == "rule"
    assert contract[0].source_text == "攻撃力上昇"
    assert contract[0].canonical_translation == "ATK Up"
    assert "Attack Increase" in contract[0].forbidden_variants
    assert contract[0].tags == ["sense"]
    assert contract[0].notes == "攻撃力上昇 is translated as ATK Up, not Attack Increase."


def test_add_alias_rejects_unknown_kind(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)

    with pytest.raises(ValueError, match="unknown alias kind: typo_variant"):
        termbase.add_alias(term_id, kind="typo_variant", text="Attack Increase", language="en")


def test_propose_rejects_empty_source_text(config):
    termbase = _create_termbase(config)

    with pytest.raises(ValueError, match="source_text must not be empty"):
        termbase.propose(
            category="ability_name",
            source_text="   ",
            canonical_translation="ATK Up",
        )


def test_propose_rejects_empty_canonical_translation(config):
    termbase = _create_termbase(config)

    with pytest.raises(ValueError, match="canonical_translation must not be empty"):
        termbase.propose(
            category="ability_name",
            source_text="攻撃力上昇",
            canonical_translation="   ",
        )


def test_add_alias_rejects_empty_text(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)

    with pytest.raises(ValueError, match="alias text must not be empty"):
        termbase.add_alias(term_id, kind="source_variant", text="   ", language="ja")


def test_add_alias_rejects_term_from_another_series(config):
    series_a = _termbase_for_series(config, "series-a", "Series A")
    series_b = _termbase_for_series(config, "series-b", "Series B")
    term_id = series_b.propose(
        category="spell",
        source_text="魔法",
        canonical_translation="Magic",
    )
    series_b.approve(term_id)

    with pytest.raises(KeyError, match=f"unknown term: {term_id}"):
        series_a.add_alias(
            term_id,
            kind="forbidden_variant",
            text="Sorcery",
            language="en",
        )

    contract = series_b.contract("魔法を使った。")

    assert len(contract) == 1
    assert contract[0].forbidden_variants == []


def test_approve_rejects_unknown_term(config):
    termbase = _create_termbase(config)

    with pytest.raises(KeyError, match="unknown term: 404"):
        termbase.approve(404)


def test_approve_same_strict_term_twice_creates_one_rule_crystal(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)

    termbase.approve(term_id)
    termbase.approve(term_id)

    contract = termbase.contract("ユンは攻撃力上昇を取るべきだと言われた。")
    with connect(termbase.config.database_path) as conn:
        active_rule_count = conn.execute(
            """
            select count(*)
            from crystals
            where crystal_type = 'rule'
              and status = 'active'
              and text = '攻撃力上昇 is translated as ATK Up.'
            """
        ).fetchone()[0]

    assert active_rule_count == 1
    assert len(contract) == 1
    assert contract[0].source_text == "攻撃力上昇"


def test_add_alias_rejects_approved_term(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)
    termbase.approve(term_id)

    with pytest.raises(
        ValueError, match="approved term aliases must be represented as rule crystals"
    ):
        termbase.add_alias(
            term_id,
            kind="forbidden_variant",
            text="Attack Increase",
            language="en",
        )


def test_contract_matches_case_insensitive_source_variant(config):
    termbase = _create_termbase(config)
    _add_rule_crystal(termbase, "atk boost is translated as ATK Up.")

    contract = termbase.contract("Yun should take ATK BOOST.")

    assert [term.canonical_translation for term in contract] == ["ATK Up"]


def test_contract_excludes_pending_terms(config):
    termbase = _create_termbase(config)
    _add_rule_crystal(
        termbase,
        "攻撃力上昇 is translated as ATK Up.",
        status="candidate",
    )

    assert termbase.contract("ユンは攻撃力上昇を取るべきだと言われた。") == []


def test_contract_isolated_by_target_language(config):
    english = _create_termbase(config, target_language="en")
    russian = _create_termbase(config, target_language="ru")
    english_id = english.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
    )
    russian_id = russian.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="Усиление атаки",
    )
    english.approve(english_id)
    russian.approve(russian_id)

    english_contract = english.contract("ユンは攻撃力上昇を取るべきだと言われた。")
    russian_contract = russian.contract("ユンは攻撃力上昇を取るべきだと言われた。")

    assert [term.canonical_translation for term in english_contract] == ["ATK Up"]
    assert [term.canonical_translation for term in russian_contract] == ["Усиление атаки"]


def test_contract_matches_source_variant(config):
    termbase = _create_termbase(config)
    _add_rule_crystal(termbase, "攻撃バフ is translated as ATK Up.")

    contract = termbase.contract("ユンは攻撃バフを取るべきだと言われた。")

    assert [term.source_text for term in contract] == ["攻撃バフ"]


def test_contract_returns_empty_list_when_no_terms_match(config):
    termbase = _create_termbase(config)
    _add_rule_crystal(termbase, "攻撃力上昇 is translated as ATK Up.")

    assert termbase.contract("ユンは防御力上昇を取るべきだと言われた。") == []


def test_rule_crystal_contract_includes_parsed_active_rule(config):
    termbase = _create_termbase(config, target_language="ru")
    crystal_id = _add_rule_crystal(
        termbase,
        "Cooking Talent is translated as Готовка, not Кулинария.",
        semantic_tags=("translation-rule", "cooking"),
    )

    contract = termbase.contract("Cooking Talent")

    assert len(contract) == 1
    assert contract[0].id == crystal_id
    assert contract[0].category == "rule"
    assert contract[0].source_text == "Cooking Talent"
    assert contract[0].canonical_translation == "Готовка"
    assert contract[0].forbidden_variants == ["Кулинария"]
    assert contract[0].tags == ["cooking", "translation-rule"]
    assert contract[0].notes == "Cooking Talent is translated as Готовка, not Кулинария."


def test_unparseable_rule_crystal_remains_out_of_contract(config):
    termbase = _create_termbase(config, target_language="ru")
    _add_rule_crystal(
        termbase,
        "Cooking Talent should usually become Готовка.",
        semantic_tags=("translation-rule", "cooking"),
    )

    assert termbase.contract("Cooking Talent") == []


def test_propose_maintains_terms_fts_row(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)

    with connect(termbase.config.database_path) as conn:
        row = conn.execute("select * from strict_terms_fts where rowid = ?", (term_id,)).fetchone()

    assert row["source_text"] == "攻撃力上昇"
    assert row["canonical_translation"] == "ATK Up"
    assert row["notes"] == "OSO Sense name."
