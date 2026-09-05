//! Semantic RAG store facade: generation manifests in the authoritative
//! SQLite database, the generation lifecycle (build, validate, activate,
//! cancel, collect), and model acquisition/loading entry points.
//!
//! Discipline (design acceptance):
//! - SQLite owns everything authoritative; semantic state is derived and
//!   rebuildable. Losing the complete LanceDB directory is a rebuild, never
//!   data loss.
//! - No SQLite write transaction ever spans inference or LanceDB I/O: batches
//!   embed and append first, then a short transaction records the receipt;
//!   activation validates everything first and then performs exactly one
//!   metadata transaction.
//! - The store never downloads a model or opens the vector store on its own:
//!   `acquire_model` and the generation APIs are the only explicit semantic
//!   operations. With the model absent or invalid every semantic path fails
//!   closed and FTS retrieval remains fully functional.
//! - Durable jobs, leases, cancellation, and crash recovery live in
//!   `semantic_jobs`, which drives this lifecycle as durable work; a direct
//!   `begin_generation` build here remains in-process and single-writer.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::semantic_embeddings::{EmbeddingIdentity, EmbeddingProvider, OnnxEmbeddingProvider};
use crate::semantic_error::SemanticError;
use crate::semantic_index::{
    IndexRow, VectorIndex, drop_generation_table, generation_table_exists, validate_slug,
};
use crate::semantic_model::{
    MODEL_BYTES, MODEL_NAME, MODEL_SHA256, ModelAcquisition, ModelStatus, sha256_file,
};

/// Sample queries executed during activation return at most this many hits.
const ACTIVATION_SAMPLE_LIMIT: usize = 10;
/// File name of the promoted model under the model directory.
const MODEL_FILE_NAME: &str = "model.onnx";

/// Ensures the derived semantic schema (generation manifests) exists. The
/// durable-jobs module extends the database with its own table on top of this.
pub(crate) fn ensure_semantic_schema(connection: &Connection) -> Result<(), SemanticError> {
    connection.execute_batch(SEMANTIC_SCHEMA_SQL)?;
    Ok(())
}

const SEMANTIC_SCHEMA_SQL: &str = "
create table if not exists semantic_generations (
    generation_id text primary key,
    status text not null check (status in ('building','active','superseded','cancelled','failed')),
    provider text not null,
    model text not null,
    model_revision text not null,
    dimensions integer not null,
    normalization text not null,
    max_input_tokens integer not null,
    max_batch_inputs integer not null,
    expected_count integer not null,
    written_count integer not null default 0,
    last_chunk_id integer not null default 0,
    active integer not null default 0 check (active in (0,1)),
    created_at text not null,
    updated_at text not null
);
create unique index if not exists one_active_semantic_generation
    on semantic_generations(active) where active = 1;
";

/// A generation manifest row: expected versus written counts plus validation
/// state. The authoritative chunks stay in `rag_chunks`; this table is the
/// derived lifecycle bookkeeping.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationManifest {
    pub generation_id: String,
    pub status: String,
    pub identity: EmbeddingIdentity,
    pub expected_count: u64,
    pub written_count: u64,
    pub last_chunk_id: i64,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// One chunk prepared for embedding: the authoritative chunk id plus its
/// token stream. The store itself resolves the chunk's series and checksum
/// from SQLite, so a caller cannot inject foreign identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticChunk {
    pub chunk_id: i64,
    pub series_slug: String,
    pub token_ids: Vec<u32>,
}

/// The sample query activation executes against the candidate generation:
/// one series plus its token stream. Document and query embeddings must
/// resolve to the provider's shared identity or activation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSample {
    pub series_slug: String,
    pub token_ids: Vec<u32>,
}

/// Semantic RAG store over the data root.
pub struct SemanticStore {
    config: HieronymusConfig,
}

impl SemanticStore {
    /// Opens the store, ensuring the derived manifest schema exists. This
    /// never downloads a model and never opens the vector store.
    pub fn open(config: &HieronymusConfig) -> Result<Self, SemanticError> {
        let connection = open_migrated(&config.database_path())?;
        ensure_semantic_schema(&connection)?;
        Ok(Self {
            config: config.clone(),
        })
    }

    /// Root of the LanceDB database holding one table per generation.
    pub fn index_root(&self) -> PathBuf {
        self.config.semantic_root().join("lancedb")
    }

