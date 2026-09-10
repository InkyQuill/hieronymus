use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const EXPECTED_SCHEMA_VERSION: u32 = 1;
const EXPECTED_SEED: &str = "hieronymus-semantic-qualification-v1";
const EXPECTED_SERIES_COUNT: usize = 100;
const EXPECTED_CHUNKS_PER_SERIES: usize = 100;
const EXPECTED_QUERY_COUNT: usize = 50;
const EXPECTED_EMBEDDING_DIMENSIONS: usize = 384;
const EXPECTED_GENERATIONS: [&str; 2] = ["generation-a", "generation-b"];
const EXPECTED_SERIES_FORMAT: &str = "qualification-series-{index:03}";
const EXPECTED_CHUNK_FORMAT: &str = "synthetic literary memory {series_index:03}/{chunk_index:03}";
const EXPECTED_QUERY_SELECTION: &str =
    "queries 0..49 target series 0..49 and chunk index (query_index * 17) % 100";
const RAG_QUERY_FIXTURE: &str =
    "compatibility/fixtures/mcp/hieronymus_rag_search/success.input.json";
const RECALL_QUERY_FIXTURE: &str =
    "compatibility/fixtures/mcp/hieronymus_recall/success.input.json";

#[derive(Clone, Debug, Deserialize)]
pub struct CorpusSpec {
    pub schema_version: u32,
    pub seed: String,
    pub series_count: usize,
    pub chunks_per_series: usize,
    pub query_count: usize,
    pub embedding_dimensions: usize,
    pub generation_ids: Vec<String>,
    pub series_slug_format: String,
    pub chunk_text_format: String,
    pub query_selection: String,
    #[serde(skip)]
    repository_root: PathBuf,
    #[serde(skip)]
    query_seeds: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chunk {
    pub id: String,
    pub series_slug: String,
    pub generation_id: String,
    pub text: String,
    pub token_ids: Vec<u32>,
    pub checksum: String,
    pub embedding: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Query {
    pub id: String,
    pub text: String,
    pub eligible_series_slug: String,
    pub target_chunk_id: String,
    pub token_ids: Vec<u32>,
    pub embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct QuerySeed {
    query: String,
}

impl CorpusSpec {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path)
            .with_context(|| format!("could not read corpus recipe {}", path.display()))?;
        let mut spec: Self = serde_json::from_slice(&bytes).context("invalid corpus recipe")?;
        spec.validate()?;
        let repository_root = path
            .ancestors()
            .nth(3)
            .context("corpus recipe must live under qualification/fixtures")?
            .to_path_buf();
        spec.query_seeds = [RAG_QUERY_FIXTURE, RECALL_QUERY_FIXTURE]
            .into_iter()
            .map(|relative| load_query_seed(&repository_root.join(relative)))
            .collect::<anyhow::Result<Vec<_>>>()?;
        spec.repository_root = repository_root;
        Ok(spec)
    }

    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == EXPECTED_SCHEMA_VERSION,
            "unsupported corpus schema"
        );
        ensure!(self.seed == EXPECTED_SEED, "unexpected corpus seed");
        ensure!(
            self.series_count == EXPECTED_SERIES_COUNT,
            "unexpected series count"
        );
        ensure!(
            self.chunks_per_series == EXPECTED_CHUNKS_PER_SERIES,
            "unexpected chunks-per-series count"
        );
        ensure!(
            self.query_count == EXPECTED_QUERY_COUNT,
            "unexpected query count"
        );
        ensure!(
            self.embedding_dimensions == EXPECTED_EMBEDDING_DIMENSIONS,
            "unexpected embedding dimensions"
        );
        ensure!(
            self.generation_ids == EXPECTED_GENERATIONS,
            "unexpected generation identifiers"
        );
        ensure!(
            self.series_slug_format == EXPECTED_SERIES_FORMAT,
            "unexpected series format"
        );
        ensure!(
            self.chunk_text_format == EXPECTED_CHUNK_FORMAT,
            "unexpected chunk format"
        );
        ensure!(
            self.query_selection == EXPECTED_QUERY_SELECTION,
            "unexpected query selection"
        );
        Ok(())
    }

    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }
}

