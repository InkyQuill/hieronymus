use hiero_core::{
    db::connect_url,
    domain::{
        AddConceptFacetInput, ConceptFilter, ConceptProposalStore, ConceptStore,
        CreateConceptInput, CreateProposalInput, UpdateConceptFacetInput, UpdateConceptInput,
    },
};
use sqlx::Row;
use uuid::Uuid;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:concepts-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .expect("test database should migrate")
}

#[tokio::test]
async fn typed_updates_persist_all_mcp_concept_and_facet_fields() {
    let pool = pool().await;
    let store = ConceptStore::new(&pool);
    let concept = store
        .create(concept("Holo", "series", "series:oso"))
        .await
        .unwrap();

    let updated = store
        .update(
            concept.id,
            UpdateConceptInput {
                description: Some("Wise wolf".into()),
                status: Some("established".into()),
                confidence: Some(0.9),
            },
        )
        .await
        .expect("concept metadata should update atomically");
    assert_eq!(
        (
            updated.description.as_str(),
            updated.status.as_str(),
            updated.confidence
        ),
        ("Wise wolf", "established", 0.9)
    );

    let facet = store
        .add_facet_typed(AddConceptFacetInput {
            concept_id: concept.id,
            language: "ru".into(),
            facet_type: "rendering".into(),
            value: "Холо".into(),
            language_tags: vec!["ru".into(), "RU".into()],
            story_scopes: vec!["volume:1".into()],
            semantic_tags: vec!["character:name".into()],
            source_crystal_id: None,
            confidence: 0.8,
            is_canonical: true,
        })
        .await
        .expect("full facet should be inserted");
    let hydrated = store.list_facets(concept.id).await.unwrap();
    assert_eq!(hydrated[0].language_tags, ["ru"]);
    assert_eq!(hydrated[0].story_scopes, ["volume:1"]);
    assert_eq!(hydrated[0].semantic_tags, ["character:name"]);
    assert_eq!((facet.confidence, facet.is_canonical), (0.8, true));

    store
        .update_facet_typed(
            facet.id,
            UpdateConceptFacetInput {
                value: Some("Хоро".into()),
                language: Some("ja-Latn".into()),
                facet_type: Some("alias".into()),
                language_tags: Some(vec!["ja-Latn".into()]),
                story_scopes: Some(vec!["volume:2".into()]),
                semantic_tags: Some(Vec::new()),
                source_crystal_id: None,
                confidence: Some(0.7),
                is_canonical: Some(false),
            },
        )
        .await
        .expect("full facet should update atomically");
    let hydrated = store.list_facets(concept.id).await.unwrap();
    assert_eq!(
        (
            hydrated[0].value.as_str(),
            hydrated[0].language.as_str(),
            hydrated[0].facet_type.as_str(),
            hydrated[0].confidence,
            hydrated[0].is_canonical,
        ),
        ("Хоро", "ja-latn", "alias", 0.7, false)
    );
    assert_eq!(hydrated[0].language_tags, ["ja-latn"]);
    assert_eq!(hydrated[0].story_scopes, ["volume:2"]);
    assert!(hydrated[0].semantic_tags.is_empty());
}

