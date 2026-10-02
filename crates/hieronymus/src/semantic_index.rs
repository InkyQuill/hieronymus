//! Exact cosine retrieval over a disposable SQLite database per generation.
//!
//! The authoritative corpus and generation/job protocol remain in the main
//! database. A candidate file never overwrites the active generation. Series
//! filtering happens in SQL before distance calculation; no ANN approximation,
//! async runtime, or additional native dependency is needed.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, params};

use crate::semantic_embeddings::EmbeddingIdentity;
use crate::semantic_error::SemanticError;

/// Separate namespace ensures old Lance artifacts cannot appear ready.
pub const INDEX_DIRECTORY: &str = "sqlite-vectors";
const FORMAT_VERSION: i64 = 1;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowFingerprint {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub generation_id: String,
}

/// Cosine distance (0 is identical direction, 2 is opposite).
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticHit {
    pub chunk_id: i64,
    pub series_slug: String,
    pub checksum: String,
    pub generation_id: String,
    pub distance: f32,
}

/// Database filename preserves '-' versus '_' in validated generation ids.
fn table_path(root: &Path, generation: &str) -> PathBuf {
    root.join(format!("generation_{generation}.sqlite3"))
}

pub fn generation_table_exists(root: &Path, generation: &str) -> bool {
    validate_slug(generation).is_ok() && table_path(root, generation).is_file()
}

fn identity_key(identity: &EmbeddingIdentity) -> String {
    serde_json::json!([
        identity.provider(),
        identity.model(),
        identity.revision(),
        identity.dimensions(),
        identity.normalization(),
        identity.tokenizer(),
        identity.max_input_tokens(),
        identity.max_batch_inputs()
    ])
    .to_string()
}

fn open_existing(path: &Path, writable: bool) -> Result<Connection, SemanticError> {
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let connection = Connection::open_with_flags(
        path,
        flags | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}

fn verify_identity(
    connection: &Connection,
    identity: &EmbeddingIdentity,
    generation: &str,
) -> Result<(), SemanticError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != FORMAT_VERSION {
        return Err(SemanticError::ValidationFailed(format!(
            "unsupported vector index format {version}"
        )));
    }
    let stored: (String, String) = connection.query_row(
        "SELECT generation_id, identity FROM index_metadata WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if stored != (generation.to_string(), identity_key(identity)) {
        return Err(SemanticError::IdentityMismatch {
            expected: format!("{generation}:{}", identity_key(identity)),
            actual: format!("{}:{}", stored.0, stored.1),
        });
    }
    Ok(())
}

/// Read-only probe: never creates/reinitializes a missing or damaged index.
pub fn generation_table_intact(
    root: &Path,
    generation: &str,
    identity: &EmbeddingIdentity,
    expected_count: u64,
) -> bool {
    let probe = || -> Result<bool, SemanticError> {
        validate_slug(generation)?;
        let connection = open_existing(&table_path(root, generation), false)?;
        verify_identity(&connection, identity, generation)?;
        let check: String = connection.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        if check != "ok" {
            return Ok(false);
        }
        let (count, mismatches):(i64,i64) = connection.query_row(
            "SELECT count(*), coalesce(sum(generation_id != ?1 OR model != ?2 OR model_revision != ?3 OR length(vector) != ?4),0) FROM vectors",
            params![generation,identity.model(),identity.revision(),(identity.dimensions()*4) as i64], |r|Ok((r.get(0)?,r.get(1)?)))?;
        if count as u64 != expected_count || mismatches != 0 {
            return Ok(false);
        }
        // Decode every vector: readable SQLite pages alone do not prove usable embeddings.
        let mut statement = connection.prepare("SELECT vector FROM vectors")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let blob: Vec<u8> = row.get(0)?;
            decode_vector(&blob, identity.dimensions())?;
        }
        Ok(true)
    };
    probe().unwrap_or(false)
}

pub struct VectorIndex {
    root: PathBuf,
    identity: EmbeddingIdentity,
    generation: String,
    connection: Connection,
}

