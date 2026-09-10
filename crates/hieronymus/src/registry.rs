use chrono::Utc;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;

/// A translation series: the language-neutral top-level container (ADR 0003)
/// with default translation directions and normalized language tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    pub slug: String,
    pub title: String,
    pub source_language: String,
    pub target_language: String,
    pub language_tags: Vec<String>,
    pub id: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(
        "invalid series slug: use lowercase letters, numbers, and hyphens, \
         starting with a letter or number"
    )]
    InvalidSlug,
    #[error("unknown series: {0}")]
    UnknownSeries(String),
    #[error("unknown series id: {0}")]
    UnknownSeriesId(i64),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}

/// Series registry over the data-root database. Opening the registry applies
/// the Rust schema to fresh databases and refuses unsupported states.
#[derive(Debug)]
pub struct Registry {
    config: HieronymusConfig,
}

impl Registry {
    pub fn open(config: &HieronymusConfig) -> Result<Self, RegistryError> {
        open_migrated(&config.database_path())?;
        Ok(Self {
            config: config.clone(),
        })
    }

    pub fn config(&self) -> &HieronymusConfig {
        &self.config
    }

    /// Create or update a series by slug. When `language_tags` is `None` the
    /// tags are seeded from the default translation directions (legacy
    /// compatibility); an explicit empty slice clears them.
    pub fn create_series(
        &self,
        slug: &str,
        title: &str,
        source_language: &str,
        target_language: &str,
        language_tags: Option<&[String]>,
    ) -> Result<Series, RegistryError> {
        validate_slug(slug)?;
        let now = now_iso8601();
        let normalized_tags = match language_tags {
            None => compat_language_tags(source_language, target_language),
            Some(tags) => normalize_language_tags(tags.iter().map(String::as_str)),
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "insert into series(
               slug,
               title,
               default_source_language,
               default_target_language,
               created_at,
               updated_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6)
             on conflict(slug) do update set
               title = excluded.title,
               default_source_language = excluded.default_source_language,
               default_target_language = excluded.default_target_language,
               updated_at = excluded.updated_at",
            rusqlite::params![slug, title, source_language, target_language, now, now],
        )?;
        let series_id: i64 =
            transaction.query_row("select id from series where slug = ?1", [slug], |row| {
                row.get(0)
            })?;
        transaction.execute(
            "insert into authority_state(series_id) values (?1) on conflict(series_id) do nothing",
            [series_id],
        )?;
        replace_series_language_tags(&transaction, series_id, &normalized_tags, &now)?;
        transaction.commit()?;
        self.get_series(slug)
    }

    pub fn get_series(&self, slug: &str) -> Result<Series, RegistryError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "select id, slug, title, default_source_language, default_target_language
                 from series where slug = ?1",
                [slug],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    RegistryError::UnknownSeries(slug.to_string())
                }
                other => other.into(),
            })?;
        let language_tags = series_language_tags(&connection, row.0)?;
        Ok(Series {
            slug: row.1,
            title: row.2,
            source_language: row.3,
            target_language: row.4,
            language_tags,
            id: Some(row.0),
        })
    }

    pub fn list_series(&self) -> Result<Vec<Series>, RegistryError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select id, slug, title, default_source_language, default_target_language
             from series order by slug",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut series = Vec::new();
        for row in rows {
            let (id, slug, title, source_language, target_language) = row?;
            let language_tags = series_language_tags(&connection, id)?;
            series.push(Series {
                slug,
                title,
                source_language,
                target_language,
                language_tags,
                id: Some(id),
            });
        }
        Ok(series)
    }

    /// Replace a series' language tags. Default translation directions are
    /// compatibility fields and stay untouched.
    pub fn set_series_language_tags(
        &self,
        series_id: i64,
        language_tags: &[String],
    ) -> Result<(), RegistryError> {
        let normalized = normalize_language_tags(language_tags.iter().map(String::as_str));
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let exists: Option<i64> = transaction
            .query_row("select id from series where id = ?1", [series_id], |row| {
                row.get(0)
            })
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        if exists.is_none() {
            return Err(RegistryError::UnknownSeriesId(series_id));
        }
        replace_series_language_tags(&transaction, series_id, &normalized, &now)?;
        transaction.commit()?;
        Ok(())
    }

    fn connection(&self) -> Result<rusqlite::Connection, RegistryError> {
        Ok(open_migrated(&self.config.database_path())?)
    }
}

fn validate_slug(slug: &str) -> Result<(), RegistryError> {
    let mut characters = slug.chars();
    let valid = match characters.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {
            characters.all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(RegistryError::InvalidSlug)
    }
}

fn normalize_language_tags<'a>(tags: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut normalized: Vec<String> = tags
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn compat_language_tags(source_language: &str, target_language: &str) -> Vec<String> {
    normalize_language_tags([source_language, target_language].into_iter())
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

fn replace_series_language_tags(
    connection: &rusqlite::Connection,
    series_id: i64,
    language_tags: &[String],
    now: &str,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "delete from series_language_tags where series_id = ?1",
        [series_id],
    )?;
    for tag in language_tags {
        connection.execute(
            "insert into series_language_tags(series_id, language_tag, created_at)
             values (?1, ?2, ?3)",
            rusqlite::params![series_id, tag, now],
        )?;
    }
    Ok(())
}

fn series_language_tags(
    connection: &rusqlite::Connection,
    series_id: i64,
) -> Result<Vec<String>, RegistryError> {
    let mut statement = connection.prepare(
        "select language_tag from series_language_tags
         where series_id = ?1 order by language_tag",
    )?;
    let rows = statement.query_map([series_id], |row| row.get::<_, String>(0))?;
    let mut tags = Vec::new();
    for tag in rows {
        tags.push(tag?);
    }
    Ok(tags)
}
