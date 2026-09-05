//! LanceDB-backed vector store for the semantic RAG lane, ported from the
//! qualified harness's `GenerationIndex`
//! (`qualification/harnesses/semantic-native/src/index.rs`).
//!
//! One LanceDB database lives under the data root; each generation owns one
//! table (`generation_<id>`), so a rebuild never touches the rows that the
//! previous active generation serves. Series filtering is applied as an
//! `only_if` predicate INSIDE the ANN query — the qualified harness proved
//! (`series-prefilter-before-ann`, `zero-cross-series-hits`) that this places
//! the predicate below the ANN operator, so a global top-k can never starve an
//! eligible series and results can never cross series.
//!
//! The library surface stays synchronous: LanceDB's async API runs on a
//! process-wide dedicated tokio runtime, and every call blocks on its result.

use arrow_array::{
    FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray, types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use lancedb::{
    DistanceType, Table, connect,
    database::CreateTableMode,
    index::{Index, vector::IvfPqIndexBuilder},
    query::{ExecutableQuery, QueryBase, Select},
};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::semantic_embeddings::EmbeddingIdentity;
use crate::semantic_error::SemanticError;

/// Name of the fixed-size float vector column.
pub const VECTOR_COLUMN: &str = "vector";
/// Partitions used by the cosine IVF-PQ index (qualified constants).
pub const ANN_NUM_PARTITIONS: u32 = 16;
/// PQ code bits; 8-bit codebooks need at least 256 training rows, so
/// populations below the partition minimum keep the flat scan (correct, just
/// unindexed) until a rebuild at scale builds the index.
pub const ANN_PQ_NUM_BITS: u32 = 8;
/// ANN probe parameter: probing every partition keeps recall at the qualified
/// operating point.
pub const ANN_NUM_PROBES: usize = 16;
/// Exact re-ranking multiplier applied to ANN candidates.
pub const ANN_REFINE_FACTOR: u32 = 8;
/// Name under which the ANN index is registered.
pub const ANN_INDEX_NAME: &str = "series_ann_ivf_pq";
/// Upper bound on waiting for index statistics to report full coverage.
const ANN_INDEX_WAIT_TIMEOUT: Duration = Duration::from_secs(300);
const ANN_INDEX_WAIT_TICK: Duration = Duration::from_millis(100);

/// LanceDB tables are stored as `<database>/<table>.lance` directories; this
/// filesystem-level existence check is how directory loss is detected without
/// opening the store.
fn table_dir(root: &Path, generation: &str) -> PathBuf {
    root.join(format!("{}.lance", table_name_for(generation)))
}

/// Whether the generation's table directory exists on disk.
pub fn generation_table_exists(root: &Path, generation: &str) -> bool {
    table_dir(root, generation).is_dir()
}

/// Process-wide runtime bridging the synchronous library surface onto
/// LanceDB's async API. One runtime, created on first use, lives for the
/// process lifetime.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("semantic bridge runtime must start")
    })
}

/// One vector record. Identifiers plus the embedding are the complete record;
/// the model columns carry the record's identity per the design contract.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexRow {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub generation_id: String,
    pub model: String,
    pub model_revision: String,
    pub vector: Vec<f32>,
}

/// Vector-free fingerprint of one stored row. Activation reconciles these
/// against the authoritative SQLite rows; the embedding never leaves the
/// index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowFingerprint {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub generation_id: String,
}

/// One nearest-neighbor hit. Distances use lance's cosine metric; vectors are
/// never returned.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticHit {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub generation_id: String,
    pub distance: f32,
}

/// A LanceDB-backed vector index over one generation.
pub struct VectorIndex {
    root: PathBuf,
    identity: EmbeddingIdentity,
    generation: String,
    table: Table,
}

