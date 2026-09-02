use anyhow::{Context, ensure};
use arrow_array::{
    FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray, types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use lancedb::{
    DistanceType, Table, connect,
    index::{Index, vector::IvfPqIndexBuilder},
    query::{ExecutableQuery, QueryBase},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::corpus::Chunk;
pub use crate::model::EMBEDDING_DIMENSIONS;

/// Name of the fixed-size float vector column.
pub const VECTOR_COLUMN: &str = "vector";
/// Partitions used by the cosine IVF-PQ index.
pub const ANN_NUM_PARTITIONS: u32 = 16;
/// PQ code bits. 8-bit codebooks require at least 256 training rows; the
/// adversarial population therefore pads the mandated 10 eligible vectors and
/// 100 closer decoys with 150 distant filler rows so the index trains at full
/// precision. Search re-ranks candidates with exact cosine distances through
/// the refine stage, so result ordering stays deterministic.
pub const ANN_PQ_NUM_BITS: u32 = 8;
/// ANN probe parameter: with `ANN_NUM_PARTITIONS` partitions, probing all of
/// them keeps recall high; the adversarial proof still holds because a global
/// top-k is beaten by 100 closer decoys regardless of partition coverage.
pub const ANN_NUM_PROBES: usize = 16;
/// Exact re-ranking multiplier applied to ANN candidates.
pub const ANN_REFINE_FACTOR: u32 = 8;
/// Name under which the ANN index is registered.
pub const ANN_INDEX_NAME: &str = "series_ann_ivf_pq";
/// Upper bound on waiting for index statistics to report full coverage.
const ANN_INDEX_WAIT_TIMEOUT: Duration = Duration::from_secs(300);
const ANN_INDEX_WAIT_TICK: Duration = Duration::from_millis(100);

/// Identity of the embedding model backing an index generation.
///
/// Only normalized names, dimensions and checksums are stored; never vectors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelIdentity {
    model: String,
    dimensions: usize,
    checksum: String,
}

impl ModelIdentity {
    pub fn new(
        model: impl Into<String>,
        dimensions: usize,
        checksum: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let model = model.into();
        let checksum = checksum.into();
        ensure!(!model.is_empty(), "model name must not be empty");
        ensure!(!checksum.is_empty(), "model checksum must not be empty");
        ensure!(
            dimensions == EMBEDDING_DIMENSIONS,
            "unsupported embedding dimensions {dimensions}: expected {EMBEDDING_DIMENSIONS}"
        );
        Ok(Self {
            model,
            dimensions,
            checksum,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    pub fn checksum(&self) -> &str {
        &self.checksum
    }
}

/// One row fed into a generation index. Identifiers and the embedding vector
/// are the complete index input; nothing else is stored.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndexRow {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub generation_id: String,
    pub vector: Vec<f32>,
}

impl IndexRow {
    /// Maps a deterministic corpus chunk onto an index row. The corpus uses
    /// string chunk ids; the index requires a numeric id, which is derived as
    /// the first eight bytes of the SHA-256 of the string id. The derivation is
    /// pure, so the same corpus always produces the same rows.
    pub fn from_corpus_chunk(chunk: &Chunk) -> Self {
        let digest = Sha256::digest(chunk.id.as_bytes());
        let chunk_id = i64::from_be_bytes(digest[..8].try_into().expect("eight digest bytes"));
        Self {
            chunk_id,
            series_slug: chunk.series_slug.clone(),
            checksum: chunk.checksum.clone(),
            generation_id: chunk.generation_id.clone(),
            vector: chunk.embedding.clone(),
        }
    }

    fn validate(&self, identity: &ModelIdentity, generation: &str) -> anyhow::Result<()> {
        ensure!(self.chunk_id > 0, "chunk id must be positive");
        validate_series_slug(&self.series_slug)?;
        ensure!(!self.checksum.is_empty(), "checksum must not be empty");
        ensure!(
            !self.generation_id.is_empty(),
            "generation id must not be empty"
        );
        ensure!(
            self.generation_id == generation,
            "row generation {} does not match the active generation {generation}",
            self.generation_id
        );
        ensure!(
            self.vector.len() == identity.dimensions(),
            "vector width {} does not match the model identity width {}",
            self.vector.len(),
            identity.dimensions()
        );
        ensure!(
            self.vector.iter().all(|value| value.is_finite()),
            "vector contains non-finite values"
        );
        Ok(())
    }
}

/// A nearest-neighbor hit. Distances use lance's cosine metric; no vectors
/// are ever returned.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchHit {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub distance: f32,
}

/// Normalized facts about the ANN index backing a generation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndexEvidence {
    pub generation_id: String,
    pub index_type: String,
    pub index_name: String,
    pub distance_metric: String,
    pub num_partitions: usize,
    pub num_indexed_rows: usize,
    pub num_unindexed_rows: usize,
    pub identity: ModelIdentity,
}

/// Normalized explain/analyze evidence for a pre-filtered ANN search.
///
/// Operators, predicates and index names are stored in normalized form with
/// cardinalities. Raw vectors never appear: the query vector is a bind
/// parameter and cannot leak into a plan string.
#[derive(Clone, Debug, Serialize)]
pub struct PrefilterPlan {
    pub ann_index_used: bool,
    pub series_predicate_below_ann: bool,
    pub eligible_cardinality: usize,
    pub requested_top_k: usize,
    pub returned_count: usize,
    pub index_name: Option<String>,
    pub distance_metric: Option<String>,
    pub predicate: String,
    pub operators: Vec<String>,
    pub explain_plan: String,
    pub analyze_plan: String,
}

impl PrefilterPlan {
    /// Rejects any plan that lacks ANN index use or predicate pushdown.
    pub fn require_ann_prefilter(&self) -> anyhow::Result<()> {
        ensure!(
            self.ann_index_used,
            "plan does not execute through the ANN index"
        );
        ensure!(
            self.series_predicate_below_ann,
            "plan does not place the series predicate below the ANN nearest-neighbor execution"
        );
        ensure!(
            self.eligible_cardinality > 0,
            "plan reports no eligible rows for the series predicate"
        );
        Ok(())
    }
}

/// A LanceDB-backed ANN index over one corpus generation.
pub struct GenerationIndex {
    root: PathBuf,
    identity: ModelIdentity,
    generation: String,
    connection: lancedb::Connection,
    table: Table,
}

impl GenerationIndex {
    /// Creates (or reopens) the index table for one generation.
    pub async fn create(
        root: &Path,
        identity: ModelIdentity,
        generation: &str,
    ) -> anyhow::Result<Self> {
        validate_series_slug(generation)?;
        let connection = connect(root.to_str().context("index root must be valid UTF-8")?)
            .execute()
            .await
            .context("could not open the lancedb store")?;
        let table_name = table_name_for(generation);
        let schema = row_schema();
        let table = connection
            .create_empty_table(table_name.clone(), schema)
            .execute()
            .await
            .with_context(|| format!("could not create table {table_name}"))?;
        Ok(Self {
            root: root.to_path_buf(),
            identity,
            generation: generation.to_string(),
            connection,
            table,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn identity(&self) -> &ModelIdentity {
        &self.identity
    }

    pub fn active_generation(&self) -> &str {
        &self.generation
    }

    /// Rebinds the handle onto the table of another, already created
    /// generation.
    pub async fn activate(&mut self, generation: &str) -> anyhow::Result<()> {
        validate_series_slug(generation)?;
        let table_name = table_name_for(generation);
        let table = self
            .connection
            .open_table(&table_name)
            .execute()
            .await
            .with_context(|| format!("could not activate generation {generation}"))?;
        self.generation = generation.to_string();
        self.table = table;
        Ok(())
    }

    /// Appends rows to the active generation.
    pub async fn append(&mut self, rows: Vec<IndexRow>) -> anyhow::Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        for row in &rows {
            row.validate(&self.identity, &self.generation)?;
        }
        let batch = rows_to_batch(&rows)?;
        self.table
            .add(batch)
            .execute()
            .await
            .context("could not append rows to the generation index")?;
        Ok(rows.len())
    }

    /// Builds the cosine IVF-PQ ANN index over the vector column and waits
    /// until index statistics report every row of the generation as indexed.
    pub async fn create_ann_index(&mut self) -> anyhow::Result<IndexEvidence> {
        let rows = self
            .table
            .count_rows(None)
            .await
            .context("could not count rows")?;
        ensure!(
            rows >= ANN_NUM_PARTITIONS as usize,
            "an ANN index needs at least {ANN_NUM_PARTITIONS} rows, found {rows}"
        );
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
            .context("could not create the ANN index")?;

        let deadline = Instant::now() + ANN_INDEX_WAIT_TIMEOUT;
        loop {
            if let Some(evidence) = self.current_evidence().await? {
                if evidence.num_unindexed_rows == 0 && evidence.num_indexed_rows == rows {
                    return Ok(evidence);
                }
            }
            ensure!(
                Instant::now() < deadline,
                "timed out waiting for the ANN index to cover all {rows} rows"
            );
            tokio::time::sleep(ANN_INDEX_WAIT_TICK).await;
        }
    }

    /// Reports normalized index statistics; rejects inconsistent coverage.
    pub async fn verify(&self) -> anyhow::Result<IndexEvidence> {
        let rows = self
            .table
            .count_rows(None)
            .await
            .context("could not count rows")?;
        match self.current_evidence().await? {
            Some(evidence) => {
                ensure!(
                    evidence.num_unindexed_rows == 0,
                    "index does not cover every row: {} unindexed",
                    evidence.num_unindexed_rows
                );
                ensure!(
                    evidence.num_indexed_rows == rows,
                    "index covers {} rows but the generation holds {rows}",
                    evidence.num_indexed_rows
                );
                Ok(evidence)
            }
            None => Ok(IndexEvidence {
                generation_id: self.generation.clone(),
                index_type: "NONE".to_string(),
                index_name: ANN_INDEX_NAME.to_string(),
                distance_metric: "none".to_string(),
                num_partitions: 0,
                num_indexed_rows: 0,
                num_unindexed_rows: 0,
                identity: self.identity.clone(),
            }),
        }
    }

    /// Pre-filtered ANN search: the `series_slug` predicate is part of the ANN
    /// query, never a post-filter or an over-fetch.
    pub async fn search(
        &self,
        series_slug: &str,
        vector: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        ensure!(
            vector.len() == self.identity.dimensions(),
            "query vector width {} does not match the indexed width {}",
            vector.len(),
            self.identity.dimensions()
        );
        let predicate = series_predicate(series_slug)?;
        let stream = self
            .table
            .query()
            .nearest_to(vector.to_vec())?
            .column(VECTOR_COLUMN)
            .only_if(predicate)
            .nprobes(ANN_NUM_PROBES)
            .refine_factor(ANN_REFINE_FACTOR)
            .limit(limit)
            .execute()
            .await
            .context("pre-filtered ANN search failed")?;
        decode_hits(stream.try_collect::<Vec<RecordBatch>>().await?)
    }

    /// The adversarial control: identical search but with the predicate
    /// applied after a global top-k, which the decoy population starves.
    pub async fn search_postfiltered(
        &self,
        series_slug: &str,
        vector: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        ensure!(
            vector.len() == self.identity.dimensions(),
            "query vector width {} does not match the indexed width {}",
            vector.len(),
            self.identity.dimensions()
        );
        let predicate = series_predicate(series_slug)?;
        let stream = self
            .table
            .query()
            .nearest_to(vector.to_vec())?
            .column(VECTOR_COLUMN)
            .only_if(predicate)
            .postfilter()
            .nprobes(ANN_NUM_PROBES)
            .limit(limit)
            .execute()
            .await
            .context("post-filtered search failed")?;
        decode_hits(stream.try_collect::<Vec<RecordBatch>>().await?)
    }

    /// Deletes one chunk from the active generation, returning the number of
    /// removed rows.
    pub async fn delete_chunk(&mut self, series_slug: &str, chunk_id: i64) -> anyhow::Result<u64> {
        let predicate = format!(
            "{} AND chunk_id = {chunk_id}",
            series_predicate(series_slug)?
        );
        let result = self
            .table
            .delete(predicate.as_str())
            .await
            .context("could not delete the chunk")?;
        Ok(result.num_deleted_rows)
    }

    /// Collects normalized explain/analyze evidence for the pre-filtered ANN
    /// search. The analyze plan is executed by the engine, so every
    /// cardinality in the evidence is measured, not assumed; the returned
    /// count is measured by executing the identical query.
    pub async fn explain_prefiltered_search(
        &self,
        series_slug: &str,
        vector: &[f32],
        limit: usize,
    ) -> anyhow::Result<PrefilterPlan> {
        ensure!(
            vector.len() == self.identity.dimensions(),
            "query vector width {} does not match the indexed width {}",
            vector.len(),
            self.identity.dimensions()
        );
        let predicate = series_predicate(series_slug)?;
        let explain_plan = self
            .prefiltered_query(&predicate, vector, limit)
            .explain_plan(true)
            .await
            .context("could not explain the pre-filtered ANN query")?;
        let analyze_plan = self
            .prefiltered_query(&predicate, vector, limit)
            .analyze_plan()
            .await
            .context("could not analyze the pre-filtered ANN query")?;
        let returned_batches = self
            .prefiltered_query(&predicate, vector, limit)
            .execute()
            .await
            .context("could not execute the analyzed pre-filtered ANN query")?;
        let returned_count =
            decode_hits(returned_batches.try_collect::<Vec<RecordBatch>>().await?)?.len();

        let plan = build_prefilter_plan(
            explain_plan,
            analyze_plan,
            series_slug,
            limit,
            returned_count,
            self.current_evidence().await?.as_ref(),
        );
        Ok(plan)
    }

    fn prefiltered_query(
        &self,
        predicate: &str,
        vector: &[f32],
        limit: usize,
    ) -> lancedb::query::VectorQuery {
        self.table
            .query()
            .nearest_to(vector.to_vec())
            .expect("query vector was validated against the indexed width")
            .column(VECTOR_COLUMN)
            .only_if(predicate.to_string())
            .nprobes(ANN_NUM_PROBES)
            .refine_factor(ANN_REFINE_FACTOR)
            .limit(limit)
    }

    async fn current_evidence(&self) -> anyhow::Result<Option<IndexEvidence>> {
        let Some(stats) = self
            .table
            .index_stats(ANN_INDEX_NAME)
            .await
            .context("could not read index statistics")?
        else {
            return Ok(None);
        };
        Ok(Some(IndexEvidence {
            generation_id: self.generation.clone(),
            index_type: stats.index_type.to_string(),
            index_name: ANN_INDEX_NAME.to_string(),
            distance_metric: stats
                .distance_type
                .map(|metric| metric.to_string())
                .unwrap_or_else(|| "none".to_string()),
            num_partitions: ANN_NUM_PARTITIONS as usize,
            num_indexed_rows: stats.num_indexed_rows,
            num_unindexed_rows: stats.num_unindexed_rows,
            identity: self.identity.clone(),
        }))
    }
}

fn build_prefilter_plan(
    explain_plan: String,
    analyze_plan: String,
    series_slug: &str,
    limit: usize,
    returned_count: usize,
    evidence: Option<&IndexEvidence>,
) -> PrefilterPlan {
    let normalized_explain = normalize_plan(&explain_plan);
    let normalized_analyze = normalize_plan(&analyze_plan);

    let ann_position = normalized_explain.find(ANN_OPERATOR);
    let ann_line = ann_position.and_then(|position| {
        normalized_explain[position..]
            .lines()
            .next()
            .map(str::to_string)
    });

    let operators = normalized_explain
        .lines()
        .filter_map(operator_of_line)
        .collect::<Vec<_>>();

    let index_name = ann_line.as_deref().and_then(attribute_of_line("name"));
    let distance_metric = ann_line
        .as_deref()
        .and_then(attribute_of_line("metric"))
        .or_else(|| {
            evidence
                .map(|evidence| evidence.distance_metric.clone())
                .filter(|metric| metric != "none")
        });

    // The plan tree is printed root first with children below, so the ANN
    // operator's prefilter source (the scan carrying the series predicate)
    // appears on a later line than the ANN operator itself. A post-filtered
    // predicate would be printed above it, which the proof rejects.
    let ann_index_used = ann_position.is_some()
        && evidence
            .map(|evidence| evidence.index_type == ANN_INDEX_TYPE)
            .unwrap_or(false);
    let series_predicate_below_ann = match (
        ann_position,
        predicate_position(&normalized_explain, series_slug),
    ) {
        (Some(ann_position), Some(predicate_position)) => predicate_position > ann_position,
        _ => false,
    };

    let eligible_cardinality = predicate_output_rows(&normalized_analyze, series_slug).unwrap_or(0);

    PrefilterPlan {
        ann_index_used,
        series_predicate_below_ann,
        eligible_cardinality,
        requested_top_k: limit,
        returned_count,
        index_name,
        distance_metric,
        predicate: normalized_predicate(&format!("series_slug = '{series_slug}'")),
        operators,
        explain_plan: normalized_explain,
        analyze_plan: normalized_analyze,
    }
}

/// Physical plan operator emitted for indexed ANN nearest-neighbor execution.
const ANN_OPERATOR: &str = "annsubindex";
/// String form of `lancedb::index::IndexType::IvfPq`.
const ANN_INDEX_TYPE: &str = "IVF_PQ";

fn normalize_plan(plan: &str) -> String {
    let mut normalized = String::with_capacity(plan.len());
    for line in plan.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut collapsed = String::with_capacity(line.len());
        let mut previous_was_space = false;
        for character in line.chars() {
            if character.is_whitespace() {
                if !previous_was_space {
                    collapsed.push(' ');
                }
                previous_was_space = true;
            } else if character != '"' {
                collapsed.push(character);
                previous_was_space = false;
            }
        }
        normalized.push_str(&collapsed.to_lowercase());
        normalized.push('\n');
    }
    normalized
}

/// DataFusion renders string literals as `utf8("eligible")` in plan output;
/// after normalization the quotes are gone and the marker below matches.
fn normalized_predicate(predicate: &str) -> String {
    predicate
        .chars()
        .filter(|character| *character != '\'')
        .collect::<String>()
        .to_lowercase()
}

fn is_predicate_line(line: &str, series_slug: &str) -> bool {
    line.contains("series_slug =") && line.contains(series_slug)
}

fn predicate_position(plan: &str, series_slug: &str) -> Option<usize> {
    let mut offset = 0;
    for line in plan.lines() {
        let end = offset + line.len();
        if is_predicate_line(line, series_slug) {
            return Some(offset);
        }
        offset = end + 1;
    }
    None
}

fn operator_of_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let name = line.split([':', ' ']).next()?;
    if name.is_empty() || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(name.to_string())
}

fn attribute_of_line(attribute: &'static str) -> impl Fn(&str) -> Option<String> {
    move |line: &str| {
        let marker = format!("{attribute}=");
        let start = line.find(&marker)? + marker.len();
        let rest = &line[start..];
        let end = rest.find(',').unwrap_or(rest.len());
        let value = rest[..end].trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    }
}

/// The prefilter scan's measured output rows: the number of rows that pass
/// the series predicate inside the ANN subtree.
fn predicate_output_rows(analyze_plan: &str, series_slug: &str) -> Option<usize> {
    analyze_plan
        .lines()
        .filter(|line| is_predicate_line(line, series_slug))
        .filter_map(|line| metric_of_line(line, "output_rows"))
        .max()
}

fn metric_of_line(line: &str, metric: &str) -> Option<usize> {
    let marker = format!("{metric}=");
    let start = line.find(&marker)? + marker.len();
    let rest = &line[start..];
    let end = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn validate_series_slug(slug: &str) -> anyhow::Result<()> {
    ensure!(!slug.is_empty(), "series slug must not be empty");
    ensure!(
        slug.chars().all(|character| character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'),
        "series slug {slug:?} is not a normalized slug"
    );
    Ok(())
}

fn series_predicate(series_slug: &str) -> anyhow::Result<String> {
    validate_series_slug(series_slug)?;
    Ok(format!("series_slug = '{series_slug}'"))
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

fn row_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("chunk_id", DataType::Int64, false),
        Field::new("series_slug", DataType::Utf8, false),
        Field::new("checksum", DataType::Utf8, false),
        Field::new("generation_id", DataType::Utf8, false),
        Field::new(
            VECTOR_COLUMN,
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_DIMENSIONS as i32,
            ),
            false,
        ),
    ]))
}

