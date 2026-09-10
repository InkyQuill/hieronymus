//! Shared audit-row, crystal-row, FTS and rule-guard helpers for the admin
//! surface (plans W2/W3): [`super::actions`] leans on all of these, and the
//! rule guard keeps a W3 action from ever minting or retiring a `rule`
//! projection outside plan M3 (ADR 0011).

use rusqlite::{Connection, Transaction};
use serde_json::{Value, json};

use crate::application::AppError;

use super::{clamp_score, now_rfc3339};

/// Write one audit row inside the caller's transaction (Python
/// `_audit_with_connection`, plus the authenticated actor). The eight
/// parameters are exactly the `audit_log` columns this layer sets.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_audit(
    transaction: &Transaction<'_>,
    actor: &str,
    action: &str,
    entity_type: &str,
    entity_id: &str,
    note: &str,
    before_json: &str,
    after_json: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "insert into audit_log(
           actor, action, entity_type, entity_id, note, before_json, after_json, created_at
         )
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            actor,
            action,
            entity_type,
            entity_id,
            note,
            before_json,
            after_json,
            now_rfc3339(),
        ],
    )?;
    Ok(())
}

/// One crystal row as a sorted JSON object (Python `_row_json`), for the
/// audit `before_json`/`after_json`.
pub(super) fn crystal_row_json(
    transaction: &Transaction<'_>,
    crystal_id: i64,
) -> Result<String, AppError> {
    let row = load_crystal_row(transaction, crystal_id)?;
    Ok(serde_json::to_string(&row).unwrap_or_else(|_| "{}".to_string()))
}

pub(super) struct CrystalRow {
    pub(super) crystal_type: String,
    pub(super) text: String,
    pub(super) title: String,
    pub(super) scope_type: String,
    pub(super) scope_key: String,
    pub(super) series_slug: String,
    pub(super) source_language: String,
    pub(super) target_language: String,
    pub(super) tags_json: String,
    pub(super) strength: f64,
    pub(super) confidence: f64,
    pub(super) status: String,
}

impl CrystalRow {
    fn as_json(&self) -> Value {
        json!({
            "crystal_type": self.crystal_type,
            "text": self.text,
            "title": self.title,
            "scope_type": self.scope_type,
            "scope_key": self.scope_key,
            "series_slug": self.series_slug,
            "source_language": self.source_language,
            "target_language": self.target_language,
            "tags_json": self.tags_json,
            "strength": self.strength,
            "confidence": self.confidence,
            "status": self.status,
        })
    }
}

impl serde::Serialize for CrystalRow {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_json().serialize(serializer)
    }
}

pub(super) fn load_crystal_row(
    connection: &Connection,
    crystal_id: i64,
) -> Result<CrystalRow, AppError> {
    connection
        .query_row(
            "select crystal_type, text, title, scope_type, scope_key, series_slug,
                    source_language, target_language, tags_json, strength, confidence, status
             from crystals where id = ?1",
            [crystal_id],
            |row| {
                Ok(CrystalRow {
                    crystal_type: row.get("crystal_type")?,
                    text: row.get("text")?,
                    title: row.get("title")?,
                    scope_type: row.get("scope_type")?,
                    scope_key: row.get("scope_key")?,
                    series_slug: row.get("series_slug")?,
                    source_language: row.get("source_language")?,
                    target_language: row.get("target_language")?,
                    tags_json: row.get("tags_json")?,
                    strength: row.get("strength")?,
                    confidence: row.get("confidence")?,
                    status: row.get("status")?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::Domain(format!("unknown crystal: {crystal_id}"))
            }
            _ => AppError::Domain("admin store is unavailable".to_string()),
        })
}

/// Reject `delete`/`merge`/`split` of a rule-crystal projection — regardless
/// of status (ADR 0011). `merge`/`split` copy `crystal_type` from the source
/// and always land `status='active'`, so allowing a `candidate` rule through
/// here would mint an ACTIVE rule projection outside M3. Rule-crystal
/// lifecycle goes through the explicit rule action (plan M3).
pub(super) fn reject_rule_crystal(row: &CrystalRow, verb: &str) -> Result<(), AppError> {
    if row.crystal_type == "rule" {
        return Err(AppError::Domain(format!(
            "cannot {verb} a rule crystal from the memory view; use the \
             rule-crystal archive action (hieronymus_rule_crystal_archive) so the \
             structured rule authority transitions with the projection"
        )));
    }
    Ok(())
}

/// Delete then re-insert the crystal's external-content FTS row (Python
/// `_replace_crystal_fts`).
pub(super) fn replace_crystal_fts(
    transaction: &Transaction<'_>,
    crystal_id: i64,
    old_title: &str,
    old_text: &str,
    title: &str,
    text: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "insert into crystals_fts(crystals_fts, rowid, title, text) values ('delete', ?1, ?2, ?3)",
        rusqlite::params![crystal_id, old_title, old_text],
    )?;
    transaction.execute(
        "insert into crystals_fts(rowid, title, text) values (?1, ?2, ?3)",
        rusqlite::params![crystal_id, title, text],
    )?;
    Ok(())
}

pub(super) fn insert_crystal_fts(
    transaction: &Transaction<'_>,
    crystal_id: i64,
    title: &str,
    text: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "insert into crystals_fts(rowid, title, text) values (?1, ?2, ?3)",
        rusqlite::params![crystal_id, title, text],
    )?;
    Ok(())
}

/// Insert one crystal row cloned from `source` with new `title`/`text`
/// (the merge/split target shape; Python inserts the same columns).
#[allow(clippy::too_many_arguments)]
pub(super) fn insert_derived_crystal(
    transaction: &Transaction<'_>,
    source: &CrystalRow,
    title: &str,
    text: &str,
    strength: f64,
    confidence: f64,
    now: &str,
) -> rusqlite::Result<i64> {
    transaction.execute(
        "insert into crystals(
           crystal_type, text, title, scope_type, scope_key, series_slug,
           source_language, target_language, tags_json, strength, confidence,
           status, created_at, updated_at
         )
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?12)",
        rusqlite::params![
            source.crystal_type,
            text,
            title,
            source.scope_type,
            source.scope_key,
            source.series_slug,
            source.source_language,
            source.target_language,
            source.tags_json,
            clamp_score(strength),
            clamp_score(confidence),
            now,
        ],
    )?;
    let new_id = transaction.last_insert_rowid();
    insert_crystal_fts(transaction, new_id, title, text)?;
    Ok(new_id)
}

pub(super) fn row_json(row: &CrystalRow) -> String {
    serde_json::to_string(row).unwrap_or_else(|_| "{}".to_string())
}
