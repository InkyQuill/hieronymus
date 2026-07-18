use super::{DbError, MISSING_FTS5_PROTOCOL_ERROR, is_memory_url, map_connect_error};

#[test]
fn memory_url_detection_uses_sqlite_url_semantics() {
    for url in [
        "sqlite::memory:",
        "sqlite://:memory:",
        "sqlite://?mode=memory",
        "sqlite://file:shared?cache=shared&mode=memory",
    ] {
        assert!(is_memory_url(url), "{url} should be an in-memory URL");
    }
    for url in [
        "sqlite://memory.sqlite",
        "sqlite://mode=memory.sqlite",
        "sqlite://file.sqlite?cache=shared",
    ] {
        assert!(!is_memory_url(url), "{url} should be a file URL");
    }
}

#[test]
fn fts5_probe_protocol_failure_maps_to_distinct_error() {
    let error = map_connect_error(
        "sqlite::memory:",
        sqlx::Error::Protocol(MISSING_FTS5_PROTOCOL_ERROR.to_owned()),
    );

    assert!(matches!(error, DbError::MissingFts5));
}