#[tokio::test]
async fn typed_updates_reject_invalid_status_confidence_and_missing_source_crystal() {
    let pool = pool().await;
    let store = ConceptStore::new(&pool);
    let concept = store.create(concept("Holo", "global", "")).await.unwrap();

    assert!(
        store
            .update(
                concept.id,
                UpdateConceptInput {
                    description: None,
                    status: Some("unknown".into()),
                    confidence: None,
                },
            )
            .await
            .is_err()
    );
    assert!(
        store
            .add_facet_typed(AddConceptFacetInput {
                concept_id: concept.id,
                language: "ru".into(),
                facet_type: "rendering".into(),
                value: "Холо".into(),
                language_tags: vec![],
                story_scopes: vec![],
                semantic_tags: vec![],
                source_crystal_id: Some(999),
                confidence: 1.1,
                is_canonical: false,
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rename_preserves_source_crystal_provenance_on_the_former_label() {
    let pool = pool().await;
    let store = ConceptStore::new(&pool);
    let concept = store
        .create(concept("Holo", "series", "series:oso"))
        .await
        .unwrap();
    let now = chrono::Utc::now();
    let crystal_id: i64 = sqlx::query(
        "INSERT INTO crystals(crystal_type,text,title,scope_type,scope_key,series_slug,source_language,target_language,tags_json,strength,confidence,source_credibility,rule_intent,status,created_at,updated_at) VALUES('lesson','rename evidence','', 'series','series:oso','oso','ja','ru','[]',0.5,0.5,'observation','','active',?,?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap()
    .last_insert_rowid();

    store
        .rename_concept_with_source(concept.id, "Horo", "mcp", Some(crystal_id))
        .await
        .expect("rename should retain the supplied provenance");

    let former = store
        .list_facets(concept.id)
        .await
        .unwrap()
        .into_iter()
        .find(|facet| facet.facet_type == "former_label")
        .expect("rename should create a former-label facet");
    assert_eq!(former.source_crystal_id, Some(crystal_id));
}

fn concept(name: &str, scope_type: &str, scope_key: &str) -> CreateConceptInput {
    CreateConceptInput {
        canonical_name: name.into(),
        scope_type: scope_type.into(),
        scope_key: scope_key.into(),
    }
}

#[tokio::test]
async fn concept_lifecycle_facets_rename_and_search_are_deterministic() {
    let pool = pool().await;
    let store = ConceptStore::new(&pool);
    let created = store
        .create(concept("Yun", "series", "series:oso"))
        .await
        .expect("concept should be created");
    let ja = store
        .add_facet(created.id, "ja", "name", "ユン")
        .await
        .expect("facet should be added");
    let ru = store
        .add_facet(created.id, "ru", "rendering", "Юн")
        .await
        .expect("facet should be added");
    store
        .set_canonical_facet(ru.id)
        .await
        .expect("canonical rendering should be selected");
    let facets = store
        .list_facets(created.id)
        .await
        .expect("facets should hydrate in one domain call");
    assert_eq!(
        facets.iter().map(|f| f.id).collect::<Vec<_>>(),
        [ja.id, ru.id]
    );
    assert_eq!(facets[0].language_tags, ["ja"]);
    assert!(facets[1].is_canonical);

    let renamed = store
        .rename_concept(created.id, "Yun / ユン", "canonical spelling")
        .await
        .expect("active concept should rename");
    assert_eq!(renamed.canonical_name, "Yun / ユン");
    let hits = store
        .search(
            "Юн",
            ConceptFilter {
                scope_type: "series".into(),
                scope_key: "series:oso".into(),
                status: None,
            },
        )
        .await
        .expect("facet FTS should find the concept");
    assert_eq!(
        hits.iter().map(|row| row.id).collect::<Vec<_>>(),
        [created.id]
    );
    let rename_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM concept_renames WHERE concept_id = ?")
            .bind(created.id)
            .fetch_one(&pool)
            .await
            .expect("rename audit should be queryable");
    assert_eq!(rename_count, 1);
    let former_labels: Vec<String> = sqlx::query_scalar(
        "SELECT value FROM concept_facets WHERE concept_id = ? AND facet_type = 'former_label' ORDER BY id",
    )
    .bind(created.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(former_labels, ["Yun"]);
}

#[tokio::test]
async fn merge_is_atomic_preserves_relationships_and_rejects_cycles() {
    let pool = pool().await;
    let store = ConceptStore::new(&pool);
    let source = store
        .create(concept("Sense", "series", "series:oso"))
        .await
        .unwrap();
    let target = store
        .create(concept("Sense skill", "series", "series:oso"))
        .await
        .unwrap();
    let source_ja = store
        .add_facet(source.id, "ja", "name", "センス")
        .await
        .unwrap();
    let source_en = store
        .add_facet(source.id, "en", "name", "センス")
        .await
        .unwrap();
    let duplicate = store
        .add_facet(target.id, "ja", "name", "センス")
        .await
        .unwrap();
    store.set_canonical_facet(source_ja.id).await.unwrap();
    store.set_canonical_facet(source_en.id).await.unwrap();
    store.set_canonical_facet(duplicate.id).await.unwrap();
    sqlx::query(
        "INSERT INTO concept_facet_story_scopes(facet_id, story_scope) VALUES (?, 'volume:5')",
    )
    .bind(source_ja.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO concept_facet_semantic_tags(facet_id, semantic_tag) VALUES (?, 'role:skill')",
    )
    .bind(source_ja.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO crystals(crystal_type,text,title,scope_type,scope_key,series_slug,source_language,target_language,tags_json,strength,confidence,source_credibility,rule_intent,status,created_at,updated_at) VALUES('lesson','sense','', 'series','series:oso','oso','ja','ru','[]',0.5,0.5,'observation','','active',?,?)")
        .bind(chrono::Utc::now()).bind(chrono::Utc::now()).execute(&pool).await.unwrap();
    let crystal_id: i64 = sqlx::query_scalar("SELECT max(id) FROM crystals")
        .fetch_one(&pool)
        .await
        .unwrap();
    store
        .link_crystal(crystal_id, source.id, "mentions", 0.8)
        .await
        .unwrap();

    store
        .merge_concepts(source.id, target.id, "duplicate")
        .await
        .unwrap();
    let merged = store.get(source.id).await.unwrap();
    assert_eq!(merged.status, "merged");
    assert_eq!(merged.merged_into_concept_id, Some(target.id));
    assert_eq!(
        store.concept_ids_for_crystal(crystal_id).await.unwrap(),
        [target.id]
    );
    let active_facets = store.list_facets(target.id).await.unwrap();
    assert_eq!(active_facets.len(), 3);
    let duplicate = active_facets
        .iter()
        .find(|facet| facet.id == duplicate.id)
        .unwrap();
    assert!(duplicate.is_canonical);
    assert_eq!(duplicate.story_scopes, ["volume:5"]);
    assert_eq!(duplicate.semantic_tags, ["role:skill"]);
    let moved = active_facets
        .iter()
        .find(|facet| facet.id == source_en.id)
        .unwrap();
    assert_eq!(moved.language, "en");
    assert!(!moved.is_canonical);
    let former = active_facets
        .iter()
        .find(|facet| facet.facet_type == "former_label")
        .unwrap();
    assert_eq!(former.value, "Sense");
    assert!(
        store
            .merge_concepts(target.id, source.id, "cycle")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn proposal_state_machine_validates_json_and_never_orphans_approval() {
    let pool = pool().await;
    let store = ConceptProposalStore::new(&pool);
    let id = store
        .create(CreateProposalInput {
            series_slug: "oso".into(),
            source_language: "ja".into(),
            target_language: "ru".into(),
            concept_text: "Sense".into(),
            source_form: "センス".into(),
            canonical_rendering: "Сенс".into(),
        })
        .await
        .unwrap();
    assert_eq!(store.list_pending().await.unwrap()[0].id, id);
    store.approve(id).await.unwrap();
    store.approve(id).await.unwrap();
    assert_eq!(store.get(id).await.unwrap().status, "approved");

    let broken = store
        .create(CreateProposalInput {
            series_slug: "oso".into(),
            source_language: "ja".into(),
            target_language: "ru".into(),
            concept_text: "Broken".into(),
            source_form: "x".into(),
            canonical_rendering: "y".into(),
        })
        .await
        .unwrap();
    sqlx::query("UPDATE concept_proposals SET approved_variants_json = 'not-json' WHERE id = ?")
        .bind(broken)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.approve(broken).await.is_err());
    let row = sqlx::query("SELECT status FROM concept_proposals WHERE id = ?")
        .bind(broken)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("status"), "pending");
}

#[tokio::test]
async fn concurrent_merge_serializes_the_source_without_lost_relationships() {
    let pool = pool().await;
    let store = ConceptStore::new(&pool);
    let source = store
        .create(concept("Duplicate", "series", "series:oso"))
        .await
        .unwrap();
    let first = store
        .create(concept("First", "series", "series:oso"))
        .await
        .unwrap();
    let second = store
        .create(concept("Second", "series", "series:oso"))
        .await
        .unwrap();

    let first_merge = store.merge_concepts(source.id, first.id, "first winner");
    let second_merge = store.merge_concepts(source.id, second.id, "second winner");
    let (left, right) = tokio::join!(first_merge, second_merge);
    assert_ne!(left.is_ok(), right.is_ok());
    let persisted = store.get(source.id).await.unwrap();
    assert!(
        matches!(persisted.merged_into_concept_id, Some(id) if id == first.id || id == second.id)
    );
}

#[tokio::test]
async fn canonical_facets_are_unique_per_concept_language_and_kind() {
    let pool = pool().await;
    let store = ConceptStore::new(&pool);
    let concept = store.create(concept("Sense", "global", "")).await.unwrap();
    let first = store
        .add_facet(concept.id, "ru", "rendering", "Сенс")
        .await
        .unwrap();
    let second = store
        .add_facet(concept.id, "ru", "rendering", "Чувство")
        .await
        .unwrap();
    store.set_canonical_facet(first.id).await.unwrap();
    store.set_canonical_facet(second.id).await.unwrap();
    let facets = store.list_facets(concept.id).await.unwrap();
    assert_eq!(facets.iter().filter(|facet| facet.is_canonical).count(), 1);
    assert!(
        facets
            .iter()
            .find(|facet| facet.id == second.id)
            .unwrap()
            .is_canonical
    );

    assert!(
        store
            .link_crystal(99, concept.id, "mentions", f64::NAN)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn hydration_chunks_more_than_sqlite_variable_limit_without_n_plus_one() {
    let pool = pool().await;
    let now = chrono::Utc::now();
    let mut transaction = pool.begin().await.unwrap();
    for index in 0..1_001 {
        let concept_id = sqlx::query(
            "INSERT INTO concepts(canonical_name, created_at, updated_at) VALUES (?, ?, ?)",
        )
        .bind(format!("Concept {index}"))
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query("INSERT INTO concept_semantic_tags(concept_id, tag, confidence, created_at) VALUES (?, ?, 0.5, ?)")
            .bind(concept_id)
            .bind(format!("tag:{index}"))
            .bind(now)
            .execute(&mut *transaction)
            .await
            .unwrap();
    }
    transaction.commit().await.unwrap();

    let records = ConceptStore::new(&pool)
        .list(ConceptFilter {
            scope_type: "global".into(),
            scope_key: String::new(),
            status: None,
        })
        .await
        .unwrap();
    assert_eq!(records.len(), 1_001);
    assert_eq!(records[500].semantic_tags, ["tag:500"]);
    assert_eq!(records[1_000].semantic_tags, ["tag:1000"]);
}
