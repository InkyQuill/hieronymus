use std::error::Error as _;

use hiero_core::{
    db::connect_url,
    domain::{
        AddCrystalInput, CrystalStore, RuleFilter, StoreError, TranslationContext,
        search_expression,
    },
};
use sqlx::Row;
use uuid::Uuid;

type InputMutation = Box<dyn Fn(&mut AddCrystalInput)>;

async fn pool() -> sqlx::SqlitePool {
    connect_url(&format!(
        "sqlite:file:crystals-{}?mode=memory&cache=shared",
        Uuid::new_v4()
    ))
    .await
    .expect("test database should migrate")
}

fn input(text: &str) -> AddCrystalInput {
    AddCrystalInput {
        crystal_type: "lesson".into(),
        title: "Inventory UI".into(),
        text: text.into(),
        scope_type: "series".into(),
        scope_key: "series:oso".into(),
        series_slug: "oso".into(),
        source_language: "ja".into(),
        target_language: "ru".into(),
        source_credibility: "observation".into(),
        rule_intent: String::new(),
        ..AddCrystalInput::default()
    }
}

async fn side_values(pool: &sqlx::SqlitePool, table: &str, column: &str, id: i64) -> Vec<String> {
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT ");
    query
        .push(column)
        .push(" AS value FROM ")
        .push(table)
        .push(" WHERE crystal_id = ")
        .push_bind(id)
        .push(" ORDER BY value");
    query
        .build()
        .fetch_all(pool)
        .await
        .expect("side-table query should work")
        .into_iter()
        .map(|row| row.get("value"))
        .collect()
}

#[tokio::test]
async fn crystal_store_adds_reads_and_searches_without_writing_fts_directly() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let id = store
        .add(input(
            "Render inventory menu labels with concise Russian nouns.",
        ))
        .await
        .expect("valid crystal should be added");

    let record = store.get(id).await.expect("crystal should exist");
    assert_eq!(record.title, "Inventory UI");
    assert_eq!(record.strength, 0.5);
    assert_eq!(record.confidence, 0.5);

    let context = TranslationContext::new("oso", "ja", "ru");
    let results = store
        .search(&context, "inventory menu", 10)
        .await
        .expect("plain search should escape FTS input");
    assert_eq!(results.iter().map(|row| row.id).collect::<Vec<_>>(), [id]);
}

#[tokio::test]
async fn rule_intent_filter_does_not_depend_on_crystal_type() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut rule = input("Always render Sense as Сенс.");
    rule.crystal_type = "observation".into();
    rule.rule_intent = "terminology_override".into();
    let id = store
        .add(rule)
        .await
        .expect("rule-intent memory should be valid");
    store
        .add(input("Ordinary lesson"))
        .await
        .expect("ordinary memory should be valid");

    let rows = store
        .list_rule_intent(RuleFilter {
            status: Some("active".into()),
            series_slug: Some("oso".into()),
            limit: 10,
        })
        .await
        .expect("filter should be valid");
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [id]);
}

#[test]
fn plain_search_expression_quotes_tokens_and_removes_operators() {
    assert_eq!(
        search_expression("inventory OR menu near (labels)"),
        "\"inventory\" \"menu\" \"labels\""
    );
    assert_eq!(search_expression("' OR *"), "");
}

