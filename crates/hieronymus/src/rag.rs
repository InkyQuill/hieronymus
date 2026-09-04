//! RAG store: checksum-aware source import, series-isolated FTS search, and
//! chunk metadata side tables. Behavior ported from Python `rag_store.py`,
//! `rag_parsing.py`, and `rag_conversion.py` over the shared
//! `rag_sources`/`rag_chunks` schema.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params, params_from_iter};
use serde_json::{Map, Value};

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::rag_models::{RagChunkRecord, RagImportResult, RagSearchHit, RagSourceRecord};
use crate::short_memory::search_expression;

pub const MAX_RAG_CHUNK_CHARS: usize = 1200;
const MAX_RAG_SEARCH_LIMIT: usize = 50;
const LANGUAGE_TAG_BOOST: f64 = 0.05;
const STORY_SCOPE_BOOST: f64 = 0.10;
const SEMANTIC_TAG_BOOST: f64 = 0.10;
const GLOSSARY_BOOST: f64 = 0.25;

const REASON_GLOSSARY: &str = "rag glossary match";
const REASON_MARKDOWN_SECTION: &str = "rag markdown section match";
const REASON_PROJECT_TEXT: &str = "rag project text match";

#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("limit must be at least 1")]
    LimitTooSmall,
    #[error("invalid RAG source: {0}")]
    InvalidSource(String),
    #[error("RAG source not found: {0}")]
    NotFound(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}

/// Import parameters for [`RagStore::import_file`], mirroring the Python
/// keyword arguments (everything except the path defaults).
#[derive(Debug, Clone)]
pub struct RagImport {
    pub source_ref: Option<String>,
    pub source_type: String,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
}

impl Default for RagImport {
    fn default() -> Self {
        Self {
            source_ref: None,
            source_type: "auto".to_string(),
            language_tags: Vec::new(),
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
        }
    }
}

impl RagImport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn source_ref(mut self, source_ref: impl Into<String>) -> Self {
        self.source_ref = Some(source_ref.into());
        self
    }

    pub fn source_type(mut self, source_type: impl Into<String>) -> Self {
        self.source_type = source_type.into();
        self
    }

    pub fn language_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.language_tags = tags.into_iter().map(Into::into).collect();
        self
    }

    pub fn story_scopes(mut self, scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.story_scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    pub fn semantic_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.semantic_tags = tags.into_iter().map(Into::into).collect();
        self
    }
}

/// RAG store over the data-root database.
pub struct RagStore {
    config: HieronymusConfig,
}

impl RagStore {
    pub fn open(config: &HieronymusConfig) -> Result<Self, RagError> {
        open_migrated(&config.database_path())?;
        Ok(Self {
            config: config.clone(),
        })
    }

    fn connection(&self) -> Result<Connection, RagError> {
        let path = self.config.database_path();
        Ok(open_migrated(Path::new(&path))?)
    }

