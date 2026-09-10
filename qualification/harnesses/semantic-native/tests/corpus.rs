use hieronymus_semantic_native_qualification::{CorpusSpec, corpus_digest, generate_corpus};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("harness lives three levels below the repository root")
        .join(path)
}

#[test]
fn corpus_is_exactly_ten_thousand_chunks_and_fifty_queries() -> anyhow::Result<()> {
    let spec = CorpusSpec::load(fixture("qualification/fixtures/semantic-corpus.json"))?;
    let (chunks, queries) = generate_corpus(&spec)?;
    assert_eq!(chunks.len(), 10_000);
    assert_eq!(queries.len(), 50);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| &chunk.series_slug)
            .collect::<BTreeSet<_>>()
            .len(),
        100
    );
    assert_eq!(queries[0].text, "Cooking Talent qualification query 000");
    assert_eq!(queries[1].text, "Sense UI qualification query 001");
    assert_eq!(queries[0].eligible_series_slug, "qualification-series-000");
    assert_eq!(queries[0].target_chunk_id, "qualification-chunk-000-000");
    assert_eq!(chunks[0].token_ids, chunks[5_000].token_ids);
    assert_eq!(chunks[0].embedding, chunks[5_000].embedding);
    assert_ne!(chunks[0].checksum, chunks[5_000].checksum);
    let (chunks_again, queries_again) = generate_corpus(&spec)?;
    assert_eq!(
        corpus_digest(&chunks, &queries),
        corpus_digest(&chunks_again, &queries_again),
    );
    Ok(())
}
