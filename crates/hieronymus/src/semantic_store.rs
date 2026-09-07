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

use rusqlite::{Connection, OpenFlags, params};

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::semantic_embeddings::{EmbeddingIdentity, EmbeddingProvider, OnnxEmbeddingProvider};
use crate::semantic_error::SemanticError;
use crate::semantic_index::{
    IndexRow, VectorIndex, drop_generation_table, generation_table_intact, validate_slug,
};
use crate::semantic_model::{
    MODEL_BYTES, MODEL_NAME, MODEL_SHA256, ModelAcquisition, ModelStatus, TOKENIZER_BYTES,
    TOKENIZER_FILE_NAME, TOKENIZER_SHA256, sha256_file,
};

/// Sample queries executed during activation return at most this many hits.
const ACTIVATION_SAMPLE_LIMIT: usize = 10;
/// How long the activation transaction waits for a competing writer (an
/// in-flight import) before giving up. Activation holds the write lock for two
/// small `update`s, so this only ever absorbs someone else's short commit.
const ACTIVATION_BUSY_TIMEOUT_MS: i64 = 5_000;
/// File name of the promoted model under the model directory.
const MODEL_FILE_NAME: &str = "model.onnx";

/// Ensures the derived semantic schema (generation manifests) exists. The
/// durable-jobs module extends the database with its own table on top of this.
/// `tokenizer` is added by an idempotent `alter table` for databases created
/// before the tokenizer joined the embedding identity (Task 9 review
/// follow-up): every manifest row records the tokenization its vectors were
/// built under.
///
/// `corpus_revision` (schema version 3) needs no such back-fill here: this
/// table is created lazily, so a v2 database that already had it is altered by
/// the 2 -> 3 step's typed converter
/// (`schema_upgrade::add_corpus_revision_column`) and one created afterwards
/// carries the column from the definition below. Both paths default it to `-1`
/// — the "begun before revisions were recorded" sentinel — so the two shapes
/// are indistinguishable to every reader.
pub(crate) fn ensure_semantic_schema(connection: &Connection) -> Result<(), SemanticError> {
    connection.execute_batch(SEMANTIC_SCHEMA_SQL)?;
    ensure_tokenizer_column(connection)?;
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
    tokenizer text not null default 'byte-fold-v1',
    max_input_tokens integer not null,
    max_batch_inputs integer not null,
    expected_count integer not null,
    corpus_revision integer not null default -1,
    written_count integer not null default 0,
    last_chunk_id integer not null default 0,
    active integer not null default 0 check (active in (0,1)),
    created_at text not null,
    updated_at text not null
);
create unique index if not exists one_active_semantic_generation
    on semantic_generations(active) where active = 1;
";

/// The `semantic_generations.corpus_revision` value of a generation that was
/// begun before corpus revisions existed (task C4). It compares as behind
/// every real revision — including revision 0, the value of a database that
/// has never recorded a text change — so such a generation always reads as
/// stale and is REBUILT. It is never relabelled with a revision it cannot be
/// shown to cover.
pub const UNKNOWN_CORPUS_REVISION: i64 = -1;