#[tokio::test]
async fn add_round_trips_every_scalar_and_normalizes_side_metadata_atomically() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut value = input("A detailed lesson.");
    value.crystal_type = "erudition".into();
    value.title = " Detail ".into();
    value.strength = 0.75;
    value.confidence = 0.8;
    value.source_credibility = "expert".into();
    value.rule_intent = " supporting_context ".into();
    value.tags = vec!["legacy".into()];
    value.language_tags = vec![" RU ".into(), "ja".into(), "ru".into(), "".into()];
    value.story_scopes = vec![" volume:1 ".into(), "chapter:2".into(), "volume:1".into()];
    value.semantic_tags = vec![" zeta ".into(), "alpha".into(), "zeta".into()];
    value.soft_origin = Some(" source note ".into());
    value.is_inferred = true;
    value.malformed_penalty = 0.25;
    value.status = "candidate".into();

    let id = store
        .add(value)
        .await
        .expect("all valid fields should persist");
    let record = store.get(id).await.expect("record should hydrate");

    assert_eq!(record.crystal_type, "erudition");
    assert_eq!(record.title, "Detail");
    assert_eq!(record.strength, 0.75);
    assert_eq!(record.confidence, 0.8);
    assert_eq!(record.source_credibility, "expert");
    assert_eq!(record.rule_intent, "supporting_context");
    assert_eq!(record.tags_json, r#"["zeta","alpha"]"#);
    assert_eq!(record.soft_origin.as_deref(), Some("source note"));
    assert!(record.is_inferred);
    assert_eq!(record.malformed_penalty, 0.25);
    assert_eq!(record.status, "candidate");
    assert_eq!(record.language_tags, ["ja", "ru"]);
    assert_eq!(record.story_scopes, ["chapter:2", "volume:1"]);
    assert_eq!(record.semantic_tags, ["alpha", "zeta"]);
    assert!(record.concept_ids.is_empty());
    assert_eq!(
        side_values(&pool, "crystal_language_tags", "language_tag", id).await,
        ["ja", "ru"]
    );
    assert_eq!(
        side_values(&pool, "crystal_story_scopes", "scope", id).await,
        ["chapter:2", "volume:1"]
    );
    assert_eq!(
        side_values(&pool, "crystal_semantic_tags", "tag", id).await,
        ["alpha", "zeta"]
    );
}

#[tokio::test]
async fn add_defaults_and_legacy_tags_are_explicit_and_stable() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut value = input("Default values.");
    value.tags = vec![" beta ".into(), "alpha".into(), "beta".into()];
    let id = store.add(value).await.expect("defaults should be accepted");
    let record = store.get(id).await.expect("record should exist");

    assert_eq!(record.strength, 0.5);
    assert_eq!(record.confidence, 0.5);
    assert_eq!(record.status, "active");
    assert_eq!(record.tags_json, r#"["beta","alpha"]"#);
    assert_eq!(record.soft_origin, None);
    assert!(!record.is_inferred);
    assert_eq!(record.supersedes_crystal_id, None);
}

#[tokio::test]
async fn add_rejects_invalid_types_statuses_scores_scope_and_text_without_rows() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let cases: Vec<(&str, InputMutation)> = vec![
        (
            "crystal_type",
            Box::new(|value| value.crystal_type = "memo".into()),
        ),
        ("status", Box::new(|value| value.status = "deleted".into())),
        ("strength", Box::new(|value| value.strength = -0.1)),
        ("strength", Box::new(|value| value.strength = f64::NAN)),
        (
            "confidence",
            Box::new(|value| value.confidence = f64::INFINITY),
        ),
        (
            "malformed_penalty",
            Box::new(|value| value.malformed_penalty = -0.1),
        ),
        ("text", Box::new(|value| value.text = "  ".into())),
        ("scope_key", Box::new(|value| value.scope_key.clear())),
        (
            "source_credibility",
            Box::new(|value| value.source_credibility = "guess".into()),
        ),
    ];
    for (field, mutate) in cases {
        let mut value = input("valid");
        mutate(&mut value);
        let error = store.add(value).await.expect_err("input must be rejected");
        assert!(matches!(error, StoreError::Validation { field: actual, .. } if actual == field));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals")
        .fetch_one(&pool)
        .await
        .expect("count should work");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn add_enforces_canonical_global_and_series_scope_identity() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut cases = Vec::new();
    let mut arbitrary = input("arbitrary");
    arbitrary.scope_type = "project".into();
    cases.push(arbitrary);
    let mut mismatched = input("mismatch");
    mismatched.scope_key = "series:other".into();
    cases.push(mismatched);
    let mut invalid_slug = input("invalid slug");
    invalid_slug.series_slug = "Bad Slug".into();
    invalid_slug.scope_key = "series:Bad Slug".into();
    cases.push(invalid_slug);
    let mut ambiguous = input("ambiguous");
    ambiguous.series_slug = "oso\nother".into();
    ambiguous.scope_key = "series:oso\nother".into();
    cases.push(ambiguous);
    let mut global_with_series = input("global mismatch");
    global_with_series.scope_type = "global".into();
    global_with_series.scope_key.clear();
    cases.push(global_with_series);

    for value in cases {
        assert!(matches!(
            store.add(value).await.unwrap_err(),
            StoreError::Validation {
                field: "scope_type" | "scope_key" | "series_slug",
                ..
            }
        ));
    }
    let global = AddCrystalInput {
        text: "global".into(),
        ..AddCrystalInput::default()
    };
    assert!(store.add(global).await.is_ok());
}

