use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use hiero_core::registry::{RegistryError, SeriesRegistry};
use serde_json::json;
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};

static DATABASE_ID: AtomicU64 = AtomicU64::new(1);

async fn fixture() -> SqlitePool {
    let id = DATABASE_ID.fetch_add(1, Ordering::Relaxed);
    let url = format!("sqlite:file:registry-{id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("in-memory registry database should connect");

    sqlx::query(
        r#"
        create table series (
          id integer primary key,
          slug text not null unique,
          title text not null,
          default_source_language text not null,
          default_target_language text not null,
          created_at text not null,
          updated_at text not null
        ) strict
        "#,
    )
    .execute(&pool)
    .await
    .expect("series fixture table should be created");
    sqlx::query(
        r#"
        create table series_language_tags (
          series_id integer not null references series(id) on delete cascade,
          language_tag text not null,
          created_at text not null default (datetime('now')),
          primary key (series_id, language_tag)
        ) strict
        "#,
    )
    .execute(&pool)
    .await
    .expect("series tag fixture table should be created");

    pool
}

#[tokio::test]
async fn create_upserts_a_slug_and_seeds_compatibility_language_tags() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);

    let created = registry
        .create("only-sense-online", "Only Sense Online", "JA", " en ")
        .await
        .expect("series should be created");
    let updated = registry
        .create("only-sense-online", "Only Sense Online Rebuild", "jp", "ru")
        .await
        .expect("duplicate slug should update the existing series");

    assert_eq!(created.id, updated.id);
    assert_eq!(updated.title, "Only Sense Online Rebuild");
    assert_eq!(updated.default_source_language, "jp");
    assert_eq!(updated.default_target_language, "ru");
    assert_eq!(updated.language_tags, ["jp", "ru"]);
    assert_eq!(registry.get("only-sense-online").await.unwrap(), updated);
}

#[tokio::test]
async fn create_with_explicit_tags_normalizes_them_without_requiring_a_direction() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);

    let tags = vec![
        "ja".to_owned(),
        " EN ".to_owned(),
        "ja".to_owned(),
        "ru".to_owned(),
    ];
    let series = registry
        .create_with_language_tags("book-of-friends", "Book of Friends", "", "", Some(&tags))
        .await
        .expect("series should be created");
    let empty = registry
        .create_with_language_tags("empty", "Empty", "ja", "en", Some(&[]))
        .await
        .expect("an explicit empty tag set should be accepted");

    assert_eq!(series.language_tags, ["en", "ja", "ru"]);
    assert!(empty.language_tags.is_empty());
}

#[tokio::test]
async fn list_orders_series_and_each_series_language_tags_stably() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);
    let beta_tags = vec!["RU".to_owned(), " en ".to_owned()];
    registry
        .create_with_language_tags("beta", "Beta", "", "", Some(&beta_tags))
        .await
        .unwrap();
    registry.create("alpha", "Alpha", "JA", "en").await.unwrap();

    let listed = registry.list().await.expect("series should list");

    assert_eq!(
        listed
            .iter()
            .map(|series| series.slug.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(listed[0].language_tags, ["en", "ja"]);
    assert_eq!(listed[1].language_tags, ["en", "ru"]);
}

#[tokio::test]
async fn get_and_set_tags_report_typed_missing_series_errors() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);

    let by_slug = registry.get("missing").await.unwrap_err();
    let by_id = registry.set_language_tags(404, &[]).await.unwrap_err();

    assert!(matches!(by_slug, RegistryError::UnknownSeries { ref slug } if slug == "missing"));
    assert!(matches!(by_id, RegistryError::UnknownSeriesId { id: 404 }));
}

#[tokio::test]
async fn create_rejects_filename_unsafe_slugs_without_writing_a_row() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);

    let error = registry
        .create("../../escape", "Unsafe", "ja", "en")
        .await
        .unwrap_err();

    assert!(matches!(error, RegistryError::InvalidSlug { .. }));
    assert!(registry.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn set_language_tags_replaces_tags_without_changing_compatibility_fields() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);
    let series = registry
        .create("only-sense-online", "Only Sense Online", "ja", "en")
        .await
        .unwrap();
    let tags = vec!["RU".to_owned(), " en ".to_owned(), "ru".to_owned()];

    registry
        .set_language_tags(series.id, &tags)
        .await
        .expect("tags should be replaced");
    let updated = registry.get("only-sense-online").await.unwrap();

    assert_eq!(updated.default_source_language, "ja");
    assert_eq!(updated.default_target_language, "en");
    assert_eq!(updated.language_tags, ["en", "ru"]);
}

#[tokio::test]
async fn set_language_tags_rolls_back_deletion_when_an_insert_fails() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);
    let series = registry
        .create("only-sense-online", "Only Sense Online", "ja", "en")
        .await
        .unwrap();
    sqlx::query(
        "create trigger reject_forbidden_tag before insert on series_language_tags when new.language_tag = 'forbidden' begin select raise(abort, 'forbidden tag'); end",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = registry
        .set_language_tags(series.id, &["forbidden".to_owned()])
        .await
        .unwrap_err();

    assert!(matches!(error, RegistryError::Database(_)));
    assert_eq!(
        registry
            .get("only-sense-online")
            .await
            .unwrap()
            .language_tags,
        ["en", "ja"]
    );
}

#[tokio::test]
async fn init_writes_private_workspace_context_atomically() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);
    registry
        .create("only-sense-online", "Only Sense Online", "ja", "en")
        .await
        .unwrap();
    let workspace = unique_workspace("init");
    std::fs::create_dir_all(&workspace).unwrap();

    registry
        .init_at("only-sense-online", &workspace)
        .await
        .expect("workspace should initialize");

    let config_path = workspace.join(".hieronymus.json");
    let payload: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        payload,
        json!({
            "series_slug": "only-sense-online",
            "source_language": "ja",
            "target_language": "en",
            "task_type": "translation"
        })
    );
    assert_eq!(temporary_files(&workspace), Vec::<String>::new());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(config_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    std::fs::remove_dir_all(workspace).unwrap();
}

fn unique_workspace(label: &str) -> std::path::PathBuf {
    let id = DATABASE_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "hiero-registry-{label}-{}-{id}",
        std::process::id()
    ))
}

fn temporary_files(workspace: &Path) -> Vec<String> {
    std::fs::read_dir(workspace)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != ".hieronymus.json")
        .collect()
}

#[tokio::test]
async fn language_tag_replacement_has_one_created_at_per_operation() {
    let pool = fixture().await;
    let registry = SeriesRegistry::new(&pool);
    let series = registry
        .create("only-sense-online", "Only Sense Online", "", "")
        .await
        .unwrap();
    registry
        .set_language_tags(series.id, &["ja".to_owned(), "en".to_owned()])
        .await
        .unwrap();

    let rows =
        sqlx::query("select distinct created_at from series_language_tags where series_id = ?")
            .bind(series.id)
            .fetch_all(&pool)
            .await
            .unwrap();

    assert_eq!(rows.len(), 1);
    let _: String = rows[0].get("created_at");
}
