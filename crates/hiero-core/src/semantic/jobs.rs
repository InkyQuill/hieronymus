use std::collections::{BTreeMap, HashMap};

use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool, Transaction};

use super::SemanticError;
use crate::{db::RagChunkRecord as RawRagChunkRecord, rag::RagChunkRecord};

const MAX_BATCH_SIZE: usize = 500;
const SCAN_PAGE_SIZE: i64 = 256;

pub struct SemanticJobQueue<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SemanticJobQueue<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn enqueue_rebuild(&self) -> Result<i64, SemanticError> {
        let id = sqlx::query_scalar(
            "INSERT INTO semantic_index_jobs(status, created_at) VALUES ('pending', ?) RETURNING id",
        )
        .bind(Utc::now())
        .fetch_one(self.pool)
        .await?;
        Ok(id)
    }

    pub async fn claim_next_batch(
        &self,
        batch_size: usize,
    ) -> Result<Vec<RagChunkRecord>, SemanticError> {
        if batch_size == 0 || batch_size > MAX_BATCH_SIZE {
            return Err(SemanticError::InvalidVector {
                reason: format!("semantic batch size must be within 1..={MAX_BATCH_SIZE}"),
            });
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some((job_id, job_generation)) = claim_job(&mut transaction).await? else {
            transaction.commit().await?;
            return Ok(vec![]);
        };
        let claim_marker = format!("claim:{job_id}");
        let mut rows = Vec::with_capacity(batch_size);
        let mut offset = 0_i64;
        while rows.len() < batch_size {
            let page = sqlx::query_as::<_, ClaimRow>(CLAIM_SCAN_SQL)
                .bind(SCAN_PAGE_SIZE)
                .bind(offset)
                .fetch_all(&mut *transaction)
                .await?;
            if page.is_empty() {
                break;
            }
            offset += i64::try_from(page.len()).unwrap_or(SCAN_PAGE_SIZE);
            for row in page {
                let checksum = checksum(&row.text);
                let already_claimed = row.indexed_at.as_deref() == Some(&claim_marker);
                let fresh = row.state_checksum.as_deref() == Some(checksum.as_str())
                    && !row
                        .state_generation
                        .as_deref()
                        .unwrap_or_default()
                        .is_empty()
                    && job_generation.as_deref().is_none_or(|generation| {
                        row.state_generation.as_deref() == Some(generation)
                    });
                if already_claimed || fresh {
                    continue;
                }
                sqlx::query(
                    r#"INSERT INTO semantic_chunk_state(chunk_id, checksum, generation_id, indexed_at)
                       VALUES (?, '', '', ?)
                       ON CONFLICT(chunk_id) DO UPDATE SET indexed_at = excluded.indexed_at"#,
                )
                .bind(row.id)
                .bind(&claim_marker)
                .execute(&mut *transaction)
                .await?;
                rows.push(row);
                if rows.len() == batch_size {
                    break;
                }
            }
            if offset % SCAN_PAGE_SIZE != 0 {
                break;
            }
        }
        if rows.is_empty() {
            sqlx::query("UPDATE semantic_index_jobs SET status = 'completed', completed_at = ? WHERE id = ? AND status = 'running'")
                .bind(Utc::now()).bind(job_id).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        hydrate_claim_rows(self.pool, rows).await
    }

    pub async fn mark_indexed(
        &self,
        chunk_ids: &[i64],
        generation_id: &str,
    ) -> Result<(), SemanticError> {
        if uuid::Uuid::parse_str(generation_id).is_err() {
            return Err(SemanticError::InvalidVector {
                reason: "generation_id must be a UUID".into(),
            });
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let running_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM semantic_index_jobs WHERE status = 'running'")
                .fetch_one(&mut *transaction)
                .await?;
        if running_count > 0 {
            let assigned = sqlx::query(
                "UPDATE semantic_index_jobs SET generation_id = ? WHERE status = 'running' AND (generation_id IS NULL OR generation_id = ?)",
            )
            .bind(generation_id)
            .bind(generation_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if assigned == 0 {
                return Err(SemanticError::InvalidVector {
                    reason: "running semantic job belongs to a different generation".into(),
                });
            }
        }
        for chunk_id in chunk_ids {
            let text: Option<String> =
                sqlx::query_scalar("SELECT text FROM rag_chunks WHERE id = ?")
                    .bind(chunk_id)
                    .fetch_optional(&mut *transaction)
                    .await?;
            let Some(text) = text else {
                return Err(SemanticError::InvalidVector {
                    reason: format!("unknown RAG chunk {chunk_id}"),
                });
            };
            sqlx::query(
                r#"INSERT INTO semantic_chunk_state(chunk_id, checksum, generation_id, indexed_at)
                   VALUES (?, ?, ?, ?)
                   ON CONFLICT(chunk_id) DO UPDATE SET checksum = excluded.checksum,
                     generation_id = excluded.generation_id, indexed_at = excluded.indexed_at"#,
            )
            .bind(chunk_id)
            .bind(checksum(&text))
            .bind(generation_id)
            .bind(Utc::now())
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Assigns the generation allocated by the vector index to a pending full rebuild.
    pub async fn assign_generation(
        &self,
        job_id: i64,
        generation_id: &str,
    ) -> Result<(), SemanticError> {
        if uuid::Uuid::parse_str(generation_id).is_err() {
            return Err(SemanticError::InvalidVector {
                reason: "generation_id must be a UUID".into(),
            });
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let other_running: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM semantic_index_jobs WHERE status = 'running' AND id <> ?",
        )
        .bind(job_id)
        .fetch_one(&mut *transaction)
        .await?;
        if other_running != 0 {
            return Err(SemanticError::InvalidVector {
                reason: "another semantic rebuild is already running".into(),
            });
        }
        let changed = sqlx::query(
            "UPDATE semantic_index_jobs SET status = 'running', generation_id = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(generation_id)
        .bind(job_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(SemanticError::InvalidVector {
                reason: format!("semantic job {job_id} is not pending"),
            });
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn mark_failed(&self, job_id: i64, error: &str) -> Result<(), SemanticError> {
        let clean = error.chars().take(2_000).collect::<String>();
        let changed = sqlx::query("UPDATE semantic_index_jobs SET status = 'failed', completed_at = ? WHERE id = ? AND status IN ('pending', 'running')")
            .bind(Utc::now()).bind(job_id).execute(self.pool).await?.rows_affected();
        if changed == 0 {
            return Err(SemanticError::InvalidVector {
                reason: format!("semantic job {job_id} is not pending or running: {clean}"),
            });
        }
        Ok(())
    }
}

async fn claim_job(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<Option<(i64, Option<String>)>, SemanticError> {
    if let Some(row) = sqlx::query(
        "SELECT id, generation_id FROM semantic_index_jobs WHERE status = 'running' ORDER BY id LIMIT 1",
    )
    .fetch_optional(&mut **transaction)
    .await?
    {
        return Ok(Some((row.get("id"), row.get("generation_id"))));
    }
    let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM semantic_index_jobs WHERE status = 'pending' ORDER BY id LIMIT 1",
    )
    .fetch_optional(&mut **transaction)
    .await?
    else {
        return Ok(None);
    };
    let changed = sqlx::query(
        "UPDATE semantic_index_jobs SET status = 'running' WHERE id = ? AND status = 'pending'",
    )
    .bind(id)
    .execute(&mut **transaction)
    .await?
    .rows_affected();
    if changed == 1 {
        Ok(Some((id, None)))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Clone, FromRow)]
struct ClaimRow {
    id: i64,
    source_id: i64,
    series_slug: String,
    chunk_kind: String,
    text: String,
    display_text: String,
    location: String,
    metadata_json: String,
    created_at: chrono::DateTime<Utc>,
    source_ref: String,
    state_checksum: Option<String>,
    state_generation: Option<String>,
    indexed_at: Option<String>,
}

const CLAIM_SCAN_SQL: &str = r#"
SELECT c.*, r.source_ref, s.checksum AS state_checksum,
       s.generation_id AS state_generation, s.indexed_at
FROM rag_chunks c
JOIN rag_sources r ON r.id = c.source_id AND r.series_slug = c.series_slug
LEFT JOIN semantic_chunk_state s ON s.chunk_id = c.id
ORDER BY c.id LIMIT ? OFFSET ?
"#;

async fn hydrate_claim_rows(
    pool: &SqlitePool,
    rows: Vec<ClaimRow>,
) -> Result<Vec<RagChunkRecord>, SemanticError> {
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let tags = hydrate_tags(pool, &ids).await?;
    rows.into_iter()
        .map(|row| {
            let metadata =
                serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&row.metadata_json)
                    .map_err(|error| SemanticError::CorruptIndex {
                        reason: format!("invalid RAG chunk metadata: {error}"),
                    })?;
            let values = tags.get(&row.id).cloned().unwrap_or_default();
            Ok(RagChunkRecord {
                record: RawRagChunkRecord {
                    id: row.id,
                    source_id: row.source_id,
                    series_slug: row.series_slug,
                    chunk_kind: row.chunk_kind,
                    text: row.text,
                    display_text: row.display_text,
                    location: row.location,
                    metadata_json: row.metadata_json,
                    created_at: row.created_at,
                },
                source_ref: row.source_ref,
                metadata,
                language_tags: values.0,
                story_scopes: values.1,
                semantic_tags: values.2,
            })
        })
        .collect()
}

type Tags = (Vec<String>, Vec<String>, Vec<String>);

async fn hydrate_tags(pool: &SqlitePool, ids: &[i64]) -> Result<HashMap<i64, Tags>, SemanticError> {
    let mut result = ids
        .iter()
        .map(|id| (*id, Tags::default()))
        .collect::<HashMap<_, _>>();
    if ids.is_empty() {
        return Ok(result);
    }
    for (table, column, slot) in [
        ("rag_chunk_language_tags", "language_tag", 0),
        ("rag_chunk_story_scopes", "story_scope", 1),
        ("rag_chunk_semantic_tags", "semantic_tag", 2),
    ] {
        let mut builder = QueryBuilder::new(format!(
            "SELECT chunk_id, {column} AS value FROM {table} WHERE chunk_id IN ("
        ));
        let mut separated = builder.separated(",");
        for id in ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") ORDER BY chunk_id, value");
        for row in builder.build().fetch_all(pool).await? {
            let entry = result.entry(row.get("chunk_id")).or_default();
            match slot {
                0 => entry.0.push(row.get("value")),
                1 => entry.1.push(row.get("value")),
                _ => entry.2.push(row.get("value")),
            }
        }
    }
    Ok(result)
}

fn checksum(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