    /// Import a RAG file into the store. Re-importing the same
    /// `(series_slug, source_ref)`: an identical checksum (and source/content
    /// type) only refreshes the chunk metadata tags; a changed import
    /// atomically replaces the source with its chunks and derived rows. A
    /// source that fails to parse never touches the database.
    pub fn import_file(
        &self,
        series_slug: &str,
        path: &Path,
        import: &RagImport,
    ) -> Result<RagImportResult, RagError> {
        let clean_source_ref = match &import.source_ref {
            Some(source_ref) => source_ref.clone(),
            None => path.display().to_string(),
        };
        let normalized =
            normalize_rag_source(path, &self.config.data_root().join("rag-normalized"))?;
        let source_type_hint = if normalized.path == path {
            import.source_type.as_str()
        } else {
            // Converted sources land as managed markdown; parse format follows
            // the normalized file, like the Python store.
            "auto"
        };
        let parsed = load_rag_file(&normalized.path, source_type_hint)?;

        let mut parsed_metadata = parsed.metadata.clone();
        parsed_metadata.insert(
            "original_path".to_string(),
            Value::String(normalized.original_path.display().to_string()),
        );
        parsed_metadata.insert(
            "normalized_path".to_string(),
            Value::String(normalized.path.display().to_string()),
        );
        parsed_metadata.insert(
            "normalized_format".to_string(),
            Value::String(normalized.format.clone()),
        );

        let clean_language_tags = clean_text_values(&import.language_tags);
        let clean_story_scopes = clean_text_values(&import.story_scopes);
        let clean_semantic_tags = clean_text_values(&import.semantic_tags);

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing = source_row(&transaction, series_slug, &clean_source_ref)?;

        if let Some(row) = &existing {
            if row.checksum == parsed.checksum
                && row.source_type == parsed.source_type
                && row.content_type == parsed.content_type
            {
                let chunk_count = refresh_source_chunk_tags(
                    &transaction,
                    row.id,
                    series_slug,
                    &clean_language_tags,
                    &clean_story_scopes,
                    &clean_semantic_tags,
                )?;
                transaction.commit()?;
                return Ok(RagImportResult {
                    source: row.clone(),
                    chunk_count,
                    skipped: true,
                    normalized_path: normalized.path.display().to_string(),
                    normalized_format: normalized.format,
                });
            }
            transaction.execute(
                "delete from rag_sources
                 where id = ?1
                   and series_slug = ?2",
                params![row.id, series_slug],
            )?;
        }

        let now = now_iso8601();
        transaction.execute(
            "insert into rag_sources(
               series_slug, source_ref, source_type, content_type,
               checksum, metadata_json, created_at, updated_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                series_slug,
                clean_source_ref,
                parsed.source_type,
                parsed.content_type,
                parsed.checksum,
                serde_json::to_string(&parsed_metadata)?,
                now,
                now,
            ],
        )?;
        let source_id = transaction.last_insert_rowid();
        for chunk in &parsed.chunks {
            transaction.execute(
                "insert into rag_chunks(
                   source_id, series_slug, chunk_kind, text, display_text,
                   location, metadata_json, created_at
                 )
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    source_id,
                    series_slug,
                    chunk.chunk_kind,
                    chunk.text,
                    chunk.display_text,
                    chunk.location,
                    serde_json::to_string(&chunk.metadata)?,
                    now,
                ],
            )?;
            let chunk_id = transaction.last_insert_rowid();
            for value in &clean_language_tags {
                transaction.execute(
                    "insert into rag_chunk_language_tags(chunk_id, language_tag)
                     values (?1, ?2)",
                    params![chunk_id, value],
                )?;
            }
            for value in &clean_story_scopes {
                transaction.execute(
                    "insert into rag_chunk_story_scopes(chunk_id, story_scope)
                     values (?1, ?2)",
                    params![chunk_id, value],
                )?;
            }
            for value in &clean_semantic_tags {
                transaction.execute(
                    "insert into rag_chunk_semantic_tags(chunk_id, semantic_tag)
                     values (?1, ?2)",
                    params![chunk_id, value],
                )?;
            }
        }
        transaction.commit()?;

        Ok(RagImportResult {
            source: RagSourceRecord {
                id: source_id,
                series_slug: series_slug.to_string(),
                source_ref: clean_source_ref,
                source_type: parsed.source_type.to_string(),
                content_type: parsed.content_type,
                checksum: parsed.checksum,
                metadata: parsed_metadata,
            },
            chunk_count: parsed.chunks.len(),
            skipped: false,
            normalized_path: normalized.path.display().to_string(),
            normalized_format: normalized.format,
        })
    }

    /// Series-isolated FTS search over `rag_chunks_fts` with glossary and
    /// typed metadata boosts applied before candidate truncation.
    pub fn search(
        &self,
        series_slug: &str,
        query: &str,
        limit: usize,
        language_tags: &[String],
        story_scopes: &[String],
        semantic_tags: &[String],
    ) -> Result<Vec<RagSearchHit>, RagError> {
        if limit == 0 {
            return Err(RagError::LimitTooSmall);
        }
        let expression = search_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let bounded_limit = limit.min(MAX_RAG_SEARCH_LIMIT) as i64;
        let clean_language_tags = clean_text_values(language_tags);
        let clean_story_scopes = clean_text_values(story_scopes);
        let clean_semantic_tags = clean_text_values(semantic_tags);
        let connection = self.connection()?;
        register_casefold_collation(&connection)?;

        let mut next_index = 1usize;
        let glossary_index = next_index;
        next_index += 1;
        let (language_score, language_parameters) = tag_boost_sql(
            "rag_chunk_language_tags",
            "language_tag",
            &clean_language_tags,
            LANGUAGE_TAG_BOOST,
            next_index,
        );
        next_index += language_parameters.len();
        let (story_score, story_parameters) = tag_boost_sql(
            "rag_chunk_story_scopes",
            "story_scope",
            &clean_story_scopes,
            STORY_SCOPE_BOOST,
            next_index,
        );
        next_index += story_parameters.len();
        let (semantic_score, semantic_parameters) = tag_boost_sql(
            "rag_chunk_semantic_tags",
            "semantic_tag",
            &clean_semantic_tags,
            SEMANTIC_TAG_BOOST,
            next_index,
        );
        next_index += semantic_parameters.len();
        let expression_index = next_index;
        let series_index = next_index + 1;
        let limit_index = next_index + 2;

        let mut parameters: Vec<rusqlite::types::Value> = Vec::with_capacity(next_index + 3);
        parameters.push(rusqlite::types::Value::from(GLOSSARY_BOOST));
        parameters.extend(language_parameters);
        parameters.extend(story_parameters);
        parameters.extend(semantic_parameters);
        parameters.push(rusqlite::types::Value::from(expression));
        parameters.push(rusqlite::types::Value::from(series_slug.to_string()));
        parameters.push(rusqlite::types::Value::from(bounded_limit));

        let sql = format!(
            "select
               rag_chunks.id,
               rag_chunks.source_id,
               rag_chunks.series_slug,
               rag_chunks.chunk_kind,
               rag_chunks.text,
               rag_chunks.display_text,
               rag_chunks.location,
               rag_chunks.metadata_json,
               rag_sources.source_ref,
               (
                 max(-bm25(rag_chunks_fts), 0.000001)
                 + case
                     when rag_chunks.chunk_kind = 'glossary_entry' then ?{glossary_index}
                     else 0.0
                   end
                 + {language_score}
                 + {story_score}
                 + {semantic_score}
               ) as score
             from rag_chunks_fts
             join rag_chunks
               on rag_chunks.id = rag_chunks_fts.rowid
             join rag_sources
               on rag_sources.id = rag_chunks.source_id
              and rag_sources.series_slug = rag_chunks.series_slug
             where rag_chunks_fts match ?{expression_index}
               and rag_chunks.series_slug = ?{series_index}
             order by score desc, rag_chunks.id
             limit ?{limit_index}"
        );
        let mut statement = connection.prepare(&sql)?;
        let rows: Vec<ChunkRow> = statement
            .query_map(params_from_iter(parameters.iter()), |row| {
                Ok(ChunkRow {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    series_slug: row.get(2)?,
                    chunk_kind: row.get(3)?,
                    text: row.get(4)?,
                    display_text: row.get(5)?,
                    location: row.get(6)?,
                    metadata_json: row.get(7)?,
                    source_ref: row.get(8)?,
                    score: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let chunk_ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        let chunk_language_tags = text_values_for_chunks(
            &connection,
            "rag_chunk_language_tags",
            "language_tag",
            &chunk_ids,
        )?;
        let chunk_story_scopes = text_values_for_chunks(
            &connection,
            "rag_chunk_story_scopes",
            "story_scope",
            &chunk_ids,
        )?;
        let chunk_semantic_tags = text_values_for_chunks(
            &connection,
            "rag_chunk_semantic_tags",
            "semantic_tag",
            &chunk_ids,
        )?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let metadata = match serde_json::from_str::<Value>(&row.metadata_json) {
                    Ok(Value::Object(map)) => map,
                    _ => Map::new(),
                };
                let chunk = RagChunkRecord {
                    id: row.id,
                    source_id: row.source_id,
                    series_slug: row.series_slug,
                    source_ref: row.source_ref,
                    chunk_kind: row.chunk_kind.clone(),
                    text: row.text,
                    display_text: row.display_text,
                    location: row.location,
                    metadata,
                    language_tags: chunk_language_tags
                        .get(&row.id)
                        .cloned()
                        .unwrap_or_default(),
                    story_scopes: chunk_story_scopes.get(&row.id).cloned().unwrap_or_default(),
                    semantic_tags: chunk_semantic_tags
                        .get(&row.id)
                        .cloned()
                        .unwrap_or_default(),
                };
                RagSearchHit {
                    chunk,
                    score: row.score,
                    reason: reason_for_chunk_kind(&row.chunk_kind).to_string(),
                }
            })
            .collect())
    }
}

