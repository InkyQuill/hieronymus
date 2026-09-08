//! A single SQLite snapshot, followed by a freshness read outside that snapshot.
use crate::{authority_models::DecisionResultV1, data_root::HieronymusConfig};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, thiserror::Error)]
pub enum CoherentReadError {
    #[error("unknown series: {0}")]
    UnknownSeries(String),
    #[error("registered series has no valid authority state: {0}")]
    MissingAuthorityState(String),
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
pub fn revision(db: &Connection, series_slug: &str) -> Result<u64, CoherentReadError> {
    let state: Option<Option<i64>> = db.query_row(
        "select a.revision from series s left join authority_state a on s.id=a.series_id where s.slug=?",
        [series_slug], |r| r.get(0),
    ).optional()?;
    match state {
        None => Err(CoherentReadError::UnknownSeries(series_slug.into())),
        Some(Some(value)) if value >= 0 => Ok(value as u64),
        _ => Err(CoherentReadError::MissingAuthorityState(series_slug.into())),
    }
}
/// The callback must only read domain data and must use the supplied connection
/// for every participating SQLite read. Side effects belong after publication.
pub fn stable_read<T, E: From<CoherentReadError>>(
    config: &HieronymusConfig,
    series_slug: &str,
    required: Option<&str>,
    read: impl FnMut(&Connection) -> Result<T, E>,
) -> Result<Observed<T>, E> {
    stable_read_with_publish(config, series_slug, required, read, |_| Ok(true))
}

/// Publish only after the external revision check. Return false on a late
/// revision conflict without committing effects; that consumes the same retry
/// budget as a snapshot conflict. Other errors terminate the operation.
pub fn stable_read_with_publish<T, E: From<CoherentReadError>>(
    config: &HieronymusConfig,
    series_slug: &str,
    required: Option<&str>,
    mut read: impl FnMut(&Connection) -> Result<T, E>,
    mut publish: impl FnMut(&mut Observed<T>) -> Result<bool, E>,
) -> Result<Observed<T>, E> {
    for _ in 0..2 {
        let mut db = read_connection(config).map_err(CoherentReadError::from)?;
        let tx = db.transaction().map_err(CoherentReadError::from)?;
        let observed = revision(&tx, series_slug)?;
        let corpus = crate::rag::current_corpus_revision(&tx).map_err(CoherentReadError::from)?;
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
        let fresh = read_connection(config).map_err(CoherentReadError::from)?;
        if revision(&fresh, series_slug)? == observed
            && crate::rag::current_corpus_revision(&fresh).map_err(CoherentReadError::from)?
                == corpus
        {
            let mut result = Observed {
                resulting_revision: observed,
                value,
            };
            if publish(&mut result)? {
                return Ok(result);
            }
        }
    }
    Err(CoherentReadError::StaleContext.into())
}

fn read_connection(config: &HieronymusConfig) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}