#[tokio::test]
async fn list_and_search_hide_incoherent_legacy_scope_rows() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let now = chrono::Utc::now();
    sqlx::query("INSERT INTO crystals(crystal_type, text, scope_type, scope_key, series_slug, strength, confidence, rule_intent, status, created_at, updated_at) VALUES ('observation', 'Incoherent hidden token', 'series', 'series:other', 'oso', 0.5, 0.5, 'correction', 'active', ?, ?)")
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        store
            .list_rule_intent(RuleFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .search(
                &TranslationContext::new("oso", "ja", "ru"),
                "Incoherent hidden",
                10,
            )
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn translation_context_metadata_normalization_preserves_first_seen_order() {
    let context = TranslationContext::new("oso", "JA", "RU").with_metadata(
        &[" RU ".into(), "ja".into(), "ru".into()],
        &["volume:2".into(), "chapter:1".into(), "volume:2".into()],
        &["zeta".into(), "alpha".into(), "zeta".into()],
        &["second".into(), "first".into(), "second".into()],
    );
    assert_eq!(context.language_tags, ["ru", "ja"]);
    assert_eq!(context.story_scopes, ["volume:2", "chapter:1"]);
    assert_eq!(context.semantic_tags, ["zeta", "alpha"]);
    assert_eq!(context.tags, ["second", "first"]);
}

#[tokio::test]
async fn get_missing_is_a_typed_not_found_error() {
    let pool = pool().await;
    let error = CrystalStore::new(&pool)
        .get(404)
        .await
        .expect_err("missing crystal should fail");
    assert!(matches!(error, StoreError::NotFound { id: 404 }));
}

#[tokio::test]
async fn add_foreign_key_failure_rolls_back_the_base_row_and_fts_trigger() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut value = input("Dangling replacement");
    value.supersedes_crystal_id = Some(999);
    let error = store
        .add(value)
        .await
        .expect_err("missing superseded crystal must violate the foreign key");
    assert!(matches!(error, StoreError::Constraint { .. }));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let fts_count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals_fts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fts_count, 0);
}

#[tokio::test]
async fn archive_is_idempotent_and_removes_the_crystal_from_search() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let id = store.add(input("Unique archive phrase")).await.unwrap();
    assert_eq!(store.archive(id).await.unwrap().status, "archived");
    assert_eq!(store.archive(id).await.unwrap().status, "archived");
    assert!(
        store
            .search(
                &TranslationContext::new("oso", "ja", "ru"),
                "archive phrase",
                10
            )
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn supersede_is_atomic_and_rejects_dimension_or_status_conflicts() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let old_id = store.add(input("Old guidance")).await.unwrap();
    let replacement = store
        .supersede(old_id, input("New guidance"))
        .await
        .expect("matching replacement should succeed");
    assert_eq!(replacement.supersedes_crystal_id, Some(old_id));
    assert_eq!(store.get(old_id).await.unwrap().status, "superseded");

    let mut second = input("Another replacement");
    second.scope_key = "series:different".into();
    assert!(store.supersede(old_id, second).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2, "failed supersession must roll back its insert");
}

