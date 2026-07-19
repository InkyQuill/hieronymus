use hiero_core::domain::{ParsedRule, parse_rule};

#[test]
fn parses_supported_rule_grammar_and_normalizes_whitespace() {
    assert_eq!(
        parse_rule("  ガンツ is translated as Gantz, not Ganz.  "),
        Some(ParsedRule {
            source_text: "ガンツ".into(),
            canonical: "Gantz".into(),
            forbidden: vec!["Ganz".into()],
        })
    );
    assert_eq!(
        parse_rule("攻撃力上昇 is translated as Attack Increase"),
        Some(ParsedRule {
            source_text: "攻撃力上昇".into(),
            canonical: "Attack Increase".into(),
            forbidden: vec![],
        })
    );
}

#[test]
fn rejects_ambiguous_or_noncanonical_free_text_without_guessing() {
    for text in [
        "",
        "A translates as B.",
        "A IS TRANSLATED AS B.",
        "A is translated as B, not C, not D.",
        "A is translated as B is translated as C.",
        "A, not X is translated as B.",
        "A is translated as .",
        " is translated as B.",
        "A is translated as B。",
        "A is translated as B..",
        "A is translated as B?",
        "A is translated as B!",
        "A is translated as B, not C..",
        "A is translated as B, not C?",
        "A is translated as B, not C!",
        "A is translated as B, not C。",
        "A is translated as B;",
        "A is translated as B,",
        "A is translated as B…",
        "A is translated as B،",
        "A is translated as B、",
        "A is translated as B, not C;",
        "A is translated as B, not C…",
    ] {
        assert_eq!(parse_rule(text), None, "{text:?} must be rejected");
    }
}

#[test]
fn preserves_supported_internal_punctuation() {
    assert_eq!(
        parse_rule("Mr. A is translated as B/C, not D-E."),
        Some(ParsedRule {
            source_text: "Mr. A".into(),
            canonical: "B/C".into(),
            forbidden: vec!["D-E".into()],
        })
    );
    assert!(parse_rule("A is translated as B²").is_some());
    assert!(parse_rule("A is translated as B★").is_some());
    assert!(parse_rule("A is translated as BΩ").is_some());
    assert!(parse_rule("Who? is translated as Кто").is_some());
}
