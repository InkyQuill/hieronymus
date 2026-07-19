use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};

use crate::db::{CrystalRecord, TaskSessionRecord};

use super::scoring::{ARCHIVE_STRENGTH_THRESHOLD, ScoreDelta, apply_score_delta, event_delta};

const SELECT_CRYSTAL: &str = "SELECT * FROM crystals WHERE id = ?";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackEvent {
    pub crystal_id: i64,
    pub event_type: String,
    pub source_role: String,
    pub evidence: Option<String>,
    pub session_id: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FeedbackError {
    #[error("unknown feedback event_type: {event_type}")]
    UnknownEventType { event_type: String },
    #[error("unknown crystal: {id}")]
    CrystalNotFound { id: i64 },
    #[error("unknown session: {id}")]
    SessionNotFound { id: i64 },
    #[error("feedback session {session_id} does not match crystal {crystal_id} {field}")]
    SessionContext {
        session_id: i64,
        crystal_id: i64,
        field: &'static str,
    },
    #[error("crystal {crystal_id} appears in both useful and miss outcomes")]
    OutcomeOverlap { crystal_id: i64 },
    #[error("no activation for crystal {crystal_id} in session {session_id}")]
    ActivationNotFound { session_id: i64, crystal_id: i64 },
    #[error(
        "activation outcome conflict for crystal {crystal_id} in session {session_id}: existing {existing}, requested {requested}"
    )]
    OutcomeConflict {
        session_id: i64,
        crystal_id: i64,
        existing: String,
        requested: String,
    },
    #[error("feedback {operation} failed: {source}")]
    Database {
        operation: &'static str,
        #[source]
        source: sqlx::Error,
    },
}

pub type Result<T> = std::result::Result<T, FeedbackError>;

pub struct FeedbackStore<'a> {
    pool: &'a SqlitePool,
}

impl<'a> FeedbackStore<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record(&self, event: FeedbackEvent) -> Result<i64> {
        let mut transaction = begin_immediate(self.pool, "record").await?;
        let result = record_on(&mut transaction, event).await;
        commit_write(transaction, "record", result).await
    }

    pub(crate) async fn record_in(
        &self,
        transaction: &mut Transaction<'static, Sqlite>,
        event: FeedbackEvent,
    ) -> Result<i64> {
        record_on(transaction, event).await
    }

    pub async fn record_recall_outcome(
        &self,
        session_id: i64,
        useful: &[i64],
        miss: &[i64],
    ) -> Result<()> {
        let useful = useful.iter().copied().collect::<BTreeSet<_>>();
        let miss = miss.iter().copied().collect::<BTreeSet<_>>();
        if let Some(crystal_id) = useful.intersection(&miss).next().copied() {
            return Err(FeedbackError::OutcomeOverlap { crystal_id });
        }
        let requested = useful
            .iter()
            .map(|&id| (id, "useful"))
            .chain(miss.iter().map(|&id| (id, "miss")))
            .collect::<BTreeMap<_, _>>();
        let mut transaction = begin_immediate(self.pool, "record recall outcome").await?;
        let result = async {
            let session = require_session(&mut transaction, session_id).await?;
            for (crystal_id, outcome) in requested {
                record_activation_outcome(&mut transaction, &session, crystal_id, outcome).await?;
            }
            Ok(())
        }
        .await;
        commit_write(transaction, "record recall outcome", result).await
    }
}

async fn record_activation_outcome(
    transaction: &mut Transaction<'static, Sqlite>,
    session: &TaskSessionRecord,
    crystal_id: i64,
    outcome: &'static str,
) -> Result<()> {
    let session_id = session.id;
    let crystal = get_crystal(transaction.as_mut(), crystal_id).await?;
    validate_context(&crystal, session)?;
    let outcomes = sqlx::query_scalar::<_, Option<String>>(
        "SELECT outcome FROM crystal_activations WHERE session_id = ? AND crystal_id = ? ORDER BY id",
    )
    .bind(session_id)
    .bind(crystal_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|source| database("read activation outcomes", source))?;
    if outcomes.is_empty() {
        return Err(FeedbackError::ActivationNotFound {
            session_id,
            crystal_id,
        });
    }
    if let Some(existing) = outcomes
        .iter()
        .flatten()
        .find(|existing| existing.as_str() != outcome)
    {
        return Err(FeedbackError::OutcomeConflict {
            session_id,
            crystal_id,
            existing: existing.clone(),
            requested: outcome.to_owned(),
        });
    }
    if outcomes.iter().all(Option::is_some) {
        return Ok(());
    }
    if outcomes.iter().any(Option::is_some) {
        update_pending_outcomes(transaction, session_id, crystal_id, outcome).await?;
        return Ok(());
    }
    let event_type = match outcome {
        "useful" => "recalled_useful",
        "miss" => "recalled_miss",
        _ => unreachable!("outcome is selected internally"),
    };
    record_on(
        transaction,
        FeedbackEvent {
            crystal_id,
            event_type: event_type.to_owned(),
            source_role: "recall_feedback".to_owned(),
            evidence: None,
            session_id: Some(session_id),
        },
    )
    .await?;
    update_pending_outcomes(transaction, session_id, crystal_id, outcome).await
}