struct ChunkRow {
    id: i64,
    source_id: i64,
    series_slug: String,
    chunk_kind: String,
    text: String,
    display_text: String,
    location: String,
    metadata_json: String,
    source_ref: String,
    score: f64,
}

fn reason_for_chunk_kind(chunk_kind: &str) -> &'static str {
    if chunk_kind == "glossary_entry" {
        return REASON_GLOSSARY;
    }
    if chunk_kind == "markdown_section" {
        return REASON_MARKDOWN_SECTION;
    }
    REASON_PROJECT_TEXT
}

fn source_row(
    connection: &Connection,
    series_slug: &str,
    source_ref: &str,
) -> Result<Option<RagSourceRecord>, RagError> {
    let mut statement = connection.prepare(
        "select id, series_slug, source_ref, source_type, content_type, checksum, metadata_json
         from rag_sources
         where series_slug = ?1
           and source_ref = ?2",
    )?;
    let mut rows = statement.query(params![series_slug, source_ref])?;
    match rows.next()? {
        Some(row) => Ok(Some(RagSourceRecord {
            id: row.get(0)?,
            series_slug: row.get(1)?,
            source_ref: row.get(2)?,
            source_type: row.get(3)?,
            content_type: row.get(4)?,
            checksum: row.get(5)?,
            metadata: json_object(&row.get::<_, String>(6)?)?,
        })),
        None => Ok(None),
    }
}

fn refresh_source_chunk_tags(
    connection: &Connection,
    source_id: i64,
    series_slug: &str,
    language_tags: &[String],
    story_scopes: &[String],
    semantic_tags: &[String],
) -> Result<usize, RagError> {
    let chunk_ids: Vec<i64> = {
        let mut statement = connection.prepare(
            "select id
             from rag_chunks
             where source_id = ?1
               and series_slug = ?2
             order by id",
        )?;
        let rows = statement.query_map(params![source_id, series_slug], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for chunk_id in &chunk_ids {
        replace_text_values(
            connection,
            "rag_chunk_language_tags",
            "language_tag",
            *chunk_id,
            language_tags,
        )?;
        replace_text_values(
            connection,
            "rag_chunk_story_scopes",
            "story_scope",
            *chunk_id,
            story_scopes,
        )?;
        replace_text_values(
            connection,
            "rag_chunk_semantic_tags",
            "semantic_tag",
            *chunk_id,
            semantic_tags,
        )?;
    }
    Ok(chunk_ids.len())
}

fn replace_text_values(
    connection: &Connection,
    table: &str,
    column: &str,
    chunk_id: i64,
    values: &[String],
) -> Result<(), RagError> {
    connection.execute(
        &format!("delete from {table} where chunk_id = ?1"),
        [chunk_id],
    )?;
    for value in values {
        connection.execute(
            &format!("insert into {table}(chunk_id, {column}) values (?1, ?2)"),
            params![chunk_id, value],
        )?;
    }
    Ok(())
}

fn text_values_for_chunks(
    connection: &Connection,
    table: &str,
    column: &str,
    chunk_ids: &[i64],
) -> Result<HashMap<i64, Vec<String>>, RagError> {
    if chunk_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = (1..=chunk_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "select chunk_id, {column}
         from {table}
         where chunk_id in ({placeholders})
         order by chunk_id, {column}"
    );
    let mut statement = connection.prepare(&sql)?;
    let parameters: Vec<rusqlite::types::Value> = chunk_ids
        .iter()
        .map(|id| rusqlite::types::Value::from(*id))
        .collect();
    let mut rows = statement.query(params_from_iter(parameters.iter()))?;
    let mut mapping: HashMap<i64, Vec<String>> = HashMap::new();
    while let Some(row) = rows.next()? {
        mapping
            .entry(row.get::<_, i64>(0)?)
            .or_default()
            .push(row.get::<_, String>(1)?);
    }
    Ok(mapping)
}

/// SQL fragment adding a boost when a chunk carries one of the requested tag
/// values (casefolded comparison), plus its ordered bind parameters. The
/// custom `casefold` collation (registered below; the bundled SQLite build
/// does not ship the 3.44 built-in) matches the Python `casefold` function.
fn register_casefold_collation(connection: &Connection) -> Result<(), RagError> {
    connection.create_collation("casefold", |left: &str, right: &str| {
        left.to_lowercase().cmp(&right.to_lowercase())
    })?;
    Ok(())
}
fn tag_boost_sql(
    table: &str,
    column: &str,
    values: &[String],
    boost: f64,
    start_index: usize,
) -> (String, Vec<rusqlite::types::Value>) {
    if values.is_empty() {
        return ("0.0".to_string(), Vec::new());
    }
    let placeholders = (0..values.len())
        .map(|offset| format!("?{}", start_index + offset))
        .collect::<Vec<_>>()
        .join(", ");
    let boost_index = start_index + values.len();
    let sql = format!(
        "case when exists (
             select 1
             from {table} tag_values
             where tag_values.chunk_id = rag_chunks.id
               and tag_values.{column} collate casefold in ({placeholders})
           )
           then ?{boost_index} else 0.0 end"
    );
    let mut parameters: Vec<rusqlite::types::Value> = values
        .iter()
        .map(|value| rusqlite::types::Value::from(value.clone()))
        .collect();
    parameters.push(rusqlite::types::Value::from(boost));
    (sql, parameters)
}

fn json_object(raw: &str) -> Result<Map<String, Value>, RagError> {
    let value = serde_json::from_str::<Value>(raw)?;
    Ok(match value {
        Value::Object(map) => map,
        _ => Map::new(),
    })
}

/// Order-preserving cleanup: trim, drop empties, deduplicate (Python
/// `_clean_text_tuple`).
fn clean_text_values(values: &[String]) -> Vec<String> {
    let mut cleaned: Vec<String> = Vec::new();
    for value in values {
        let clean = value.trim();
        if clean.is_empty() || cleaned.iter().any(|existing| existing == clean) {
            continue;
        }
        cleaned.push(clean.to_string());
    }
    cleaned
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

// ---------------------------------------------------------------------------
// File parsing (Python `rag_parsing.py`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ParsedRagChunk {
    pub chunk_kind: &'static str,
    pub text: String,
    pub display_text: String,
    pub location: String,
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct ParsedRagFile {
    pub source_type: &'static str,
    pub content_type: String,
    pub checksum: String,
    pub chunks: Vec<ParsedRagChunk>,
    pub metadata: Map<String, Value>,
}

pub fn load_rag_file(path: &Path, source_type: &str) -> Result<ParsedRagFile, RagError> {
    if !path.exists() {
        return Err(RagError::NotFound(path.to_path_buf()));
    }
    if !path.is_file() {
        // Python raises a ValueError here, which import wraps as an invalid
        // RAG source.
        return Err(RagError::InvalidSource(format!(
            "RAG source is not a file: {}",
            path.display()
        )));
    }
    let raw_bytes = std::fs::read(path)?;
    let checksum = sha256_hex(&raw_bytes);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_lowercase();
    if extension.is_empty() {
        return Err(RagError::InvalidSource(format!(
            "Unsupported RAG source extension: {}",
            path.display()
        )));
    }
    let suffix = format!(".{extension}");
    let resolved_source_type = resolve_source_type(&suffix, source_type)?;

    let chunks = match extension.as_str() {
        "txt" => parse_text(path)?,
        "md" | "markdown" => parse_markdown(path)?,
        "csv" => parse_delimited_glossary(path, ',')?,
        "tsv" => parse_delimited_glossary(path, '\t')?,
        "json" => parse_json_glossary(path)?,
        "yaml" | "yml" => parse_yaml_glossary(path)?,
        _ => {
            return Err(RagError::InvalidSource(format!(
                "Unsupported RAG source extension: {suffix}"
            )));
        }
    };
    if chunks.is_empty() {
        return Err(RagError::InvalidSource(format!(
            "RAG source produced no chunks: {}",
            path.display()
        )));
    }

    Ok(ParsedRagFile {
        source_type: resolved_source_type,
        content_type: extension,
        checksum,
        chunks,
        metadata: Map::new(),
    })
}

fn resolve_source_type(suffix: &str, source_type: &str) -> Result<&'static str, RagError> {
    let is_text = matches!(suffix, ".txt" | ".md" | ".markdown");
    let is_glossary = matches!(suffix, ".csv" | ".tsv" | ".json" | ".yaml" | ".yml");
    if !is_text && !is_glossary {
        return Err(RagError::InvalidSource(format!(
            "Unsupported RAG source extension: {suffix}"
        )));
    }
    match source_type {
        "auto" => Ok(if suffix == ".txt" {
            "text"
        } else if is_text {
            "markdown"
        } else {
            "glossary"
        }),
        "text" => {
            if suffix == ".txt" {
                Ok("text")
            } else if is_text {
                Ok("markdown")
            } else {
                Err(RagError::InvalidSource(format!(
                    "Text RAG sources do not support {suffix} files"
                )))
            }
        }
        "glossary" => {
            if is_glossary {
                Ok("glossary")
            } else {
                Err(RagError::InvalidSource(format!(
                    "Glossary RAG sources do not support {suffix} files"
                )))
            }
        }
        other => Err(RagError::InvalidSource(format!(
            "Unsupported RAG source type: {other}"
        ))),
    }
}

/// Read a UTF-8 text file with Python-`splitlines`-compatible line
/// separators normalized to `\n`.
fn read_text(path: &Path) -> Result<String, RagError> {
    let raw_bytes = std::fs::read(path)?;
    let text = String::from_utf8(raw_bytes)
        .map_err(|error| RagError::InvalidSource(format!("{error}")))?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n").replace(
        [
            '\u{000B}', '\u{000C}', '\u{001C}', '\u{001D}', '\u{001E}', '\u{0085}', '\u{2028}',
            '\u{2029}',
        ],
        "\n",
    ))
}