    /// Path of the acquired embedding model file.
    pub fn model_path(&self) -> PathBuf {
        self.config
            .semantic_root()
            .join("models")
            .join(MODEL_NAME)
            .join(MODEL_FILE_NAME)
    }

    /// Cheap availability verdict over the local model file (presence and
    /// pinned size; cryptographic verification happens at provider load).
    pub fn model_status(&self) -> ModelStatus {
        let path = self.model_path();
        match std::fs::metadata(&path) {
            Err(_) => ModelStatus::Missing,
            Ok(metadata) => {
                if metadata.is_dir() {
                    ModelStatus::Invalid(format!(
                        "{} is a directory, not the pinned model file",
                        path.display()
                    ))
                } else if metadata.len() != MODEL_BYTES {
                    ModelStatus::Invalid(format!(
                        "model file is {} bytes, expected {MODEL_BYTES}",
                        metadata.len()
                    ))
                } else {
                    ModelStatus::Available
                }
            }
        }
    }

    /// Acquires the pinned model: streaming download into a temporary file,
    /// SHA-256 verification against the pinned digest, then an atomic
    /// promotion into the data root. Explicitly invoked only; nothing else in
    /// this store touches the network. Re-acquiring replaces the file
    /// atomically.
    pub fn acquire_model(
        &self,
        transport: &dyn crate::semantic_model::ModelTransport,
        url: &str,
    ) -> Result<ModelAcquisition, SemanticError> {
        self.acquire_model_verifying(transport, url, MODEL_SHA256, MODEL_BYTES)
    }

    /// Acquisition against explicitly given expectations. The pinned model
    /// uses [`SemanticStore::acquire_model`]; other verified artifacts (such
    /// as the ONNX runtime archive) can use this entry point.
    pub fn acquire_model_verifying(
        &self,
        transport: &dyn crate::semantic_model::ModelTransport,
        url: &str,
        expected_sha256: &str,
        expected_bytes: u64,
    ) -> Result<ModelAcquisition, SemanticError> {
        let destination = self.model_path();
        let model_dir = destination
            .parent()
            .ok_or_else(|| SemanticError::Store("model path has no parent".to_string()))?
            .to_path_buf();
        std::fs::create_dir_all(&model_dir)?;
        let temporary = tempfile::Builder::new()
            .prefix(format!(".{MODEL_FILE_NAME}.").as_str())
            .suffix(".tmp")
            .rand_bytes(8)
            .tempfile_in(&model_dir)?;
        let temporary_path = temporary.path().to_path_buf();
        let written = transport.download_to(url, temporary.path(), expected_bytes)?;
        if written != expected_bytes {
            return Err(SemanticError::ValidationFailed(format!(
                "model download is {written} bytes, expected {expected_bytes}"
            )));
        }
        let checksum = sha256_file(temporary.path())?;
        if checksum != expected_sha256.trim().to_ascii_lowercase() {
            // The temporary file is dropped (and removed) on every return.
            return Err(SemanticError::ChecksumMismatch {
                expected: expected_sha256.to_string(),
                actual: checksum,
            });
        }
        temporary.as_file().sync_all()?;
        temporary
            .persist(&destination)
            .map_err(|error| SemanticError::Promotion {
                from: temporary_path,
                to: destination.clone(),
                message: error.to_string(),
            })?;
        Ok(ModelAcquisition {
            path: destination,
            bytes: written,
            checksum,
        })
    }

    /// Loads the real ONNX provider from the acquired model. Fails closed
    /// with [`SemanticError::ModelUnavailable`] while the model is missing or
    /// fails its cheap pre-check, leaving FTS retrieval fully functional.
    pub fn load_embedding_provider(
        &self,
        runtime_library: &Path,
    ) -> Result<OnnxEmbeddingProvider, SemanticError> {
        match self.model_status() {
            ModelStatus::Available => {}
            ModelStatus::Missing => {
                return Err(SemanticError::ModelUnavailable(
                    "no embedding model has been acquired; run an explicit model acquisition first"
                        .to_string(),
                ));
            }
            ModelStatus::Invalid(reason) => {
                return Err(SemanticError::ModelUnavailable(format!(
                    "the local embedding model failed its pre-check: {reason}"
                )));
            }
        }
        OnnxEmbeddingProvider::load(runtime_library, &self.model_path())
    }

