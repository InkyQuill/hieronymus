import pytest

from hieronymus.db import connect
from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def _create_termbase(config) -> Termbase:
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    return Termbase(
        config.database_path,
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
    )


def _propose_sense_name(termbase: Termbase) -> int:
    return termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
        tags=["sense"],
        notes="OSO Sense name.",
    )


def test_contract_returns_terms_found_in_raw_text(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.approve(term_id)

    contract = termbase.contract("ユンは攻撃力上昇を取るべきだと言われた。")

    assert contract[0].source_text == "攻撃力上昇"
    assert contract[0].canonical_translation == "ATK Up"
    assert "Attack Increase" in contract[0].forbidden_variants
    assert contract[0].tags == ["sense"]
    assert contract[0].notes == "OSO Sense name."


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


def test_approve_rejects_unknown_term(config):
    termbase = _create_termbase(config)

    with pytest.raises(KeyError, match="unknown term: 404"):
        termbase.approve(404)


def test_contract_matches_case_insensitive_source_variant(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)
    termbase.add_alias(
        term_id,
        kind="source_variant",
        text="atk boost",
        language="en",
        case_sensitive=False,
    )
    termbase.approve(term_id)

    contract = termbase.contract("Yun should take ATK BOOST.")

    assert [term.canonical_translation for term in contract] == ["ATK Up"]


def test_contract_excludes_pending_terms(config):
    termbase = _create_termbase(config)
    _propose_sense_name(termbase)

    assert termbase.contract("ユンは攻撃力上昇を取るべきだと言われた。") == []


def test_contract_matches_source_variant(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)
    termbase.add_alias(term_id, kind="source_variant", text="攻撃バフ", language="ja")
    termbase.approve(term_id)

    contract = termbase.contract("ユンは攻撃バフを取るべきだと言われた。")

    assert [term.source_text for term in contract] == ["攻撃力上昇"]


def test_contract_returns_empty_list_when_no_terms_match(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)
    termbase.approve(term_id)

    assert termbase.contract("ユンは防御力上昇を取るべきだと言われた。") == []


def test_propose_maintains_terms_fts_row(config):
    termbase = _create_termbase(config)
    term_id = _propose_sense_name(termbase)

    with connect(termbase.database_path) as conn:
        row = conn.execute("select * from strict_terms_fts where rowid = ?", (term_id,)).fetchone()

    assert row["source_text"] == "攻撃力上昇"
    assert row["canonical_translation"] == "ATK Up"
    assert row["notes"] == "OSO Sense name."