/// Adds the `tokenizer` column when it is missing (pre-identity databases).
/// The default backfills every existing manifest with the tokenizer this port
/// line has always used, so old identities stay comparable.
fn ensure_tokenizer_column(connection: &Connection) -> Result<(), SemanticError> {
    let has_column: bool = connection
        .prepare("pragma table_info(semantic_generations)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .any(|name| name.as_deref() == Ok("tokenizer"));
    if !has_column {
        connection.execute(
            &format!(
                "alter table semantic_generations add column tokenizer text not null default '{}'",
                crate::semantic_embeddings::BYTE_FOLD_TOKENIZER_ID
            ),
            [],
        )?;
    }
    Ok(())
}

/// A generation manifest row: expected versus written counts plus validation
/// state. The authoritative chunks stay in `rag_chunks`; this table is the
/// derived lifecycle bookkeeping.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationManifest {
    pub generation_id: String,
    pub status: String,
    pub identity: EmbeddingIdentity,
    /// The authoritative corpus revision this generation was begun at, or
    /// [`UNKNOWN_CORPUS_REVISION`] for a generation that predates revisions.
    /// A generation covers the corpus only when this equals the current
    /// revision — an equal chunk count proves nothing, because replacing a
    /// document with a same-length one changes every vector it owns.
    pub corpus_revision: i64,
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
        Self::model_path_for(&self.config)
    }

    /// The model path for a data root, without opening any store (report-only
    /// surfaces).
    pub fn model_path_for(config: &HieronymusConfig) -> PathBuf {
        if let Some(directory) = std::env::var_os("HIERO_SEMANTIC_MODEL_DIR") {
            return PathBuf::from(directory).join("model.onnx");
        }
        if let Some(root) = crate::semantic_arming::bundled_asset_root() {
            return root.join("models/minilm/model.onnx");
        }
        config
            .semantic_root()
            .join("models")
            .join(MODEL_NAME)
            .join(MODEL_FILE_NAME)
    }

    /// Cheap availability verdict over the local model file (presence and
    /// pinned size; cryptographic verification happens at provider load).
    pub fn model_status(&self) -> ModelStatus {
        Self::model_status_at(&self.model_path())
    }

    /// The same verdict for a data root, without opening any store.
    pub fn model_status_for(config: &HieronymusConfig) -> ModelStatus {
        Self::model_status_at(&Self::model_path_for(config))
    }

    fn model_status_at(path: &Path) -> ModelStatus {
        match std::fs::metadata(path) {
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
        self.acquire_verified_artifact(
            transport,
            url,
            &self.model_path(),
            MODEL_FILE_NAME,
            expected_sha256,
            expected_bytes,
        )
    }

    /// Path of the acquired pinned tokenizer asset.
    pub fn tokenizer_path(&self) -> PathBuf {
        Self::tokenizer_path_for(&self.config)
    }

    /// The tokenizer path for a data root, without opening any store.
    pub fn tokenizer_path_for(config: &HieronymusConfig) -> PathBuf {
        if let Some(directory) = std::env::var_os("HIERO_SEMANTIC_MODEL_DIR") {
            return PathBuf::from(directory).join("tokenizer.json");
        }
        if let Some(root) = crate::semantic_arming::bundled_asset_root() {
            return root.join("models/minilm/tokenizer.json");
        }
        config
            .semantic_root()
            .join("models")
            .join(MODEL_NAME)
            .join(TOKENIZER_FILE_NAME)
    }

    /// Cheap availability verdict over the local tokenizer asset (presence
    /// and pinned size; the SHA-256 is verified at load time).
    pub fn tokenizer_status(&self) -> ModelStatus {
        Self::artifact_status_at(&self.tokenizer_path(), TOKENIZER_BYTES, "tokenizer")
    }

    fn artifact_status_at(path: &Path, expected_bytes: u64, kind: &str) -> ModelStatus {
        match std::fs::metadata(path) {
            Err(_) => ModelStatus::Missing,
            Ok(metadata) => {
                if metadata.is_dir() {
                    ModelStatus::Invalid(format!(
                        "{} is a directory, not the pinned {kind} file",
                        path.display()
                    ))
                } else if metadata.len() != expected_bytes {
                    ModelStatus::Invalid(format!(
                        "{kind} file is {} bytes, expected {expected_bytes}",
                        metadata.len()
                    ))
                } else {
                    ModelStatus::Available
                }
            }
        }
    }

    /// Acquires the pinned tokenizer asset with the same discipline as the
    /// model: streaming download into a temporary file in the destination
    /// directory, SHA-256 verification against the pinned digest, then an
    /// atomic promotion. No override expectations: the tokenizer is part of
    /// the embedding identity and is never swapped for an unpinned asset.
    pub fn acquire_tokenizer(
        &self,
        transport: &dyn crate::semantic_model::ModelTransport,
        url: &str,
    ) -> Result<ModelAcquisition, SemanticError> {
        self.acquire_verified_artifact(
            transport,
            url,
            &self.tokenizer_path(),
            TOKENIZER_FILE_NAME,
            TOKENIZER_SHA256,
            TOKENIZER_BYTES,
        )
    }

    /// The shared verified acquisition flow: temp file next to the
    /// destination, checksum against the expectation, atomic promotion.
    fn acquire_verified_artifact(
        &self,
        transport: &dyn crate::semantic_model::ModelTransport,
        url: &str,
        destination: &Path,
        file_name: &str,
        expected_sha256: &str,
        expected_bytes: u64,
    ) -> Result<ModelAcquisition, SemanticError> {
        let model_dir = destination
            .parent()
            .ok_or_else(|| SemanticError::Store("artifact path has no parent".to_string()))?
            .to_path_buf();
        std::fs::create_dir_all(&model_dir)?;
        let temporary = tempfile::Builder::new()
            .prefix(format!(".{file_name}.").as_str())
            .suffix(".tmp")
            .rand_bytes(8)
            .tempfile_in(&model_dir)?;
        let temporary_path = temporary.path().to_path_buf();
        let written = transport.download_to(url, temporary.path(), expected_bytes)?;
        if written != expected_bytes {
            return Err(SemanticError::ValidationFailed(format!(
                "artifact download is {written} bytes, expected {expected_bytes}"
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
            .persist(destination)
            .map_err(|error| SemanticError::Promotion {
                from: temporary_path,
                to: destination.to_path_buf(),
                message: error.to_string(),
            })?;
        Ok(ModelAcquisition {
            path: destination.to_path_buf(),
            bytes: written,
            checksum,
        })
    }

    /// Loads the pinned [`ModelTokenizer`](crate::semantic_tokenizer::ModelTokenizer) from the acquired asset. Fails
    /// closed when the asset is missing, wrongly sized, or fails its SHA-256
    /// verification against the pinned digest.
    pub fn load_model_tokenizer(
        &self,
    ) -> Result<crate::semantic_tokenizer::ModelTokenizer, SemanticError> {
        let path = self.tokenizer_path();
        match self.tokenizer_status() {
            ModelStatus::Available => {}
            ModelStatus::Missing => {
                return Err(SemanticError::ModelUnavailable(
                    "no tokenizer asset has been acquired; run an explicit model acquisition first"
                        .to_string(),
                ));
            }
            ModelStatus::Invalid(reason) => {
                return Err(SemanticError::ModelUnavailable(format!(
                    "the local tokenizer asset failed its pre-check: {reason}"
                )));
            }
        }
        let bytes = std::fs::read(&path)?;
        let checksum = crate::semantic_model::sha256_file(&path)?;
        if checksum != TOKENIZER_SHA256 {
            return Err(SemanticError::ChecksumMismatch {
                expected: TOKENIZER_SHA256.to_string(),
                actual: checksum,
            });
        }
        crate::semantic_tokenizer::ModelTokenizer::from_bytes(&bytes)
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
    /// identity, the current authoritative chunk count, and the corpus
    /// revision it is being built against; any existing row for the id is
    /// rejected (this slice has no resume: abandon with `cancel_generation`
    /// and start a new generation).
    ///
    /// The revision is read inside the transaction that inserts it, so the
    /// recorded coverage claim is exactly the corpus state the insert saw.
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
        let corpus_revision = crate::rag::current_corpus_revision(&transaction)?;
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
               normalization, tokenizer, max_input_tokens, max_batch_inputs,
               expected_count, corpus_revision, written_count, last_chunk_id, active,
               created_at, updated_at
             )
             values (?1, 'building', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, 0, 0, ?12, ?12)",
            params![
                generation_id,
                identity.provider(),
                identity.model(),
                identity.revision(),
                identity.dimensions() as i64,
                identity.normalization(),
                identity.tokenizer(),
                identity.max_input_tokens() as i64,
                identity.max_batch_inputs() as i64,
                expected_count,
                corpus_revision,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(GenerationManifest {
            generation_id: generation_id.to_string(),
            status: "building".to_string(),
            identity: identity.clone(),
            corpus_revision,
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

    /// Validates the candidate generation (count, checksum, dimension,
    /// corpus-revision, and sample-query checks against the authoritative
    /// rows; the previous active generation keeps serving throughout), then
    /// activates it in exactly one metadata transaction.
    ///
    /// Both halves of the identity the candidate claims are checked against
    /// CURRENT authoritative state (task C4): the embedding identity against
    /// the provider that is about to answer queries, and the corpus revision
    /// against `corpus_revision.revision`. A candidate begun at revision N can
    /// never be activated once the corpus has moved to N+1 — its vectors
    /// describe text that is no longer what the store holds — and refusing
    /// here is what stops a rebuild that started before an import from
    /// publishing itself after it.
    ///
    /// The revision check deliberately runs AFTER the count/checksum/dimension
    /// checks rather than beside the identity check at the top. Those checks
    /// name the specific way a candidate went stale ("checksum mismatch", "count
    /// mismatch"), which is far more actionable than "the revision moved"; the
    /// revision check is the exact backstop for whatever fingerprint
    /// reconciliation cannot see — chunks added and removed in equal numbers,
    /// a replacement whose texts hash identically behind the cursor — not a
    /// replacement for them.
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

        // Fail closed before taking the write lock, so a doomed candidate is
        // refused without contending with the very import that doomed it. The
        // authoritative check is the one inside the transaction below; this is
        // the cheap one.
        ensure_covers_corpus_revision(&self.connection()?, &manifest)?;

        // Activation is exactly one SQLite transaction. The partial unique
        // index admits only one active row, so the previous active generation
        // is superseded BEFORE the candidate is promoted; on any failure the
        // transaction is dropped and rolls back to the previous state.
        //
        // `Immediate`: the corpus-revision re-check below has to be atomic
        // with the promotion it authorizes. A deferred transaction would take
        // its read snapshot first and only then contend for the write lock, so
        // an import committing in that window would either be missed or turn
        // into a `SQLITE_BUSY_SNAPSHOT` surprise. Holding the write lock from
        // the start means the revision this reads is the revision that is
        // still current when the candidate goes active.
        let now = now_iso8601();
        let mut connection = self.connection()?;
        connection.pragma_update(None, "busy_timeout", ACTIVATION_BUSY_TIMEOUT_MS)?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure_covers_corpus_revision(&transaction, &manifest)?;
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

    /// Drops the active generation out of the active slot because it can no
    /// longer serve: its embedding identity no longer matches the running
    /// provider, or its index did not survive on disk. The row is marked
    /// `failed` (terminal, therefore GC-able) with `active = 0`, so the corpus
    /// reads as uncovered and a rebuild is queued.
    ///
    /// Invalidating, not relabelling, is the point: a generation whose
    /// identity or index no longer holds up must not keep the active slot
    /// under a friendlier status, because every reader resolves "the current
    /// index" through that slot. This is never data loss — the authoritative
    /// chunks never left `rag_chunks`, so the recovery is a rebuild.
    ///
    /// Returns the invalidated generation id, or `None` when nothing was
    /// active.
    pub fn invalidate_active_generation(&self) -> Result<Option<String>, SemanticError> {
        let Some(active) = self.active_generation()? else {
            return Ok(None);
        };
        let connection = self.connection()?;
        connection.execute(
            "update semantic_generations
             set status = 'failed', active = 0, updated_at = ?2
             where generation_id = ?1",
            params![active.generation_id, now_iso8601()],
        )?;
        Ok(Some(active.generation_id))
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
                    normalization, tokenizer, max_input_tokens, max_batch_inputs,
                    corpus_revision
             from semantic_generations
             where active = 1",
        )?;
        let mut rows = statement.query([])?;
        match rows.next()? {
            Some(row) => Ok(Some(manifest_from_row(row)?)),
            None => Ok(None),
        }
    }

    /// Read-only active-generation probe for report-only surfaces (doctor):
    /// never creates the database, never ensures the derived schema. A missing
    /// database or missing table reads as "no active generation, intact".
    /// Returns the active manifest (if any) plus whether its index survived on
    /// disk.
    ///
    /// This is the ONE manifest reader that runs against an arbitrary data
    /// root rather than one `open_migrated` has already accepted, so it cannot
    /// assume the current schema. `hiero doctor` and `hiero semantic status`
    /// must report on a root that has not been upgraded yet — including a
    /// schema-version-2 root that armed semantics, whose
    /// `semantic_generations` predates `corpus_revision` — so the column is
    /// substituted with [`UNKNOWN_CORPUS_REVISION`] when it is absent. That is
    /// not a papered-over read: a generation from before revisions were
    /// recorded genuinely has no proven coverage, which is exactly what the
    /// sentinel means.
    pub fn probe_active_generation(
        config: &HieronymusConfig,
    ) -> Result<(Option<GenerationManifest>, bool), SemanticError> {
        let path = config.database_path();
        if !path.exists() {
            return Ok((None, true));
        }
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let table_present: i64 = connection.query_row(
            "select count(*) from sqlite_master where type = 'table'
                 and name = 'semantic_generations'",
            [],
            |row| row.get(0),
        )?;
        if table_present == 0 {
            return Ok((None, true));
        }
        let has_revision_column = connection
            .prepare("select name from pragma_table_info('semantic_generations')")?
            .query_map([], |row| row.get::<_, String>(0))?
            .any(|name| name.as_deref() == Ok("corpus_revision"));
        let revision_column = if has_revision_column {
            "corpus_revision".to_string()
        } else {
            format!("{UNKNOWN_CORPUS_REVISION} as corpus_revision")
        };
        let mut statement = connection.prepare(&format!(
            "select generation_id, status, provider, model, model_revision, dimensions,
                    expected_count, written_count, last_chunk_id, active, created_at, updated_at,
                    normalization, tokenizer, max_input_tokens, max_batch_inputs,
                    {revision_column}
             from semantic_generations
             where active = 1"
        ))?;
        let mut rows = statement.query([])?;
        let manifest = match rows.next()? {
            Some(row) => Some(manifest_from_row(row)?),
            None => None,
        };
        let intact = match &manifest {
            Some(active) => {
                active.written_count == active.expected_count
                    && generation_table_intact(
                        &config.semantic_root().join("lancedb"),
                        &active.generation_id,
                        &active.identity,
                        active.expected_count,
                    )
            }
            None => true,
        };
        Ok((manifest, intact))
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
                    normalization, tokenizer, max_input_tokens, max_batch_inputs,
                    corpus_revision
             from semantic_generations
             where generation_id = ?1",
        )?;
        let mut rows = statement.query(params![generation_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(manifest_from_row(row)?)),
            None => Ok(None),
        }
    }

    /// Whether the existing table opens and matches the durable manifest's
    /// count and stored identity. Missing or corrupt data reports `false`; the recovery is a rebuild
    /// (the authoritative rows never left SQLite), never data loss.
    pub fn active_generation_intact(&self) -> Result<bool, SemanticError> {
        match self.active_generation()? {
            None => Ok(true),
            Some(active) => Ok(active.written_count == active.expected_count
                && generation_table_intact(
                    &self.index_root(),
                    &active.generation_id,
                    &active.identity,
                    active.expected_count,
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

/// Refuse a candidate whose recorded corpus revision is no longer the
/// authoritative one (task C4).
///
/// `ValidationFailed` on purpose, matching every other activation refusal:
/// `semantic_jobs::activate_candidate` maps it to `JobOutcome::Failed` and
/// marks both the job and its candidate generation terminal, which is exactly
/// right — the candidate is unusable, not merely delayed. Reconciliation then
/// sees an uncovered corpus (plus, on the import path, a durable work intent)
/// and queues a fresh whole-corpus rebuild. `InvalidState` would be wrong
/// here: that variant means "another worker won the activation race" and sends
/// the job down `resolve_against_manifest` instead.
fn ensure_covers_corpus_revision(
    connection: &Connection,
    manifest: &GenerationManifest,
) -> Result<(), SemanticError> {
    let authoritative = crate::rag::current_corpus_revision(connection)?;
    if manifest.corpus_revision == authoritative {
        return Ok(());
    }
    let recorded = if manifest.corpus_revision == UNKNOWN_CORPUS_REVISION {
        "no recorded corpus revision (it predates corpus revisions)".to_string()
    } else {
        format!("corpus revision {}", manifest.corpus_revision)
    };
    Err(SemanticError::ValidationFailed(format!(
        "generation {} was built against {recorded} but the authoritative corpus is now at \
         revision {authoritative}; activating it would publish an index of text the store no \
         longer holds, so a fresh whole-corpus rebuild is required",
        manifest.generation_id
    )))
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
            row.get::<_, String>(13)?,
            row.get::<_, i64>(14)? as usize,
            row.get::<_, i64>(15)? as usize,
        )?,
        corpus_revision: row.get(16)?,
        expected_count: row.get::<_, i64>(6)? as u64,
        written_count: row.get::<_, i64>(7)? as u64,
        last_chunk_id: row.get(8)?,
        active: row.get::<_, i64>(9)? == 1,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

/// Vector-free fingerprint of one chunk text. The store writes it into every
/// `IndexRow` at build time; the query-time semantic lane recomputes it to
/// detect corrupt hits.
pub(crate) fn sha256_text(text: &str) -> String {
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

/// The upgrade protocol's in-transaction variant of
/// [`SemanticStore::begin_generation`]: registers the candidate generation
/// through the caller's connection (a `&Connection` cannot commit), refusing
/// autocommit so the upgrade transaction owns the commit. The manifest
/// freezes the authoritative chunk count AND the corpus revision as seen
/// inside that transaction, so the coverage claim it records is exactly the
/// corpus the caller serialized against.
pub(crate) fn begin_generation_in_transaction(
    connection: &Connection,
    generation_id: &str,
    identity: &EmbeddingIdentity,
) -> Result<GenerationManifest, SemanticError> {
    if connection.is_autocommit() {
        return Err(SemanticError::InvalidState(
            "generation creation requires a caller-owned transaction".to_string(),
        ));
    }
    validate_slug(generation_id)?;
    ensure_semantic_schema(connection)?;
    let expected_count: i64 =
        connection.query_row("select count(*) from rag_chunks", [], |row| row.get(0))?;
    let corpus_revision = crate::rag::current_corpus_revision(connection)?;
    let now = now_iso8601();
    let existing: i64 = connection.query_row(
        "select count(*) from semantic_generations where generation_id = ?1",
        params![generation_id],
        |row| row.get(0),
    )?;
    if existing > 0 {
        return Err(SemanticError::InvalidState(format!(
            "generation {generation_id} already has a manifest"
        )));
    }
    connection.execute(
        "insert into semantic_generations(
           generation_id, status, provider, model, model_revision, dimensions,
           normalization, tokenizer, max_input_tokens, max_batch_inputs,
           expected_count, corpus_revision, written_count, last_chunk_id, active,
           created_at, updated_at
         )
         values (?1, 'building', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, 0, 0, ?12, ?12)",
        params![
            generation_id,
            identity.provider(),
            identity.model(),
            identity.revision(),
            identity.dimensions() as i64,
            identity.normalization(),
            identity.tokenizer(),
            identity.max_input_tokens() as i64,
            identity.max_batch_inputs() as i64,
            expected_count,
            corpus_revision,
            now,
        ],
    )?;
    Ok(GenerationManifest {
        generation_id: generation_id.to_string(),
        status: "building".to_string(),
        identity: identity.clone(),
        corpus_revision,
        expected_count: expected_count as u64,
        written_count: 0,
        last_chunk_id: 0,
        active: false,
        created_at: now.clone(),
        updated_at: now,
    })
}
