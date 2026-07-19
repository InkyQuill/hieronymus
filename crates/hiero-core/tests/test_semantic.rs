use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

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

#[test]
fn reciprocal_rank_fusion_counts_only_the_first_occurrence_per_lane() {
    let fused = reciprocal_rank_fusion(
        &[(10, 9.0), (10, 8.0), (20, 7.0)],
        &[(30, 0.1), (30, 0.2), (20, 0.3)],
        60.0,
    );

    assert_eq!(
        fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![20, 10, 30]
    );
    assert_eq!(fused[1].1, fused[2].1);
}

async fn generation_contract(index: Arc<dyn SemanticIndex>) {
    let first = index.begin_rebuild().await.unwrap();
    assert!(matches!(
        index
            .upsert_vectors(
                first,
                vec![
                    VectorRecord {
                        chunk_id: 99,
                        embedding: vec![1.0, 0.0],
                    },
                    VectorRecord {
                        chunk_id: 99,
                        embedding: vec![0.0, 1.0],
                    },
                ],
            )
            .await,
        Err(SemanticError::InvalidVector { .. })
    ));
    assert_eq!(
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
            .unwrap(),
        2
    );
    assert!(matches!(
        index
            .upsert_vectors(
                first,
                vec![VectorRecord {
                    chunk_id: 40,
                    embedding: vec![1.0, 0.0, 0.0],
                }],
            )
            .await,
        Err(SemanticError::InvalidVector { .. })
    ));
    assert_eq!(
        index
            .upsert_vectors(
                first,
                vec![VectorRecord {
                    chunk_id: 20,
                    embedding: vec![0.0, 1.0],
                }],
            )
            .await
            .unwrap(),
        1,
        "upsert returns input records, including replacements"
    );
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

async fn empty_generation_contract(index: Arc<dyn SemanticIndex>) {
    let generation = index.begin_rebuild().await.unwrap();
    index.activate_generation(generation).await.unwrap();
    assert_eq!(
        index
            .active_generation()
            .await
            .unwrap()
            .unwrap()
            .vector_count,
        0
    );
    assert!(
        index
            .search(
                &[1.0, 0.0],
                &SearchFilter {
                    series_slug: "book".into(),
                },
                5,
            )
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(index.delete_vectors(&[10]).await.unwrap(), 0);
}

#[tokio::test]
async fn fake_index_supports_an_empty_generation() {
    empty_generation_contract(Arc::new(FakeSemanticIndex::new())).await;
}

#[tokio::test]
async fn lancedb_index_supports_an_empty_generation() {
    let temp = tempfile::tempdir().unwrap();
    empty_generation_contract(Arc::new(LanceDbIndex::open(temp.path()).await.unwrap())).await;
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

#[tokio::test]
async fn ort_runtime_initializes_once_under_concurrency_and_retries_failures() {
    let temp = tempfile::tempdir().unwrap();
    tokio::fs::write(temp.path().join("model.onnx"), b"injected")
        .await
        .unwrap();
    tokio::fs::write(temp.path().join("tokenizer.json"), b"injected")
        .await
        .unwrap();
    let initializes = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(OrtEmbeddingProvider::with_initializer(temp.path(), 2, {
        let initializes = initializes.clone();
        Arc::new(move |_, _, _| {
            initializes.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(|texts: &[String]| {
                Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
            }))
        })
    }));
    let (first, second) = tokio::join!(provider.embed_query("moon"), provider.embed_query("stars"));
    assert_eq!(first.unwrap(), vec![1.0, 0.0]);
    assert_eq!(second.unwrap(), vec![1.0, 0.0]);
    assert_eq!(initializes.load(Ordering::SeqCst), 1);

    let attempts = Arc::new(AtomicUsize::new(0));
    let retrying = OrtEmbeddingProvider::with_initializer(temp.path(), 2, {
        let attempts = attempts.clone();
        Arc::new(move |_, _, _| {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(SemanticError::Ort("injected initialization failure".into()));
            }
            Ok(Arc::new(|texts: &[String]| {
                Ok(texts.iter().map(|_| vec![0.0, 1.0]).collect())
            }))
        })
    });
    assert!(matches!(
        retrying.embed_query("first").await,
        Err(SemanticError::Ort(_))
    ));
    assert_eq!(retrying.embed_query("retry").await.unwrap(), vec![0.0, 1.0]);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
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
    queue.mark_indexed(&first, &generation).await.unwrap();
    queue.mark_indexed(&second, &generation).await.unwrap();
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
async fn mark_failed_is_terminal_observable_and_old_tokens_cannot_mutate() {
    let pool = semantic_pool().await;
    let queue = SemanticJobQueue::new(&pool);
    let failed = queue.enqueue_rebuild().await.unwrap();
    let failed_claim = queue.claim_next_batch(1).await.unwrap();
    assert_eq!(failed_claim[0].id, 10);
    queue
        .mark_failed(&failed_claim, "injected failure")
        .await
        .unwrap();

    assert_eq!(failed, failed_claim.job_id());
    assert!(queue.claim_next_batch(1).await.unwrap().is_empty());
    let (status, last_error, failed_at): (String, String, Option<String>) = sqlx::query_as(
        "SELECT status, last_error, failed_at FROM semantic_index_jobs WHERE id = ?",
    )
    .bind(failed)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(last_error, "injected failure");
    assert!(failed_at.is_some());
    let first_generation = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        queue.mark_indexed(&failed_claim, &first_generation).await,
        Err(SemanticError::InvalidVector { .. })
    ));
    assert!(matches!(
        queue.mark_failed(&failed_claim, "late failure").await,
        Err(SemanticError::InvalidVector { .. })
    ));
}

