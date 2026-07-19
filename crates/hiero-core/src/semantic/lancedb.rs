use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow_array::{
    FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator, StringArray,
    types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use fs4::FileExt;
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use super::{
    GenerationId, GenerationInfo, IndexHealth, SearchFilter, SearchHit, SemanticError,
    SemanticIndex, VectorRecord,
    index::{validate_query, validate_vectors},
};

const ACTIVE_FILE: &str = "ACTIVE";
const TABLE: &str = "vectors";
const GENERATION_TABLE: &str = "_generation";
const LIFECYCLE_LOCK: &str = ".lifecycle.lock";

struct LifecycleGuard(std::fs::File);

impl Drop for LifecycleGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

pub struct LanceDbIndex {
    root: PathBuf,
    pool: Option<SqlitePool>,
}

impl LanceDbIndex {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SemanticError> {
        Self::open_inner(path, None).await
    }

    pub async fn open_with_pool(
        path: impl AsRef<Path>,
        pool: SqlitePool,
    ) -> Result<Self, SemanticError> {
        Self::open_inner(path, Some(pool)).await
    }

    async fn open_inner(
        path: impl AsRef<Path>,
        pool: Option<SqlitePool>,
    ) -> Result<Self, SemanticError> {
        reject_symlink_components(path.as_ref()).await?;
        if tokio::fs::symlink_metadata(path.as_ref())
            .await
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(SemanticError::CorruptIndex {
                reason: "semantic index root must not be a symlink".into(),
            });
        }
        tokio::fs::create_dir_all(path.as_ref()).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(path.as_ref(), std::fs::Permissions::from_mode(0o700))
                .await?;
        }
        let root = tokio::fs::canonicalize(path.as_ref()).await?;
        Ok(Self { root, pool })
    }

    async fn lifecycle_lock(&self) -> Result<LifecycleGuard, SemanticError> {
        let path = self.root.join(LIFECYCLE_LOCK);
        let file = tokio::task::spawn_blocking(move || {
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let file = options.open(path)?;
            FileExt::lock_exclusive(&file)?;
            Ok::<_, std::io::Error>(file)
        })
        .await??;
        let guard = LifecycleGuard(file);
        self.validate_layout_locked().await?;
        Ok(guard)
    }

    async fn validate_layout_locked(&self) -> Result<PathBuf, SemanticError> {
        let generations = self.root.join("generations");
        match tokio::fs::symlink_metadata(&generations).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(SemanticError::CorruptIndex {
                    reason: "semantic generations path must be a real directory".into(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir(&generations).await?;
            }
            Err(error) => return Err(error.into()),
        }
        let canonical = tokio::fs::canonicalize(&generations).await?;
        if !canonical.starts_with(&self.root) {
            return Err(SemanticError::CorruptIndex {
                reason: "semantic generations path escapes the index root".into(),
            });
        }
        let active = self.root.join(ACTIVE_FILE);
        if tokio::fs::symlink_metadata(active)
            .await
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(SemanticError::CorruptIndex {
                reason: "semantic ACTIVE pointer must not be a symlink".into(),
            });
        }
        Ok(canonical)
    }

    async fn validate_generation_locked(
        &self,
        generation: GenerationId,
    ) -> Result<PathBuf, SemanticError> {
        let generations = self.validate_layout_locked().await?;
        let path = self.generation_path(generation);
        let metadata = tokio::fs::symlink_metadata(&path).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SemanticError::MissingGeneration(generation)
            } else {
                error.into()
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SemanticError::CorruptIndex {
                reason: "semantic generation must be a real directory".into(),
            });
        }
        let canonical = tokio::fs::canonicalize(path).await?;
        if !canonical.starts_with(generations) {
            return Err(SemanticError::CorruptIndex {
                reason: "semantic generation escapes the generations root".into(),
            });
        }
        Ok(canonical)
    }

    fn generation_path(&self, generation: GenerationId) -> PathBuf {
        self.root.join("generations").join(generation.0.to_string())
    }

    async fn read_active(&self) -> Result<Option<GenerationId>, SemanticError> {
        let path = self.root.join(ACTIVE_FILE);
        match tokio::fs::read_to_string(path).await {
            Ok(value) => uuid::Uuid::parse_str(value.trim())
                .map(GenerationId)
                .map(Some)
                .map_err(|error| SemanticError::CorruptIndex {
                    reason: format!("invalid active generation pointer: {error}"),
                }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn connection(
        &self,
        generation: GenerationId,
    ) -> Result<lancedb::Connection, SemanticError> {
        let path = self.validate_generation_locked(generation).await?;
        let uri = path.to_string_lossy().into_owned();
        Ok(lancedb::connect(&uri).execute().await?)
    }

    async fn metadata(&self, chunk_ids: &[i64]) -> Result<Vec<(String, String)>, SemanticError> {
        let Some(pool) = &self.pool else {
            return Ok(chunk_ids
                .iter()
                .map(|_| (String::new(), String::new()))
                .collect());
        };
        let mut values = Vec::with_capacity(chunk_ids.len());
        for chunk_id in chunk_ids {
            let row = sqlx::query("SELECT series_slug, text FROM rag_chunks WHERE id = ?")
                .bind(chunk_id)
                .fetch_optional(pool)
                .await?;
            let Some(row) = row else {
                return Err(SemanticError::InvalidVector {
                    reason: format!("unknown RAG chunk {chunk_id}"),
                });
            };
            let text: String = row.get("text");
            values.push((row.get("series_slug"), hex_checksum(text.as_bytes())));
        }
        Ok(values)
    }

    async fn valid_hit(
        &self,
        generation: GenerationId,
        chunk_id: i64,
        checksum: &str,
        series: &str,
        wanted: &str,
    ) -> Result<bool, SemanticError> {
        if !series.is_empty() && series != wanted {
            return Ok(false);
        }
        let Some(pool) = &self.pool else {
            return Ok(true);
        };
        let row = sqlx::query("SELECT c.text, c.series_slug, s.checksum, s.generation_id FROM rag_chunks c JOIN semantic_chunk_state s ON s.chunk_id = c.id WHERE c.id = ?")
            .bind(chunk_id).fetch_optional(pool).await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let text: String = row.get("text");
        Ok(row.get::<String, _>("series_slug") == wanted
            && row.get::<String, _>("checksum") == checksum
            && row.get::<String, _>("generation_id") == generation.0.to_string()
            && checksum == hex_checksum(text.as_bytes()))
    }

    async fn ensure_generation_complete(
        &self,
        generation: GenerationId,
    ) -> Result<(), SemanticError> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };
        let mut indexed = HashMap::<i64, (String, String)>::new();
        let db = self.connection(generation).await?;
        if db
            .table_names()
            .execute()
            .await?
            .iter()
            .any(|name| name == TABLE)
        {
            let table = db.open_table(TABLE).execute().await?;
            let generation_size = table.count_rows(None).await?;
            let batches = table
                .query()
                .limit(generation_size)
                .execute()
                .await?
                .try_collect::<Vec<_>>()
                .await?;
            for batch in batches {
                let ids = batch
                    .column_by_name("chunk_id")
                    .and_then(|value| value.as_any().downcast_ref::<Int64Array>())
                    .ok_or_else(|| SemanticError::CorruptIndex {
                        reason: "chunk_id column missing".into(),
                    })?;
                let series = batch
                    .column_by_name("series_slug")
                    .and_then(|value| value.as_any().downcast_ref::<StringArray>())
                    .ok_or_else(|| SemanticError::CorruptIndex {
                        reason: "series_slug column missing".into(),
                    })?;
                let checksums = batch
                    .column_by_name("checksum")
                    .and_then(|value| value.as_any().downcast_ref::<StringArray>())
                    .ok_or_else(|| SemanticError::CorruptIndex {
                        reason: "checksum column missing".into(),
                    })?;
                for row in 0..batch.num_rows() {
                    if indexed
                        .insert(
                            ids.value(row),
                            (
                                series.value(row).to_owned(),
                                checksums.value(row).to_owned(),
                            ),
                        )
                        .is_some()
                    {
                        return Err(SemanticError::CorruptIndex {
                            reason: format!(
                                "generation contains duplicate chunk {}",
                                ids.value(row)
                            ),
                        });
                    }
                }
            }
        }
        let authoritative = sqlx::query("SELECT id, series_slug, text FROM rag_chunks ORDER BY id")
            .fetch_all(pool)
            .await?;
        if indexed.len() != authoritative.len() {
            return Err(SemanticError::CorruptIndex {
                reason: format!(
                    "generation has {} vectors for {} authoritative chunks",
                    indexed.len(),
                    authoritative.len()
                ),
            });
        }
        for row in authoritative {
            let id: i64 = row.get("id");
            let series: String = row.get("series_slug");
            let text: String = row.get("text");
            if indexed.get(&id) != Some(&(series, hex_checksum(text.as_bytes()))) {
                return Err(SemanticError::CorruptIndex {
                    reason: format!("generation has missing or stale vector for chunk {id}"),
                });
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SemanticIndex for LanceDbIndex {
    async fn health(&self) -> Result<IndexHealth, SemanticError> {
        match self.active_generation().await {
            Ok(active) => Ok(IndexHealth {
                ok: true,
                vector_count: active.as_ref().map_or(0, |value| value.vector_count),
                active_generation: active.map(|value| value.id),
            }),
            Err(_) => Ok(IndexHealth {
                ok: false,
                vector_count: 0,
                active_generation: None,
            }),
        }
    }

    async fn active_generation(&self) -> Result<Option<GenerationInfo>, SemanticError> {
        let Some(id) = self.read_active().await? else {
            return Ok(None);
        };
        let db = self.connection(id).await?;
        let tables = db.table_names().execute().await?;
        let vector_count = if tables.iter().any(|name| name == TABLE) {
            db.open_table(TABLE)
                .execute()
                .await?
                .count_rows(None)
                .await?
        } else {
            0
        };
        let metadata = tokio::fs::metadata(self.generation_path(id)).await?;
        let created_at = metadata
            .created()
            .or_else(|_| metadata.modified())
            .map(chrono::DateTime::<chrono::Utc>::from)
            .unwrap_or_else(|_| chrono::Utc::now());
        Ok(Some(GenerationInfo {
            id,
            created_at,
            vector_count,
        }))
    }

    async fn begin_rebuild(&self) -> Result<GenerationId, SemanticError> {
        let _guard = self.lifecycle_lock().await?;
        let id = GenerationId(uuid::Uuid::new_v4());
        tokio::fs::create_dir(self.generation_path(id)).await?;
        self.validate_generation_locked(id).await?;
        let db = self.connection(id).await?;
        db.create_empty_table(
            GENERATION_TABLE,
            Arc::new(Schema::new(vec![Field::new(
                "generation_id",
                DataType::Utf8,
                false,
            )])),
        )
        .execute()
        .await?;
        Ok(id)
    }

    async fn upsert_vectors(
        &self,
        generation: GenerationId,
        vectors: Vec<VectorRecord>,
    ) -> Result<usize, SemanticError> {
        let _guard = self.lifecycle_lock().await?;
        self.validate_generation_locked(generation).await?;
        validate_vectors(&vectors)?;
        let dimensions = i32::try_from(vectors[0].embedding.len()).map_err(|_| {
            SemanticError::InvalidVector {
                reason: "vector dimensions exceed LanceDB limits".into(),
            }
        })?;
        let ids = vectors
            .iter()
            .map(|value| value.chunk_id)
            .collect::<Vec<_>>();
        let metadata = self.metadata(&ids).await?;
        let schema = Arc::new(Schema::new(vec![
            Field::new("chunk_id", DataType::Int64, false),
            Field::new("series_slug", DataType::Utf8, false),
            Field::new("checksum", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    dimensions,
                ),
                false,
            ),
        ]));
        let series = metadata
            .iter()
            .map(|value| value.0.as_str())
            .collect::<Vec<_>>();
        let checksums = metadata
            .iter()
            .map(|value| value.1.as_str())
            .collect::<Vec<_>>();
        let arrays: Vec<Arc<dyn arrow_array::Array>> = vec![
            Arc::new(Int64Array::from(ids.clone())),
            Arc::new(StringArray::from(series)),
            Arc::new(StringArray::from(checksums)),
            Arc::new(
                FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                    vectors.iter().map(|value| {
                        Some(
                            value
                                .embedding
                                .iter()
                                .copied()
                                .map(Some)
                                .collect::<Vec<_>>(),
                        )
                    }),
                    dimensions,
                ),
            ),
        ];
        let batch = RecordBatch::try_new(schema.clone(), arrays)?;
        let reader = RecordBatchIterator::new([Ok(batch)].into_iter(), schema);
        let db = self.connection(generation).await?;
        let tables = db.table_names().execute().await?;
        if tables.iter().any(|name| name == TABLE) {
            let table = db.open_table(TABLE).execute().await?;
            let mut merge = table.merge_insert(&["chunk_id"]);
            merge
                .when_matched_update_all(None)
                .when_not_matched_insert_all();
            merge.execute(Box::new(reader)).await?;
        } else {
            db.create_table(TABLE, Box::new(reader)).execute().await?;
        }
        Ok(vectors.len())
    }

    async fn search(
        &self,
        query: &[f32],
        filter: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<SearchHit>, SemanticError> {
        validate_query(query)?;
        let generation = self
            .read_active()
            .await?
            .ok_or(SemanticError::NoActiveGeneration)?;
        let db = self.connection(generation).await?;
        if !db
            .table_names()
            .execute()
            .await?
            .iter()
            .any(|name| name == TABLE)
        {
            return Ok(vec![]);
        }
        let table =
            db.open_table(TABLE)
                .execute()
                .await
                .map_err(|error| SemanticError::CorruptIndex {
                    reason: error.to_string(),
                })?;
        if limit == 0 {
            return Ok(vec![]);
        }
        let escaped_series = filter.series_slug.replace('\'', "''");
        let predicate = format!("series_slug = '{escaped_series}'");
        let lance_filter = self.pool.as_ref().map(|_| predicate.clone());
        let generation_size = table.count_rows(lance_filter.clone()).await?;
        if generation_size == 0 {
            return Ok(vec![]);
        }
        let mut requested = limit.saturating_mul(4).max(limit).min(generation_size);
        let mut hits;
        loop {
            let nearest = table.query().nearest_to(query)?;
            let nearest = if let Some(predicate) = &lance_filter {
                nearest.only_if(predicate)
            } else {
                nearest
            };
            let batches = nearest
                .limit(requested)
                .execute()
                .await?
                .try_collect::<Vec<_>>()
                .await?;
            hits = Vec::new();
            for batch in batches {
                let ids = batch
                    .column_by_name("chunk_id")
                    .and_then(|value| value.as_any().downcast_ref::<Int64Array>())
                    .ok_or_else(|| SemanticError::CorruptIndex {
                        reason: "chunk_id column missing".into(),
                    })?;
                let distances = batch
                    .column_by_name("_distance")
                    .and_then(|value| value.as_any().downcast_ref::<Float32Array>())
                    .ok_or_else(|| SemanticError::CorruptIndex {
                        reason: "distance column missing".into(),
                    })?;
                let series = batch
                    .column_by_name("series_slug")
                    .and_then(|value| value.as_any().downcast_ref::<StringArray>())
                    .ok_or_else(|| SemanticError::CorruptIndex {
                        reason: "series_slug column missing".into(),
                    })?;
                let checksums = batch
                    .column_by_name("checksum")
                    .and_then(|value| value.as_any().downcast_ref::<StringArray>())
                    .ok_or_else(|| SemanticError::CorruptIndex {
                        reason: "checksum column missing".into(),
                    })?;
                for row in 0..batch.num_rows() {
                    if self
                        .valid_hit(
                            generation,
                            ids.value(row),
                            checksums.value(row),
                            series.value(row),
                            &filter.series_slug,
                        )
                        .await?
                    {
                        hits.push(SearchHit {
                            chunk_id: ids.value(row),
                            distance: distances.value(row),
                        });
                    }
                }
            }
            if hits.len() >= limit || requested >= generation_size {
                break;
            }
            requested = requested.saturating_mul(2).min(generation_size);
        }
        hits.sort_by(|left, right| {
            left.distance
                .total_cmp(&right.distance)
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    async fn delete_vectors(&self, chunk_ids: &[i64]) -> Result<usize, SemanticError> {
        if chunk_ids.is_empty() {
            return Ok(0);
        }
        let _guard = self.lifecycle_lock().await?;
        let generation = self
            .read_active()
            .await?
            .ok_or(SemanticError::NoActiveGeneration)?;
        let table = self
            .connection(generation)
            .await?
            .open_table(TABLE)
            .execute()
            .await?;
        let before = table.count_rows(None).await?;
        let predicate = chunk_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        table.delete(&format!("chunk_id IN ({predicate})")).await?;
        Ok(before - table.count_rows(None).await?)
    }

    async fn activate_generation(&self, generation: GenerationId) -> Result<(), SemanticError> {
        let _guard = self.lifecycle_lock().await?;
        self.validate_generation_locked(generation).await?;
        self.ensure_generation_complete(generation).await?;
        let temp = self
            .root
            .join(format!(".{ACTIVE_FILE}.{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&temp, generation.0.to_string()).await?;
        tokio::fs::rename(temp, self.root.join(ACTIVE_FILE)).await?;
        Ok(())
    }

    async fn cancel_generation(&self, generation: GenerationId) -> Result<(), SemanticError> {
        let _guard = self.lifecycle_lock().await?;
        self.validate_generation_locked(generation).await?;
        if self.read_active().await? == Some(generation) {
            return Err(SemanticError::Cancelled);
        }
        let path = self.generation_path(generation);
        if !path.exists() {
            return Err(SemanticError::MissingGeneration(generation));
        }
        tokio::fs::remove_dir_all(path).await?;
        Ok(())
    }

    async fn close(&self) -> Result<(), SemanticError> {
        let _guard = self.lifecycle_lock().await?;
        Ok(())
    }
}

fn hex_checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

async fn reject_symlink_components(path: &Path) -> Result<(), SemanticError> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut ancestors = absolute.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        match tokio::fs::symlink_metadata(ancestor).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(SemanticError::CorruptIndex {
                    reason: format!(
                        "semantic index path contains symlink component {}",
                        ancestor.display()
                    ),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
