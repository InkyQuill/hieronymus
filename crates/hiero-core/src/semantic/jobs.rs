use std::{
    collections::{BTreeMap, HashMap},
    ops::Deref,
};

use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqlitePool, Transaction};

use super::SemanticError;
use crate::{db::RagChunkRecord as RawRagChunkRecord, rag::RagChunkRecord};

const MAX_BATCH_SIZE: usize = 500;
const SCAN_PAGE_SIZE: i64 = 256;

/// An owned semantic-work claim. Completion and failure require this token.
#[derive(Debug)]
pub struct ClaimedSemanticBatch {
    job_id: i64,
    token: uuid::Uuid,
    generation_id: Option<String>,
    chunks: Vec<RagChunkRecord>,
}

impl ClaimedSemanticBatch {
    #[must_use]
    pub const fn job_id(&self) -> i64 {
        self.job_id
    }
    #[must_use]
    pub fn token(&self) -> String {
        self.token.to_string()
    }
    #[must_use]
    pub fn generation_id(&self) -> Option<&str> {
        self.generation_id.as_deref()
    }
}

impl Deref for ClaimedSemanticBatch {
    type Target = [RagChunkRecord];
    fn deref(&self) -> &Self::Target {
        &self.chunks
    }
}

pub struct SemanticJobQueue<'a> {
    pool: &'a SqlitePool,
}

struct ClaimPause {
    claimed: std::sync::Arc<tokio::sync::Notify>,
    resume: std::sync::Arc<tokio::sync::Notify>,
}

