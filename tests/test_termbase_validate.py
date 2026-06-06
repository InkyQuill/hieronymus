from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def test_validate_flags_forbidden_variant(config):
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
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.approve(term_id)

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    assert findings[0].severity == "high"
    assert findings[0].observed == "Attack Increase"
    assert findings[0].expected == "ATK Up"


def test_validate_flags_missing_canonical_when_source_present(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = Termbase(series.database_path)
    term_id = termbase.propose(
        category="person_name",
        source_text="ガンツ",
        canonical_translation="Ganz",
    )
    termbase.approve(term_id)

    findings = termbase.validate(raw_text="ガンツが笑った。", translated_text="Gantz laughed.")

    assert findings[0].kind == "missing_canonical"
    assert findings[0].expected == "Ganz"
