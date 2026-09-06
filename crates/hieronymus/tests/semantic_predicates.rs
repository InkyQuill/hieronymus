use hieronymus::semantic_index::series_predicate;
#[test]
fn predicate_rejects_control_and_quote_injection() {
    assert_eq!(
        series_predicate("book-one").unwrap(),
        "series_slug = 'book-one'"
    );
    for slug in ["x' OR true", "x;drop", "x\ny", ""] {
        assert!(series_predicate(slug).is_err(), "{slug:?}");
    }
}
