use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

pub use crate::db::SeriesRecord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesWithLanguageTags {
    pub series: SeriesRecord,
    pub language_tags: Vec<String>,
}

impl SeriesRecord {
    fn with_tags(self, language_tags: Vec<String>) -> SeriesWithLanguageTags {
        SeriesWithLanguageTags {
            series: self,
            language_tags,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    #[error(
        "invalid series slug {slug:?}: use lowercase letters, numbers, and hyphens, starting with a letter or number"
    )]
    InvalidSlug { slug: String },
    #[error("unknown series: {slug}")]
    UnknownSeries { slug: String },
    #[error("unknown series id: {id}")]
    UnknownSeriesId { id: i64 },
    #[error("could not resolve the current workspace directory")]
    CurrentDirectory(#[source] std::io::Error),
    #[error("could not serialize workspace configuration")]
    SerializeWorkspace(#[source] serde_json::Error),
    #[error("could not write workspace configuration at {path}")]
    WriteWorkspace {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("registry database operation failed")]
    Database(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, RegistryError>;

#[derive(Debug, Clone, Copy)]
pub struct SeriesRegistry<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SeriesRegistry<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        slug: &str,
        title: &str,
        source_lang: &str,
        target_lang: &str,
    ) -> Result<SeriesWithLanguageTags> {
        self.create_with_language_tags(slug, title, source_lang, target_lang, None)
            .await
    }

    pub async fn create_with_language_tags(
        &self,
        slug: &str,
        title: &str,
        source_lang: &str,
        target_lang: &str,
        language_tags: Option<&[String]>,
    ) -> Result<SeriesWithLanguageTags> {
        validate_slug(slug)?;
        let language_tags = language_tags.map_or_else(
            || normalize_language_tags(&[source_lang.to_owned(), target_lang.to_owned()]),
            normalize_language_tags,
        );
        let now = Utc::now();
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            r#"
            insert into series(
              slug, title, default_source_language, default_target_language, created_at, updated_at
            )
            values (?, ?, ?, ?, ?, ?)
            on conflict(slug) do update set
              title = excluded.title,
              default_source_language = excluded.default_source_language,
              default_target_language = excluded.default_target_language,
              updated_at = excluded.updated_at
            "#,
        )
        .bind(slug)
        .bind(title)
        .bind(source_lang)
        .bind(target_lang)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        let row = fetch_series_by_slug(&mut transaction, slug)
            .await?
            .ok_or_else(|| RegistryError::UnknownSeries {
                slug: slug.to_owned(),
            })?;
        replace_language_tags(&mut transaction, row.id, &language_tags, now).await?;
        transaction.commit().await?;

        Ok(row.with_tags(language_tags))
    }

    pub async fn list(&self) -> Result<Vec<SeriesWithLanguageTags>> {
        let rows = sqlx::query_as::<_, SeriesRecord>(
            r#"
            select id, slug, title, default_source_language, default_target_language,
                   created_at, updated_at
            from series
            order by slug
            "#,
        )
        .fetch_all(self.pool)
        .await?;
        let tag_rows = sqlx::query_as::<_, (i64, String)>(
            r#"
            select series_id, language_tag
            from series_language_tags
            order by series_id, language_tag
            "#,
        )
        .fetch_all(self.pool)
        .await?;
        let mut tags_by_series = HashMap::<i64, Vec<String>>::new();
        for (series_id, tag) in tag_rows {
            tags_by_series.entry(series_id).or_default().push(tag);
        }

        Ok(rows
            .into_iter()
            .map(|row| {
                let tags = tags_by_series.remove(&row.id).unwrap_or_default();
                row.with_tags(tags)
            })
            .collect())
    }

    pub async fn get(&self, slug: &str) -> Result<SeriesWithLanguageTags> {
        let mut connection = self.pool.acquire().await?;
        let row = sqlx::query_as::<_, SeriesRecord>(
            r#"
            select id, slug, title, default_source_language, default_target_language,
                   created_at, updated_at
            from series
            where slug = ?
            "#,
        )
        .bind(slug)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| RegistryError::UnknownSeries {
            slug: slug.to_owned(),
        })?;
        let tags = sqlx::query_scalar::<_, String>(
            r#"
            select language_tag
            from series_language_tags
            where series_id = ?
            order by language_tag
            "#,
        )
        .bind(row.id)
        .fetch_all(&mut *connection)
        .await?;

        Ok(row.with_tags(tags))
    }