#[tokio::test]
async fn concurrent_supersede_allows_only_one_replacement_without_lost_update() {
    let pool = pool().await;
    let old_id = CrystalStore::new(&pool).add(input("Old")).await.unwrap();
    let first_pool = pool.clone();
    let second_pool = pool.clone();
    let first = tokio::spawn(async move {
        CrystalStore::new(&first_pool)
            .supersede(old_id, input("First"))
            .await
    });
    let second = tokio::spawn(async move {
        CrystalStore::new(&second_pool)
            .supersede(old_id, input("Second"))
            .await
    });
    let outcomes = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    let replacements: i64 =
        sqlx::query_scalar("SELECT count(*) FROM crystals WHERE supersedes_crystal_id = ?")
            .bind(old_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(replacements, 1);
}

#[tokio::test]
async fn links_are_directional_unique_and_returned_from_either_endpoint() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let first = store.add(input("First")).await.unwrap();
    let second = store.add(input("Second")).await.unwrap();
    store.link(first, second, "related").await.unwrap();
    store.link(first, second, "related").await.unwrap();
    store.link(second, first, "supports").await.unwrap();
    assert!(store.link(first, first, "related").await.is_err());
    let rows = store.linked(first).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|(_, weight)| *weight == 1.0));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystal_links")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn validation_and_lowest_confidence_follow_rule_intent_not_crystal_type() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut deterministic = input("Rule-like observation");
    deterministic.rule_intent = "correction".into();
    deterministic.confidence = 0.01;
    let rule_id = store.add(deterministic).await.unwrap();
    let mut low = input("Low");
    low.confidence = 0.1;
    let low_id = store.add(low).await.unwrap();
    let mut high = input("High");
    high.confidence = 0.8;
    let high_id = store.add(high).await.unwrap();

    assert!(store.validate_rule(rule_id).await.unwrap().ok);
    assert!(!store.validate_rule(low_id).await.unwrap().ok);
    assert_eq!(
        store
            .lowest_confidence(&[high_id, rule_id, low_id, low_id], 5)
            .await
            .unwrap(),
        [low_id, high_id]
    );
}

#[tokio::test]
async fn replacing_scopes_and_tags_is_normalized_atomic_and_preserves_other_metadata() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut value = input("Metadata");
    value.title = "Preserved".into();
    value.story_scopes = vec!["old".into()];
    value.semantic_tags = vec!["old".into()];
    let id = store.add(value).await.unwrap();
    store
        .set_story_scopes(id, &[" b ".into(), "a".into(), "b".into()])
        .await
        .unwrap();
    let record = store
        .set_semantic_tags(id, &[" z ".into(), "y".into(), "z".into()])
        .await
        .unwrap();

    assert_eq!(record.title, "Preserved");
    assert_eq!(record.tags_json, r#"["z","y"]"#);
    assert_eq!(
        side_values(&pool, "crystal_story_scopes", "scope", id).await,
        ["a", "b"]
    );
    assert_eq!(
        side_values(&pool, "crystal_semantic_tags", "tag", id).await,
        ["y", "z"]
    );
}

#[tokio::test]
async fn failed_metadata_replacement_rolls_back_without_touching_existing_rows() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut value = input("Metadata");
    value.semantic_tags = vec!["old".into()];
    let id = store.add(value).await.unwrap();
    assert!(store.set_semantic_tags(999, &["new".into()]).await.is_err());
    assert_eq!(
        side_values(&pool, "crystal_semantic_tags", "tag", id).await,
        ["old"]
    );
}