impl VectorIndex {
    /// Publish a complete empty schema atomically; existing files are validated,
    /// never repaired in place. Recovery owns rebuilding an incompatible index.
    pub fn open(
        root: &Path,
        identity: EmbeddingIdentity,
        generation: &str,
    ) -> Result<Self, SemanticError> {
        validate_slug(generation)?;
        std::fs::create_dir_all(root)?;
        let path = table_path(root, generation);
        if !path.try_exists()? {
            let temporary = tempfile::NamedTempFile::new_in(root)?;
            {
                let mut connection = open_existing(temporary.path(), true)?;
                let tx = connection.transaction()?;
                tx.execute_batch("CREATE TABLE index_metadata(singleton INTEGER PRIMARY KEY CHECK(singleton=1),generation_id TEXT NOT NULL,identity TEXT NOT NULL) STRICT;
                CREATE TABLE vectors(chunk_id INTEGER PRIMARY KEY CHECK(chunk_id>0),series_slug TEXT NOT NULL,checksum TEXT NOT NULL CHECK(length(checksum)>0),generation_id TEXT NOT NULL,model TEXT NOT NULL,model_revision TEXT NOT NULL,vector BLOB NOT NULL) STRICT;
                CREATE INDEX vectors_by_series ON vectors(series_slug,chunk_id);
                PRAGMA user_version=1;")?;
                tx.execute(
                    "INSERT INTO index_metadata VALUES(1,?1,?2)",
                    params![generation, identity_key(&identity)],
                )?;
                tx.commit()?;
            }
            temporary.as_file().sync_all()?;
            if let Err(error) = temporary.persist_noclobber(&path)
                && error.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(error.error.into());
            }
            // Another opener may have published first; validate its identity below.
        }
        let connection = open_existing(&path, true)?;
        verify_identity(&connection, &identity, generation)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        Ok(Self {
            root: root.to_path_buf(),
            identity,
            generation: generation.to_string(),
            connection,
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

    /// Validate the entire batch first, then commit all rows or none. A duplicate
    /// chunk is an error, never a silent overwrite of a previous receipt.
    pub fn append(&mut self, rows: Vec<IndexRow>) -> Result<usize, SemanticError> {
        for row in &rows {
            validate_slug(&row.series_slug)?;
            if row.chunk_id <= 0
                || row.checksum.is_empty()
                || row.generation_id != self.generation
                || row.model != self.identity.model()
                || row.model_revision != self.identity.revision()
            {
                return Err(SemanticError::InvalidEmbedding(
                    "row identifiers or model do not match the generation".into(),
                ));
            }
            validate_vector(&row.vector, self.identity.dimensions())?;
        }
        let tx = self.connection.transaction()?;
        {
            let mut insert = tx.prepare("INSERT INTO vectors VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
            for row in &rows {
                let blob: Vec<u8> = row.vector.iter().flat_map(|v| v.to_le_bytes()).collect();
                insert.execute(params![
                    row.chunk_id,
                    row.series_slug,
                    row.checksum,
                    row.generation_id,
                    row.model,
                    row.model_revision,
                    blob
                ])?;
            }
        }
        tx.commit()?;
        Ok(rows.len())
    }

    pub fn search(
        &self,
        series_slug: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticHit>, SemanticError> {
        validate_slug(series_slug)?;
        self.search_rows(Some(series_slug), vector, limit)
    }

    /// Adversarial test control; serving callers always use the prefiltered path.
    pub fn search_postfiltered(
        &self,
        series_slug: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticHit>, SemanticError> {
        validate_slug(series_slug)?;
        Ok(self
            .search_rows(None, vector, limit)?
            .into_iter()
            .filter(|h| h.series_slug == series_slug)
            .collect())
    }

    fn search_rows(
        &self,
        series: Option<&str>,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticHit>, SemanticError> {
        validate_vector(vector, self.identity.dimensions())?;
        if limit == 0 {
            return Err(SemanticError::ValidationFailed(
                "search limit must be at least 1".into(),
            ));
        }
        let sql = if series.is_some() {
            "SELECT chunk_id,series_slug,checksum,generation_id,model,model_revision,vector FROM vectors WHERE series_slug=?1"
        } else {
            "SELECT chunk_id,series_slug,checksum,generation_id,model,model_revision,vector FROM vectors"
        };
        let mut statement = self.connection.prepare(sql)?;
        let mut rows = statement.query(rusqlite::params_from_iter(series))?;
        let mut heap = BinaryHeap::<RankedHit>::new();
        let query_norm = norm_squared(vector).sqrt();
        while let Some(row) = rows.next()? {
            let generation: String = row.get(3)?;
            let model: String = row.get(4)?;
            let revision: String = row.get(5)?;
            if generation != self.generation
                || model != self.identity.model()
                || revision != self.identity.revision()
            {
                return Err(SemanticError::ValidationFailed(
                    "stored vector identity does not match generation".into(),
                ));
            }
            let stored = decode_vector(&row.get::<_, Vec<u8>>(6)?, self.identity.dimensions())?;
            let dot: f64 = stored
                .iter()
                .zip(vector)
                .map(|(&a, &b)| f64::from(a) * f64::from(b))
                .sum();
            let distance =
                (1.0 - dot / (norm_squared(&stored).sqrt() * query_norm)).clamp(0.0, 2.0);
            let hit = RankedHit {
                distance,
                hit: SemanticHit {
                    chunk_id: row.get(0)?,
                    series_slug: row.get(1)?,
                    checksum: row.get(2)?,
                    generation_id: generation,
                    distance: distance as f32,
                },
            };
            if heap.len() < limit {
                heap.push(hit);
            } else if heap.peek().is_some_and(|worst| hit < *worst) {
                heap.pop();
                heap.push(hit);
            }
        }
        Ok(heap.into_sorted_vec().into_iter().map(|r| r.hit).collect())
    }

    pub fn count_rows(&self) -> Result<usize, SemanticError> {
        Ok(self
            .connection
            .query_row("SELECT count(*) FROM vectors", [], |r| r.get::<_, i64>(0))?
            as usize)
    }
    pub fn snapshot_rows(&self, limit: usize) -> Result<Vec<RowFingerprint>, SemanticError> {
        let mut statement=self.connection.prepare("SELECT chunk_id,series_slug,checksum,generation_id FROM vectors ORDER BY chunk_id LIMIT ?1")?;
        Ok(statement
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
                Ok(RowFingerprint {
                    chunk_id: r.get(0)?,
                    series_slug: r.get(1)?,
                    checksum: r.get(2)?,
                    generation_id: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn stored_vector_width(&self) -> Result<Option<usize>, SemanticError> {
        use rusqlite::OptionalExtension;
        let length: Option<i64> = self
            .connection
            .query_row("SELECT length(vector) FROM vectors LIMIT 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(length.map(|n| n as usize / 4))
    }
}

/// Called only by generation GC after the authoritative manifest rejects active
/// generations. Handles are dropped before GC; remove SQLite sidecars as well.
pub fn drop_generation_table(root: &Path, generation: &str) -> Result<bool, SemanticError> {
    validate_slug(generation)?;
    let path = table_path(root, generation);
    let existed = match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        match std::fs::remove_file(PathBuf::from(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(existed)
}

fn norm_squared(vector: &[f32]) -> f64 {
    vector.iter().map(|&v| f64::from(v) * f64::from(v)).sum()
}
fn validate_vector(vector: &[f32], dimensions: usize) -> Result<(), SemanticError> {
    if vector.len() != dimensions
        || !vector.iter().all(|v| v.is_finite())
        || norm_squared(vector) == 0.0
    {
        return Err(SemanticError::InvalidEmbedding(
            "vector has wrong width, non-finite values, or zero norm".into(),
        ));
    }
    Ok(())
}
fn decode_vector(blob: &[u8], dimensions: usize) -> Result<Vec<f32>, SemanticError> {
    if blob.len() != dimensions * 4 {
        return Err(SemanticError::InvalidEmbedding(
            "stored vector has wrong byte length".into(),
        ));
    }
    let (chunks, _) = blob.as_chunks::<4>();
    let vector: Vec<f32> = chunks.iter().map(|b| f32::from_le_bytes(*b)).collect();
    validate_vector(&vector, dimensions)?;
    Ok(vector)
}

struct RankedHit {
    distance: f64,
    hit: SemanticHit,
}
impl PartialEq for RankedHit {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for RankedHit {}
impl PartialOrd for RankedHit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedHit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .total_cmp(&other.distance)
            .then(self.hit.chunk_id.cmp(&other.hit.chunk_id))
    }
}

/// Retained predicate helper for callers validating the public slug contract.
/// SQLite serving queries use bound parameters rather than this string.
pub fn series_predicate(series_slug: &str) -> Result<String, SemanticError> {
    validate_slug(series_slug)?;
    Ok(format!("series_slug = '{series_slug}'"))
}
pub fn validate_slug(slug: &str) -> Result<(), SemanticError> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(SemanticError::InvalidSlug(format!(
            "slug {slug:?} is not a normalized slug"
        )));
    }
    Ok(())
}
