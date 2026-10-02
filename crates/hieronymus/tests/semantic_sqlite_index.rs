use hieronymus::semantic_embeddings::{
    EmbeddingIdentity, EmbeddingProvider, FakeEmbeddingProvider,
};
use hieronymus::semantic_index::{IndexRow, VectorIndex, generation_table_intact};

fn identity() -> EmbeddingIdentity {
    FakeEmbeddingProvider::new(3).identity().clone()
}
fn row(id: i64, generation: &str, vector: Vec<f32>) -> IndexRow {
    let identity = identity();
    IndexRow {
        chunk_id: id,
        series_slug: "book".into(),
        checksum: format!("checksum-{id}"),
        generation_id: generation.into(),
        model: identity.model().into(),
        model_revision: identity.revision().into(),
        vector,
    }
}

#[test]
fn cosine_ranking_is_exact_scale_invariant_and_ties_use_chunk_id() {
    let root = tempfile::tempdir().unwrap();
    let mut index = VectorIndex::open(root.path(), identity(), "g").unwrap();
    index
        .append(vec![
            row(3, "g", vec![0., 1., 0.]),
            row(2, "g", vec![10., 0., 0.]),
            row(1, "g", vec![1., 0., 0.]),
            row(4, "g", vec![-1., 0., 0.]),
        ])
        .unwrap();
    let hits = index.search("book", &[2., 0., 0.], 4).unwrap();
    assert_eq!(
        hits.iter().map(|h| h.chunk_id).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(
        hits.iter().map(|h| h.distance).collect::<Vec<_>>(),
        [0., 0., 1., 2.]
    );
    assert_eq!(index.search("book", &[2., 0., 0.], 2).unwrap(), hits[..2]);
}

#[test]
fn validation_and_constraint_failure_roll_back_whole_batch() {
    let root = tempfile::tempdir().unwrap();
    let mut index = VectorIndex::open(root.path(), identity(), "g").unwrap();
    let good = row(1, "g", vec![1., 0., 0.]);
    assert!(
        index
            .append(vec![good.clone(), row(2, "g", vec![0.; 3])])
            .is_err()
    );
    assert_eq!(index.count_rows().unwrap(), 0);
    assert!(index.append(vec![good.clone(), good.clone()]).is_err());
    assert_eq!(
        index.count_rows().unwrap(),
        0,
        "first insert rolls back on duplicate second row"
    );
    index.append(vec![good]).unwrap();
    for bad in [
        vec![0.; 3],
        vec![f32::NAN; 3],
        vec![f32::INFINITY; 3],
        vec![1.; 2],
    ] {
        assert!(index.search("book", &bad, 10).is_err());
    }
    assert!(index.search("book", &[1., 0., 0.], 0).is_err());
}

#[test]
fn reopening_checks_complete_identity_and_generation_names_do_not_collide() {
    let root = tempfile::tempdir().unwrap();
    let original = identity();
    let mut first = VectorIndex::open(root.path(), original.clone(), "a-b").unwrap();
    first.append(vec![row(1, "a-b", vec![1., 0., 0.])]).unwrap();
    drop(first);
    let second = VectorIndex::open(root.path(), original.clone(), "a_b").unwrap();
    assert_eq!(second.count_rows().unwrap(), 0);
    let changed = EmbeddingIdentity::new(
        original.provider(),
        original.model(),
        original.revision(),
        original.dimensions(),
        original.normalization(),
        "different-tokenizer",
        original.max_input_tokens(),
        original.max_batch_inputs(),
    )
    .unwrap();
    assert!(VectorIndex::open(root.path(), changed, "a-b").is_err());
    assert_eq!(
        VectorIndex::open(root.path(), original, "a-b")
            .unwrap()
            .count_rows()
            .unwrap(),
        1
    );
}

#[test]
fn missing_and_legacy_artifacts_are_not_ready_and_probe_creates_nothing() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("generation_g.lance")).unwrap();
    assert!(!generation_table_intact(root.path(), "g", &identity(), 0));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn published_generation_survives_reopen_and_incomplete_next_generation() {
    let root = tempfile::tempdir().unwrap();
    let mut old = VectorIndex::open(root.path(), identity(), "old").unwrap();
    old.append(vec![row(1, "old", vec![1., 0., 0.])]).unwrap();
    let mut next = VectorIndex::open(root.path(), identity(), "next").unwrap();
    next.append(vec![row(2, "next", vec![0., 1., 0.])]).unwrap();
    drop(next);
    drop(old);
    assert!(generation_table_intact(root.path(), "old", &identity(), 1));
    let reopened = VectorIndex::open(root.path(), identity(), "old").unwrap();
    assert_eq!(
        reopened.search("book", &[1., 0., 0.], 10).unwrap()[0].chunk_id,
        1
    );
}