#[tokio::test]
async fn enriched_metadata_is_returned_by_get_list_and_search_for_multiple_rows() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut ids = Vec::new();
    for index in 0..3 {
        let mut value = input(&format!("Batched metadata token {index}"));
        value.rule_intent = "correction".into();
        value.language_tags = vec![format!("lang-{index}"), "shared".into()];
        value.story_scopes = vec![format!("chapter:{index}")];
        value.semantic_tags = vec![format!("tag-{index}")];
        ids.push(store.add(value).await.unwrap());
    }
    let now = chrono::Utc::now();
    let concept_id = sqlx::query("INSERT INTO concepts(canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at) VALUES ('Inventory', 'global', '', 'established', 0.9, ?, ?)")
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
    sqlx::query("INSERT INTO crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at) VALUES (?, ?, 'mentions', 0.9, ?)")
        .bind(ids[1])
        .bind(concept_id)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

    let fetched = store.get(ids[1]).await.unwrap();
    assert_eq!(fetched.language_tags, ["lang-1", "shared"]);
    assert_eq!(fetched.story_scopes, ["chapter:1"]);
    assert_eq!(fetched.semantic_tags, ["tag-1"]);
    assert_eq!(fetched.concept_ids, [concept_id]);

    let listed = store.list_rule_intent(RuleFilter::default()).await.unwrap();
    assert_eq!(listed.len(), 3);
    assert!(
        listed
            .iter()
            .all(|crystal| !crystal.language_tags.is_empty())
    );

    let searched = store
        .search(
            &TranslationContext::new("oso", "ja", "ru"),
            "Batched metadata",
            10,
        )
        .await
        .unwrap();
    assert_eq!(searched.len(), 3);
    assert!(
        searched
            .iter()
            .all(|crystal| !crystal.semantic_tags.is_empty())
    );
}

#[tokio::test]
async fn fts_triggers_track_text_updates_and_deletes_and_pass_integrity_check() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let id = store.add(input("Before token")).await.unwrap();
    sqlx::query("UPDATE crystals SET text = 'After token' WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let ctx = TranslationContext::new("oso", "ja", "ru");
    assert!(store.search(&ctx, "Before", 10).await.unwrap().is_empty());
    assert_eq!(store.search(&ctx, "After", 10).await.unwrap()[0].id, id);
    sqlx::query("INSERT INTO crystals_fts(crystals_fts, rank) VALUES ('integrity-check', 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM crystals WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.search(&ctx, "After", 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn search_is_scope_language_status_rank_and_tie_break_deterministic() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut low = input("Guarded crafting phrase");
    low.strength = 0.1;
    let low_id = store.add(low).await.unwrap();
    let mut high = input("Guarded crafting phrase");
    high.strength = 0.9;
    let high_id = store.add(high).await.unwrap();
    let mut other = input("Guarded crafting phrase");
    other.series_slug = "other".into();
    other.scope_key = "series:other".into();
    store.add(other).await.unwrap();

    let scored = store
        .search_scored(
            &TranslationContext::new("oso", "ja", "ru"),
            "Guarded crafting",
            50,
        )
        .await
        .unwrap();
    assert_eq!(
        scored.iter().map(|(row, _)| row.id).collect::<Vec<_>>(),
        [high_id, low_id]
    );
    assert!(scored[0].1 > scored[1].1);
}

#[tokio::test]
async fn raw_search_expression_reports_syntax_errors_as_typed_failures() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    store.add(input("Guarded crafting")).await.unwrap();
    let error = store
        .search_expression(&TranslationContext::new("oso", "ja", "ru"), "NEAR(", 10)
        .await
        .expect_err("invalid raw syntax should fail");
    assert!(matches!(error, StoreError::FtsExpression));
    assert!(
        store
            .search(&TranslationContext::new("oso", "ja", "ru"), "NEAR(", 10,)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn raw_search_classifies_all_match_parser_errors_without_hiding_operational_errors() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    store.add(input("Guarded crafting")).await.unwrap();
    let ctx = TranslationContext::new("oso", "ja", "ru");
    for expression in [
        "unknown:term",
        "unknown : term",
        "{unknown title}:term",
        "несуществующая:term",
        "\"unterminated",
        "AND guarded",
        "guarded OR",
        "*unsupported",
    ] {
        assert!(
            matches!(
                store
                    .search_expression(&ctx, expression, 10)
                    .await
                    .unwrap_err(),
                StoreError::FtsExpression
            ),
            "expected parser classification for {expression:?}"
        );
    }
    pool.close().await;
    assert!(matches!(
        store
            .search_expression(&ctx, "guarded", 10)
            .await
            .unwrap_err(),
        StoreError::Database { .. }
    ));
}

#[tokio::test]
async fn raw_search_schema_failure_remains_an_operational_database_error() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    store.add(input("Guarded crafting")).await.unwrap();
    sqlx::query("DROP TABLE crystals_fts")
        .execute(&pool)
        .await
        .expect("test should remove the FTS schema object");

    let error = store
        .search_expression(&TranslationContext::new("oso", "ja", "ru"), "guarded", 10)
        .await
        .expect_err("missing FTS schema must fail");

    assert!(matches!(error, StoreError::Database { .. }));
}

