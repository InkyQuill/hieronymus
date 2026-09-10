//! Strengthened FTS5 fallback proofs (Task 11).
//!
//! The helpers consume the deterministic corpus recipe and operate only on a
//! temporary SQLite database. The FTS path is compiled and exercised without
//! the native features: no model, runtime, or vector index participates, and
//! no download probe exists on this path.

use hieronymus_semantic_native_qualification::fts::{self, FtsReceipt};

fn expected_fts_id_digests() -> anyhow::Result<Vec<String>> {
    fts::expected_fts_id_digests()
}

fn run_all_fts_queries() -> anyhow::Result<Vec<FtsReceipt>> {
    fts::run_all_fts_queries()
}

fn digest_receipts(receipts: &[FtsReceipt]) -> Vec<String> {
    fts::digest_receipts(receipts)
}

fn delete_and_rebuild_fts() -> anyhow::Result<()> {
    fts::delete_and_rebuild_fts()
}

#[test]
fn fts_fallback_is_nonempty_isolated_and_rebuild_equivalent() -> anyhow::Result<()> {
    let expected = expected_fts_id_digests()?;
    let before = run_all_fts_queries()?;
    assert!(before.iter().all(|receipt| receipt.eligible_count > 0));
    assert!(before.iter().all(|receipt| receipt.cross_series_count == 0));
    assert_eq!(digest_receipts(&before), expected);
    delete_and_rebuild_fts()?;
    assert_eq!(digest_receipts(&run_all_fts_queries()?), expected);
    Ok(())
}

#[test]
fn expected_fts_digests_cover_all_fifty_queries() -> anyhow::Result<()> {
    let expected = expected_fts_id_digests()?;
    assert_eq!(expected.len(), 50);
    assert!(
        expected
            .iter()
            .all(|digest| digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit())),
        "every digest must be lower-case hexadecimal SHA-256"
    );
    Ok(())
}
