use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{
    Decode, FromRow, Row, Sqlite, Type, TypeInfo, ValueRef,
    error::BoxDynError,
    sqlite::{SqliteRow, SqliteTypeInfo, SqliteValueRef},
};

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct SeriesRecord {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub default_source_language: String,
    pub default_target_language: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct TaskSessionRecord {
    pub id: i64,
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    pub task_type: String,
    pub volume: String,
    pub chapter: String,
    pub status: String,
    pub cycle_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct ShortTermMemoryRecord {
    pub id: i64,
    pub session_id: i64,
    pub source_role: String,
    pub kind: String,
    pub text: String,
    pub source_ref: String,
    /// Raw storage payload. Parsing belongs at the service boundary so malformed legacy data is visible.
    pub metadata_json: String,
    pub source_credibility: Option<String>,
    pub rule_intent: Option<String>,
    pub soft_origin: Option<String>,
    pub source_crystal_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrystalRecord {
    pub id: i64,
    pub crystal_type: String,
    pub text: String,
    pub title: String,
    pub scope_type: String,
    pub scope_key: String,
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    /// Raw storage payload; callers may choose their own validated tag representation.
    pub tags_json: String,
    pub strength: f64,
    pub confidence: f64,
    pub source_credibility: String,
    pub rule_intent: String,
    pub soft_origin: Option<String>,
    pub is_inferred: bool,
    pub malformed_penalty: f64,
    pub supersedes_crystal_id: Option<i64>,
    pub status: String,
    pub created_cycle: i64,
    pub last_activated_cycle: Option<i64>,
    pub last_reinforced_cycle: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl<'r> FromRow<'r, SqliteRow> for CrystalRecord {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            crystal_type: row.try_get("crystal_type")?,
            text: row.try_get("text")?,
            title: row.try_get("title")?,
            scope_type: row.try_get("scope_type")?,
            scope_key: row.try_get("scope_key")?,
            series_slug: row.try_get("series_slug")?,
            source_language: row.try_get("source_language")?,
            target_language: row.try_get("target_language")?,
            tags_json: row.try_get("tags_json")?,
            strength: row.try_get("strength")?,
            confidence: row.try_get("confidence")?,
            source_credibility: row.try_get("source_credibility")?,
            rule_intent: row.try_get("rule_intent")?,
            soft_origin: row.try_get("soft_origin")?,
            is_inferred: strict_bool(row, "is_inferred")?,
            malformed_penalty: row.try_get("malformed_penalty")?,
            supersedes_crystal_id: row.try_get("supersedes_crystal_id")?,
            status: row.try_get("status")?,
            created_cycle: row.try_get("created_cycle")?,
            last_activated_cycle: row.try_get("last_activated_cycle")?,
            last_reinforced_cycle: row.try_get("last_reinforced_cycle")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
pub struct CrystalActivationRecord {
    pub id: i64,
    pub crystal_id: i64,
    pub session_id: i64,
    pub recall_query: String,
    pub rank: i64,
    pub score: f64,
    pub reason: String,
    pub outcome: Option<String>,
    pub cycle_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct CrystalLinkRecord {
    pub source_crystal_id: i64,
    pub target_crystal_id: i64,
    pub link_type: String,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize, Deserialize)]
pub struct ConceptRecord {
    pub id: i64,
    pub canonical_name: String,
    pub description: String,
    pub scope_type: String,
    pub scope_key: String,
    pub status: String,
    pub confidence: f64,
    pub merged_into_concept_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConceptFacetRecord {
    pub id: i64,
    pub concept_id: i64,
    pub language: String,
    pub facet_type: String,
    pub value: String,
    pub source_crystal_id: Option<i64>,
    pub confidence: f64,
    pub is_canonical: bool,
    pub superseded_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl<'r> FromRow<'r, SqliteRow> for ConceptFacetRecord {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            concept_id: row.try_get("concept_id")?,
            language: row.try_get("language")?,
            facet_type: row.try_get("facet_type")?,
            value: row.try_get("value")?,
            source_crystal_id: row.try_get("source_crystal_id")?,
            confidence: row.try_get("confidence")?,
            is_canonical: strict_bool(row, "is_canonical")?,
            superseded_at: row.try_get("superseded_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct ConceptProposalRecord {
    pub id: i64,
    pub dream_run_id: Option<i64>,
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    pub concept_text: String,
    pub source_form: String,
    pub canonical_rendering: String,
    /// Raw JSON is preserved exactly for migration and API compatibility.
    pub approved_variants_json: String,
    /// Raw JSON is preserved exactly for migration and API compatibility.
    pub forbidden_variants_json: String,
    pub rationale: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct ConceptMergeProposalRecord {
    pub id: i64,
    pub source_concept_id: i64,
    pub target_concept_id: i64,
    pub rationale: String,
    pub status: String,
    pub dream_run_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEventRecord {
    pub id: i64,
    pub crystal_id: Option<i64>,
    pub session_id: Option<i64>,
    pub event_type: String,
    pub source_role: String,
    pub evidence: String,
    pub strength_delta: f64,
    pub confidence_delta: f64,
    pub applied: bool,
    pub cycle_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl<'r> FromRow<'r, SqliteRow> for MemoryEventRecord {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            crystal_id: row.try_get("crystal_id")?,
            session_id: row.try_get("session_id")?,
            event_type: row.try_get("event_type")?,
            source_role: row.try_get("source_role")?,
            evidence: row.try_get("evidence")?,
            strength_delta: row.try_get("strength_delta")?,
            confidence_delta: row.try_get("confidence_delta")?,
            applied: strict_bool(row, "applied")?,
            cycle_id: row.try_get("cycle_id")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct RagChunkRecord {
    pub id: i64,
    pub source_id: i64,
    pub series_slug: String,
    pub chunk_kind: String,
    pub text: String,
    pub display_text: String,
    pub location: String,
    /// Raw storage payload; decoding is intentionally deferred to RAG consumers.
    pub metadata_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct DreamRunRecord {
    pub id: i64,
    pub cycle_id: i64,
    pub status: String,
    pub provider: String,
    pub input_count: i64,
    pub created_crystal_count: i64,
    pub proposal_count: i64,
    pub error: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown {kind} label: {label}")]
pub struct PersistedLabelError {
    kind: &'static str,
    label: String,
}

macro_rules! persisted_enum {
    ($name:ident, $kind:literal, { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = PersistedLabelError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($label => Ok(Self::$variant),)+
                    _ => Err(PersistedLabelError { kind: $kind, label: value.to_owned() }),
                }
            }
        }

        impl TryFrom<String> for $name {
            type Error = PersistedLabelError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl Type<Sqlite> for $name {
            fn type_info() -> SqliteTypeInfo { <String as Type<Sqlite>>::type_info() }
            fn compatible(ty: &SqliteTypeInfo) -> bool { <String as Type<Sqlite>>::compatible(ty) }
        }

        impl<'r> Decode<'r, Sqlite> for $name {
            fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
                let label = <String as Decode<Sqlite>>::decode(value)?;
                Ok(label.parse()?)
            }
        }
    };
}

persisted_enum!(CrystalType, "crystal type", {
    Lesson => "lesson", Rule => "rule", Thought => "thought", Observation => "observation",
    ConceptNote => "concept_note", Concept => "concept", Erudition => "erudition",
});
persisted_enum!(TaskSessionStatus, "task session status", {
    Active => "active", Completed => "completed", Dreamed => "dreamed",
});
persisted_enum!(CrystalStatus, "crystal status", {
    Active => "active", Candidate => "candidate", Archived => "archived",
    Rejected => "rejected", Superseded => "superseded",
});
persisted_enum!(ConceptStatus, "concept status", {
    Candidate => "candidate", Established => "established", Archived => "archived", Merged => "merged",
});
persisted_enum!(ConceptProposalStatus, "concept proposal status", {
    Pending => "pending", Approved => "approved", Rejected => "rejected",
});
persisted_enum!(DreamRunStatus, "dream run status", {
    Running => "running", Completed => "completed", Failed => "failed", Skipped => "skipped",
});
persisted_enum!(RecallOutcome, "recall outcome", { Useful => "useful", Miss => "miss" });

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub(crate) struct StrictTermRow {
    pub(crate) id: i64,
    pub(crate) series_slug: String,
    pub(crate) source_language: String,
    pub(crate) target_language: String,
    pub(crate) category: String,
    pub(crate) source_text: String,
    pub(crate) canonical_translation: String,
    pub(crate) status: String,
    pub(crate) notes: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StrictTermAliasRow {
    pub(crate) id: i64,
    pub(crate) term_id: i64,
    pub(crate) language: String,
    pub(crate) text: String,
    pub(crate) kind: String,
    pub(crate) case_sensitive: bool,
}

impl<'r> FromRow<'r, SqliteRow> for StrictTermAliasRow {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            term_id: row.try_get("term_id")?,
            language: row.try_get("language")?,
            text: row.try_get("text")?,
            kind: row.try_get("kind")?,
            case_sensitive: strict_bool(row, "case_sensitive")?,
        })
    }
}

fn strict_bool(row: &SqliteRow, column: &'static str) -> Result<bool, sqlx::Error> {
    let raw = row.try_get_raw(column)?;
    if raw.is_null() {
        return Err(boolean_decode_error(column, SqliteBooleanError::Null));
    }

    let storage_class = raw.type_info().name().to_owned();
    if storage_class != "INTEGER" {
        return Err(boolean_decode_error(
            column,
            SqliteBooleanError::StorageClass { storage_class },
        ));
    }

    match row.try_get::<i64, _>(column)? {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(boolean_decode_error(
            column,
            SqliteBooleanError::OutOfRange { value },
        )),
    }
}

#[derive(Debug, thiserror::Error)]
enum SqliteBooleanError {
    #[error("expected a non-null SQLite INTEGER boolean 0 or 1, got NULL")]
    Null,
    #[error("expected a non-null SQLite INTEGER boolean 0 or 1, got {storage_class}")]
    StorageClass { storage_class: String },
    #[error("expected a SQLite INTEGER boolean 0 or 1, got INTEGER {value}")]
    OutOfRange { value: i64 },
}

fn boolean_decode_error(column: &'static str, source: SqliteBooleanError) -> sqlx::Error {
    sqlx::Error::ColumnDecode {
        index: column.to_owned(),
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::{StrictTermAliasRow, StrictTermRow};
    use sqlx::{AssertSqlSafe, SqlitePool};

    #[tokio::test]
    async fn strict_term_row_decodes_without_becoming_public_api() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let row = sqlx::query_as::<_, StrictTermRow>(
            "SELECT 1 AS id, 'book' AS series_slug, 'en' AS source_language, 'ru' AS target_language, 'name' AS category, 'Name' AS source_text, 'Имя' AS canonical_translation, 'active' AS status, '' AS notes, '2026-07-18T12:34:56Z' AS created_at, '2026-07-18T13:34:56Z' AS updated_at",
        )
        .fetch_one(&pool)
        .await
        .expect("legacy strict term should decode");
        assert_eq!(row.canonical_translation, "Имя");
    }

    #[tokio::test]
    async fn strict_term_alias_rejects_invalid_integer_boolean() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let row = sqlx::query_as::<_, StrictTermAliasRow>(
            "SELECT 1 AS id, 2 AS term_id, 'ru' AS language, 'Имя' AS text, 'approved' AS kind, 1 AS case_sensitive",
        )
        .fetch_one(&pool)
        .await
        .expect("legacy strict alias should decode");
        assert!(row.case_sensitive);

        let false_row = sqlx::query_as::<_, StrictTermAliasRow>(
            "SELECT 1 AS id, 2 AS term_id, 'ru' AS language, 'Имя' AS text, 'approved' AS kind, 0 AS case_sensitive",
        )
        .fetch_one(&pool)
        .await
        .expect("legacy false boolean should decode");
        assert!(!false_row.case_sensitive);

        let error = sqlx::query_as::<_, StrictTermAliasRow>(
            "SELECT 1 AS id, 2 AS term_id, 'ru' AS language, 'Имя' AS text, 'approved' AS kind, -1 AS case_sensitive",
        )
        .fetch_one(&pool)
        .await
        .expect_err("invalid legacy boolean must fail");
        assert!(error.to_string().contains("case_sensitive"));
    }

    #[tokio::test]
    async fn strict_term_alias_rejects_null_and_non_integer_booleans() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        for (expression, expected_type) in [
            ("NULL", "NULL"),
            ("'1'", "TEXT"),
            ("1.0", "REAL"),
            ("X'31'", "BLOB"),
        ] {
            let query = format!(
                "SELECT 1 AS id, 2 AS term_id, 'ru' AS language, 'Имя' AS text, 'approved' AS kind, {expression} AS case_sensitive"
            );
            let error = sqlx::query_as::<_, StrictTermAliasRow>(AssertSqlSafe(query))
                .fetch_one(&pool)
                .await
                .expect_err("legacy booleans require a non-null INTEGER");
            let message = error.to_string();
            assert!(message.contains("case_sensitive"));
            assert!(
                message.contains(expected_type),
                "missing {expected_type}: {message}"
            );
        }
    }
}
