//! Recall feedback consumption (ADR 0011 §Recall Feedback): feedback
//! addresses one specific recall invocation. A request carries the
//! invocation's `recall_id`, `useful`/`miss` activation-id lists, and an
//! idempotency key. Immediate deltas are applied through the event-sourced
//! scoring pattern (`memory_events` rows plus clamped crystal updates) in one
//! transaction; the idempotency key (stored as the event `evidence`) and the
//! activation outcomes together form the at-most-once ledger that dreaming
//! later relies on — it never reapplies these deltas.
//!
//! Negative feedback touches only graded memory scores. `term_rules` and the
//! deterministic contract projection are never reachable from here.

use rusqlite::Connection;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;

/// Python `IMMEDIATE_EVENT_DELTAS["recalled_useful"]`: one notch below
/// `confirmed_by_user` — an agent finding a memory useful is weaker evidence
/// than a human confirming it.
pub const RECALLED_USEFUL_DELTAS: (f64, f64) = (0.06, 0.04);

/// Python `IMMEDIATE_EVENT_DELTAS["recalled_miss"]`: deliberately softer than
/// `contradicted_by_user` — "not relevant this time" is not "this is wrong".
pub const RECALLED_MISS_DELTAS: (f64, f64) = (-0.05, -0.03);

/// Python `PASSIVE_EVENT_DELTAS["recalled_again"]`: strength-only, since mere
/// resurfacing implies nothing about correctness.
pub const RECALLED_AGAIN_DELTAS: (f64, f64) = (0.02, 0.0);

/// Negative score deltas on advisory rule-intent crystals decay at half rate
/// (the `rule_intent` dampener; ADR 0011 keeps structured `term_rules`
/// authority entirely outside passive decay).
pub const DECAY_DAMPENER: f64 = 0.5;

#[derive(Debug, thiserror::Error)]
pub enum FeedbackError {
    #[error("idempotency key must not be empty")]
    EmptyIdempotencyKey,
    #[error("recall_id must not be empty")]
    EmptyRecallId,
    #[error("feedback must name at least one activation id")]
    NoActivationIds,
    #[error("unknown recall invocation: {0}")]
    UnknownRecall(String),
    #[error("activation ids do not belong to recall invocation {recall_id}: {mismatched:?}")]
    ActivationMismatch {
        recall_id: String,
        mismatched: Vec<i64>,
    },
    #[error("activation ids cannot be both useful and miss: {conflicting:?}")]
    ConflictingOutcome { conflicting: Vec<i64> },
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}

/// One feedback request against one recall invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallFeedback {
    pub recall_id: String,
    pub useful_activation_ids: Vec<i64>,
    pub missed_activation_ids: Vec<i64>,
    pub idempotency_key: String,
}

/// Store outcome. `applied: false` marks an explicit already-applied replay:
/// the same idempotency key, or activations that already carry an outcome,
/// mutate nothing and report `applied = false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackOutcome {
    pub applied: bool,
    pub useful_count: usize,
    pub miss_count: usize,
}

/// Recall feedback store over the data-root database.
pub struct FeedbackStore {
    config: HieronymusConfig,
}

impl FeedbackStore {
    pub fn open(config: &HieronymusConfig) -> Result<Self, FeedbackError> {
        open_migrated(&config.database_path())?;
        Ok(Self {
            config: config.clone(),
        })
    }

    /// Apply one feedback request. All mutations — memory events, clamped
    /// score deltas, activation outcomes — happen in one transaction, so a
    /// rejected request mutates nothing.
    pub fn record_recall_outcome(
        &self,
        request: &RecallFeedback,
    ) -> Result<FeedbackOutcome, FeedbackError> {
        if request.idempotency_key.trim().is_empty() {
            return Err(FeedbackError::EmptyIdempotencyKey);
        }
        if request.recall_id.trim().is_empty() {
            return Err(FeedbackError::EmptyRecallId);
        }
        let mut connection = open_migrated(&self.config.database_path())?;
        let transaction = connection.transaction()?;
        let outcome = record_recall_outcome_tx(&transaction, request)?;
        transaction.commit()?;
        Ok(outcome)
    }
}

