use std::sync::Arc;

use hiero_core::rag::{RagStore, RetrievalMode, SearchOptions};
use hiero_core::semantic::{
    EmbeddingProvider, FakeEmbeddingProvider, FakeSemanticIndex, LanceDbIndex,
    OrtEmbeddingProvider, SearchFilter, SemanticError, SemanticIndex, SemanticJobQueue,
    VectorRecord, reciprocal_rank_fusion,
};
use hiero_core::{
    domain::{AddCrystalInput, CrystalStore, MemorySource, TranslationContext, WorkspaceStore},
    recall::RecallService,
};

#[test]
fn reciprocal_rank_fusion_merges_lanes_and_uses_stable_id_ties() {
    let fused = reciprocal_rank_fusion(&[(20, 9.0), (10, 8.0)], &[(30, 0.1), (10, 0.2)], 60.0);

    assert_eq!(
        fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![10, 20, 30]
    );
    assert!(fused[0].1 > fused[1].1);
    assert_eq!(fused[1].1, fused[2].1);
}

async fn generation_contract(index: Arc<dyn SemanticIndex>) {
    let first = index.begin_rebuild().await.unwrap();
    index
        .upsert_vectors(
            first,
            vec![
                VectorRecord {
                    chunk_id: 10,
                    embedding: vec![1.0, 0.0],
                },
                VectorRecord {
                    chunk_id: 20,
                    embedding: vec![0.0, 1.0],
                },
            ],
        )
        .await
        .unwrap();
    assert!(
        index
            .search(
                &[1.0, 0.0],
                &SearchFilter {
                    series_slug: "book".into()
                },
                5
            )
            .await
            .is_err()
    );
    index.activate_generation(first).await.unwrap();
    let hits = index
        .search(
            &[1.0, 0.0],
            &SearchFilter {
                series_slug: "book".into(),
            },
            5,
        )
        .await
        .unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        vec![10, 20]
    );

    let second = index.begin_rebuild().await.unwrap();
    index
        .upsert_vectors(
            second,
            vec![VectorRecord {
                chunk_id: 30,
                embedding: vec![1.0, 0.0],
            }],
        )
        .await
        .unwrap();
    index.cancel_generation(second).await.unwrap();
    let hits = index
        .search(
            &[1.0, 0.0],
            &SearchFilter {
                series_slug: "book".into(),
            },
            5,
        )
        .await
        .unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        vec![10, 20]
    );

    assert_eq!(index.delete_vectors(&[10]).await.unwrap(), 1);
    let hits = index
        .search(
            &[1.0, 0.0],
            &SearchFilter {
                series_slug: "book".into(),
            },
            5,
        )
        .await
        .unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        vec![20]
    );
}

#[tokio::test]
async fn fake_index_obeys_generation_contract() {
    generation_contract(Arc::new(FakeSemanticIndex::new())).await;
}

#[tokio::test]
async fn temporary_lancedb_index_obeys_generation_contract() {
    let temp = tempfile::tempdir().unwrap();
    generation_contract(Arc::new(LanceDbIndex::open(temp.path()).await.unwrap())).await;
}

#[tokio::test]
async fn fake_embeddings_are_deterministic_and_validate_dimensions() {
    let provider = FakeEmbeddingProvider::new(4);
    let first = provider.embed_query("moon").await.unwrap();
    assert_eq!(first, provider.embed_query("moon").await.unwrap());
    assert_eq!(first.len(), 4);
    assert!(matches!(
        FakeEmbeddingProvider::new(0).embed_query("moon").await,
        Err(SemanticError::InvalidVector { .. })
    ));
}

#[tokio::test]
async fn ort_model_loading_is_lazy_and_missing_model_is_typed() {
    let temp = tempfile::tempdir().unwrap();
    let provider = OrtEmbeddingProvider::new_offline(temp.path());
    assert_eq!(provider.dimensions(), 384);
    assert!(matches!(
        provider.embed_query("moon").await,
        Err(SemanticError::MissingModel { .. })
    ));
}