#[tokio::test]
async fn expired_claims_are_reowned_but_live_leases_and_max_attempts_are_enforced() {
    let pool = semantic_pool().await;
    sqlx::query("DELETE FROM rag_chunks WHERE id IN (20, 30)")
        .execute(&pool)
        .await
        .unwrap();
    let instant = Arc::new(std::sync::Mutex::new(
        "2026-07-19T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap(),
    ));
    let queue = SemanticJobQueue::with_clock(&pool, chrono::Duration::seconds(5), {
        let instant = instant.clone();
        Arc::new(move || *instant.lock().unwrap())
    });
    let job = queue.enqueue_rebuild().await.unwrap();
    let first = queue.claim_next_batch(1).await.unwrap();
    assert!(queue.claim_next_batch(1).await.unwrap().is_empty());

    *instant.lock().unwrap() += chrono::Duration::seconds(6);
    let second = queue.claim_next_batch(1).await.unwrap();
    assert_ne!(first.token(), second.token());
    assert!(matches!(
        queue
            .mark_indexed(&first, &uuid::Uuid::new_v4().to_string())
            .await,
        Err(SemanticError::InvalidVector { .. })
    ));
    let attempts: i64 = sqlx::query_scalar(
        "SELECT attempts FROM semantic_batch_claims WHERE job_id = ? AND chunk_id = 10",
    )
    .bind(job)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(attempts, 2);

    *instant.lock().unwrap() += chrono::Duration::seconds(6);
    let third = queue.claim_next_batch(1).await.unwrap();
    assert_ne!(second.token(), third.token());
    *instant.lock().unwrap() += chrono::Duration::seconds(6);
    assert!(matches!(
        queue.claim_next_batch(1).await,
        Err(SemanticError::InvalidVector { .. })
    ));
    let (status, error): (String, String) =
        sqlx::query_as("SELECT status, last_error FROM semantic_index_jobs WHERE id = ?")
            .bind(job)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert!(error.contains("exceeded 3 claim attempts"));
    let attempts: i64 = sqlx::query_scalar(
        "SELECT attempts FROM semantic_batch_claims WHERE job_id = ? AND chunk_id = 10",
    )
    .bind(job)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(attempts, 3);
    assert!(queue.claim_next_batch(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn empty_second_claim_does_not_complete_a_job_with_an_in_flight_batch() {
    let pool = semantic_pool().await;
    let queue = SemanticJobQueue::new(&pool);
    let job = queue.enqueue_rebuild().await.unwrap();
    let first = queue.claim_next_batch(10).await.unwrap();
    assert_eq!(first.len(), 3);

    assert!(queue.claim_next_batch(10).await.unwrap().is_empty());
    let status: String = sqlx::query_scalar("SELECT status FROM semantic_index_jobs WHERE id = ?")
        .bind(job)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "running");

    let generation = uuid::Uuid::new_v4().to_string();
    queue.mark_indexed(&first, &generation).await.unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM semantic_index_jobs WHERE id = ?")
        .bind(job)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "completed");
}

#[tokio::test]
async fn hydration_error_rolls_back_claim_and_leaves_chunk_reclaimable() {
    let pool = semantic_pool().await;
    sqlx::query("UPDATE rag_chunks SET metadata_json = 'not-json' WHERE id = 10")
        .execute(&pool)
        .await
        .unwrap();
    let queue = SemanticJobQueue::new(&pool);
    queue.enqueue_rebuild().await.unwrap();

    assert!(matches!(
        queue.claim_next_batch(1).await,
        Err(SemanticError::CorruptIndex { .. })
    ));
    let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM semantic_batch_claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 0);

    sqlx::query("UPDATE rag_chunks SET metadata_json = '{}' WHERE id = 10")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(queue.claim_next_batch(1).await.unwrap()[0].id, 10);
}

