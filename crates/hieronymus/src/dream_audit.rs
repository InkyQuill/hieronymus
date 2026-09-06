//! Dream audit storage (port of `dream_audit.py`): append-only audit entries
//! per dream run with recursive payload redaction. Secrets and unrestricted
//! source text are never stored in audit payloads by default; only the
//! outbound HTTP header builder (a later provider slice) may see keys.

use chrono::Utc;
use rusqlite::Connection;
use serde_json::Value;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;

const REDACTED: &str = "[REDACTED]";
const SECRET_KEYS: [&str; 7] = [
    "apikey",
    "authorization",
    "xapikey",
    "xgoogapikey",
    "anthropicversion",
    "token",
    "bearer",
];

#[derive(Debug, thiserror::Error)]
pub enum DreamAuditError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
    #[error("{0}")]
    Json(String),
}

/// One stored audit entry; `payload` is the redacted JSON projection.
#[derive(Debug, Clone, PartialEq)]
pub struct DreamAuditEntry {
    pub id: i64,
    pub dream_run_id: i64,
    pub phase_run_id: Option<i64>,
    pub event_type: String,
    pub severity: String,
    pub summary: String,
    pub payload: Value,
    pub created_at: String,
}

/// Append-only audit store over the data-root database.
pub struct DreamAuditStore {
    config: HieronymusConfig,
}

impl DreamAuditStore {
    pub fn open(config: &HieronymusConfig) -> Result<Self, DreamAuditError> {
        open_migrated(&config.database_path())?;
        Ok(Self {
            config: config.clone(),
        })
    }

    /// Redact and append one audit entry, returning its row id. Opens its own
    /// connection: only for events that are not part of an atomic
    /// domain+phase commit (provider request/response audit, run-level
    /// events, post-rollback failure records). The atomic paths use
    /// [`DreamAuditStore::append_in_transaction`].
    pub fn append(
        &self,
        dream_run_id: i64,
        phase_run_id: Option<i64>,
        event_type: &str,
        severity: &str,
        summary: &str,
        payload: &Value,
    ) -> Result<i64, DreamAuditError> {
        let connection = self.connection()?;
        insert_audit(
            &connection,
            dream_run_id,
            phase_run_id,
            event_type,
            severity,
            summary,
            payload,
        )
    }

    /// Redact and append one audit entry inside the caller's transaction,
    /// returning its row id. Same arguments and redaction as
    /// [`DreamAuditStore::append`], but the entry lives or dies with the
    /// transaction, so a phase's domain mutations, its phase-completed
    /// status, and its audit record commit atomically (task D3; ADR 0005
    /// immutable dream audit). Must never open another connection.
    #[allow(clippy::too_many_arguments)]
    pub fn append_in_transaction(
        transaction: &rusqlite::Transaction<'_>,
        dream_run_id: i64,
        phase_run_id: Option<i64>,
        event_type: &str,
        severity: &str,
        summary: &str,
        payload: &Value,
    ) -> Result<i64, DreamAuditError> {
        insert_audit(
            transaction,
            dream_run_id,
            phase_run_id,
            event_type,
            severity,
            summary,
            payload,
        )
    }

    pub fn list_for_run(&self, dream_run_id: i64) -> Result<Vec<DreamAuditEntry>, DreamAuditError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select id, dream_run_id, phase_run_id, event_type, severity, summary,
                    payload_json, created_at
             from dream_audit_entries
             where dream_run_id = ?1
             order by id",
        )?;
        let rows = statement.query_map([dream_run_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (
                id,
                dream_run_id,
                phase_run_id,
                event_type,
                severity,
                summary,
                payload_json,
                created_at,
            ) = row?;
            let payload: Value = serde_json::from_str(&payload_json)
                .map_err(|error| DreamAuditError::Json(error.to_string()))?;
            entries.push(DreamAuditEntry {
                id,
                dream_run_id,
                phase_run_id,
                event_type,
                severity,
                summary,
                payload,
                created_at,
            });
        }
        Ok(entries)
    }

    fn connection(&self) -> Result<Connection, DreamAuditError> {
        Ok(open_migrated(&self.config.database_path())?)
    }
}

/// The shared audit insert behind [`DreamAuditStore::append`] and
/// [`DreamAuditStore::append_in_transaction`]: redaction applied, one row
/// inserted, row id returned. Accepts either a plain connection or a
/// transaction (deref).
#[allow(clippy::too_many_arguments)]
fn insert_audit(
    connection: &Connection,
    dream_run_id: i64,
    phase_run_id: Option<i64>,
    event_type: &str,
    severity: &str,
    summary: &str,
    payload: &Value,
) -> Result<i64, DreamAuditError> {
    let payload_json = serde_json::to_string(&redact_payload(payload))
        .map_err(|error| DreamAuditError::Json(error.to_string()))?;
    connection.execute(
        "insert into dream_audit_entries(
           dream_run_id, phase_run_id, event_type, severity, summary,
           payload_json, created_at
         )
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            dream_run_id,
            phase_run_id,
            event_type,
            severity,
            summary,
            payload_json,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

/// Commit one audited operation atomically (task D3): open one immediate
/// write transaction, run `operation` — the domain mutations, the
/// phase-completed status update, and the redacted audit append — and commit.
/// An operation error or a commit error rolls the whole unit back, so a dream
/// phase can never leave durable domain mutations without their durable
/// completion and audit records (ADR 0005; ADR 0010/0011 audited
/// transitions). The caller owns the post-rollback failure audit: it must be
/// written through a separate connection AFTER this returns `Err`.
pub fn commit_audited<T>(
    conn: &mut rusqlite::Connection,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let result = operation(&tx)?;
    tx.commit()?;
    Ok(result)
}

fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| *character != '-' && *character != '_')
        .collect::<String>()
        .to_lowercase();
    SECRET_KEYS.contains(&normalized.as_str())
        || normalized.contains("token")
        || normalized.contains("bearer")
}

/// Recursively replace every secret-keyed field's value with `[REDACTED]`.
pub fn redact_payload(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let item = if is_secret_key(key) {
                        Value::String(REDACTED.to_string())
                    } else {
                        redact_payload(item)
                    };
                    (key.clone(), item)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_payload).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redaction_keys_are_normalized_and_recursive() {
        let payload = json!({
            "x-api-key": "secret",
            "nested": {"ACCESS_TOKEN": "secret", "keep": "safe"},
            "list": [{"Bearer_Token": "secret"}],
            "model": "safe"
        });
        assert_eq!(
            redact_payload(&payload),
            json!({
                "x-api-key": "[REDACTED]",
                "nested": {"ACCESS_TOKEN": "[REDACTED]", "keep": "safe"},
                "list": [{"Bearer_Token": "[REDACTED]"}],
                "model": "safe"
            })
        );
    }

    #[test]
    fn token_like_substring_keys_are_redacted() {
        let payload = json!({"modelid": "x", "tokenize": false, "scoped": 1});
        assert_eq!(
            redact_payload(&payload),
            json!({"modelid": "x", "tokenize": "[REDACTED]", "scoped": 1})
        );
    }
}