    /// Registers a new candidate generation. The manifest freezes the model
    /// identity and the current authoritative chunk count; any existing row
    /// for the id is rejected (this slice has no resume: abandon with
    /// `cancel_generation` and start a new generation).
    pub fn begin_generation(
        &self,
        generation_id: &str,
        identity: &EmbeddingIdentity,
    ) -> Result<GenerationManifest, SemanticError> {
        validate_slug(generation_id)?;
        let mut connection = self.connection()?;
        let expected_count: i64 =
            connection.query_row("select count(*) from rag_chunks", [], |row| row.get(0))?;
        let now = now_iso8601();
        let transaction = connection.transaction()?;
        let existing: i64 = transaction.query_row(
            "select count(*) from semantic_generations where generation_id = ?1",
            params![generation_id],
            |row| row.get(0),
        )?;
        if existing > 0 {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} already has a manifest"
            )));
        }
        transaction.execute(
            "insert into semantic_generations(
               generation_id, status, provider, model, model_revision, dimensions,
               normalization, max_input_tokens, max_batch_inputs,
               expected_count, written_count, last_chunk_id, active, created_at, updated_at
             )
             values (?1, 'building', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, 0, ?10, ?10)",
            params![
                generation_id,
                identity.provider(),
                identity.model(),
                identity.revision(),
                identity.dimensions() as i64,
                identity.normalization(),
                identity.max_input_tokens() as i64,
                identity.max_batch_inputs() as i64,
                expected_count,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(GenerationManifest {
            generation_id: generation_id.to_string(),
            status: "building".to_string(),
            identity: identity.clone(),
            expected_count: expected_count as u64,
            written_count: 0,
            last_chunk_id: 0,
            active: false,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// The next bounded window of authoritative chunk ids the generation
    /// still needs, in id order.
    pub fn pending_chunk_ids(
        &self,
        generation_id: &str,
        limit: usize,
    ) -> Result<Vec<i64>, SemanticError> {
        if limit == 0 {
            return Err(SemanticError::ValidationFailed(
                "pending window limit must be at least 1".to_string(),
            ));
        }
        let manifest = self
            .generation_manifest(generation_id)?
            .ok_or_else(|| SemanticError::NotFound(format!("generation {generation_id}")))?;
        if manifest.status != "building" {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} is {} and cannot take writes",
                manifest.status
            )));
        }
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select id from rag_chunks
             where id > ?1
             order by id
             limit ?2",
        )?;
        let rows = statement.query_map(params![manifest.last_chunk_id, limit as i64], |row| {
            row.get::<_, i64>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(SemanticError::from)
    }

    /// The series slug of one authoritative chunk (callers pass it back with
    /// the chunk's token stream).
    pub fn chunk_series_slug(&self, chunk_id: i64) -> Result<Option<String>, SemanticError> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("select series_slug from rag_chunks where id = ?1")?;
        let mut rows = statement.query(params![chunk_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Embeds and appends one bounded batch, then records the receipt.
    ///
    /// Native I/O (inference, LanceDB append) happens BEFORE the short
    /// manifest transaction — no SQLite write transaction ever spans native
    /// I/O. The store resolves each chunk's series and checksum from the
    /// authoritative rows, so stale or foreign identifiers are rejected.
    pub fn write_batch(
        &self,
        generation_id: &str,
        provider: &mut dyn EmbeddingProvider,
        batch: &[SemanticChunk],
    ) -> Result<usize, SemanticError> {
        if batch.is_empty() {
            return Err(SemanticError::ValidationFailed(
                "write batches must not be empty".to_string(),
            ));
        }
        let manifest = self
            .generation_manifest(generation_id)?
            .ok_or_else(|| SemanticError::NotFound(format!("generation {generation_id}")))?;
        if manifest.status != "building" {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} is {} and cannot take writes",
                manifest.status
            )));
        }
        ensure_same_identity(&manifest.identity, provider.identity())?;
        if batch.len() > manifest.identity.max_batch_inputs() {
            return Err(SemanticError::InvalidEmbedding(format!(
                "batch of {} exceeds the provider limit of {} inputs",
                batch.len(),
                manifest.identity.max_batch_inputs()
            )));
        }

        // Native section: inference and LanceDB append, no open transaction.
        let mut index =
            VectorIndex::open(&self.index_root(), manifest.identity.clone(), generation_id)?;
        let mut rows = Vec::with_capacity(batch.len());
        let mut highest_chunk_id = manifest.last_chunk_id;
        for chunk in batch {
            if chunk.chunk_id <= manifest.last_chunk_id {
                return Err(SemanticError::ValidationFailed(format!(
                    "chunk {} sits behind the generation cursor {}",
                    chunk.chunk_id, manifest.last_chunk_id
                )));
            }
            let (series_slug, text) = self
                .chunk_row(chunk.chunk_id)?
                .ok_or_else(|| SemanticError::NotFound(format!("rag chunk {}", chunk.chunk_id)))?;
            if series_slug != chunk.series_slug {
                return Err(SemanticError::ValidationFailed(format!(
                    "chunk {} belongs to series {series_slug}, not {}",
                    chunk.chunk_id, chunk.series_slug
                )));
            }
            let checksum = sha256_text(&text);
            let vector = provider.embed_document(&chunk.token_ids)?;
            rows.push(IndexRow {
                chunk_id: chunk.chunk_id,
                series_slug,
                checksum,
                generation_id: generation_id.to_string(),
                model: manifest.identity.model().to_string(),
                model_revision: manifest.identity.revision().to_string(),
                vector,
            });
            highest_chunk_id = highest_chunk_id.max(chunk.chunk_id);
        }
        let appended = index.append(rows)?;
        drop(index);

        // Short receipt transaction, strictly after all native I/O.
        let connection = self.connection()?;
        let updated = connection.execute(
            "update semantic_generations
             set written_count = written_count + ?1,
                 last_chunk_id = max(last_chunk_id, ?2),
                 updated_at = ?3
             where generation_id = ?4
               and status = 'building'",
            params![
                appended as i64,
                highest_chunk_id,
                now_iso8601(),
                generation_id,
            ],
        )?;
        if updated == 0 {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} left the building state while its batch was in flight"
            )));
        }
        Ok(appended)
    }

    /// Validates the candidate generation (count, checksum, dimension, and
    /// sample-query checks against the authoritative rows and the previous
    /// active generation keeps serving throughout), then activates it in
    /// exactly one metadata transaction.
    pub fn activate_generation(
        &self,
        generation_id: &str,
        provider: &mut dyn EmbeddingProvider,
        sample: &SemanticSample,
    ) -> Result<(), SemanticError> {
        let manifest = self
            .generation_manifest(generation_id)?
            .ok_or_else(|| SemanticError::NotFound(format!("generation {generation_id}")))?;
        if manifest.status != "building" {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} is {} and cannot be activated",
                manifest.status
            )));
        }
        ensure_same_identity(&manifest.identity, provider.identity())?;
        validate_slug(&sample.series_slug)?;
        manifest.identity.check_tokens(sample.token_ids.len())?;

        // Validation section: native reads only, no write transaction open.
        let index =
            VectorIndex::open(&self.index_root(), manifest.identity.clone(), generation_id)?;
        let stored_rows = index.count_rows()?;
        if stored_rows != manifest.written_count as usize
            || stored_rows != manifest.expected_count as usize
        {
            return Err(SemanticError::ValidationFailed(format!(
                "generation {generation_id} count mismatch: index holds {stored_rows} rows, \
                 manifest counts {} written / {} expected",
                manifest.written_count, manifest.expected_count
            )));
        }
        self.verify_checksums(&index, &manifest)?;
        let width = index.stored_vector_width()?;
        match width {
            Some(width) if width == manifest.identity.dimensions() => {}
            Some(width) => {
                return Err(SemanticError::ValidationFailed(format!(
                    "generation {generation_id} stores {width}-wide vectors, expected {}",
                    manifest.identity.dimensions()
                )));
            }
            None => {
                return Err(SemanticError::ValidationFailed(format!(
                    "generation {generation_id} holds no vectors to activate"
                )));
            }
        }
        let query_vector = provider.embed_query(&sample.token_ids)?;
        let hits = index.search(&sample.series_slug, &query_vector, ACTIVATION_SAMPLE_LIMIT)?;
        if hits.is_empty() {
            return Err(SemanticError::ValidationFailed(format!(
                "generation {generation_id} sample query returned no rows for series {}",
                sample.series_slug
            )));
        }
        if let Some(leaked) = hits
            .iter()
            .find(|hit| hit.series_slug != sample.series_slug)
        {
            return Err(SemanticError::ValidationFailed(format!(
                "generation {generation_id} sample query leaked chunk {} from series {}",
                leaked.chunk_id, leaked.series_slug
            )));
        }
        drop(index);

        // Activation is exactly one SQLite transaction. The partial unique
        // index admits only one active row, so the previous active generation
        // is superseded BEFORE the candidate is promoted; on any failure the
        // transaction is dropped and rolls back to the previous state.
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "update semantic_generations
             set status = 'superseded', active = 0, updated_at = ?2
             where active = 1
               and generation_id <> ?1",
            params![generation_id, now],
        )?;
        let claimed = transaction.execute(
            "update semantic_generations
             set status = 'active', active = 1, updated_at = ?2
             where generation_id = ?1
               and status = 'building'",
            params![generation_id, now],
        )?;
        if claimed == 0 {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} left the building state before activation"
            )));
        }
        transaction.commit()?;
        Ok(())
    }

    /// Reconciles the generation's stored fingerprints against the current
    /// authoritative chunk rows behind its cursor. Any re-imported, replaced,
    /// or deleted chunk makes the stored checksums stale and rejects
    /// activation.
    fn verify_checksums(
        &self,
        index: &VectorIndex,
        manifest: &GenerationManifest,
    ) -> Result<(), SemanticError> {
        let expected = self.chunk_checksums_behind_cursor(manifest.last_chunk_id)?;
        let stored = index.snapshot_rows(manifest.expected_count as usize)?;
        if stored.len() != expected.len() {
            return Err(SemanticError::ValidationFailed(format!(
                "generation {} checksum mismatch: index holds {} rows behind cursor {}, \
                 authoritative rows count {}",
                manifest.generation_id,
                stored.len(),
                manifest.last_chunk_id,
                expected.len()
            )));
        }
        let stale = stored.iter().find(|row| {
            expected
                .get(&row.chunk_id)
                .map(|checksum| checksum != &row.checksum)
                .unwrap_or(true)
        });
        match stale {
            None => {
                let foreign = stored
                    .iter()
                    .find(|row| row.generation_id != manifest.generation_id);
                match foreign {
                    None => Ok(()),
                    Some(row) => Err(SemanticError::ValidationFailed(format!(
                        "generation {} stores chunk {} from generation {}",
                        manifest.generation_id, row.chunk_id, row.generation_id
                    ))),
                }
            }
            Some(row) => {
                let current = expected
                    .get(&row.chunk_id)
                    .map(String::as_str)
                    .unwrap_or("<no longer present>");
                Err(SemanticError::ValidationFailed(format!(
                    "generation {} checksum mismatch for chunk {}: stored {}, authoritative {current}",
                    manifest.generation_id, row.chunk_id, row.checksum
                )))
            }
        }
    }

    /// Vector-free fingerprints (chunk id to text checksum) of every
    /// authoritative row behind the cursor. Generation activation and job
    /// takeover reconcile the stored rows against these.
    pub fn chunk_checksums_behind_cursor(
        &self,
        last_chunk_id: i64,
    ) -> Result<std::collections::HashMap<i64, String>, SemanticError> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("select id, text from rag_chunks where id <= ?1 order by id")?;
        let rows = statement.query_map(params![last_chunk_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut checksums = std::collections::HashMap::new();
        for row in rows {
            let (id, text) = row?;
            checksums.insert(id, sha256_text(&text));
        }
        Ok(checksums)
    }

    /// Marks a building generation cancelled. The manifest keeps its counts
    /// until garbage collection removes it; it can never activate.
    pub fn cancel_generation(&self, generation_id: &str) -> Result<(), SemanticError> {
        let connection = self.connection()?;
        let updated = connection.execute(
            "update semantic_generations
             set status = 'cancelled', updated_at = ?2
             where generation_id = ?1
               and status = 'building'",
            params![generation_id, now_iso8601()],
        )?;
        if updated == 0 {
            return Err(SemanticError::InvalidState(format!(
                "generation {generation_id} is not building and cannot be cancelled"
            )));
        }
        Ok(())
    }

    /// Garbage collection: drops the LanceDB tables and manifest rows of
    /// terminal generations only (superseded, cancelled, failed). The active
    /// generation and any building generation are never collected — with no
    /// reader registry yet, terminal state is the only provably unreferenced
    /// condition.
    pub fn collect_garbage(&self) -> Result<Vec<String>, SemanticError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select generation_id from semantic_generations
             where status in ('superseded', 'cancelled', 'failed')
             order by generation_id",
        )?;
        let terminal: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut collected = Vec::with_capacity(terminal.len());
        for generation_id in terminal {
            // Native I/O first: only after the table is gone (or never
            // existed) does the manifest row disappear.
            drop_generation_table(&self.index_root(), &generation_id)?;
            let updated = connection.execute(
                "delete from semantic_generations where generation_id = ?1
                   and status in ('superseded', 'cancelled', 'failed')",
                params![generation_id],
            )?;
            if updated == 1 {
                collected.push(generation_id);
            }
        }
        Ok(collected)
    }

    /// The currently active generation, if any.
    pub fn active_generation(&self) -> Result<Option<GenerationManifest>, SemanticError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select generation_id, status, provider, model, model_revision, dimensions,
                    expected_count, written_count, last_chunk_id, active, created_at, updated_at,
                    normalization, max_input_tokens, max_batch_inputs
             from semantic_generations
             where active = 1",
        )?;
        let mut rows = statement.query([])?;
        match rows.next()? {
            Some(row) => Ok(Some(manifest_from_row(row)?)),
            None => Ok(None),
        }
    }

    /// The manifest of one generation, if present.
    pub fn generation_manifest(
        &self,
        generation_id: &str,
    ) -> Result<Option<GenerationManifest>, SemanticError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select generation_id, status, provider, model, model_revision, dimensions,
                    expected_count, written_count, last_chunk_id, active, created_at, updated_at,
                    normalization, max_input_tokens, max_batch_inputs
             from semantic_generations
             where generation_id = ?1",
        )?;
        let mut rows = statement.query(params![generation_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(manifest_from_row(row)?)),
            None => Ok(None),
        }
    }

    /// Whether the active generation's table survived on disk. Losing the
    /// complete LanceDB directory reports `false`; the recovery is a rebuild
    /// (the authoritative rows never left SQLite), never data loss.
    pub fn active_generation_intact(&self) -> Result<bool, SemanticError> {
        match self.active_generation()? {
            None => Ok(true),
            Some(active) => Ok(generation_table_exists(
                &self.index_root(),
                &active.generation_id,
            )),
        }
    }

    fn connection(&self) -> Result<Connection, SemanticError> {
        Ok(open_migrated(&self.config.database_path())?)
    }

    /// The series and text of one authoritative chunk: `(series_slug, text)`.
    /// Durable jobs hand the text to the tokenizer and carry the series back
    /// with the chunk's token stream.
    pub fn chunk_row(&self, chunk_id: i64) -> Result<Option<(String, String)>, SemanticError> {
        let connection = self.connection()?;
        let mut statement =
            connection.prepare("select series_slug, text from rag_chunks where id = ?1")?;
        let mut rows = statement.query(params![chunk_id])?;
        match rows.next()? {
            Some(row) => Ok(Some((row.get(0)?, row.get(1)?))),
            None => Ok(None),
        }
    }
}

