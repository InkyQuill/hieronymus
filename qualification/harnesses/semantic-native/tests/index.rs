use hieronymus_semantic_native_qualification::{
    GenerationIndex, IndexRow, ModelIdentity, OnnxEmbeddingProvider, corpus_digest,
    generate_corpus, index_input_digest,
};
use std::path::{Path, PathBuf};

const MODEL_SHA256: &str = "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452";
const ADVERSARIAL_SERIES: &str = "eligible";
const NUM_PARTITIONS: usize = 16;
const EMBEDDING_DIMENSIONS: usize = 384;

fn qualification_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("harness lives two levels below the qualification root")
        .to_path_buf()
}

fn artifact(relative: &str) -> PathBuf {
    qualification_root().join(".artifacts").join(relative)
}

/// The fixed adversarial query vector: a unit vector along axis 0.
fn needle() -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    vector[0] = 1.0;
    vector
}

fn expected_eligible_ids() -> Vec<i64> {
    (1..=10).collect()
}

/// Unit vector rotated `angle` radians away from the needle within the
/// (axis 0, axis 1 + `fan`) plane. Cosine distance to the needle grows
/// monotonically with `angle`, so rankings are stable under ANN quantization.
fn rotated_vector(angle: f32, fan: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    vector[0] = angle.cos();
    vector[1 + (fan % (EMBEDDING_DIMENSIONS - 1))] = angle.sin();
    vector
}

fn adversarial_rows() -> Vec<IndexRow> {
    let mut rows = Vec::new();
    // One hundred ineligible decoys, all strictly closer to the needle than
    // every eligible vector: their angles (0.055..0.555 rad) stay below the
    // smallest eligible angle (0.65 rad), so a global top-10 starves the
    // eligible series under post-filtering.
    for id in 11..=110_i64 {
        let decoy_index = (id - 11) as f32;
        rows.push(IndexRow {
            chunk_id: id,
            series_slug: "ineligible".to_string(),
            checksum: format!("decoy-checksum-{id:03}"),
            generation_id: "generation-a".to_string(),
            vector: rotated_vector(0.055 + 0.005 * decoy_index, (id - 11) as usize),
        });
    }
    // Ten eligible vectors, all farther from the needle than every decoy:
    // distances 2 * (1 - cos(0.6 + 0.05 * i)) for i in 1..=10. Only the
    // series pre-filter can get them back into a top-10 result.
    for id in 1..=10_i64 {
        rows.push(IndexRow {
            chunk_id: id,
            series_slug: ADVERSARIAL_SERIES.to_string(),
            checksum: format!("eligible-checksum-{id:02}"),
            generation_id: "generation-a".to_string(),
            vector: rotated_vector(0.6 + 0.05 * id as f32, 0),
        });
    }
    // Distant filler rows pad the training set past the 256-row minimum of an
    // 8-bit PQ codebook without touching the adversarial ordering: at angle
    // pi/2 they are farther from the needle than every eligible vector.
    for offset in 0..150_i64 {
        rows.push(IndexRow {
            chunk_id: 1000 + offset,
            series_slug: "filler".to_string(),
            checksum: format!("filler-checksum-{offset:03}"),
            generation_id: "generation-a".to_string(),
            vector: rotated_vector(std::f32::consts::FRAC_PI_2, offset as usize),
        });
    }
    rows
}

async fn populated_adversarial_index() -> anyhow::Result<GenerationIndex> {
    // The store is intentionally leaked: the brief requires this helper to
    // hand back only the index handle, so the on-disk store must outlive it.
    let root = tempfile::tempdir()?.keep();
    let identity = ModelIdentity::new("all-MiniLM-L6-v2", EMBEDDING_DIMENSIONS, MODEL_SHA256)?;
    let mut index = GenerationIndex::create(&root, identity, "generation-a").await?;
    index.append(adversarial_rows()).await?;
    index.create_ann_index().await?;
    Ok(index)
}