fn parse_text(path: &Path) -> Result<Vec<ParsedRagChunk>, RagError> {
    let text = read_text(path)?;
    let mut chunks = Vec::new();
    for (index, paragraph) in paragraphs(&text).into_iter().enumerate() {
        let base_location = format!("paragraph {}", index + 1);
        for (location, text) in located_chunk_texts(&paragraph, &base_location) {
            chunks.push(ParsedRagChunk {
                chunk_kind: "text",
                display_text: text.clone(),
                text,
                location,
                metadata: Map::new(),
            });
        }
    }
    Ok(chunks)
}

fn parse_markdown(path: &Path) -> Result<Vec<ParsedRagChunk>, RagError> {
    let text = read_text(path)?;
    let mut chunks: Vec<ParsedRagChunk> = Vec::new();
    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    let mut paragraph_lines: Vec<String> = Vec::new();
    let mut paragraph_count = 0usize;
    let mut fence: Option<(char, usize)> = None;

    for line in text.lines() {
        if let Some((fence_character, minimum_length)) = fence {
            paragraph_lines.push(line.to_string());
            if is_closing_fence(line, fence_character, minimum_length) {
                fence = None;
            }
            continue;
        }
        if let Some((character, length)) = opening_fence(line) {
            fence = Some((character, length));
            paragraph_lines.push(line.to_string());
            continue;
        }
        if line.starts_with("    ") || line.starts_with('\t') {
            paragraph_lines.push(line.to_string());
            continue;
        }
        if let Some((level, heading)) = markdown_heading(line) {
            flush_markdown_paragraph(
                &mut paragraph_lines,
                &mut paragraph_count,
                &heading_stack,
                &mut chunks,
            );
            heading_stack.retain(|(old_level, _)| *old_level < level);
            heading_stack.push((level, heading));
            continue;
        }
        if line.trim().is_empty() {
            flush_markdown_paragraph(
                &mut paragraph_lines,
                &mut paragraph_count,
                &heading_stack,
                &mut chunks,
            );
            continue;
        }
        paragraph_lines.push(line.to_string());
    }
    flush_markdown_paragraph(
        &mut paragraph_lines,
        &mut paragraph_count,
        &heading_stack,
        &mut chunks,
    );
    Ok(chunks)
}