pub(crate) fn record_recall_outcome_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &RecallFeedback,
) -> Result<FeedbackOutcome, FeedbackError> {
    // Replay detection, half one: the idempotency key is stored as the
    // evidence of the immediate feedback events.
    let replayed: i64 = transaction.query_row(
        "select count(*) from memory_events
         where event_type in ('recalled_useful', 'recalled_miss')
           and evidence = ?1",
        [&request.idempotency_key],
        |row| row.get(0),
    )?;
    if replayed > 0 {
        return Ok(FeedbackOutcome {
            applied: false,
            useful_count: 0,
            miss_count: 0,
        });
    }

    // Validation: every named activation id must belong to this invocation,
    // and no id may carry both outcomes.
    let useful_set: std::collections::HashSet<i64> =
        request.useful_activation_ids.iter().copied().collect();
    let miss_set: std::collections::HashSet<i64> =
        request.missed_activation_ids.iter().copied().collect();
    let conflicting: Vec<i64> = {
        let mut conflicting: Vec<i64> = useful_set.intersection(&miss_set).copied().collect();
        conflicting.sort_unstable();
        conflicting
    };
    if !conflicting.is_empty() {
        return Err(FeedbackError::ConflictingOutcome { conflicting });
    }
    let mut requested: Vec<i64> = request
        .useful_activation_ids
        .iter()
        .chain(request.missed_activation_ids.iter())
        .copied()
        .collect();
    requested.sort_unstable();
    requested.dedup();
    let mut known: Vec<(i64, i64, i64)> = Vec::new();
    {
        let mut statement = transaction.prepare(
            "select a.id, a.crystal_id, a.session_id
             from crystal_activations a
             where a.recall_id = ?1 order by a.id",
        )?;
        let rows = statement.query_map([&request.recall_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            known.push(row?);
        }
    }
    if known.is_empty() {
        return Err(FeedbackError::UnknownRecall(request.recall_id.clone()));
    }
    if requested.is_empty() {
        return Err(FeedbackError::NoActivationIds);
    }
    let known_ids: std::collections::HashSet<i64> = known.iter().map(|(id, _, _)| *id).collect();
    let mismatched: Vec<i64> = requested
        .iter()
        .copied()
        .filter(|id| !known_ids.contains(id))
        .collect();
    if !mismatched.is_empty() {
        return Err(FeedbackError::ActivationMismatch {
            recall_id: request.recall_id.clone(),
            mismatched,
        });
    }

    // Replay detection, half two: activations already carrying an outcome
    // have consumed their feedback. No partial application.
    let requested_json = format!(
        "[{}]",
        requested
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let already_scored: i64 = transaction.query_row(
        "select count(*) from crystal_activations
         where id in (select value from json_each(?1))
           and outcome is not null",
        [&requested_json],
        |row| row.get(0),
    )?;
    if already_scored > 0 {
        return Ok(FeedbackOutcome {
            applied: false,
            useful_count: 0,
            miss_count: 0,
        });
    }

    let now = now_iso8601();
    let mut useful_count = 0usize;
    let mut miss_count = 0usize;
    for (activation_id, crystal_id, session_id) in &known {
        let (event_type, outcome, deltas) = if useful_set.contains(activation_id) {
            useful_count += 1;
            ("recalled_useful", "useful", RECALLED_USEFUL_DELTAS)
        } else if miss_set.contains(activation_id) {
            miss_count += 1;
            ("recalled_miss", "miss", RECALLED_MISS_DELTAS)
        } else {
            continue;
        };
        transaction.execute(
            "insert into memory_events(
               crystal_id, session_id, event_type, source_role, evidence,
               strength_delta, confidence_delta, applied, created_at
             )
             values (?1, ?2, ?3, 'agent', ?4, ?5, ?6, 1, ?7)",
            rusqlite::params![
                crystal_id,
                session_id,
                event_type,
                request.idempotency_key,
                deltas.0,
                deltas.1,
                now
            ],
        )?;
        apply_score_delta(transaction, *crystal_id, deltas.0, deltas.1, &now)?;
        transaction.execute(
            "update crystal_activations set outcome = ?1 where id = ?2",
            rusqlite::params![outcome, activation_id],
        )?;
    }
    Ok(FeedbackOutcome {
        applied: true,
        useful_count,
        miss_count,
    })
}

/// The single score-delta primitive of the event-sourced scoring pattern:
/// apply one strength/confidence delta pair to a crystal, clamped to
/// `[0, 1]`. Negative (decay) deltas are dampened by [`DECAY_DAMPENER`] for
/// crystals with non-empty `rule_intent` — advisory rules fade slower but
/// nothing is immune. Returns `false` when the crystal no longer exists.
pub fn apply_score_delta(
    connection: &Connection,
    crystal_id: i64,
    strength_delta: f64,
    confidence_delta: f64,
    now: &str,
) -> Result<bool, rusqlite::Error> {
    let row: Option<(f64, f64, String)> = connection
        .query_row(
            "select strength, confidence, rule_intent from crystals where id = ?1",
            [crystal_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let Some((strength, confidence, rule_intent)) = row else {
        return Ok(false);
    };
    let dampen = |delta: f64| -> f64 {
        if delta < 0.0 && !rule_intent.trim().is_empty() {
            delta * DECAY_DAMPENER
        } else {
            delta
        }
    };
    connection.execute(
        "update crystals
         set strength = ?1, confidence = ?2, updated_at = ?3
         where id = ?4",
        rusqlite::params![
            (strength + dampen(strength_delta)).clamp(0.0, 1.0),
            (confidence + dampen(confidence_delta)).clamp(0.0, 1.0),
            now,
            crystal_id
        ],
    )?;
    Ok(true)
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}