impl<'a> SemanticJobQueue<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn enqueue_rebuild(&self) -> Result<i64, SemanticError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO semantic_index_jobs(status, created_at) VALUES ('pending', ?) RETURNING id",
        ).bind(Utc::now()).fetch_one(self.pool).await?)
    }

    pub async fn claim_next_batch(
        &self,
        batch_size: usize,
    ) -> Result<ClaimedSemanticBatch, SemanticError> {
        self.claim_next_batch_inner(batch_size, None).await
    }

    async fn claim_next_batch_inner(
        &self,
        batch_size: usize,
        pause: Option<&ClaimPause>,
    ) -> Result<ClaimedSemanticBatch, SemanticError> {
        if batch_size == 0 || batch_size > MAX_BATCH_SIZE {
            return Err(SemanticError::InvalidVector {
                reason: format!("semantic batch size must be within 1..={MAX_BATCH_SIZE}"),
            });
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let Some((job_id, generation_id)) = claim_job(&mut tx).await? else {
            tx.commit().await?;
            return Ok(empty_batch());
        };
        let token = uuid::Uuid::new_v4();
        let mut rows = Vec::with_capacity(batch_size);
        let mut after_id = i64::MIN;
        while rows.len() < batch_size {
            let page = sqlx::query_as::<_, ClaimRow>(CLAIM_SCAN_SQL)
                .bind(after_id)
                .bind(job_id)
                .bind(SCAN_PAGE_SIZE)
                .fetch_all(&mut *tx)
                .await?;
            if page.is_empty() {
                break;
            }
            let page_was_full = page.len() == usize::try_from(SCAN_PAGE_SIZE).unwrap_or(usize::MAX);
            after_id = page.last().map_or(after_id, |row| row.id);
            for row in page {
                if is_fresh(&row, generation_id.as_deref()) {
                    continue;
                }
                sqlx::query(
                    "INSERT INTO semantic_batch_claims(claim_token, job_id, chunk_id, generation_id, created_at) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(token.to_string()).bind(job_id).bind(row.id)
                .bind(&generation_id).bind(Utc::now()).execute(&mut *tx).await?;
                rows.push(row);
                if rows.len() == batch_size {
                    break;
                }
            }
            if !page_was_full {
                break;
            }
        }
        if let Some(pause) = pause {
            pause.claimed.notify_one();
            pause.resume.notified().await;
        }
        // Hydration is DB-only and remains in the claim transaction. Cancellation or an error
        // rolls back the transaction and therefore cannot strand a claim.
        let chunks = hydrate_claim_rows(&mut tx, rows).await?;
        if chunks.is_empty() {
            maybe_complete(&mut tx, job_id, generation_id.as_deref()).await?;
        }
        tx.commit().await?;
        Ok(ClaimedSemanticBatch {
            job_id,
            token,
            generation_id,
            chunks,
        })
    }

    /// Commits only the owned batch and returns the number of input claims committed.
    pub async fn mark_indexed(
        &self,
        batch: &ClaimedSemanticBatch,
        generation_id: &str,
    ) -> Result<usize, SemanticError> {
        validate_generation(generation_id)?;
        if batch.chunks.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let current: Option<String> = sqlx::query_scalar(
            "SELECT generation_id FROM semantic_index_jobs WHERE id = ? AND status = 'running'",
        )
        .bind(batch.job_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| invalid_claim(batch))?;
        if current
            .as_deref()
            .is_some_and(|value| value != generation_id)
            || batch
                .generation_id
                .as_deref()
                .is_some_and(|value| value != generation_id)
        {
            return Err(SemanticError::InvalidVector {
                reason: "semantic claim belongs to a different generation".into(),
            });
        }
        let owned: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM semantic_batch_claims WHERE claim_token = ? AND job_id = ?",
        )
        .bind(batch.token.to_string())
        .bind(batch.job_id)
        .fetch_one(&mut *tx)
        .await?;
        if usize::try_from(owned).ok() != Some(batch.chunks.len()) {
            return Err(invalid_claim(batch));
        }
        let bound = sqlx::query(
            "UPDATE semantic_index_jobs SET generation_id = ? WHERE id = ? AND status = 'running' AND (generation_id IS NULL OR generation_id = ?)",
        ).bind(generation_id).bind(batch.job_id).bind(generation_id)
            .execute(&mut *tx).await?.rows_affected();
        if bound != 1 {
            return Err(invalid_claim(batch));
        }
        for chunk in &batch.chunks {
            sqlx::query(
                "INSERT INTO semantic_chunk_state(chunk_id, checksum, generation_id, indexed_at) VALUES (?, ?, ?, ?) ON CONFLICT(chunk_id) DO UPDATE SET checksum = excluded.checksum, generation_id = excluded.generation_id, indexed_at = excluded.indexed_at",
            ).bind(chunk.id).bind(checksum(&chunk.text)).bind(generation_id).bind(Utc::now())
                .execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM semantic_batch_claims WHERE claim_token = ? AND job_id = ?")
            .bind(batch.token.to_string())
            .bind(batch.job_id)
            .execute(&mut *tx)
            .await?;
        maybe_complete(&mut tx, batch.job_id, Some(generation_id)).await?;
        tx.commit().await?;
        Ok(batch.chunks.len())
    }

    /// Releases only this worker's claim; the same chunks remain reclaimable.
    pub async fn mark_failed(
        &self,
        batch: &ClaimedSemanticBatch,
        error: &str,
    ) -> Result<(), SemanticError> {
        let _clean = error.chars().take(2_000).collect::<String>();
        let changed =
            sqlx::query("DELETE FROM semantic_batch_claims WHERE claim_token = ? AND job_id = ?")
                .bind(batch.token.to_string())
                .bind(batch.job_id)
                .execute(self.pool)
                .await?
                .rows_affected();
        if changed != batch.chunks.len() as u64 {
            return Err(invalid_claim(batch));
        }
        Ok(())
    }

    pub async fn assign_generation(
        &self,
        job_id: i64,
        generation_id: &str,
    ) -> Result<(), SemanticError> {
        validate_generation(generation_id)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let other: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM semantic_index_jobs WHERE status = 'running' AND id <> ?",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        if other != 0 {
            return Err(SemanticError::InvalidVector {
                reason: "another semantic rebuild is already running".into(),
            });
        }
        let changed = sqlx::query(
            "UPDATE semantic_index_jobs SET status = 'running', generation_id = ? WHERE id = ? AND status = 'pending'",
        ).bind(generation_id).bind(job_id).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(SemanticError::InvalidVector {
                reason: format!("semantic job {job_id} is not pending"),
            });
        }
        tx.commit().await?;
        Ok(())
    }
}

fn empty_batch() -> ClaimedSemanticBatch {
    ClaimedSemanticBatch {
        job_id: 0,
        token: uuid::Uuid::nil(),
        generation_id: None,
        chunks: vec![],
    }
}

fn validate_generation(value: &str) -> Result<(), SemanticError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| SemanticError::InvalidVector {
            reason: "generation_id must be a UUID".into(),
        })
}

fn invalid_claim(batch: &ClaimedSemanticBatch) -> SemanticError {
    SemanticError::InvalidVector {
        reason: format!("semantic batch claim {} is no longer owned", batch.token),
    }
}

