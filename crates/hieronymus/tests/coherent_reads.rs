use hieronymus::{
    coherent_reads::{CoherentReadError, stable_read},
    data_root::HieronymusConfig,
    db::open_migrated,
};

fn fixture() -> (tempfile::TempDir, HieronymusConfig, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(dir.path());
    let db = open_migrated(&config.database_path()).unwrap();
    db.execute_batch("insert into series(id,slug,title,default_source_language,default_target_language,created_at,updated_at) values(1,'book','Book','en','ru','now','now'); insert into authority_state(series_id,revision) values(1,0);").unwrap();
    (dir, config, db)
}

#[test]
fn unknown_series_cannot_publish_revision_zero() {
    let (_dir, config, _db) = fixture();
    let result: Result<_, CoherentReadError> = stable_read(&config, "absent", None, |_| Ok(()));
    assert!(matches!(result, Err(CoherentReadError::UnknownSeries(_))));
}

#[test]
fn correction_during_snapshot_retries_once_with_fresh_data() {
    let (_dir, config, writer) = fixture();
    let mut calls = 0;
    let result: Result<_, CoherentReadError> = stable_read(&config, "book", None, |snapshot| {
        calls += 1;
        let before = hieronymus::coherent_reads::revision(snapshot, "book")?;
        if calls == 1 {
            writer.execute("update authority_state set revision=1", [])?;
        }
        assert_eq!(
            hieronymus::coherent_reads::revision(snapshot, "book")?,
            before
        );
        Ok(before)
    });
    let result = result.unwrap();
    assert_eq!(calls, 2);
    assert_eq!((result.resulting_revision, result.value), (1, 1));
}

#[test]
fn continued_churn_fails_after_two_snapshots() {
    let (_dir, config, writer) = fixture();
    let mut calls = 0;
    let result: Result<_, CoherentReadError> = stable_read(&config, "book", None, |_| {
        calls += 1;
        writer.execute("update authority_state set revision=revision+1", [])?;
        Ok(())
    });
    assert!(matches!(result, Err(CoherentReadError::StaleContext)));
    assert_eq!(calls, 2);
}

#[test]
fn late_publication_conflict_shares_retry_budget_without_duplicate_effects() {
    let (_dir, config, writer) = fixture();
    let mut reads = 0;
    let mut publications = 0;
    let mut effects = 0;
    let result: Result<_, CoherentReadError> = hieronymus::coherent_reads::stable_read_with_publish(
        &config,
        "book",
        None,
        |_| {
            reads += 1;
            Ok(())
        },
        |observed| {
            publications += 1;
            if publications == 1 {
                writer.execute("update authority_state set revision=1", [])?;
            }
            if hieronymus::coherent_reads::revision(&writer, "book")? != observed.resulting_revision
            {
                return Ok(false);
            }
            effects += 1;
            Ok(true)
        },
    );
    assert_eq!(result.unwrap().resulting_revision, 1);
    assert_eq!((reads, publications, effects), (2, 2, 1));
}

#[test]
fn registered_series_missing_authority_state_cannot_publish() {
    let (_dir, config, db) = fixture();
    db.execute("delete from authority_state", []).unwrap();
    let result: Result<_, CoherentReadError> = stable_read(&config, "book", None, |_| Ok(()));
    assert!(matches!(
        result,
        Err(CoherentReadError::MissingAuthorityState(_))
    ));
}