#[tokio::test]
async fn ort_rejects_empty_and_corrupt_local_artifacts_without_network() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("model.onnx"), [])
        .await
        .unwrap();
    tokio::fs::write(temp.path().join("tokenizer.json"), [])
        .await
        .unwrap();
    let empty = OrtEmbeddingProvider::new_offline(temp.path());
    assert!(matches!(
        empty.embed_query("moon").await,
        Err(SemanticError::MissingModel { .. })
    ));

    tokio::fs::write(temp.path().join("model.onnx"), b"not onnx")
        .await
        .unwrap();
    tokio::fs::write(temp.path().join("tokenizer.json"), b"not json")
        .await
        .unwrap();
    let corrupt = OrtEmbeddingProvider::new_offline(temp.path());
    assert!(matches!(
        corrupt.embed_query("moon").await,
        Err(SemanticError::Ort(_))
    ));
}

#[tokio::test]
async fn ort_boundary_rejects_bad_batch_cardinality_and_non_finite_vectors() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("model.onnx"), b"injected")
        .await
        .unwrap();
    tokio::fs::write(temp.path().join("tokenizer.json"), b"injected")
        .await
        .unwrap();
    let cardinality =
        OrtEmbeddingProvider::with_inference(temp.path(), 2, Arc::new(|_| Ok(vec![])));
    assert!(matches!(
        cardinality.embed_query("moon").await,
        Err(SemanticError::InvalidVector { .. })
    ));
    let non_finite = OrtEmbeddingProvider::with_inference(
        temp.path(),
        2,
        Arc::new(|_| Ok(vec![vec![f32::NAN, 0.0]])),
    );
    assert!(matches!(
        non_finite.embed_query("moon").await,
        Err(SemanticError::InvalidVector { .. })
    ));
}

async fn semantic_pool() -> sqlx::SqlitePool {
    let url = format!(
        "sqlite:file:semantic-{}?mode=memory&cache=shared",
        uuid::Uuid::new_v4()
    );
    let pool = hiero_core::db::connect_url(&url).await.unwrap();
    sqlx::query("INSERT INTO series(slug, title, default_source_language, default_target_language, created_at, updated_at) VALUES ('book', 'Book', 'en', 'ru', ?, ?)")
        .bind(chrono::Utc::now()).bind(chrono::Utc::now()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO rag_sources(id, series_slug, source_ref, source_type, content_type, checksum, metadata_json, created_at, updated_at) VALUES (1, 'book', 'test', 'text', 'text/plain', 'source', '{}', ?, ?)")
        .bind(chrono::Utc::now()).bind(chrono::Utc::now()).execute(&pool).await.unwrap();
    for (id, text) in [(10_i64, "alpha"), (20, "beta"), (30, "gamma")] {
        sqlx::query("INSERT INTO rag_chunks(id, source_id, series_slug, chunk_kind, text, display_text, location, metadata_json, created_at) VALUES (?, 1, 'book', 'text', ?, ?, '', '{}', ?)")
            .bind(id).bind(text).bind(text).bind(chrono::Utc::now()).execute(&pool).await.unwrap();
    }
    pool
}

#[tokio::test]
async fn job_queue_claims_bounded_distinct_batches_and_tracks_checksums() {
    let pool = semantic_pool().await;
    let queue = SemanticJobQueue::new(&pool);
    let job = queue.enqueue_rebuild().await.unwrap();

    let first = queue.claim_next_batch(2).await.unwrap();
    let second = queue.claim_next_batch(2).await.unwrap();
    assert_eq!(
        first.iter().map(|chunk| chunk.id).collect::<Vec<_>>(),
        vec![10, 20]
    );
    assert_eq!(
        second.iter().map(|chunk| chunk.id).collect::<Vec<_>>(),
        vec![30]
    );

    let unassigned: Option<String> =
        sqlx::query_scalar("SELECT generation_id FROM semantic_index_jobs WHERE id = ?")
            .bind(job)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(unassigned, None, "the index owns generation allocation");
    let generation = uuid::Uuid::new_v4().to_string();
    queue
        .mark_indexed(&[10, 20, 30], &generation)
        .await
        .unwrap();
    assert!(queue.claim_next_batch(2).await.unwrap().is_empty());
    let status: String = sqlx::query_scalar("SELECT status FROM semantic_index_jobs WHERE id = ?")
        .bind(job)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "completed");
    let persisted_generation: String =
        sqlx::query_scalar("SELECT generation_id FROM semantic_index_jobs WHERE id = ?")
            .bind(job)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(persisted_generation, generation);

    sqlx::query("UPDATE rag_chunks SET text = 'alpha changed' WHERE id = 10")
        .execute(&pool)
        .await
        .unwrap();
    queue.enqueue_rebuild().await.unwrap();
    let stale = queue.claim_next_batch(10).await.unwrap();
    assert_eq!(
        stale.iter().map(|chunk| chunk.id).collect::<Vec<_>>(),
        vec![10]
    );
}