#[tokio::test]
async fn assigned_rebuild_generation_reclaims_every_prior_generation_chunk() {
    let pool = semantic_pool().await;
    let queue = SemanticJobQueue::new(&pool);
    let old = uuid::Uuid::new_v4().to_string();
    let seed_job = queue.enqueue_rebuild().await.unwrap();
    queue.assign_generation(seed_job, &old).await.unwrap();
    let seed = queue.claim_next_batch(10).await.unwrap();
    queue.mark_indexed(&seed, &old).await.unwrap();
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
    let job = queue.enqueue_rebuild().await.unwrap();
    queue
        .assign_generation(job, &generation.0.to_string())
        .await
        .unwrap();
    let claim = queue.claim_next_batch(10).await.unwrap();
    queue
        .mark_indexed(&claim, &generation.0.to_string())
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
async fn partial_rebuild_state_does_not_invalidate_the_active_generation() {
    let pool = semantic_pool().await;
    let temp = tempfile::tempdir().unwrap();
    let index = LanceDbIndex::open_with_pool(temp.path(), pool.clone())
        .await
        .unwrap();
    let active = index.begin_rebuild().await.unwrap();
    index
        .upsert_vectors(
            active,
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
    let job = queue.enqueue_rebuild().await.unwrap();
    queue
        .assign_generation(job, &active.0.to_string())
        .await
        .unwrap();
    let claim = queue.claim_next_batch(10).await.unwrap();
    queue
        .mark_indexed(&claim, &active.0.to_string())
        .await
        .unwrap();
    index.activate_generation(active).await.unwrap();

    let rebuilding = index.begin_rebuild().await.unwrap();
    index
        .upsert_vectors(
            rebuilding,
            vec![VectorRecord {
                chunk_id: 10,
                embedding: vec![1.0, 0.0],
            }],
        )
        .await
        .unwrap();
    let job = queue.enqueue_rebuild().await.unwrap();
    queue
        .assign_generation(job, &rebuilding.0.to_string())
        .await
        .unwrap();
    let partial = queue.claim_next_batch(1).await.unwrap();
    queue
        .mark_indexed(&partial, &rebuilding.0.to_string())
        .await
        .unwrap();

    let filter = SearchFilter {
        series_slug: "book".into(),
    };
    assert!(
        index
            .search(&[1.0, 0.0], &filter, 5,)
            .await
            .unwrap()
            .iter()
            .any(|hit| hit.chunk_id == 10)
    );
    assert_eq!(index.active_generation().await.unwrap().unwrap().id, active);

    sqlx::query("UPDATE rag_chunks SET text = 'changed alpha' WHERE id = 10")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        index
            .search(&[1.0, 0.0], &filter, 5)
            .await
            .unwrap()
            .iter()
            .all(|hit| hit.chunk_id != 10)
    );
}

#[tokio::test]
async fn lancedb_search_backfills_past_more_than_four_limits_of_stale_hits() {
    let pool = semantic_pool().await;
    for id in 100_i64..130 {
        let text = format!("chunk-{id}");
        sqlx::query("INSERT INTO rag_chunks(id, source_id, series_slug, chunk_kind, text, display_text, location, metadata_json, created_at) VALUES (?, 1, 'book', 'text', ?, ?, '', '{}', ?)")
            .bind(id).bind(&text).bind(&text).bind(chrono::Utc::now())
            .execute(&pool).await.unwrap();
    }
    let temp = tempfile::tempdir().unwrap();
    let index = LanceDbIndex::open_with_pool(temp.path(), pool.clone())
        .await
        .unwrap();
    let generation = index.begin_rebuild().await.unwrap();
    let vectors = [10_i64, 20, 30]
        .into_iter()
        .chain(100_i64..130)
        .map(|id| VectorRecord {
            chunk_id: id,
            embedding: if id < 125 {
                vec![1.0, (id as f32) / 100_000.0]
            } else {
                vec![0.0, 1.0]
            },
        })
        .collect::<Vec<_>>();
    index.upsert_vectors(generation, vectors).await.unwrap();
    let queue = SemanticJobQueue::new(&pool);
    let job = queue.enqueue_rebuild().await.unwrap();
    queue
        .assign_generation(job, &generation.0.to_string())
        .await
        .unwrap();
    let claim = queue.claim_next_batch(100).await.unwrap();
    queue
        .mark_indexed(&claim, &generation.0.to_string())
        .await
        .unwrap();
    index.activate_generation(generation).await.unwrap();

    sqlx::query("UPDATE rag_chunks SET text = text || '-stale' WHERE id < 125")
        .execute(&pool)
        .await
        .unwrap();
    let hits = index
        .search(
            &[1.0, 0.0],
            &SearchFilter {
                series_slug: "book".into(),
            },
            3,
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().all(|hit| hit.chunk_id >= 125));
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

#[cfg(unix)]
#[tokio::test]
async fn lancedb_rejects_symlinked_root_generations_entries_and_active_pointer() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let linked_root = temp.path().join("linked-root");
    symlink(outside.path(), &linked_root).unwrap();
    assert!(matches!(
        LanceDbIndex::open(&linked_root).await,
        Err(SemanticError::CorruptIndex { .. })
    ));
    let linked_parent = temp.path().join("linked-parent");
    symlink(outside.path(), &linked_parent).unwrap();
    assert!(matches!(
        LanceDbIndex::open(linked_parent.join("child")).await,
        Err(SemanticError::CorruptIndex { .. })
    ));

    let root = temp.path().join("index");
    tokio::fs::create_dir(&root).await.unwrap();
    symlink(outside.path(), root.join("generations")).unwrap();
    assert!(matches!(
        LanceDbIndex::open(&root).await,
        Err(SemanticError::CorruptIndex { .. })
    ));

    tokio::fs::remove_file(root.join("generations"))
        .await
        .unwrap();
    let index = LanceDbIndex::open(&root).await.unwrap();
    let generation = index.begin_rebuild().await.unwrap();
    index.cancel_generation(generation).await.unwrap();
    symlink(outside.path(), index_path(&root, generation)).unwrap();
    assert!(matches!(
        index
            .upsert_vectors(
                generation,
                vec![VectorRecord {
                    chunk_id: 1,
                    embedding: vec![1.0],
                }],
            )
            .await,
        Err(SemanticError::CorruptIndex { .. })
    ));

    tokio::fs::remove_file(index_path(&root, generation))
        .await
        .unwrap();
    let generation = index.begin_rebuild().await.unwrap();
    let outside_active = outside.path().join("active");
    tokio::fs::write(&outside_active, b"untouched")
        .await
        .unwrap();
    symlink(&outside_active, root.join("ACTIVE")).unwrap();
    assert!(matches!(
        index.activate_generation(generation).await,
        Err(SemanticError::CorruptIndex { .. })
    ));
    assert_eq!(
        tokio::fs::read(&outside_active).await.unwrap(),
        b"untouched"
    );
}

#[cfg(unix)]
fn index_path(
    root: &std::path::Path,
    generation: hiero_core::semantic::GenerationId,
) -> std::path::PathBuf {
    root.join("generations").join(generation.0.to_string())
}

#[tokio::test]
async fn activation_and_cancellation_are_serialized_across_index_handles() {
    let temp = tempfile::tempdir().unwrap();
    let first = Arc::new(LanceDbIndex::open(temp.path()).await.unwrap());
    let second = Arc::new(LanceDbIndex::open(temp.path()).await.unwrap());
    let generation = first.begin_rebuild().await.unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));

    let activate = {
        let first = first.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            first.activate_generation(generation).await
        })
    };
    let cancel = {
        let second = second.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            second.cancel_generation(generation).await
        })
    };
    barrier.wait().await;
    let activated = activate.await.unwrap();
    let cancelled = cancel.await.unwrap();
    let active = first.active_generation().await;

    match (activated, cancelled) {
        (Ok(()), Err(SemanticError::Cancelled)) => {
            assert_eq!(active.unwrap().unwrap().id, generation);
        }
        (Err(SemanticError::MissingGeneration(id)), Ok(())) => {
            assert_eq!(id, generation);
            assert!(active.unwrap().is_none());
        }
        result => panic!("lifecycle operations were not serialized: {result:?}"),
    }
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
