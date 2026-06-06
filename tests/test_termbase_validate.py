from hieronymus.registry import Registry
from hieronymus.termbase import Termbase


def _termbase_for_series(config, series) -> Termbase:
    return Termbase(
        config.database_path,
        series_slug=series.slug,
        source_language=series.source_language,
        target_language=series.target_language,
    )


def test_validate_flags_forbidden_variant(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
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

    finding = findings[0]
    assert finding.term_id == term_id
    assert finding.kind == "forbidden_variant"
    assert finding.severity == "high"
    assert finding.expected == "ATK Up"
    assert finding.observed == "Attack Increase"
    assert "is forbidden" in finding.message


def test_validate_flags_case_insensitive_forbidden_variant(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
    )
    termbase.add_alias(
        term_id,
        kind="forbidden_variant",
        text="Attack Increase",
        language="en",
        case_sensitive=False,
    )
    termbase.approve(term_id)

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up attack increase.",
    )

    finding = findings[0]
    assert finding.term_id == term_id
    assert finding.kind == "forbidden_variant"
    assert finding.severity == "high"
    assert finding.expected == "ATK Up"
    assert finding.observed == "Attack Increase"
    assert "is forbidden" in finding.message


def test_validate_flags_missing_canonical_when_source_present(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="person_name",
        source_text="ガンツ",
        canonical_translation="Ganz",
    )
    termbase.approve(term_id)

    findings = termbase.validate(raw_text="ガンツが笑った。", translated_text="Gantz laughed.")

    finding = findings[0]
    assert finding.term_id == term_id
    assert finding.kind == "missing_canonical"
    assert finding.severity == "medium"
    assert finding.expected == "Ganz"
    assert finding.observed == ""
    assert "approved form 'Ganz'" in finding.message


def test_validate_accepts_approved_variant_as_final_form(config):
    series = Registry(config).create_series(
        slug="gantz",
        title="Gantz",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="person_name",
        source_text="ガンツ",
        canonical_translation="Ganz",
    )
    termbase.add_alias(term_id, kind="approved_variant", text="Gantz", language="en")
    termbase.approve(term_id)

    findings = termbase.validate(raw_text="ガンツが笑った。", translated_text="Gantz laughed.")

    assert findings == []


def test_validate_returns_no_findings_for_clean_translation(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.approve(term_id)

    findings = termbase.validate(
        raw_text="攻撃力上昇を取るべきだ。",
        translated_text="You should pick up ATK Up.",
    )

    assert findings == []


def test_validate_returns_no_findings_without_contracted_term(config):
    series = Registry(config).create_series(
        slug="only-sense-online",
        title="Only Sense Online",
        source_language="ja",
        target_language="en",
    )
    termbase = _termbase_for_series(config, series)
    term_id = termbase.propose(
        category="ability_name",
        source_text="攻撃力上昇",
        canonical_translation="ATK Up",
    )
    termbase.add_alias(term_id, kind="forbidden_variant", text="Attack Increase", language="en")
    termbase.approve(term_id)

    findings = termbase.validate(
        raw_text="敏捷性上昇を取るべきだ。",
        translated_text="You should pick up Attack Increase.",
    )

    assert findings == []