#[tokio::test]
async fn search_uses_series_prefilter_before_ann() -> anyhow::Result<()> {
    let index = populated_adversarial_index().await?;
    let plan = index
        .explain_prefiltered_search(ADVERSARIAL_SERIES, &needle(), 10)
        .await?;
    assert!(plan.ann_index_used);
    assert!(plan.series_predicate_below_ann);
    assert_eq!(plan.eligible_cardinality, 10);
    assert_eq!(plan.requested_top_k, 10);
    assert!(plan.returned_count > 0);
    // The evidence record must accept this plan as a proven pre-filter.
    plan.require_ann_prefilter()?;
    let hits = index.search(ADVERSARIAL_SERIES, &needle(), 10).await?;
    assert_eq!(hits.len(), 10);
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        expected_eligible_ids()
    );
    // The adversarial control: a global top-k with post-filtering cannot
    // recover the eligible rows because 100 closer decoys crowd them out.
    let postfiltered = index
        .search_postfiltered(ADVERSARIAL_SERIES, &needle(), 10)
        .await?;
    assert!(
        postfiltered.len() < 10,
        "post-filtered search leaked through: {postfiltered:?}"
    );
    Ok(())
}

#[tokio::test]
async fn flat_scan_or_missing_index_fails_the_proof() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?.keep();
    let identity = ModelIdentity::new("all-MiniLM-L6-v2", EMBEDDING_DIMENSIONS, MODEL_SHA256)?;
    let mut index = GenerationIndex::create(&root, identity, "generation-a").await?;
    index.append(adversarial_rows()).await?;
    // No ANN index exists yet: the explainable proof must refuse to claim one.
    let plan = index
        .explain_prefiltered_search(ADVERSARIAL_SERIES, &needle(), 10)
        .await?;
    assert!(!plan.ann_index_used);
    // The evidence record must reject a plan without ANN index use.
    assert!(
        plan.require_ann_prefilter().is_err(),
        "a flat-scan plan must not pass the pre-filter proof"
    );
    let evidence = index.verify().await?;
    assert_eq!(evidence.num_indexed_rows, 0);
    index.create_ann_index().await?;
    let evidence = index.verify().await?;
    assert_eq!(evidence.index_type, "IVF_PQ");
    assert_eq!(evidence.distance_metric, "cosine");
    assert_eq!(evidence.num_partitions, NUM_PARTITIONS);
    assert_eq!(evidence.num_unindexed_rows, 0);
    assert_eq!(evidence.num_indexed_rows, 260);
    Ok(())
}

#[tokio::test]
async fn ann_roundtrip_append_find_delete_and_refresh() -> anyhow::Result<()> {
    let mut index = populated_adversarial_index().await?;
    let synthetic = IndexRow {
        chunk_id: 111,
        series_slug: ADVERSARIAL_SERIES.to_string(),
        checksum: "synthetic-checksum-111".to_string(),
        generation_id: "generation-a".to_string(),
        vector: rotated_vector(0.005, 0),
    };
    index.append(vec![synthetic.clone()]).await?;
    let hits = index.search(ADVERSARIAL_SERIES, &needle(), 1).await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk_id, 111);
    assert_eq!(hits[0].checksum, "synthetic-checksum-111");

    let deleted = index
        .delete_chunk(ADVERSARIAL_SERIES, synthetic.chunk_id)
        .await?;
    assert_eq!(deleted, 1);
    index.create_ann_index().await?;
    let hits = index.search(ADVERSARIAL_SERIES, &needle(), 11).await?;
    assert_eq!(hits.len(), 10);
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        expected_eligible_ids()
    );
    Ok(())
}

