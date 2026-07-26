use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::db::DreamRunRecord;

const MAX_DETAIL_BYTES: usize = 4_096;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_PAYLOAD_DEPTH: usize = 32;
const MAX_PAYLOAD_NODES: usize = 4_096;
const MAX_PAYLOAD_STRING_BYTES: usize = MAX_PAYLOAD_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DreamRunCompletion {
    pub input_count: i64,
    pub created_crystal_count: i64,
    pub proposal_count: i64,
}

impl DreamRunCompletion {
    #[must_use]
    pub const fn new(input_count: i64, created_crystal_count: i64, proposal_count: i64) -> Self {
        Self {
            input_count,
            created_crystal_count,
            proposal_count,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PhaseRunStart<'a> {
    pub dream_run_id: i64,
    pub phase: &'a str,
    pub provider_profile: &'a str,
    pub provider_type: &'a str,
    pub model: &'a str,
    pub input_count: i64,
    pub prompt_hash: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct DreamPhaseRunRecord {
    pub id: i64,
    pub dream_run_id: i64,
    pub phase: String,
    pub provider_profile: String,
    pub provider_type: String,
    pub model: String,
    pub status: String,
    pub input_count: i64,
    pub output_count: i64,
    pub error: String,
    pub prompt_hash: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct DreamAuditEntry {
    pub id: i64,
    pub dream_run_id: i64,
    pub phase_run_id: Option<i64>,
    pub event_type: String,
    pub severity: String,
    pub summary: String,
    pub payload_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum DreamAuditError {
    #[error("dream audit database operation failed")]
    Database(#[source] sqlx::Error),
    #[error(
        "dream audit lifecycle conflict for {entity} {id}: expected {expected}, found {actual}"
    )]
    LifecycleConflict {
        entity: &'static str,
        id: i64,
        expected: &'static str,
        actual: String,
    },
    #[error("invalid dream audit detail: {0}")]
    InvalidDetail(&'static str),
}

impl From<sqlx::Error> for DreamAuditError {
    fn from(source: sqlx::Error) -> Self {
        Self::Database(source)
    }
}

pub struct DreamAuditStore<'a> {
    pool: &'a SqlitePool,
}

impl<'a> DreamAuditStore<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn start_run(
        &self,
        cycle_id: i64,
        provider: &str,
    ) -> Result<DreamRunRecord, DreamAuditError> {
        validate_nonnegative("cycle id", cycle_id)?;
        validate_detail(provider)?;
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let run = sqlx::query_as::<_, DreamRunRecord>(
            "INSERT INTO dream_runs(cycle_id, status, provider, created_at) VALUES (?, 'running', ?, ?) RETURNING *",
        )
        .bind(cycle_id)
        .bind(provider)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            run.id,
            None,
            "run_started",
            "info",
            "dream run started",
            &Value::Null,
        )
        .await?;
        tx.commit().await?;
        Ok(run)
    }

    pub async fn complete_run(
        &self,
        run_id: i64,
        counts: DreamRunCompletion,
    ) -> Result<(), DreamAuditError> {
        validate_completion(counts)?;
        finish_run(self.pool, run_id, counts, "completed", "", "run_completed").await
    }

    pub async fn fail_run(
        &self,
        run_id: i64,
        counts: DreamRunCompletion,
        error: &str,
    ) -> Result<(), DreamAuditError> {
        validate_completion(counts)?;
        finish_run(
            self.pool,
            run_id,
            counts,
            "failed",
            &redact_error(error),
            "run_failed",
        )
        .await
    }

    pub async fn start_phase(
        &self,
        input: PhaseRunStart<'_>,
    ) -> Result<DreamPhaseRunRecord, DreamAuditError> {
        validate_nonnegative("phase input count", input.input_count)?;
        for detail in [
            input.phase,
            input.provider_profile,
            input.provider_type,
            input.model,
            input.prompt_hash,
        ] {
            validate_detail(detail)?;
        }
        let mut tx = self.pool.begin().await?;
        let run_status: String = sqlx::query_scalar("SELECT status FROM dream_runs WHERE id = ?")
            .bind(input.dream_run_id)
            .fetch_one(&mut *tx)
            .await?;
        if run_status != "running" {
            return Err(lifecycle("run", input.dream_run_id, "running", run_status));
        }
        let phase = sqlx::query_as::<_, DreamPhaseRunRecord>(
            "INSERT INTO dream_phase_runs(dream_run_id, phase, provider_profile, provider_type, model, status, input_count, prompt_hash, created_at) VALUES (?, ?, ?, ?, ?, 'running', ?, ?, ?) RETURNING *",
        )
        .bind(input.dream_run_id)
        .bind(input.phase)
        .bind(input.provider_profile)
        .bind(input.provider_type)
        .bind(input.model)
        .bind(input.input_count)
        .bind(input.prompt_hash)
        .bind(Utc::now())
        .fetch_one(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            input.dream_run_id,
            Some(phase.id),
            "phase_started",
            "info",
            "dream phase started",
            &Value::Null,
        )
        .await?;
        tx.commit().await?;
        Ok(phase)
    }

    pub async fn complete_phase(
        &self,
        phase_run_id: i64,
        output_count: i64,
    ) -> Result<(), DreamAuditError> {
        validate_nonnegative("phase output count", output_count)?;
        finish_phase(
            self.pool,
            phase_run_id,
            output_count,
            "completed",
            "",
            "phase_completed",
        )
        .await
    }

    pub async fn fail_phase(&self, phase_run_id: i64, error: &str) -> Result<(), DreamAuditError> {
        finish_phase(
            self.pool,
            phase_run_id,
            0,
            "failed",
            &redact_error(error),
            "phase_failed",
        )
        .await
    }

    pub async fn record(
        &self,
        run_id: i64,
        phase_run_id: Option<i64>,
        event_type: &str,
        summary: &str,
        payload: &Value,
    ) -> Result<i64, DreamAuditError> {
        let mut tx = self.pool.begin().await?;
        let id = insert_audit(
            &mut tx,
            run_id,
            phase_run_id,
            event_type,
            "info",
            summary,
            payload,
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    pub(crate) async fn record_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        run_id: i64,
        phase_run_id: Option<i64>,
        event_type: &str,
        summary: &str,
        payload: &Value,
    ) -> Result<i64, DreamAuditError> {
        insert_audit(
            transaction,
            run_id,
            phase_run_id,
            event_type,
            "info",
            summary,
            payload,
        )
        .await
    }

    pub async fn list_for_run(&self, run_id: i64) -> Result<Vec<DreamAuditEntry>, DreamAuditError> {
        Ok(sqlx::query_as::<_, DreamAuditEntry>(
            "SELECT * FROM dream_audit_entries WHERE dream_run_id = ? ORDER BY id",
        )
        .bind(run_id)
        .fetch_all(self.pool)
        .await?)
    }
}

async fn finish_run(
    pool: &SqlitePool,
    run_id: i64,
    counts: DreamRunCompletion,
    target: &'static str,
    error: &str,
    event: &str,
) -> Result<(), DreamAuditError> {
    let mut tx = pool.begin().await?;
    let current: (String, i64, i64, i64, String) = sqlx::query_as(
        "SELECT status, input_count, created_crystal_count, proposal_count, error FROM dream_runs WHERE id = ?",
    )
    .bind(run_id)
    .fetch_one(&mut *tx)
    .await?;
    if current.0 == target
        && current.1 == counts.input_count
        && current.2 == counts.created_crystal_count
        && current.3 == counts.proposal_count
        && current.4 == error
    {
        tx.commit().await?;
        return Ok(());
    }
    if current.0 != "running" {
        return Err(lifecycle("run", run_id, "running", current.0));
    }
    if target == "completed" {
        let running_phases: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM dream_phase_runs WHERE dream_run_id = ? AND status = 'running'",
        )
        .bind(run_id)
        .fetch_one(&mut *tx)
        .await?;
        if running_phases != 0 {
            return Err(DreamAuditError::InvalidDetail(
                "a run cannot complete while phases are running",
            ));
        }
    }
    if target == "failed" {
        let failed_phase_ids: Vec<i64> = sqlx::query_scalar(
            "UPDATE dream_phase_runs SET status = 'failed', error = ?, completed_at = ? WHERE dream_run_id = ? AND status = 'running' RETURNING id",
        )
        .bind(error)
        .bind(Utc::now())
        .bind(run_id)
        .fetch_all(&mut *tx)
        .await?;
        for phase_id in failed_phase_ids {
            insert_audit(
                &mut tx,
                run_id,
                Some(phase_id),
                "phase_failed",
                "error",
                "dream phase failed",
                &Value::Null,
            )
            .await?;
        }
    }
    sqlx::query("UPDATE dream_runs SET status = ?, input_count = ?, created_crystal_count = ?, proposal_count = ?, error = ?, completed_at = ? WHERE id = ? AND status = 'running'")
        .bind(target)
        .bind(counts.input_count)
        .bind(counts.created_crystal_count)
        .bind(counts.proposal_count)
        .bind(error)
        .bind(Utc::now())
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    insert_audit(
        &mut tx,
        run_id,
        None,
        event,
        if target == "failed" { "error" } else { "info" },
        if target == "failed" {
            "dream run failed"
        } else {
            "dream run completed"
        },
        &Value::Null,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn finish_phase(
    pool: &SqlitePool,
    phase_run_id: i64,
    output_count: i64,
    target: &'static str,
    error: &str,
    event: &str,
) -> Result<(), DreamAuditError> {
    let mut tx = pool.begin().await?;
    let current: (i64, String, i64, String) = sqlx::query_as(
        "SELECT dream_run_id, status, output_count, error FROM dream_phase_runs WHERE id = ?",
    )
    .bind(phase_run_id)
    .fetch_one(&mut *tx)
    .await?;
    if current.1 == target && current.2 == output_count && current.3 == error {
        tx.commit().await?;
        return Ok(());
    }
    if current.1 != "running" {
        return Err(lifecycle("phase", phase_run_id, "running", current.1));
    }
    sqlx::query("UPDATE dream_phase_runs SET status = ?, output_count = ?, error = ?, completed_at = ? WHERE id = ? AND status = 'running'")
        .bind(target)
        .bind(output_count)
        .bind(error)
        .bind(Utc::now())
        .bind(phase_run_id)
        .execute(&mut *tx)
        .await?;
    insert_audit(
        &mut tx,
        current.0,
        Some(phase_run_id),
        event,
        if target == "failed" { "error" } else { "info" },
        if target == "failed" {
            "dream phase failed"
        } else {
            "dream phase completed"
        },
        &Value::Null,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn insert_audit(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: i64,
    phase_run_id: Option<i64>,
    event_type: &str,
    severity: &str,
    summary: &str,
    payload: &Value,
) -> Result<i64, DreamAuditError> {
    for detail in [event_type, severity] {
        validate_detail(detail)?;
    }
    let summary = redact_and_bound_text(summary);
    if let Some(phase_run_id) = phase_run_id {
        let phase_parent: i64 =
            sqlx::query_scalar("SELECT dream_run_id FROM dream_phase_runs WHERE id = ?")
                .bind(phase_run_id)
                .fetch_one(&mut **tx)
                .await?;
        if phase_parent != run_id {
            return Err(DreamAuditError::InvalidDetail(
                "phase run does not belong to dream run",
            ));
        }
    }
    validate_payload_budget(payload)?;
    let payload = redact_payload(payload);
    let payload_json = serde_json::to_string(&payload)
        .map_err(|_| DreamAuditError::InvalidDetail("payload is not serializable"))?;
    if payload_json.len() > MAX_PAYLOAD_BYTES {
        return Err(DreamAuditError::InvalidDetail("payload exceeds 64 KiB"));
    }
    let result = sqlx::query("INSERT INTO dream_audit_entries(dream_run_id, phase_run_id, event_type, severity, summary, payload_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(run_id)
        .bind(phase_run_id)
        .bind(event_type)
        .bind(severity)
        .bind(summary)
        .bind(payload_json)
        .bind(Utc::now())
        .execute(&mut **tx)
        .await?;
    Ok(result.last_insert_rowid())
}

fn validate_detail(detail: &str) -> Result<(), DreamAuditError> {
    if detail.len() > MAX_DETAIL_BYTES {
        Err(DreamAuditError::InvalidDetail("text exceeds 4 KiB"))
    } else {
        Ok(())
    }
}

fn validate_nonnegative(label: &'static str, value: i64) -> Result<(), DreamAuditError> {
    if value < 0 {
        Err(DreamAuditError::InvalidDetail(label))
    } else {
        Ok(())
    }
}

fn validate_completion(counts: DreamRunCompletion) -> Result<(), DreamAuditError> {
    validate_nonnegative("run input count", counts.input_count)?;
    validate_nonnegative("created crystal count", counts.created_crystal_count)?;
    validate_nonnegative("proposal count", counts.proposal_count)
}

fn validate_payload_budget(root: &Value) -> Result<(), DreamAuditError> {
    let mut stack = vec![(root, 0_usize)];
    let mut scheduled_nodes = 1_usize;
    let mut estimated_bytes = 1_usize;
    while let Some((value, depth)) = stack.pop() {
        if depth > MAX_PAYLOAD_DEPTH {
            return Err(DreamAuditError::InvalidDetail(
                "payload exceeds maximum depth",
            ));
        }
        let remaining_nodes = MAX_PAYLOAD_NODES.saturating_sub(scheduled_nodes);
        match value {
            Value::Object(map) => {
                if map.len() > remaining_nodes {
                    return Err(DreamAuditError::InvalidDetail("payload has too many nodes"));
                }
                for (key, value) in map {
                    if key.len() > MAX_DETAIL_BYTES {
                        return Err(DreamAuditError::InvalidDetail("payload key exceeds 4 KiB"));
                    }
                    consume_payload_bytes(
                        &mut estimated_bytes,
                        json_encoded_string_len(key).saturating_add(3),
                    )?;
                    scheduled_nodes += 1;
                    stack.push((value, depth.saturating_add(1)));
                }
            }
            Value::Array(values) => {
                if values.len() > remaining_nodes {
                    return Err(DreamAuditError::InvalidDetail("payload has too many nodes"));
                }
                consume_payload_bytes(&mut estimated_bytes, values.len().saturating_mul(2))?;
                for value in values {
                    scheduled_nodes += 1;
                    stack.push((value, depth.saturating_add(1)));
                }
            }
            Value::String(value) => {
                if value.len() > MAX_PAYLOAD_STRING_BYTES {
                    return Err(DreamAuditError::InvalidDetail(
                        "payload string exceeds 64 KiB",
                    ));
                }
                consume_payload_bytes(&mut estimated_bytes, json_encoded_string_len(value))?;
            }
            Value::Number(_) => consume_payload_bytes(&mut estimated_bytes, 32)?,
            Value::Bool(_) => consume_payload_bytes(&mut estimated_bytes, 5)?,
            Value::Null => consume_payload_bytes(&mut estimated_bytes, 4)?,
        }
    }
    Ok(())
}

fn consume_payload_bytes(total: &mut usize, amount: usize) -> Result<(), DreamAuditError> {
    *total = total.saturating_add(amount);
    if *total > MAX_PAYLOAD_BYTES {
        Err(DreamAuditError::InvalidDetail("payload exceeds 64 KiB"))
    } else {
        Ok(())
    }
}

fn json_encoded_string_len(value: &str) -> usize {
    value.chars().fold(2_usize, |total, character| {
        let encoded = match character {
            '"' | '\\' | '\u{0008}' | '\t' | '\n' | '\u{000C}' | '\r' => 2,
            '\u{0000}'..='\u{001F}' => 6,
            _ => character.len_utf8(),
        };
        total.saturating_add(encoded)
    })
}

fn redact_payload(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        if is_sensitive_name(key) {
                            Value::String("[REDACTED]".to_owned())
                        } else {
                            redact_payload(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_payload).collect()),
        Value::String(value) if contains_sensitive_value(value) => {
            Value::String("[REDACTED]".to_owned())
        }
        _ => value.clone(),
    }
}

fn redact_error(error: &str) -> String {
    redact_and_bound_text(error)
}

fn redact_and_bound_text(value: &str) -> String {
    if contains_sensitive_value(value) {
        "[REDACTED]".to_owned()
    } else {
        truncate_utf8(value, MAX_DETAIL_BYTES).to_owned()
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn is_sensitive_name(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    normalized == "key"
        || normalized.ends_with("key")
        || [
            "apikey",
            "token",
            "bearer",
            "secret",
            "password",
            "credential",
            "cookie",
            "authorization",
            "privatekey",
            "clientsecret",
            "anthropicversion",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn contains_sensitive_value(value: &str) -> bool {
    [
        "api_key",
        "api-key",
        "api key",
        "apikey",
        "authorization",
        "bearer ",
        "bearer=",
        "token=",
        "token:",
        "secret=",
        "secret:",
        "password=",
        "password:",
        "credential=",
        "credential:",
        "cookie=",
        "cookie:",
        "private_key",
        "private-key",
        "private key",
        "client_secret",
        "client-secret",
        "client secret",
    ]
    .iter()
    .any(|marker| contains_ascii_case_insensitive(value, marker))
}

fn contains_ascii_case_insensitive(value: &str, marker: &str) -> bool {
    value
        .as_bytes()
        .windows(marker.len())
        .any(|window| window.eq_ignore_ascii_case(marker.as_bytes()))
}

fn lifecycle(
    entity: &'static str,
    id: i64,
    expected: &'static str,
    actual: String,
) -> DreamAuditError {
    DreamAuditError::LifecycleConflict {
        entity,
        id,
        expected,
        actual,
    }
}

#[cfg(test)]
mod tests {
    use super::json_encoded_string_len;

    #[test]
    fn json_encoded_string_length_accounts_for_quotes_slashes_controls_and_unicode() {
        assert_eq!(json_encoded_string_len("plain"), 7);
        assert_eq!(json_encoded_string_len("\"\\\n\u{0001}é"), 16);
    }
}