#[tokio::test]
async fn raw_fts_expression_error_never_exposes_secret_input() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    store.add(input("Guarded crafting")).await.unwrap();
    let sentinel = "secret-fts-sentinel-9f4c2e";
    let expression = format!("\"{sentinel}");

    let error = store
        .search_expression(&TranslationContext::new("oso", "ja", "ru"), &expression, 10)
        .await
        .expect_err("unterminated raw expression must fail");

    assert!(matches!(error, StoreError::FtsExpression));
    assert!(!error.to_string().contains(sentinel));
    assert!(!format!("{error:?}").contains(sentinel));
    let mut source = error.source();
    while let Some(cause) = source {
        assert!(!cause.to_string().contains(sentinel));
        assert!(!format!("{cause:?}").contains(sentinel));
        source = cause.source();
    }
}

#[tokio::test]
async fn lowest_confidence_honors_limits_above_fifty() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let mut ids = Vec::new();
    for index in 0..60 {
        let mut value = input(&format!("candidate {index}"));
        value.confidence = f64::from(index) / 100.0;
        ids.push(store.add(value).await.unwrap());
    }
    let selected = store.lowest_confidence(&ids, 55).await.unwrap();
    assert_eq!(selected.len(), 55);
    assert_eq!(selected, ids[..55]);
    assert!(matches!(
        store.lowest_confidence(&ids, 0).await.unwrap_err(),
        StoreError::Validation { field: "limit", .. }
    ));
}

#[tokio::test]
async fn dropped_immediate_transaction_rolls_back_and_releases_the_pooled_connection() {
    let pool = pool().await;
    {
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("INSERT INTO crystals(crystal_type, text, scope_type, strength, confidence, status, created_at, updated_at) VALUES ('lesson', 'cancelled', 'global', 0.5, 0.5, 'active', ?, ?)")
            .bind(chrono::Utc::now())
            .bind(chrono::Utc::now())
            .execute(&mut *transaction)
            .await
            .unwrap();
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        CrystalStore::new(&pool)
            .add(input("next mutation succeeds"))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn side_table_failure_after_base_insert_is_rolled_back_by_transaction_drop() {
    let pool = pool().await;
    sqlx::query("CREATE TRIGGER reject_tag BEFORE INSERT ON crystal_semantic_tags BEGIN SELECT RAISE(ABORT, 'injected tag failure'); END")
        .execute(&pool)
        .await
        .unwrap();
    let store = CrystalStore::new(&pool);
    let mut value = input("must roll back");
    value.semantic_tags = vec!["tag".into()];
    assert!(store.add(value).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM crystals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn raw_search_expression_supports_operators_and_limits_are_validated() {
    let pool = pool().await;
    let store = CrystalStore::new(&pool);
    let first = store.add(input("Guarded crafting")).await.unwrap();
    let second = store.add(input("Inventory labels")).await.unwrap();
    let ctx = TranslationContext::new("oso", "ja", "ru");
    let rows = store
        .search_expression(&ctx, "guarded OR inventory", 10)
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|(record, _)| record.id).collect::<Vec<_>>(),
        [first, second]
    );
    assert!(matches!(
        store.search(&ctx, "anything", 0).await.unwrap_err(),
        StoreError::Validation { field: "limit", .. }
    ));
    assert!(matches!(
        store
            .list_rule_intent(RuleFilter {
                status: Some("unknown".into()),
                ..RuleFilter::default()
            })
            .await
            .unwrap_err(),
        StoreError::Validation {
            field: "status",
            ..
        }
    ));
}