fn flush_markdown_paragraph(
    paragraph_lines: &mut Vec<String>,
    paragraph_count: &mut usize,
    heading_stack: &[(usize, String)],
    chunks: &mut Vec<ParsedRagChunk>,
) {
    let paragraph = paragraph_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    paragraph_lines.clear();
    if paragraph.is_empty() {
        return;
    }
    *paragraph_count += 1;
    let heading_path = heading_stack
        .iter()
        .map(|(_, heading)| heading.as_str())
        .collect::<Vec<_>>()
        .join(" > ");
    let base_location = if heading_path.is_empty() {
        format!("paragraph {paragraph_count}")
    } else {
        format!("{heading_path} paragraph {paragraph_count}")
    };
    for (location, text) in located_chunk_texts(&paragraph, &base_location) {
        chunks.push(ParsedRagChunk {
            chunk_kind: "markdown_section",
            display_text: text.clone(),
            text,
            location,
            metadata: Map::new(),
        });
    }
}

/// `^(#{1,6})\s+(.+?)\s*#*\s*$` on the stripped line, with the lazy group
/// keeping the longest trailing `\s*#*\s*` suffix out of the heading text.
fn markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.chars().next()?.is_whitespace() {
        return None;
    }
    let remainder = rest.trim_start();
    if remainder.is_empty() {
        return None;
    }
    let characters: Vec<(usize, char)> = remainder.char_indices().collect();
    let mut index = characters.len();
    while index > 0 && characters[index - 1].1.is_whitespace() {
        index -= 1;
    }
    while index > 0 && characters[index - 1].1 == '#' {
        index -= 1;
    }
    while index > 0 && characters[index - 1].1.is_whitespace() {
        index -= 1;
    }
    let cut = if index >= characters.len() {
        remainder.len()
    } else if index == 0 {
        // The whole remainder is trailing decoration; the lazy group still
        // needs at least one character.
        characters[0].0 + characters[0].1.len_utf8()
    } else {
        characters[index].0
    };
    Some((level, remainder[..cut].to_string()))
}

/// `^ {0,3}(?P<fence>`{3,}|~{3,}).*$`
fn opening_fence(line: &str) -> Option<(char, usize)> {
    let characters: Vec<char> = line.chars().collect();
    let mut index = 0usize;
    while index < 3 && index < characters.len() && characters[index] == ' ' {
        index += 1;
    }
    let character = *characters.get(index)?;
    if character != '`' && character != '~' {
        return None;
    }
    let length = characters[index..]
        .iter()
        .take_while(|current| **current == character)
        .count();
    if length < 3 {
        return None;
    }
    Some((character, length))
}

/// ` {0,3}{char}{minimum_length,}\s*` fullmatch.
fn is_closing_fence(line: &str, character: char, minimum_length: usize) -> bool {
    let characters: Vec<char> = line.chars().collect();
    let mut index = 0usize;
    while index < 3 && index < characters.len() && characters[index] == ' ' {
        index += 1;
    }
    let mut length = 0usize;
    while index < characters.len() && characters[index] == character {
        index += 1;
        length += 1;
    }
    length >= minimum_length
        && characters[index..]
            .iter()
            .all(|character| character.is_whitespace())
}

fn parse_delimited_glossary(path: &Path, delimiter: char) -> Result<Vec<ParsedRagChunk>, RagError> {
    let text = read_text(path)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter as u8)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|error| RagError::InvalidSource(format!("{error}")))?
        .iter()
        .map(|header| header.trim().to_string())
        .collect();
    if headers.is_empty() || headers.iter().any(|header| header.is_empty()) {
        return Err(RagError::InvalidSource(
            "Delimited glossary requires non-empty headers".to_string(),
        ));
    }
    let mut unique_headers = headers.clone();
    unique_headers.sort();
    unique_headers.dedup();
    if unique_headers.len() != headers.len() {
        return Err(RagError::InvalidSource(
            "Delimited glossary contains duplicate headers".to_string(),
        ));
    }

    let mut chunks = Vec::new();
    for (offset, record) in reader.records().enumerate() {
        let record = record.map_err(|error| RagError::InvalidSource(format!("{error}")))?;
        let row_number = offset + 2;
        if record.len() > headers.len() {
            return Err(RagError::InvalidSource(format!(
                "Malformed delimited row {row_number}: extra fields"
            )));
        }
        let mut pairs: Vec<(String, Value)> = Vec::new();
        for (index, field) in record.iter().enumerate() {
            let value = field.trim();
            if value.is_empty() {
                continue;
            }
            pairs.push((headers[index].clone(), Value::String(value.to_string())));
        }
        if pairs.is_empty() {
            continue;
        }
        chunks.push(glossary_chunk(pairs, &format!("row {row_number}")));
    }
    Ok(chunks)
}

fn parse_json_glossary(path: &Path) -> Result<Vec<ParsedRagChunk>, RagError> {
    let text = read_text(path)?;
    let data = serde_json::from_str::<Value>(&text)
        .map_err(|error| RagError::InvalidSource(format!("{error}")))?;
    parse_structured_glossary(&data)
}

fn parse_yaml_glossary(path: &Path) -> Result<Vec<ParsedRagChunk>, RagError> {
    let text = read_text(path)?;
    let data = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text)
        .map_err(|error| RagError::InvalidSource(format!("invalid YAML glossary: {error}")))?;
    parse_structured_glossary(&yaml_to_json(&data))
}

fn parse_structured_glossary(data: &Value) -> Result<Vec<ParsedRagChunk>, RagError> {
    let mut chunks = Vec::new();
    match data {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                chunks.push(glossary_chunk(
                    metadata_pairs_from_value(item),
                    &format!("entry {}", index + 1),
                ));
            }
        }
        Value::Object(entries) => {
            for (index, (key, value)) in entries.iter().enumerate() {
                let mut pairs = metadata_pairs_from_value(value);
                pairs.retain(|(existing, _)| existing != "key");
                pairs.push(("key".to_string(), Value::String(key.clone())));
                chunks.push(glossary_chunk(pairs, &format!("entry {}", index + 1)));
            }
        }
        Value::Null => {}
        _ => {
            return Err(RagError::InvalidSource(
                "Glossary data must be a list or mapping".to_string(),
            ));
        }
    }
    Ok(chunks)
}

/// Python `_metadata_from_value`: mapping entries drop nulls, scalars become
/// `{"value": scalar}`.
fn metadata_pairs_from_value(value: &Value) -> Vec<(String, Value)> {
    match value {
        Value::Object(entries) => entries
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        other => vec![("value".to_string(), other.clone())],
    }
}

