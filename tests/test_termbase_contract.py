from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def test_contract_returns_terms_found_in_raw_text(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = Termbase(series.database_path)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
        tags=["sense"],
        notes="OSO Sense name.",
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.approve(term_id)

    contract = termbase.contract("ユンは攻撃力上昇を取るべきだと言われた。")

    assert contract[0].source_text == "攻撃力上昇"
    assert contract[0].canonical_translation == "ATK Up"
    assert "Attack Increase" in contract[0].forbidden_variants