/// Loads the pinned corpus recipe from the repository fixture path, resolved
/// at compile time from this crate's manifest directory (never `$HOME`). The
/// path is built from manifest ancestors so it stays free of `..` components,
/// which `CorpusSpec::load` relies on to recover the repository root.
pub fn load_default_spec() -> anyhow::Result<CorpusSpec> {
    let qualification_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .context("the harness lives two levels below the qualification root")?;
    CorpusSpec::load(qualification_root.join("fixtures/semantic-corpus.json"))
}

fn load_query_seed(path: &Path) -> anyhow::Result<String> {
    let bytes =
        fs::read(path).with_context(|| format!("could not read query seed {}", path.display()))?;
    let seed: QuerySeed = serde_json::from_slice(&bytes).context("invalid query seed")?;
    ensure!(!seed.query.is_empty(), "query seed must not be empty");
    Ok(seed.query)
}

fn synthetic_digest(seed: &str, series_index: usize, chunk_index: usize) -> [u8; 32] {
    Sha256::new()
        .chain_update(seed.as_bytes())
        .chain_update((series_index as u32).to_be_bytes())
        .chain_update((chunk_index as u32).to_be_bytes())
        .finalize()
        .into()
}

fn token_ids(digest: &[u8; 32]) -> Vec<u32> {
    digest
        .chunks_exact(4)
        .map(|word| u32::from_be_bytes(word.try_into().expect("four-byte digest word")))
        .collect()
}

fn fake_vector(digest: &[u8; 32], dimensions: usize) -> Vec<f32> {
    (0..dimensions)
        .map(|index| f32::from(digest[index % digest.len()]) / 127.5 - 1.0)
        .collect()
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn series_slug(index: usize) -> String {
    format!("qualification-series-{index:03}")
}

fn chunk_id(series_index: usize, chunk_index: usize) -> String {
    format!("qualification-chunk-{series_index:03}-{chunk_index:03}")
}

pub fn generate_corpus(spec: &CorpusSpec) -> anyhow::Result<(Vec<Chunk>, Vec<Query>)> {
    spec.validate()?;
    ensure!(
        spec.query_seeds.len() == 2,
        "the two compatibility query seeds are required"
    );

    let mut chunks = Vec::with_capacity(spec.series_count * spec.chunks_per_series);
    for series_index in 0..spec.series_count {
        for chunk_index in 0..spec.chunks_per_series {
            let digest = synthetic_digest(&spec.seed, series_index, chunk_index);
            let token_series_index = series_index % (spec.series_count / 2);
            let token_digest = synthetic_digest(&spec.seed, token_series_index, chunk_index);
            chunks.push(Chunk {
                id: chunk_id(series_index, chunk_index),
                series_slug: series_slug(series_index),
                generation_id: spec.generation_ids[series_index % spec.generation_ids.len()]
                    .clone(),
                text: format!("synthetic literary memory {series_index:03}/{chunk_index:03}"),
                token_ids: token_ids(&token_digest),
                checksum: hex_digest(&digest),
                embedding: fake_vector(&token_digest, spec.embedding_dimensions),
            });
        }
    }

    let queries = (0..spec.query_count)
        .map(|query_index| {
            let chunk_index = (query_index * 17) % spec.chunks_per_series;
            let digest = synthetic_digest(&spec.seed, query_index, chunk_index);
            Query {
                id: format!("qualification-query-{query_index:03}"),
                text: format!(
                    "{} qualification query {query_index:03}",
                    spec.query_seeds[query_index % spec.query_seeds.len()]
                ),
                eligible_series_slug: series_slug(query_index),
                target_chunk_id: chunk_id(query_index, chunk_index),
                token_ids: token_ids(&digest),
                embedding: fake_vector(&digest, spec.embedding_dimensions),
            }
        })
        .collect();
    Ok((chunks, queries))
}

pub fn corpus_digest(chunks: &[Chunk], queries: &[Query]) -> String {
    let mut digest = Sha256::new();
    let bytes = serde_json::to_vec(&(chunks, queries))
        .expect("serializing in-memory corpus values cannot fail");
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}