#[tokio::test]
async fn incomplete_second_generation_stays_isolated() -> anyhow::Result<()> {
    let mut index = populated_adversarial_index().await?;
    assert_eq!(index.active_generation(), "generation-a");
    let identity = ModelIdentity::new("all-MiniLM-L6-v2", EMBEDDING_DIMENSIONS, MODEL_SHA256)?;
    let mut second = GenerationIndex::create(index.root(), identity, "generation-b").await?;
    // The second generation is incomplete: one interesting row plus filler
    // that its own IVF-PQ index needs for training.
    let mut rows = vec![IndexRow {
        chunk_id: 7,
        series_slug: ADVERSARIAL_SERIES.to_string(),
        checksum: "generation-b-only".to_string(),
        generation_id: "generation-b".to_string(),
        vector: needle(),
    }];
    for offset in 0..255_i64 {
        rows.push(IndexRow {
            chunk_id: 2000 + offset,
            series_slug: "filler".to_string(),
            checksum: format!("generation-b-filler-{offset:03}"),
            generation_id: "generation-b".to_string(),
            vector: rotated_vector(std::f32::consts::FRAC_PI_2, offset as usize),
        });
    }
    second.append(rows).await?;
    second.create_ann_index().await?;
    // The incomplete second generation only sees its own rows.
    let hits = second.search(ADVERSARIAL_SERIES, &needle(), 10).await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].checksum, "generation-b-only");
    // The active first generation is untouched by the second one.
    let hits = index.search(ADVERSARIAL_SERIES, &needle(), 10).await?;
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        expected_eligible_ids()
    );
    // Activating another generation rebinds the handle to its table.
    index.activate("generation-b").await?;
    assert_eq!(index.active_generation(), "generation-b");
    let hits = index.search(ADVERSARIAL_SERIES, &needle(), 10).await?;
    assert_eq!(hits[0].checksum, "generation-b-only");
    Ok(())
}

#[test]
fn full_corpus_generation_is_bit_identical_across_runs() -> anyhow::Result<()> {
    let spec = hieronymus_semantic_native_qualification::CorpusSpec::load(
        qualification_root().join("fixtures/semantic-corpus.json"),
    )?;
    let (chunks, queries) = generate_corpus(&spec)?;
    let (chunks_again, queries_again) = generate_corpus(&spec)?;
    assert_eq!(chunks.len(), 10_000);
    assert_eq!(queries.len(), 50);
    assert_eq!(
        corpus_digest(&chunks, &queries),
        corpus_digest(&chunks_again, &queries_again)
    );
    let rows: Vec<IndexRow> = chunks
        .iter()
        .map(|chunk| IndexRow::from_corpus_chunk(chunk))
        .collect();
    let rows_again: Vec<IndexRow> = chunks_again
        .iter()
        .map(|chunk| IndexRow::from_corpus_chunk(chunk))
        .collect();
    assert_eq!(index_input_digest(&rows), index_input_digest(&rows_again));
    // Every row carries the deterministic per-chunk checksum and generation.
    assert_eq!(rows.len(), 10_000);
    assert!(
        rows.iter()
            .all(|row| row.vector.len() == EMBEDDING_DIMENSIONS)
    );
    assert!(
        rows.iter()
            .all(|row| row.generation_id == "generation-a" || row.generation_id == "generation-b")
    );
    // Derived chunk ids feed a validator that requires positivity: not one of
    // the 10,000 rows may derive to a non-positive id.
    let non_positive: Vec<i64> = rows
        .iter()
        .filter(|row| row.chunk_id <= 0)
        .map(|row| row.chunk_id)
        .collect();
    assert!(
        non_positive.is_empty(),
        "corpus-derived chunk ids must be positive, {} of 10,000 are non-positive (first: {:?})",
        non_positive.len(),
        non_positive.first()
    );
    Ok(())
}

#[test]
fn onnx_provider_rejects_checksum_mismatch() {
    let provider = OnnxEmbeddingProvider::load(
        &artifact("models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so"),
        &artifact("models/all-MiniLM-L6-v2/model.onnx"),
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    let error = provider.err().expect("checksum mismatch must be rejected");
    assert!(
        error.to_string().to_lowercase().contains("checksum"),
        "unexpected error: {error}"
    );
}

#[test]
fn onnx_provider_embeds_mean_pooled_l2_normalized_deterministic_vectors() -> anyhow::Result<()> {
    let mut provider = OnnxEmbeddingProvider::load(
        &artifact("models/onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so"),
        &artifact("models/all-MiniLM-L6-v2/model.onnx"),
        MODEL_SHA256,
    )?;
    assert_eq!(provider.model_checksum(), MODEL_SHA256);
    let tokens: Vec<u32> = (1010..1042).collect();
    let first = provider.embed(&tokens)?;
    let second = provider.embed(&tokens)?;
    assert_eq!(first.len(), EMBEDDING_DIMENSIONS);
    assert_eq!(first, second);
    let norm: f32 = first.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-4,
        "embedding must be L2 normalized, got {norm}"
    );
    let other = provider.embed(&(2010..2042).collect::<Vec<u32>>())?;
    assert_ne!(first, other);
    Ok(())
}