fn rows_to_batch(rows: &[IndexRow]) -> anyhow::Result<RecordBatch> {
    let chunk_id = Int64Array::from_iter(rows.iter().map(|row| Some(row.chunk_id)));
    let series_slug =
        StringArray::from_iter_values(rows.iter().map(|row| row.series_slug.as_str()));
    let checksum = StringArray::from_iter_values(rows.iter().map(|row| row.checksum.as_str()));
    let generation_id =
        StringArray::from_iter_values(rows.iter().map(|row| row.generation_id.as_str()));
    let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        rows.iter().map(|row| {
            Some(
                row.vector
                    .iter()
                    .map(|value| Some(*value))
                    .collect::<Vec<_>>(),
            )
        }),
        EMBEDDING_DIMENSIONS as i32,
    );
    RecordBatch::try_new(
        row_schema(),
        vec![
            Arc::new(chunk_id),
            Arc::new(series_slug),
            Arc::new(checksum),
            Arc::new(generation_id),
            Arc::new(vectors),
        ],
    )
    .context("could not build the index row batch")
}

fn decode_hits(batches: Vec<RecordBatch>) -> anyhow::Result<Vec<SearchHit>> {
    let mut hits = Vec::new();
    for batch in batches {
        let chunk_ids = batch
            .column_by_name("chunk_id")
            .context("result batch is missing chunk_id")?
            .as_any()
            .downcast_ref::<Int64Array>()
            .context("chunk_id column has the wrong type")?;
        let series = batch
            .column_by_name("series_slug")
            .context("result batch is missing series_slug")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("series_slug column has the wrong type")?;
        let checksums = batch
            .column_by_name("checksum")
            .context("result batch is missing checksum")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("checksum column has the wrong type")?;
        let distances = batch
            .column_by_name("_distance")
            .context("result batch is missing _distance")?
            .as_any()
            .downcast_ref::<Float32Array>()
            .context("_distance column has the wrong type")?;
        for index in 0..batch.num_rows() {
            hits.push(SearchHit {
                chunk_id: chunk_ids.value(index),
                series_slug: series.value(index).to_string(),
                checksum: checksums.value(index).to_string(),
                distance: distances.value(index),
            });
        }
    }
    Ok(hits)
}

/// Stable digest over the exact inputs an index generation would store.
pub fn index_input_digest(rows: &[IndexRow]) -> String {
    let mut digest = Sha256::new();
    let bytes = serde_json::to_vec(rows).expect("serializing in-memory index rows cannot fail");
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}