async fn update_pending_outcomes(
    transaction: &mut Transaction<'static, Sqlite>,
    session_id: i64,
    crystal_id: i64,
    outcome: &str,
) -> Result<()> {
    sqlx::query("UPDATE crystal_activations SET outcome = ? WHERE session_id = ? AND crystal_id = ? AND outcome IS NULL")
        .bind(outcome)
        .bind(session_id)
        .bind(crystal_id)
        .execute(&mut **transaction)
        .await
        .map_err(|source| database("update activation outcomes", source))?;
    Ok(())
}

async fn record_on(
    transaction: &mut Transaction<'static, Sqlite>,
    event: FeedbackEvent,
) -> Result<i64> {
    let (delta, immediate) =
        event_delta(&event.event_type).ok_or_else(|| FeedbackError::UnknownEventType {
            event_type: event.event_type.clone(),
        })?;
    let crystal = get_crystal(transaction.as_mut(), event.crystal_id).await?;
    if let Some(session_id) = event.session_id {
        let session = require_session(transaction, session_id).await?;
        validate_context(&crystal, &session)?;
    }
    let now = Utc::now();
    let inserted = sqlx::query("INSERT INTO memory_events(crystal_id, session_id, event_type, source_role, evidence, strength_delta, confidence_delta, applied, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(event.crystal_id)
        .bind(event.session_id)
        .bind(&event.event_type)
        .bind(&event.source_role)
        .bind(event.evidence.as_deref().unwrap_or(""))
        .bind(delta.strength)
        .bind(delta.confidence)
        .bind(immediate)
        .bind(now)
        .execute(&mut **transaction)
        .await
        .map_err(|source| database("insert audit event", source))?;
    if immediate {
        update_score(transaction, &crystal, delta, &event.event_type, now).await?;
    }
    Ok(inserted.last_insert_rowid())
}

async fn update_score(
    transaction: &mut Transaction<'static, Sqlite>,
    crystal: &CrystalRecord,
    delta: ScoreDelta,
    event_type: &str,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let (strength, confidence, mut status) = apply_score_delta(crystal, delta);
    if event_type == "deleted_by_user" && strength < ARCHIVE_STRENGTH_THRESHOLD {
        status = "archived".to_owned();
    }
    sqlx::query(
        "UPDATE crystals SET strength = ?, confidence = ?, status = ?, updated_at = ? WHERE id = ?",
    )
    .bind(strength)
    .bind(confidence)
    .bind(status)
    .bind(now)
    .bind(crystal.id)
    .execute(&mut **transaction)
    .await
    .map_err(|source| database("update crystal score", source))?;
    Ok(())
}

async fn get_crystal(connection: &mut SqliteConnection, id: i64) -> Result<CrystalRecord> {
    sqlx::query_as(SELECT_CRYSTAL)
        .bind(id)
        .fetch_optional(connection)
        .await
        .map_err(|source| database("read crystal", source))?
        .ok_or(FeedbackError::CrystalNotFound { id })
}

async fn require_session(
    transaction: &mut Transaction<'static, Sqlite>,
    id: i64,
) -> Result<TaskSessionRecord> {
    sqlx::query_as("SELECT * FROM task_sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|source| database("read session", source))?
        .ok_or(FeedbackError::SessionNotFound { id })
}

fn validate_context(crystal: &CrystalRecord, session: &TaskSessionRecord) -> Result<()> {
    match crystal.scope_type.as_str() {
        "global" if crystal.scope_key.is_empty() && crystal.series_slug.is_empty() => {}
        "series"
            if !crystal.series_slug.is_empty()
                && crystal.scope_key == format!("series:{}", crystal.series_slug) =>
        {
            if session.series_slug != crystal.series_slug {
                return Err(FeedbackError::SessionContext {
                    session_id: session.id,
                    crystal_id: crystal.id,
                    field: "series_slug",
                });
            }
        }
        _ => {
            return Err(FeedbackError::SessionContext {
                session_id: session.id,
                crystal_id: crystal.id,
                field: "scope_type",
            });
        }
    }
    for (field, valid) in [
        (
            "source_language",
            crystal.source_language.is_empty()
                || session.source_language == crystal.source_language,
        ),
        (
            "target_language",
            crystal.target_language.is_empty()
                || session.target_language == crystal.target_language,
        ),
    ] {
        if !valid {
            return Err(FeedbackError::SessionContext {
                session_id: session.id,
                crystal_id: crystal.id,
                field,
            });
        }
    }
    Ok(())
}

async fn begin_immediate(
    pool: &SqlitePool,
    operation: &'static str,
) -> Result<Transaction<'static, Sqlite>> {
    pool.begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|source| database(operation, source))
}

async fn commit_write<T>(
    transaction: Transaction<'static, Sqlite>,
    operation: &'static str,
    result: Result<T>,
) -> Result<T> {
    match result {
        Ok(value) => transaction
            .commit()
            .await
            .map(|()| value)
            .map_err(|source| database(operation, source)),
        Err(error) => Err(error),
    }
}

fn database(operation: &'static str, source: sqlx::Error) -> FeedbackError {
    FeedbackError::Database { operation, source }
}
