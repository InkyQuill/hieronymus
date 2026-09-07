//! A single SQLite snapshot, followed by a freshness read outside that snapshot.
use crate::{authority_models::DecisionResultV1, data_root::HieronymusConfig, db::open_migrated};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, thiserror::Error)]
pub enum CoherentReadError {
    #[error("authority changed during both read attempts")]
    StaleContext,
    #[error("required decision is missing, tentative, or not yet visible")]
    DecisionNotApplied,
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}
#[derive(Debug, Clone, PartialEq)]
pub struct Observed<T> {
    pub resulting_revision: u64,
    pub value: T,
}
pub fn revision(db: &Connection, series_slug: &str) -> rusqlite::Result<u64> {
    db.query_row(
        "select a.revision from authority_state a join series s on s.id=a.series_id where s.slug=?",
        [series_slug],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map(|r| r.unwrap_or(0) as u64)
}
/// The callback must only read domain data and must use the supplied connection
/// for every participating SQLite read. Side effects belong after publication.
pub fn stable_read<T, E: From<CoherentReadError>>(
    config: &HieronymusConfig,
    series_slug: &str,
    required: Option<&str>,
    mut read: impl FnMut(&Connection) -> Result<T, E>,
) -> Result<Observed<T>, E> {
    for _ in 0..2 {
        let mut db = open_migrated(&config.database_path()).map_err(CoherentReadError::from)?;
        let tx = db.transaction().map_err(CoherentReadError::from)?;
        let observed = revision(&tx, series_slug).map_err(CoherentReadError::from)?;
        if let Some(id) = required {
            let receipt:Option<(String,String)>=tx.query_row("select d.status,d.result_json from decision_records d join series s on s.id=d.series_id where d.decision_id=?1 and s.slug=?2",rusqlite::params![id,series_slug],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(CoherentReadError::from)?;
            let Some((status, json)) = receipt else {
                return Err(CoherentReadError::DecisionNotApplied.into());
            };
            let result: DecisionResultV1 =
                serde_json::from_str(&json).map_err(|_| CoherentReadError::DecisionNotApplied)?;
            if status != "applied"
                || matches!(result, DecisionResultV1::Tentative { .. })
                || result.receipt().resulting_revision > observed
            {
                return Err(CoherentReadError::DecisionNotApplied.into());
            }
        }
        let value = read(&tx)?;
        drop(tx);
        // This read sees current state, never the transaction's pinned snapshot.
        if revision(&db, series_slug).map_err(CoherentReadError::from)? == observed {
            return Ok(Observed {
                resulting_revision: observed,
                value,
            });
        }
    }
    Err(CoherentReadError::StaleContext.into())
}