#[tokio::test]
async fn failed_job_is_reclaimable_and_generation_cannot_change_mid_job() {
    let pool = semantic_pool().await;
    let queue = SemanticJobQueue::new(&pool);
    let failed = queue.enqueue_rebuild().await.unwrap();
    assert_eq!(queue.claim_next_batch(1).await.unwrap()[0].id, 10);
    queue.mark_failed(failed, "injected failure").await.unwrap();

    queue.enqueue_rebuild().await.unwrap();
    assert_eq!(queue.claim_next_batch(1).await.unwrap()[0].id, 10);
    let first_generation = uuid::Uuid::new_v4().to_string();
    queue.mark_indexed(&[10], &first_generation).await.unwrap();
    let other_generation = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        queue.mark_indexed(&[20], &other_generation).await,
        Err(SemanticError::InvalidVector { .. })
    ));
    let state: Option<String> =
        sqlx::query_scalar("SELECT generation_id FROM semantic_chunk_state WHERE chunk_id = 20")
            .fetch_optional(&pool)
            .await
            .unwrap()
            .flatten();
    assert_ne!(state.as_deref(), Some(other_generation.as_str()));
}

#[tokio::test]
async fn assigned_rebuild_generation_reclaims_every_prior_generation_chunk() {
    let pool = semantic_pool().await;
    let queue = SemanticJobQueue::new(&pool);
    let old = uuid::Uuid::new_v4().to_string();
    queue.mark_indexed(&[10, 20, 30], &old).await.unwrap();
    let job = queue.enqueue_rebuild().await.unwrap();
    let next = uuid::Uuid::new_v4().to_string();
    queue.assign_generation(job, &next).await.unwrap();

    let claimed = queue.claim_next_batch(10).await.unwrap();
    assert_eq!(
        claimed.iter().map(|chunk| chunk.id).collect::<Vec<_>>(),
        vec![10, 20, 30]
    );
}

#[tokio::test]
async fn lancedb_with_sqlite_excludes_stale_vectors_and_wrong_series() {
    let pool = semantic_pool().await;
    let temp = tempfile::tempdir().unwrap();
    let index = LanceDbIndex::open_with_pool(temp.path(), pool.clone())
        .await
        .unwrap();
    let generation = index.begin_rebuild().await.unwrap();
    index
        .upsert_vectors(
            generation,
            vec![
                VectorRecord {
                    chunk_id: 10,
                    embedding: vec![1.0, 0.0],
                },
                VectorRecord {
                    chunk_id: 20,
                    embedding: vec![0.0, 1.0],
                },
                VectorRecord {
                    chunk_id: 30,
                    embedding: vec![0.0, 1.0],
                },
            ],
        )
        .await
        .unwrap();
    let queue = SemanticJobQueue::new(&pool);
    queue
        .mark_indexed(&[10, 20, 30], &generation.0.to_string())
        .await
        .unwrap();
    index.activate_generation(generation).await.unwrap();

    assert_eq!(
        index
            .search(
                &[1.0, 0.0],
                &SearchFilter {
                    series_slug: "book".into()
                },
                1
            )
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        index
            .search(
                &[1.0, 0.0],
                &SearchFilter {
                    series_slug: "other".into()
                },
                5
            )
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE rag_chunks SET text = 'changed' WHERE id = 10")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        index
            .search(
                &[1.0, 0.0],
                &SearchFilter {
                    series_slug: "book".into()
                },
                5
            )
            .await
            .unwrap()
            .iter()
            .all(|hit| hit.chunk_id != 10)
    );

    let partial = index.begin_rebuild().await.unwrap();
    index
        .upsert_vectors(
            partial,
            vec![VectorRecord {
                chunk_id: 20,
                embedding: vec![0.0, 1.0],
            }],
        )
        .await
        .unwrap();
    assert!(matches!(
        index.activate_generation(partial).await,
        Err(SemanticError::CorruptIndex { .. })
    ));
    assert_eq!(
        index.active_generation().await.unwrap().unwrap().id,
        generation
    );
}