impl VectorIndex {
    /// Opens (or creates) the generation's table. Reopening a partially built
    /// table is allowed; every appended row is still validated against the
    /// identity and the generation.
    pub fn open(
        root: &Path,
        identity: EmbeddingIdentity,
        generation: &str,
    ) -> Result<Self, SemanticError> {
        validate_generation_id(generation)?;
        let connection = runtime().block_on(async {
            connect(root.to_str().ok_or_else(|| {
                SemanticError::Store("index root must be valid UTF-8".to_string())
            })?)
            .execute()
            .await
            .map_err(|error| SemanticError::Store(format!("could not open the store: {error}")))
        })?;
        let table_name = table_name_for(generation);
        let dimensions = identity.dimensions();
        let table = runtime().block_on(async {
            connection
                .create_empty_table(&table_name, row_schema(dimensions))
                .mode(CreateTableMode::exist_ok(|request| request))
                .execute()
                .await
                .map_err(|error| {
                    SemanticError::Store(format!("could not create table {table_name}: {error}"))
                })
        })?;
        drop(connection);
        Ok(Self {
            root: root.to_path_buf(),
            identity,
            generation: generation.to_string(),
            table,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Appends validated rows to the generation. Rows from another generation,
    /// another model identity, or with mismatched widths are rejected before
    /// any native I/O.
    pub fn append(&mut self, rows: Vec<IndexRow>) -> Result<usize, SemanticError> {
        if rows.is_empty() {
            return Ok(0);
        }
        for row in &rows {
            self.validate_row(row)?;
        }
        let batch = rows_to_batch(&rows, self.identity.dimensions())?;
        runtime()
            .block_on(self.table.add(batch).execute())
            .map_err(|error| {
                SemanticError::Store(format!("could not append rows to the generation: {error}"))
            })?;
        Ok(rows.len())
    }

    fn validate_row(&self, row: &IndexRow) -> Result<(), SemanticError> {
        if row.chunk_id <= 0 {
            return Err(SemanticError::InvalidEmbedding(format!(
                "chunk id must be positive, got {}",
                row.chunk_id
            )));
        }
        validate_slug(&row.series_slug)?;
        if row.checksum.is_empty() {
            return Err(SemanticError::InvalidEmbedding(
                "chunk checksum must not be empty".to_string(),
            ));
        }
        if row.generation_id != self.generation {
            return Err(SemanticError::InvalidEmbedding(format!(
                "row generation {} does not match the generation {}",
                row.generation_id, self.generation
            )));
        }
        if row.model != self.identity.model() || row.model_revision != self.identity.revision() {
            return Err(SemanticError::InvalidEmbedding(format!(
                "row model {}@{} does not match the generation identity {}@{}",
                row.model,
                row.model_revision,
                self.identity.model(),
                self.identity.revision()
            )));
        }
        if row.vector.len() != self.identity.dimensions() {
            return Err(SemanticError::InvalidEmbedding(format!(
                "vector width {} does not match the identity width {}",
                row.vector.len(),
                self.identity.dimensions()
            )));
        }
        if !row.vector.iter().all(|value| value.is_finite()) {
            return Err(SemanticError::InvalidEmbedding(
                "vector contains non-finite values".to_string(),
            ));
        }
        Ok(())
    }

    /// Pre-filtered ANN search: the `series_slug` predicate is part of the ANN
    /// query itself, never a post-filter or an over-fetch.
    pub fn search(
        &self,
        series_slug: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticHit>, SemanticError> {
        if vector.len() != self.identity.dimensions() {
            return Err(SemanticError::InvalidEmbedding(format!(
                "query vector width {} does not match the indexed width {}",
                vector.len(),
                self.identity.dimensions()
            )));
        }
        if limit == 0 {
            return Err(SemanticError::ValidationFailed(
                "search limit must be at least 1".to_string(),
            ));
        }
        validate_slug(series_slug)?;
        let predicate = format!("series_slug = '{series_slug}'");
        // ANN parameters only apply to indexed tables; the flat scan (a
        // correct, unindexed fallback) answers small generations directly.
        let indexed = self.ann_index_stats()?.is_some();
        let batches = runtime().block_on(async {
            let mut query = self
                .table
                .query()
                .nearest_to(vector.to_vec())
                .map_err(|error| {
                    SemanticError::Store(format!("could not bind the query vector: {error}"))
                })?
                .column(VECTOR_COLUMN)
                .only_if(predicate)
                .limit(limit);
            if indexed {
                query = query
                    .nprobes(ANN_NUM_PROBES)
                    .refine_factor(ANN_REFINE_FACTOR);
            }
            query
                .execute()
                .await
                .map_err(|error| {
                    SemanticError::Store(format!("pre-filtered search failed: {error}"))
                })?
                .try_collect::<Vec<RecordBatch>>()
                .await
                .map_err(|error| SemanticError::Store(format!("search stream failed: {error}")))
        })?;
        decode_hits(batches)
    }

    /// The adversarial control for tests: identical search with the predicate
    /// applied after a global top-k, which closer decoy series starve.
    pub fn search_postfiltered(
        &self,
        series_slug: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticHit>, SemanticError> {
        if vector.len() != self.identity.dimensions() {
            return Err(SemanticError::InvalidEmbedding(format!(
                "query vector width {} does not match the indexed width {}",
                vector.len(),
                self.identity.dimensions()
            )));
        }
        validate_slug(series_slug)?;
        let predicate = format!("series_slug = '{series_slug}'");
        let batches = runtime().block_on(async {
            self.table
                .query()
                .nearest_to(vector.to_vec())
                .map_err(|error| {
                    SemanticError::Store(format!("could not bind the query vector: {error}"))
                })?
                .column(VECTOR_COLUMN)
                .only_if(predicate)
                .postfilter()
                .limit(limit)
                .execute()
                .await
                .map_err(|error| {
                    SemanticError::Store(format!("post-filtered search failed: {error}"))
                })?
                .try_collect::<Vec<RecordBatch>>()
                .await
                .map_err(|error| SemanticError::Store(format!("search stream failed: {error}")))
        })?;
        decode_hits(batches)
    }

    /// Builds the cosine IVF-PQ ANN index over the vector column and waits
    /// until index statistics report every row of the generation as indexed.
    pub fn create_ann_index(&mut self) -> Result<(), SemanticError> {
        let rows = self.count_rows()?;
        if rows < ANN_NUM_PARTITIONS as usize {
            return Err(SemanticError::ValidationFailed(format!(
                "an ANN index needs at least {ANN_NUM_PARTITIONS} rows, found {rows}"
            )));
        }
        runtime().block_on(async {
            self.table
                .create_index(
                    &[VECTOR_COLUMN],
                    Index::IvfPq(
                        IvfPqIndexBuilder::default()
                            .distance_type(DistanceType::Cosine)
                            .num_partitions(ANN_NUM_PARTITIONS)
                            .num_bits(ANN_PQ_NUM_BITS),
                    ),
                )
                .name(ANN_INDEX_NAME.to_string())
                .execute()
                .await
                .map_err(|error| {
                    SemanticError::Store(format!("could not create the ANN index: {error}"))
                })
        })?;

        let deadline = Instant::now() + ANN_INDEX_WAIT_TIMEOUT;
        loop {
            let stats = self.ann_index_stats()?;
            if let Some(stats) = stats
                && stats.num_unindexed_rows == 0
                && stats.num_indexed_rows == rows
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(SemanticError::Store(format!(
                    "timed out waiting for the ANN index to cover all {rows} rows"
                )));
            }
            std::thread::sleep(ANN_INDEX_WAIT_TICK);
        }
    }

    /// Current ANN index statistics, `None` when no index exists yet.
    pub fn ann_index_stats(
        &self,
    ) -> Result<Option<lancedb::index::IndexStatistics>, SemanticError> {
        runtime().block_on(async {
            self.table
                .index_stats(ANN_INDEX_NAME)
                .await
                .map_err(|error| {
                    SemanticError::Store(format!("could not read index statistics: {error}"))
                })
        })
    }

    /// Counts every stored row of the generation.
    pub fn count_rows(&self) -> Result<usize, SemanticError> {
        runtime().block_on(async {
            self.table
                .count_rows(None)
                .await
                .map_err(|error| SemanticError::Store(format!("could not count rows: {error}")))
        })
    }

    /// Reads back vector-free fingerprints of up to `limit` stored rows so
    /// activation can reconcile the index against the authoritative SQLite
    /// rows. The embedding column is never read.
    pub fn snapshot_rows(&self, limit: usize) -> Result<Vec<RowFingerprint>, SemanticError> {
        let batches = runtime().block_on(async {
            self.table
                .query()
                .select(Select::Columns(vec![
                    "chunk_id".to_string(),
                    "series_slug".to_string(),
                    "checksum".to_string(),
                    "generation_id".to_string(),
                ]))
                .limit(limit)
                .execute()
                .await
                .map_err(|error| {
                    SemanticError::Store(format!("could not snapshot generation rows: {error}"))
                })?
                .try_collect::<Vec<RecordBatch>>()
                .await
                .map_err(|error| SemanticError::Store(format!("snapshot stream failed: {error}")))
        })?;
        let mut rows = Vec::new();
        for batch in batches {
            let chunk_ids = column::<Int64Array>(&batch, "chunk_id")?;
            let series = column::<StringArray>(&batch, "series_slug")?;
            let checksums = column::<StringArray>(&batch, "checksum")?;
            let generations = column::<StringArray>(&batch, "generation_id")?;
            for index in 0..batch.num_rows() {
                rows.push(RowFingerprint {
                    chunk_id: chunk_ids.value(index),
                    series_slug: series.value(index).to_string(),
                    checksum: checksums.value(index).to_string(),
                    generation_id: generations.value(index).to_string(),
                });
            }
        }
        Ok(rows)
    }

    /// Width of one stored embedding, or `None` when the generation holds no
    /// rows. Activation uses this as the stored-side dimension check.
    pub fn stored_vector_width(&self) -> Result<Option<usize>, SemanticError> {
        use arrow_array::Array as _;
        let batches = runtime().block_on(async {
            self.table
                .query()
                .select(Select::Columns(vec![VECTOR_COLUMN.to_string()]))
                .limit(1)
                .execute()
                .await
                .map_err(|error| {
                    SemanticError::Store(format!("could not read a stored vector: {error}"))
                })?
                .try_collect::<Vec<RecordBatch>>()
                .await
                .map_err(|error| SemanticError::Store(format!("vector read failed: {error}")))
        })?;
        for batch in batches {
            let vectors = column::<FixedSizeListArray>(&batch, VECTOR_COLUMN)?;
            if vectors.len() > 0 {
                return Ok(Some(vectors.value_length() as usize));
            }
        }
        Ok(None)
    }
}

/// Drops the generation's table, returning whether anything was removed.
/// A missing table is treated as already collected.
pub fn drop_generation_table(root: &Path, generation: &str) -> Result<bool, SemanticError> {
    validate_generation_id(generation)?;
    let connection =
        runtime().block_on(async {
            connect(root.to_str().ok_or_else(|| {
                SemanticError::Store("index root must be valid UTF-8".to_string())
            })?)
            .execute()
            .await
            .map_err(|error| SemanticError::Store(format!("could not open the store: {error}")))
        })?;
    let table_name = table_name_for(generation);
    let drop_error_context = table_name.clone();
    runtime().block_on(async {
        let names = connection
            .table_names()
            .execute()
            .await
            .map_err(|error| SemanticError::Store(format!("could not list tables: {error}")))?;
        if !names.iter().any(|name| name.as_str() == table_name) {
            return Ok(false);
        }
        connection
            .drop_table(table_name, &[])
            .await
            .map_err(|error| {
                SemanticError::Store(format!(
                    "could not drop table {drop_error_context}: {error}"
                ))
            })?;
        Ok(true)
    })
}

fn column<'a, T: arrow_array::Array + 'static>(
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a T, SemanticError> {
    batch
        .column_by_name(name)
        .ok_or_else(|| SemanticError::Store(format!("batch is missing {name}")))?
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| SemanticError::Store(format!("{name} column has the wrong type")))
}

fn decode_hits(batches: Vec<RecordBatch>) -> Result<Vec<SemanticHit>, SemanticError> {
    let mut hits = Vec::new();
    for batch in batches {
        let chunk_ids = column::<Int64Array>(&batch, "chunk_id")?;
        let series = column::<StringArray>(&batch, "series_slug")?;
        let checksums = column::<StringArray>(&batch, "checksum")?;
        let generations = column::<StringArray>(&batch, "generation_id")?;
        let distances = column::<Float32Array>(&batch, "_distance")?;
        for index in 0..batch.num_rows() {
            hits.push(SemanticHit {
                chunk_id: chunk_ids.value(index),
                series_slug: series.value(index).to_string(),
                checksum: checksums.value(index).to_string(),
                generation_id: generations.value(index).to_string(),
                distance: distances.value(index),
            });
        }
    }
    Ok(hits)
}

/// Generation ids and series slugs share the harness's normalized-slug shape.
pub fn validate_slug(slug: &str) -> Result<(), SemanticError> {
    if slug.is_empty() {
        return Err(SemanticError::InvalidSlug(
            "slug must not be empty".to_string(),
        ));
    }
    if !slug.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
    }) {
        return Err(SemanticError::InvalidSlug(format!(
            "slug {slug:?} is not a normalized slug"
        )));
    }
    Ok(())
}

fn validate_generation_id(generation: &str) -> Result<(), SemanticError> {
    validate_slug(generation)
}

fn table_name_for(generation: &str) -> String {
    format!(
        "generation_{}",
        generation
            .chars()
            .map(|character| if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            })
            .collect::<String>()
    )
}

fn row_schema(dimensions: usize) -> SchemaRef {
    std::sync::Arc::new(Schema::new(vec![
        Field::new("chunk_id", DataType::Int64, false),
        Field::new("series_slug", DataType::Utf8, false),
        Field::new("checksum", DataType::Utf8, false),
        Field::new("generation_id", DataType::Utf8, false),
        Field::new("model", DataType::Utf8, false),
        Field::new("model_revision", DataType::Utf8, false),
        Field::new(
            VECTOR_COLUMN,
            DataType::FixedSizeList(
                std::sync::Arc::new(Field::new("item", DataType::Float32, true)),
                dimensions as i32,
            ),
            false,
        ),
    ]))
}

fn rows_to_batch(rows: &[IndexRow], dimensions: usize) -> Result<RecordBatch, SemanticError> {
    let chunk_id = Int64Array::from_iter(rows.iter().map(|row| Some(row.chunk_id)));
    let series_slug =
        StringArray::from_iter_values(rows.iter().map(|row| row.series_slug.as_str()));
    let checksum = StringArray::from_iter_values(rows.iter().map(|row| row.checksum.as_str()));
    let generation_id =
        StringArray::from_iter_values(rows.iter().map(|row| row.generation_id.as_str()));
    let model = StringArray::from_iter_values(rows.iter().map(|row| row.model.as_str()));
    let model_revision =
        StringArray::from_iter_values(rows.iter().map(|row| row.model_revision.as_str()));
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter().map(|row| {
            Some(
                row.vector
                    .iter()
                    .map(|value| Some(*value))
                    .collect::<Vec<_>>(),
            )
        }),
        dimensions as i32,
    );
    RecordBatch::try_new(
        row_schema(dimensions),
        vec![
            std::sync::Arc::new(chunk_id),
            std::sync::Arc::new(series_slug),
            std::sync::Arc::new(checksum),
            std::sync::Arc::new(generation_id),
            std::sync::Arc::new(model),
            std::sync::Arc::new(model_revision),
            std::sync::Arc::new(vectors),
        ],
    )
    .map_err(|error| SemanticError::Store(format!("could not build the row batch: {error}")))
}