fn glossary_chunk(pairs: Vec<(String, Value)>, location: &str) -> ParsedRagChunk {
    let text = pairs
        .iter()
        .map(|(key, value)| format!("{key}: {}", scalar_display(value)))
        .collect::<Vec<_>>()
        .join("\n");
    let mut metadata = Map::new();
    for (key, value) in pairs {
        metadata.insert(key, value);
    }
    ParsedRagChunk {
        chunk_kind: "glossary_entry",
        display_text: text.clone(),
        text,
        location: location.to_string(),
        metadata,
    }
}

/// Python string interpolation of scalar values (strings render unquoted).
fn scalar_display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn yaml_to_json(value: &serde_yaml_ng::Value) -> Value {
    match value {
        serde_yaml_ng::Value::Null => Value::Null,
        serde_yaml_ng::Value::Bool(flag) => Value::Bool(*flag),
        serde_yaml_ng::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Value::from(integer)
            } else if let Some(unsigned) = number.as_u64() {
                Value::from(unsigned)
            } else {
                Value::from(number.as_f64().unwrap_or_default())
            }
        }
        serde_yaml_ng::Value::String(text) => Value::String(text.clone()),
        serde_yaml_ng::Value::Sequence(items) => {
            Value::Array(items.iter().map(yaml_to_json).collect())
        }
        serde_yaml_ng::Value::Mapping(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(yaml_key_to_string(key), yaml_to_json(value));
            }
            Value::Object(object)
        }
        serde_yaml_ng::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

fn yaml_key_to_string(key: &serde_yaml_ng::Value) -> String {
    match key {
        serde_yaml_ng::Value::String(text) => text.clone(),
        other => yaml_to_json(other).to_string(),
    }
}

// ---------------------------------------------------------------------------
// Text chunking (Python `_paragraphs` and friends)
// ---------------------------------------------------------------------------

/// Blocks separated by whitespace runs containing at least two newlines
/// (`re.split(r"\n\s*\n", ...)`), each block's lines joined by single spaces.
fn paragraphs(text: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut newlines_in_run = 0usize;
    for character in text.chars() {
        if character.is_whitespace() {
            if character == '\n' {
                newlines_in_run += 1;
            }
            current.push(character);
            continue;
        }
        if newlines_in_run >= 2 {
            let block = format_block(&current);
            if !block.is_empty() {
                blocks.push(block);
            }
            current.clear();
        }
        newlines_in_run = 0;
        current.push(character);
    }
    let block = format_block(&current);
    if !block.is_empty() {
        blocks.push(block);
    }
    blocks
}

fn format_block(block: &str) -> String {
    block
        .split('\n')
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn located_chunk_texts(text: &str, base_location: &str) -> Vec<(String, String)> {
    let parts = split_chunk_text(text);
    if parts.len() == 1 {
        return vec![(base_location.to_string(), parts[0].clone())];
    }
    parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| (format!("{base_location} part {}", index + 1), part))
        .collect()
}