#[tokio::test]
async fn corrupt_active_pointer_is_typed_and_health_is_degraded() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("ACTIVE"), "not-a-generation")
        .await
        .unwrap();
    let index = LanceDbIndex::open(temp.path()).await.unwrap();
    assert!(!index.health().await.unwrap().ok);
    assert!(matches!(
        index.active_generation().await,
        Err(SemanticError::CorruptIndex { .. })
    ));
}

#[tokio::test]
async fn hybrid_rag_degrades_to_fts_and_reports_semantic_health() {
    let pool = semantic_pool().await;
    let index = Arc::new(FakeSemanticIndex::new());
    let store = RagStore::with_semantic(&pool, Arc::new(FakeEmbeddingProvider::new(2)), index);
    let options = SearchOptions {
        limit: 5,
        retrieval_mode: RetrievalMode::Hybrid,
        ..Default::default()
    };
    let results = store.search("book", "alpha", options).await.unwrap();
    assert_eq!(
        results
            .iter()
            .map(|result| result.chunk.id)
            .collect::<Vec<_>>(),
        vec![10]
    );
    let diagnostic = store.semantic_diagnostic();
    assert!(!diagnostic.ok);
    assert!(diagnostic.message.unwrap().contains("active generation"));
}

#[tokio::test]
async fn hybrid_rag_fuses_vector_only_and_lexical_hits_stably() {
    let pool = semantic_pool().await;
    let embeddings = Arc::new(FakeEmbeddingProvider::new(2));
    let query = embeddings.embed_query("alpha").await.unwrap();
    let index = Arc::new(FakeSemanticIndex::new());
    let generation = index.begin_rebuild().await.unwrap();
    index
        .upsert_vectors(
            generation,
            vec![
                VectorRecord {
                    chunk_id: 20,
                    embedding: query.clone(),
                },
                VectorRecord {
                    chunk_id: 30,
                    embedding: vec![0.0, 1.0],
                },
            ],
        )
        .await
        .unwrap();
    index.activate_generation(generation).await.unwrap();
    let store = RagStore::with_semantic(&pool, embeddings, index);
    let options = SearchOptions {
        limit: 5,
        retrieval_mode: RetrievalMode::Hybrid,
        ..Default::default()
    };

    let results = store.search("book", "alpha", options).await.unwrap();
    assert_eq!(
        results
            .iter()
            .map(|result| result.chunk.id)
            .collect::<Vec<_>>(),
        vec![10, 20, 30]
    );
    assert!(store.semantic_diagnostic().ok);
}

#[tokio::test]
async fn semantic_recall_keeps_rule_lane_scoring_and_degrades_without_failure() {
    let pool = semantic_pool().await;
    let context = TranslationContext::new("book", "en", "ru");
    let session = WorkspaceStore::new(&pool)
        .start_session(&context, "translate", "1", "1")
        .await
        .unwrap();
    let mut rule = AddCrystalInput {
        crystal_type: "rule".into(),
        title: "Rule".into(),
        text: "Always render alpha as Альфа.".into(),
        scope_type: "series".into(),
        scope_key: "series:book".into(),
        series_slug: "book".into(),
        source_language: "en".into(),
        target_language: "ru".into(),
        rule_intent: "terminology".into(),
        source_credibility: "user_rule".into(),
        ..Default::default()
    };
    rule.strength = 0.8;
    rule.confidence = 0.8;
    let rule_id = CrystalStore::new(&pool).add(rule).await.unwrap();
    let service = RecallService::with_semantic(
        &pool,
        Arc::new(FakeEmbeddingProvider::new(2)),
        Arc::new(FakeSemanticIndex::new()),
    );

    let results = service
        .recall(session.id, &context, "alpha", 3)
        .await
        .unwrap();
    assert_eq!(results[0].id, rule_id);
    assert_eq!(results[0].source, MemorySource::LongTerm);
    assert!(
        results
            .iter()
            .any(|result| result.source == MemorySource::Rag)
    );
    assert!(!service.semantic_diagnostic().ok);
}
