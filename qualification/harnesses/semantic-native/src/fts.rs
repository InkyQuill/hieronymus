//! Strengthened FTS5 fallback over the deterministic corpus.
//!
//! This module compiles in every configuration — including
//! `--no-default-features`, where the ONNX model, runtime, and LanceDB index
//! are absent. Nothing here touches the network, so a download probe can
//! never fire on this path. The fallback builds a disposable external-content
//! FTS5 table over the generated corpus rows and answers each corpus query
//! with a series-scoped full-text search whose result ids are proven equal to
//! the ids derived independently from the corpus recipe.
//!
//! Digests cover ids only; no embedding vector ever enters this module.

use std::path::Path;

use anyhow::{Context, ensure};
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::corpus::{self, Query};

/// Number of corpus queries the FTS fallback must answer.
pub const FTS_QUERY_COUNT: usize = 50;

/// One series-scoped FTS search result. Counts, ids, and digests only.
#[derive(Clone, Debug, Serialize)]
pub struct FtsReceipt {
    pub query_id: String,
    pub series_slug: String,
    pub match_token: String,
    pub matched_ids: Vec<String>,
    pub eligible_count: usize,
    pub cross_series_count: usize,
    pub id_digest: String,
}

/// Canonical SHA-256 digest over an ordered id list. Both the independent
/// expectation and the SQLite search result are digested through this
/// function, so equality of digests is equality of the ordered id lists.
pub fn fts_id_digest(ids: &[String]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"hieronymus-fts-v1");
    digest.update(format!("/count/{}", ids.len()).as_bytes());
    for id in ids {
        digest.update(b"\n");
        digest.update(id.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Digests a receipt list in order.
pub fn digest_receipts(receipts: &[FtsReceipt]) -> Vec<String> {
    receipts
        .iter()
        .map(|receipt| receipt.id_digest.clone())
        .collect()
}

fn deterministic_corpus() -> anyhow::Result<(Vec<corpus::Chunk>, Vec<Query>)> {
    let spec = corpus::load_default_spec().context("could not load the corpus recipe")?;
    corpus::generate_corpus(&spec)
}

/// Independently derives each query's eligible ids from the deterministic
/// corpus recipe before any SQLite database is opened, requires at least one
/// eligible id per query, and returns 50 ordered SHA-256 digests.
pub fn expected_fts_id_digests() -> anyhow::Result<Vec<String>> {
    let (chunks, queries) = deterministic_corpus()?;
    ensure!(
        queries.len() == FTS_QUERY_COUNT,
        "corpus recipe produced {} queries, expected {FTS_QUERY_COUNT}",
        queries.len()
    );
    let mut digests = Vec::with_capacity(queries.len());
    for query in &queries {
        let mut eligible: Vec<String> = chunks
            .iter()
            .filter(|chunk| chunk.series_slug == query.eligible_series_slug)
            .map(|chunk| chunk.id.clone())
            .collect();
        eligible.sort();
        ensure!(
            !eligible.is_empty(),
            "query {} has no eligible ids for series {}",
            query.id,
            query.eligible_series_slug
        );
        digests.push(fts_id_digest(&eligible));
    }
    Ok(digests)
}

/// The FTS5 token that identifies a series inside every chunk body of that
/// series: the zero-padded series number suffix of the slug.
fn series_match_token(series_slug: &str) -> anyhow::Result<String> {
    const PREFIX: &str = "qualification-series-";
    let token = series_slug.strip_prefix(PREFIX).context(format!(
        "series slug {series_slug:?} lacks the {PREFIX:?} prefix"
    ))?;
    ensure!(
        !token.is_empty() && token.chars().all(|c| c.is_ascii_digit()),
        "series slug {series_slug:?} has no numeric series token"
    );
    Ok(token.to_string())
}

/// A disposable external-content FTS5 database over the generated corpus
/// rows. The database lives in a temporary directory (helpers) or beneath a
/// caller-supplied path (CLI); either way it is disposable state.
pub struct FtsFallback {
    _dir: Option<tempfile::TempDir>,
    connection: Connection,
    queries: Vec<Query>,
    eligible_ids: Vec<Vec<String>>,
}

impl FtsFallback {
    /// Builds a temporary FTS database over the full corpus.
    pub fn open() -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("fts-fallback.sqlite3");
        Self::open_at(&path).map(|fallback| Self {
            connection: fallback.connection,
            queries: fallback.queries,
            eligible_ids: fallback.eligible_ids,
            _dir: Some(dir),
        })
    }

    /// Builds the FTS database at an explicit path (used by the CLI beneath
    /// its validated work directory). Any previous database is discarded:
    /// the store is disposable by construction.
    pub fn open_at(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        let connection =
            Connection::open(path).with_context(|| format!("could not open {}", path.display()))?;
        connection.execute_batch(
            "pragma journal_mode = wal;
             create table fts_chunks (
                 rowid integer primary key,
                 chunk_id text not null unique,
                 series_slug text not null,
                 body text not null,
                 checksum text not null
             );
             create virtual table fts_body using fts5(
                 body, content='fts_chunks', content_rowid='rowid'
             );",
        )?;
        let (chunks, queries) = deterministic_corpus()?;
        {
            let mut insert = connection.prepare(
                "insert into fts_chunks(rowid, chunk_id, series_slug, body, checksum)
                 values (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (index, chunk) in chunks.iter().enumerate() {
                insert.execute((
                    (index + 1) as i64,
                    chunk.id.as_str(),
                    chunk.series_slug.as_str(),
                    chunk.text.as_str(),
                    chunk.checksum.as_str(),
                ))?;
            }
        }
        // Populate the FTS index from the content table.
        connection.execute("insert into fts_body(fts_body) values('rebuild')", [])?;
        let eligible_ids = queries
            .iter()
            .map(|query| {
                let mut eligible: Vec<String> = chunks
                    .iter()
                    .filter(|chunk| chunk.series_slug == query.eligible_series_slug)
                    .map(|chunk| chunk.id.clone())
                    .collect();
                eligible.sort();
                ensure!(
                    !eligible.is_empty(),
                    "query {} has no eligible ids",
                    query.id
                );
                Ok(eligible)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            _dir: None,
            connection,
            queries,
            eligible_ids,
        })
    }

    pub fn query_count(&self) -> usize {
        self.queries.len()
    }

    /// The series a query is eligible for (derived from the recipe).
    pub fn eligible_series_slug(&self, query_index: usize) -> &str {
        &self.queries[query_index].eligible_series_slug
    }

    fn search_all(&self, series_slug: &str) -> anyhow::Result<(Vec<String>, String)> {
        let token = series_match_token(series_slug)?;
        let match_argument = format!("\"{token}\"");
        let mut statement = self.connection.prepare(
            "select c.chunk_id
             from fts_body b join fts_chunks c on c.rowid = b.rowid
             where fts_body match ?1 and c.series_slug = ?2
             order by c.chunk_id",
        )?;
        let matched: Vec<String> = statement
            .query_map((&match_argument, series_slug), |row| row.get(0))?
            .collect::<Result<_, _>>()
            .context("FTS search failed")?;
        Ok((matched, token))
    }

    /// Runs one series-scoped FTS search. The `series_slug` predicate is part
    /// of the SQLite query itself, never a post-filter. The matched ids come
    /// back ordered by the SQL `order by`, and the receipt digest covers them
    /// through the same canonical digest as the independent expectation.
    pub fn search(&self, query_index: usize, series_slug: &str) -> anyhow::Result<FtsReceipt> {
        ensure!(
            query_index < self.queries.len(),
            "query index {query_index} is outside the 0..{} corpus query range",
            self.queries.len()
        );
        let query = &self.queries[query_index];
        let (matched, token) = self.search_all(series_slug)?;
        let eligible = &self.eligible_ids[query_index];
        let eligible_count = matched
            .iter()
            .filter(|id| eligible.binary_search(id).is_ok())
            .count();
        let cross_series_count = matched.len() - eligible_count;
        let id_digest = fts_id_digest(&matched);
        Ok(FtsReceipt {
            query_id: query.id.clone(),
            series_slug: series_slug.to_string(),
            match_token: token,
            matched_ids: matched,
            eligible_count,
            cross_series_count,
            id_digest,
        })
    }

    /// Deletes every entry of the FTS index (`delete-all`).
    pub fn delete_index(&self) -> anyhow::Result<()> {
        self.connection
            .execute("insert into fts_body(fts_body) values('delete-all')", [])?;
        Ok(())
    }

    /// Rebuilds the FTS index from the content table (`rebuild`).
    pub fn rebuild_index(&self) -> anyhow::Result<()> {
        self.connection
            .execute("insert into fts_body(fts_body) values('rebuild')", [])?;
        Ok(())
    }
}

/// Runs all 50 corpus queries against a fresh temporary FTS database.
pub fn run_all_fts_queries() -> anyhow::Result<Vec<FtsReceipt>> {
    let fallback = FtsFallback::open()?;
    run_queries_on(&fallback)
}

/// Proves delete/rebuild equivalence on one disposable database: the digests
/// before the delete, and after a `delete-all` plus `rebuild` cycle, must
/// both equal the independently derived expectation.
pub fn delete_and_rebuild_fts() -> anyhow::Result<()> {
    let expected = expected_fts_id_digests()?;
    let fallback = FtsFallback::open()?;
    let before = run_queries_on(&fallback)?;
    ensure!(
        digest_receipts(&before) == expected,
        "pre-delete FTS digests drift from the corpus recipe"
    );
    fallback.delete_index()?;
    let series = fallback.eligible_series_slug(0).to_string();
    let emptied = fallback.search(0, &series)?;
    ensure!(
        emptied.matched_ids.is_empty(),
        "FTS index still returns {} rows after delete-all",
        emptied.matched_ids.len()
    );
    fallback.rebuild_index()?;
    let after = run_queries_on(&fallback)?;
    ensure!(
        digest_receipts(&after) == expected,
        "rebuilt FTS digests differ from the corpus recipe"
    );
    Ok(())
}

fn run_queries_on(fallback: &FtsFallback) -> anyhow::Result<Vec<FtsReceipt>> {
    let mut receipts = Vec::with_capacity(fallback.query_count());
    for query_index in 0..fallback.query_count() {
        let series = fallback.eligible_series_slug(query_index).to_string();
        receipts.push(fallback.search(query_index, &series)?);
    }
    Ok(receipts)
}