    pub async fn set_language_tags(&self, series_id: i64, language_tags: &[String]) -> Result<()> {
        let tags = normalize_language_tags(language_tags);
        let mut transaction = self.pool.begin().await?;
        let exists = sqlx::query_scalar::<_, i64>("select id from series where id = ?")
            .bind(series_id)
            .fetch_optional(&mut *transaction)
            .await?;
        if exists.is_none() {
            return Err(RegistryError::UnknownSeriesId { id: series_id });
        }

        replace_language_tags(&mut transaction, series_id, &tags, Utc::now()).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn init(&self, slug: &str) -> Result<()> {
        let workspace = std::env::current_dir().map_err(RegistryError::CurrentDirectory)?;
        self.init_at(slug, workspace).await
    }

    pub async fn init_at(&self, slug: &str, workspace: impl AsRef<Path>) -> Result<()> {
        let series = self.get(slug).await?;
        let config = WorkspaceConfig {
            series_slug: &series.series.slug,
            source_language: &series.series.default_source_language,
            target_language: &series.series.default_target_language,
            task_type: "translation",
        };
        let mut contents =
            serde_json::to_vec_pretty(&config).map_err(RegistryError::SerializeWorkspace)?;
        contents.push(b'\n');
        atomic_write(workspace.as_ref().join(".hieronymus.json"), &contents)
    }
}

#[derive(Serialize)]
struct WorkspaceConfig<'a> {
    series_slug: &'a str,
    source_language: &'a str,
    target_language: &'a str,
    task_type: &'static str,
}

async fn fetch_series_by_slug(
    transaction: &mut Transaction<'_, Sqlite>,
    slug: &str,
) -> std::result::Result<Option<SeriesRecord>, sqlx::Error> {
    sqlx::query_as::<_, SeriesRecord>(
        r#"
        select id, slug, title, default_source_language, default_target_language,
               created_at, updated_at
        from series
        where slug = ?
        "#,
    )
    .bind(slug)
    .fetch_optional(&mut **transaction)
    .await
}

async fn replace_language_tags(
    transaction: &mut Transaction<'_, Sqlite>,
    series_id: i64,
    language_tags: &[String],
    now: DateTime<Utc>,
) -> std::result::Result<(), sqlx::Error> {
    sqlx::query("delete from series_language_tags where series_id = ?")
        .bind(series_id)
        .execute(&mut **transaction)
        .await?;
    for language_tag in language_tags {
        sqlx::query(
            "insert into series_language_tags(series_id, language_tag, created_at) values (?, ?, ?)",
        )
        .bind(series_id)
        .bind(language_tag)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

fn validate_slug(slug: &str) -> Result<()> {
    let mut bytes = slug.bytes();
    let valid_first = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    let valid_rest =
        bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid_first && valid_rest {
        Ok(())
    } else {
        Err(RegistryError::InvalidSlug {
            slug: slug.to_owned(),
        })
    }
}

fn normalize_language_tags(language_tags: &[String]) -> Vec<String> {
    let mut tags: Vec<String> = language_tags
        .iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    tags.sort_unstable();
    tags.dedup();
    tags
}

fn atomic_write(path: PathBuf, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace-config");
    let temporary_path = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let write_result = write_and_replace(&temporary_path, &path, contents);
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result.map_err(|source| RegistryError::WriteWorkspace { path, source })
}

fn write_and_replace(
    temporary_path: &Path,
    destination: &Path,
    contents: &[u8],
) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(temporary_path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary_path, destination)?;
    if let Some(parent) = destination.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}