async fn maybe_complete(
    tx: &mut Transaction<'_, Sqlite>,
    job_id: i64,
    generation: Option<&str>,
) -> Result<(), SemanticError> {
    if has_pending(tx, job_id, generation).await? {
        return Ok(());
    }
    let in_flight: i64 =
        sqlx::query_scalar("SELECT count(*) FROM semantic_batch_claims WHERE job_id = ?")
            .bind(job_id)
            .fetch_one(&mut **tx)
            .await?;
    if in_flight == 0 {
        sqlx::query(
            "UPDATE semantic_index_jobs SET status = 'completed', completed_at = ? WHERE id = ? AND status = 'running'",
        ).bind(Utc::now()).bind(job_id).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn has_pending(
    tx: &mut Transaction<'_, Sqlite>,
    job_id: i64,
    generation: Option<&str>,
) -> Result<bool, SemanticError> {
    let rows = sqlx::query_as::<_, ClaimRow>(CLAIM_SCAN_ALL_SQL)
        .bind(job_id)
        .fetch_all(&mut **tx)
        .await?;
    Ok(rows.iter().any(|row| !is_fresh(row, generation)))
}

fn is_fresh(row: &ClaimRow, generation: Option<&str>) -> bool {
    row.state_checksum.as_deref() == Some(checksum(&row.text).as_str())
        && !row
            .state_generation
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        && generation.is_none_or(|wanted| row.state_generation.as_deref() == Some(wanted))
}

async fn claim_job(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Option<(i64, Option<String>)>, SemanticError> {
    if let Some(row) = sqlx::query(
        "SELECT id, generation_id FROM semantic_index_jobs WHERE status = 'running' ORDER BY id LIMIT 1",
    ).fetch_optional(&mut **tx).await? {
        return Ok(Some((row.get("id"), row.get("generation_id"))));
    }
    let Some(id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM semantic_index_jobs WHERE status = 'pending' ORDER BY id LIMIT 1",
    )
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    let changed = sqlx::query(
        "UPDATE semantic_index_jobs SET status = 'running' WHERE id = ? AND status = 'pending'",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok((changed == 1).then_some((id, None)))
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
}

const CLAIM_SCAN_SQL: &str = r#"
SELECT c.*, r.source_ref, s.checksum AS state_checksum, s.generation_id AS state_generation
FROM rag_chunks c
JOIN rag_sources r ON r.id = c.source_id AND r.series_slug = c.series_slug
LEFT JOIN semantic_chunk_state s ON s.chunk_id = c.id
WHERE c.id > ? AND NOT EXISTS (
  SELECT 1 FROM semantic_batch_claims claim WHERE claim.job_id = ? AND claim.chunk_id = c.id
)
ORDER BY c.id LIMIT ?
"#;

const CLAIM_SCAN_ALL_SQL: &str = r#"
SELECT c.*, r.source_ref, s.checksum AS state_checksum, s.generation_id AS state_generation
FROM rag_chunks c
JOIN rag_sources r ON r.id = c.source_id AND r.series_slug = c.series_slug
LEFT JOIN semantic_chunk_state s ON s.chunk_id = c.id
WHERE NOT EXISTS (
  SELECT 1 FROM semantic_batch_claims claim WHERE claim.job_id = ? AND claim.chunk_id = c.id
)
ORDER BY c.id
"#;

async fn hydrate_claim_rows(
    tx: &mut Transaction<'_, Sqlite>,
    rows: Vec<ClaimRow>,
) -> Result<Vec<RagChunkRecord>, SemanticError> {
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let tags = hydrate_tags(tx, &ids).await?;
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

async fn hydrate_tags(
    tx: &mut Transaction<'_, Sqlite>,
    ids: &[i64],
) -> Result<HashMap<i64, Tags>, SemanticError> {
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
        for row in builder.build().fetch_all(&mut **tx).await? {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ClaimPause, SemanticJobQueue};
    use chrono::Utc;

    #[tokio::test]
    async fn cancelling_after_claim_insertion_rolls_back_ownership() {
        let url = format!(
            "sqlite:file:semantic-cancel-{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );
        let pool = crate::db::connect_url(&url).await.unwrap();
        sqlx::query("INSERT INTO series(slug, title, default_source_language, default_target_language, created_at, updated_at) VALUES ('book', 'Book', 'en', 'ru', ?, ?)")
            .bind(Utc::now()).bind(Utc::now()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO rag_sources(id, series_slug, source_ref, source_type, content_type, checksum, metadata_json, created_at, updated_at) VALUES (1, 'book', 'test', 'text', 'text/plain', 'source', '{}', ?, ?)")
            .bind(Utc::now()).bind(Utc::now()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO rag_chunks(id, source_id, series_slug, chunk_kind, text, display_text, location, metadata_json, created_at) VALUES (10, 1, 'book', 'text', 'alpha', 'alpha', '', '{}', ?)")
            .bind(Utc::now()).execute(&pool).await.unwrap();
        SemanticJobQueue::new(&pool)
            .enqueue_rebuild()
            .await
            .unwrap();

        let pause = Arc::new(ClaimPause {
            claimed: Arc::new(tokio::sync::Notify::new()),
            resume: Arc::new(tokio::sync::Notify::new()),
        });
        let worker_pool = pool.clone();
        let worker_pause = pause.clone();
        let worker = tokio::spawn(async move {
            SemanticJobQueue::new(&worker_pool)
                .claim_next_batch_inner(1, Some(&worker_pause))
                .await
        });
        pause.claimed.notified().await;
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());

        let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM semantic_batch_claims")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(claims, 0);
        assert_eq!(
            SemanticJobQueue::new(&pool)
                .claim_next_batch(1)
                .await
                .unwrap()[0]
                .id,
            10
        );
    }
}