fn ensure_same_identity(
    expected: &EmbeddingIdentity,
    actual: &EmbeddingIdentity,
) -> Result<(), SemanticError> {
    if expected == actual {
        return Ok(());
    }
    Err(SemanticError::IdentityMismatch {
        expected: format!(
            "{} {}@{} ({} dims)",
            expected.provider(),
            expected.model(),
            expected.revision(),
            expected.dimensions()
        ),
        actual: format!(
            "{} {}@{} ({} dims)",
            actual.provider(),
            actual.model(),
            actual.revision(),
            actual.dimensions()
        ),
    })
}

fn manifest_from_row(row: &rusqlite::Row<'_>) -> Result<GenerationManifest, SemanticError> {
    Ok(GenerationManifest {
        generation_id: row.get(0)?,
        status: row.get(1)?,
        identity: EmbeddingIdentity::new(
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)? as usize,
            row.get::<_, String>(12)?,
            row.get::<_, i64>(13)? as usize,
            row.get::<_, i64>(14)? as usize,
        )?,
        expected_count: row.get::<_, i64>(6)? as u64,
        written_count: row.get::<_, i64>(7)? as u64,
        last_chunk_id: row.get(8)?,
        active: row.get::<_, i64>(9)? == 1,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn sha256_text(text: &str) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}