fn split_chunk_text(text: &str) -> Vec<String> {
    let stripped = text.trim();
    if stripped.chars().count() <= MAX_RAG_CHUNK_CHARS {
        return vec![stripped.to_string()];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for segment in sentence_segments(stripped) {
        if segment.chars().count() > MAX_RAG_CHUNK_CHARS {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            chunks.extend(word_chunks(&segment));
            continue;
        }
        let candidate = format!("{current} {segment}").trim().to_string();
        if candidate.chars().count() <= MAX_RAG_CHUNK_CHARS {
            current = candidate;
        } else {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            current = segment;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Split after `.`, `!`, or `?` when followed by whitespace (`(?<=[.!?])\s+`).
fn sentence_segments(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut segments: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;
    while index < characters.len() {
        let is_boundary = matches!(characters[index], '.' | '!' | '?')
            && characters
                .get(index + 1)
                .is_some_and(|character| character.is_whitespace());
        if !is_boundary {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < characters.len() && characters[end].is_whitespace() {
            end += 1;
        }
        let segment: String = characters[start..index + 1].iter().collect();
        let trimmed = segment.trim();
        if !trimmed.is_empty() {
            segments.push(trimmed.to_string());
        }
        start = end;
        index = end;
    }
    let rest: String = characters[start..].iter().collect();
    let trimmed = rest.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }
    segments
}

fn word_chunks(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if word.chars().count() > MAX_RAG_CHUNK_CHARS {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            chunks.extend(hard_chunks(word));
            continue;
        }
        let candidate = format!("{current} {word}").trim().to_string();
        if candidate.chars().count() <= MAX_RAG_CHUNK_CHARS {
            current = candidate;
        } else {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn hard_chunks(text: &str) -> Vec<String> {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(MAX_RAG_CHUNK_CHARS)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

// ---------------------------------------------------------------------------
// Source normalization (Python `rag_conversion.py`)
// ---------------------------------------------------------------------------

pub struct NormalizedRagSource {
    pub path: PathBuf,
    pub format: String,
    pub original_path: PathBuf,
}

/// Pass supported sources through unchanged; convert HTML and DOCX into
/// managed markdown and PDF into managed text under `managed_root`
/// (Python mammoth / pypdf equivalents).
fn normalize_rag_source(path: &Path, managed_root: &Path) -> Result<NormalizedRagSource, RagError> {
    let suffix = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_lowercase();
    if suffix == "epub" {
        return Err(RagError::InvalidSource(
            "EPUB is unsupported; split EPUB into chapters before RAG import".to_string(),
        ));
    }
    let passthrough_format = match suffix.as_str() {
        "txt" => Some("text"),
        "md" | "markdown" => Some("markdown"),
        "csv" | "tsv" | "json" | "yaml" | "yml" => Some("glossary"),
        _ => None,
    };
    if let Some(format) = passthrough_format {
        return Ok(NormalizedRagSource {
            path: path.to_path_buf(),
            format: format.to_string(),
            original_path: path.to_path_buf(),
        });
    }
    if suffix == "html" {
        let markdown = html_to_markdown(&read_text(path)?);
        return write_managed_source(path, managed_root, &markdown, "markdown");
    }
    if suffix == "docx" {
        let markdown = docx_to_markdown(path)?;
        return write_managed_source(path, managed_root, &markdown, "markdown");
    }
    if suffix == "pdf" {
        let text = pdf_to_text(path)?;
        return write_managed_source(path, managed_root, &text, "text");
    }
    Err(RagError::InvalidSource(format!(
        "unsupported RAG source extension: .{suffix}"
    )))
}

/// Write converted content under `managed_root` keyed by the ORIGINAL source
/// checksum; the format selects the managed extension (`markdown` → `.md`,
/// `text` → `.txt`).
fn write_managed_source(
    source: &Path,
    managed_root: &Path,
    content: &str,
    format: &str,
) -> Result<NormalizedRagSource, RagError> {
    let normalized_text = format!("{}\n", content.trim());
    if normalized_text.trim().is_empty() {
        return Err(RagError::InvalidSource(format!(
            "source produced no extractable text: {}",
            source.display()
        )));
    }
    let extension = managed_extension(format);
    let checksum = sha256_hex(&std::fs::read(source)?);
    let destination = managed_root.join(format!("{checksum}.{extension}"));
    std::fs::create_dir_all(managed_root)?;
    std::fs::write(&destination, normalized_text)?;
    Ok(NormalizedRagSource {
        path: destination,
        format: format.to_string(),
        original_path: source.to_path_buf(),
    })
}

fn managed_extension(format: &str) -> &'static str {
    match format {
        "markdown" => "md",
        _ => "txt",
    }
}

/// Minimal DOCX-to-markdown conversion (Python used `mammoth`): the
/// `word/document.xml` member is read from the zip container, paragraphs
/// become blank-line-separated blocks, `Heading1`–`Heading6` paragraph styles
/// become ATX headings, and run text is concatenated as plain text.
/// Decompressed-size cap for the DOCX main document, so a zip bomb disguised
/// as a chapter file is rejected before being read into memory.
const MAX_DOCX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

fn docx_to_markdown(path: &Path) -> Result<String, RagError> {
    use std::io::Read as _;

    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|error| RagError::InvalidSource(format!("invalid DOCX source: {error}")))?;
    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|error| RagError::InvalidSource(format!("invalid DOCX source: {error}")))?;
    let uncompressed_size = entry.size();
    if uncompressed_size > MAX_DOCX_DOCUMENT_BYTES {
        return Err(RagError::InvalidSource(format!(
            "invalid DOCX source: word/document.xml uncompressed size {uncompressed_size} \
             exceeds limit {MAX_DOCX_DOCUMENT_BYTES}"
        )));
    }
    let mut document = String::new();
    entry
        .read_to_string(&mut document)
        .map_err(|error| RagError::InvalidSource(format!("invalid DOCX source: {error}")))?;
    docx_document_to_markdown(&document)
}

fn docx_document_to_markdown(document: &str) -> Result<String, RagError> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(document);
    let mut blocks: Vec<String> = Vec::new();
    let mut paragraph_text = String::new();
    let mut heading_level: Option<usize> = None;
    let mut inside_text_run = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                "w:p" => {
                    paragraph_text.clear();
                    heading_level = None;
                }
                "w:t" => inside_text_run = true,
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.name().as_ref() {
                "w:p" => {
                    paragraph_text.clear();
                    heading_level = None;
                }
                "w:pStyle" => {
                    if let Some(style) = event
                        .try_get_attribute("w:val")
                        .map_err(invalid_docx_source)?
                        .and_then(|attribute| {
                            attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .ok()
                        })
                    {
                        heading_level = docx_heading_level(&style);
                    }
                }
                "w:tab" => paragraph_text.push('\t'),
                "w:br" | "w:cr" => paragraph_text.push('\n'),
                _ => {}
            },
            Ok(Event::Text(event)) if inside_text_run => {
                paragraph_text.push_str(&event.xml10_content());
            }
            Ok(Event::GeneralRef(event)) if inside_text_run => {
                push_docx_entity(&event, &mut paragraph_text)?;
            }
            Ok(Event::End(event)) => match event.name().as_ref() {
                "w:t" => inside_text_run = false,
                "w:p" => {
                    let text = paragraph_text.trim();
                    if !text.is_empty() {
                        match heading_level {
                            Some(level) => blocks.push(format!("{} {text}", "#".repeat(level))),
                            None => blocks.push(text.to_string()),
                        }
                    }
                    paragraph_text.clear();
                    heading_level = None;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(invalid_docx_source(error)),
        }
    }
    Ok(blocks.join("\n\n"))
}

fn invalid_docx_source(error: impl std::fmt::Display) -> RagError {
    RagError::InvalidSource(format!("invalid DOCX source: {error}"))
}

/// Decode a general entity reference inside a text run: the five predefined
/// XML entities plus numeric character references.
fn push_docx_entity(
    entity: &quick_xml::events::BytesRef,
    text: &mut String,
) -> Result<(), RagError> {
    let decoded = match entity.as_ref() {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => match entity.resolve_char_ref() {
            Ok(Some(character)) => character,
            _ => {
                return Err(RagError::InvalidSource(format!(
                    "invalid DOCX source: unsupported entity reference: {}",
                    entity.as_ref()
                )));
            }
        },
    };
    text.push(decoded);
    Ok(())
}

/// `Heading1`–`Heading6` style ids (case-insensitive, spaces tolerated) map to
/// ATX heading levels like mammoth's default style map.
fn docx_heading_level(style: &str) -> Option<usize> {
    let normalized = style.replace(' ', "").to_lowercase();
    let digits = normalized.strip_prefix("heading")?;
    let level = digits.parse::<usize>().ok()?;
    (1..=6).contains(&level).then_some(level)
}

/// PDF text extraction (Python used `pypdf`); the output is treated as plain
/// text and lands as managed `.txt`. pdf-extract (via lopdf) panics on some
/// malformed inputs instead of returning an error, and `extract_text` only
/// touches the file at `path` (no shared state to poison), so the panic is
/// contained here. This requires unwinding panics; the workspace Cargo
/// profiles never set `panic = "abort"`.
fn pdf_to_text(path: &Path) -> Result<String, RagError> {
    std::panic::catch_unwind(|| pdf_extract::extract_text(path))
        .map_err(|payload| {
            let message = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_string());
            RagError::InvalidSource(format!(
                "invalid PDF source: pdf-extract panicked: {message}"
            ))
        })?
        .map_err(|error| RagError::InvalidSource(format!("invalid PDF source: {error}")))
}

/// Minimal HTML-to-markdown normalization: ATX headings, blank-line block
/// separation, list items, `<br>` line breaks, entity decoding; everything
/// else is unwrapped to its text.
fn html_to_markdown(html: &str) -> String {
    let without_comments = strip_html_region(html, "<!--", "-->");
    let without_scripts = strip_html_region_insensitive(&without_comments, "script");
    let without_styles = strip_html_region_insensitive(&without_scripts, "style");

    let characters: Vec<char> = without_styles.chars().collect();
    let mut markdown = String::new();
    let mut index = 0usize;
    while index < characters.len() {
        if characters[index] != '<' {
            markdown.push(characters[index]);
            index += 1;
            continue;
        }
        let Some(tag_end) = find_tag_end(&characters, index) else {
            markdown.push('<');
            index += 1;
            continue;
        };
        let tag: String = characters[index + 1..tag_end].iter().collect();
        index = tag_end + 1;
        let (closing, name) = split_tag(&tag);
        let name = name.to_lowercase();
        match (closing, name.as_str()) {
            (false, "br") | (true, "br") => markdown.push('\n'),
            (false, heading) if is_heading_tag(heading) => {
                push_block_break(&mut markdown);
                let level = heading[1..].parse::<usize>().unwrap_or(1);
                markdown.push_str(&"#".repeat(level));
                markdown.push(' ');
            }
            (true, heading) if is_heading_tag(heading) => push_block_break(&mut markdown),
            (false, "li") => {
                markdown.push('\n');
                markdown.push_str("- ");
            }
            (false, "td") | (true, "td") | (false, "th") | (true, "th") => {
                markdown.push(' ');
            }
            (_, name) if BLOCK_TAGS.contains(&name) => push_block_break(&mut markdown),
            _ => {}
        }
    }
    collapse_blank_lines(&decode_entities(&markdown))
        .trim()
        .to_string()
}

const BLOCK_TAGS: [&str; 15] = [
    "p",
    "div",
    "blockquote",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "table",
    "tr",
    "ul",
    "ol",
    "pre",
    "figure",
];

fn is_heading_tag(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 2 && bytes[0] == b'h' && bytes[1].is_ascii_digit() && bytes[1] != b'0'
}

fn push_block_break(markdown: &mut String) {
    if !markdown.is_empty() {
        markdown.push_str("\n\n");
    }
}

fn strip_html_region(text: &str, opening: &str, closing: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find(opening) {
        result.push_str(&remaining[..start]);
        match remaining[start + opening.len()..].find(closing) {
            Some(end) => {
                remaining = &remaining[start + opening.len() + end + closing.len()..];
            }
            None => {
                return result;
            }
        }
    }
    result.push_str(remaining);
    result
}

fn strip_html_region_insensitive(text: &str, tag: &str) -> String {
    // Case-insensitive scan for `<tag ...> ... </tag>`; content is dropped.
    let lowercase = text.to_lowercase();
    let mut result = String::new();
    let mut cursor = 0usize;
    let opening = format!("<{tag}");
    while let Some(relative) = lowercase[cursor..].find(&opening) {
        let start = cursor + relative;
        let Some(end_marker) = lowercase[start..].find(&format!("</{tag}")) else {
            break;
        };
        let end = start + end_marker + tag.len() + 3;
        result.push_str(&text[cursor..start]);
        cursor = end.min(text.len());
    }
    result.push_str(&text[cursor..]);
    result
}

fn find_tag_end(characters: &[char], start: usize) -> Option<usize> {
    let mut index = start + 1;
    let mut quote: Option<char> = None;
    while index < characters.len() {
        let character = characters[index];
        match quote {
            Some(active) if character == active => quote = None,
            Some(_) => {}
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                } else if character == '>' {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

fn split_tag(tag: &str) -> (bool, &str) {
    let tag = tag.trim_end_matches('/');
    if let Some(name) = tag.strip_prefix('/') {
        return (true, name.split_whitespace().next().unwrap_or(name));
    }
    (false, tag.split_whitespace().next().unwrap_or(tag))
}

fn collapse_blank_lines(markdown: &str) -> String {
    let mut result = String::with_capacity(markdown.len());
    let mut newline_run = 0usize;
    for character in markdown.chars() {
        if character == '\n' {
            newline_run += 1;
            if newline_run > 2 {
                continue;
            }
        } else {
            newline_run = 0;
        }
        result.push(character);
    }
    result
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        if characters[index] != '&' {
            result.push(characters[index]);
            index += 1;
            continue;
        }
        match decode_entity_at(&characters, index) {
            Some((decoded, next)) => {
                result.push_str(&decoded);
                index = next;
            }
            None => {
                result.push('&');
                index += 1;
            }
        }
    }
    result
}

fn decode_entity_at(characters: &[char], start: usize) -> Option<(String, usize)> {
    let mut index = start + 1;
    let mut name = String::new();
    while index < characters.len() && characters[index] != ';' && name.len() <= 10 {
        name.push(characters[index]);
        index += 1;
    }
    if index >= characters.len() || characters[index] != ';' {
        return None;
    }
    let decoded = match name.as_str() {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        _ => {
            if let Some(code) = name
                .strip_prefix('#')
                .and_then(|digits| digits.parse::<u32>().ok())
                .and_then(char::from_u32)
            {
                code
            } else {
                name.strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                    .and_then(char::from_u32)?
            }
        }
    };
    Some((decoded.to_string(), index + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn markdown_heading_extracts_level_and_text() {
        assert_eq!(markdown_heading("# Sense"), Some((1, "Sense".to_string())));
        assert_eq!(
            markdown_heading("#### Deep ##"),
            Some((4, "Deep".to_string()))
        );
        assert_eq!(markdown_heading("#hashtag"), None);
        assert_eq!(markdown_heading("####### seven"), None);
        assert_eq!(markdown_heading("#"), None);
    }

    #[test]
    fn paragraphs_split_on_blank_lines_and_join_lines() {
        assert_eq!(
            paragraphs("first\nsecond\n\nthird"),
            vec!["first second".to_string(), "third".to_string()]
        );
        assert_eq!(paragraphs("\n\n  \n"), Vec::<String>::new());
    }

    #[test]
    fn sentence_segments_split_after_terminators() {
        assert_eq!(
            sentence_segments("One. Two! Three? Four"),
            vec![
                "One.".to_string(),
                "Two!".to_string(),
                "Three?".to_string(),
                "Four".to_string()
            ]
        );
    }

    #[test]
    fn html_to_markdown_normalizes_headings_and_paragraphs() {
        assert_eq!(
            html_to_markdown("<h1>Sense</h1><p>Sense menu note.</p>"),
            "# Sense\n\nSense menu note."
        );
    }
}
