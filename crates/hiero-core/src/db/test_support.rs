use std::str::FromStr;

use sqlx::{SqliteConnection, sqlite::SqliteConnectOptions};

use super::{DbError, Fts5Probe, Fts5ProbeError, build_pool_with_probe, memory_settings};

#[test]
fn memory_url_detection_uses_sqlite_url_semantics() {
    for url in [
        "sqlite::memory:",
        "sqlite://:memory:",
        "sqlite://?mode=memory",
        "sqlite://file:shared?cache=shared&mode=memory",
    ] {
        assert!(
            memory_settings(url).is_memory,
            "{url} should be an in-memory URL"
        );
    }
    for url in [
        "sqlite://memory.sqlite",
        "sqlite://mode=memory.sqlite",
        "sqlite://file.sqlite?cache=shared",
    ] {
        assert!(
            !memory_settings(url).is_memory,
            "{url} should be a file URL"
        );
    }
}

struct MissingFts5Probe;

impl Fts5Probe for MissingFts5Probe {
    async fn check(&self, _connection: &mut SqliteConnection) -> Result<(), Fts5ProbeError> {
        Err(Fts5ProbeError::Missing)
    }
}

#[tokio::test]
async fn injected_preflight_failure_exits_pool_construction_as_missing_fts5() {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("test connection options should parse");

    let error = build_pool_with_probe(
        options,
        "injected preflight test".to_owned(),
        &MissingFts5Probe,
    )
    .await
    .expect_err("missing FTS5 should fail before constructing the pool");

    assert!(matches!(error, DbError::MissingFts5));
}
