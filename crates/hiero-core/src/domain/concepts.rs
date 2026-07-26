use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::Utc;
use sqlx::{AssertSqlSafe, QueryBuilder, Sqlite, SqliteConnection, SqlitePool, Transaction};

use crate::db::{ConceptFacetRecord, ConceptProposalRecord, ConceptRecord};

use super::models::{
    Concept, ConceptFacet, ConceptFilter, ConceptProposal, CreateConceptInput, CreateProposalInput,
};

const CONCEPT_COLUMNS: &str = "id, canonical_name, description, scope_type, scope_key, status, confidence, merged_into_concept_id, created_at, updated_at";
const FACET_COLUMNS: &str = "id, concept_id, language, facet_type, value, source_crystal_id, confidence, is_canonical, superseded_at, created_at, updated_at";
const PROPOSAL_COLUMNS: &str = "id, dream_run_id, series_slug, source_language, target_language, concept_text, source_form, canonical_rendering, approved_variants_json, forbidden_variants_json, rationale, status, created_at, updated_at";
const HYDRATION_CHUNK_SIZE: usize = 500;
const RECALL_IN_CHUNK_SIZE: usize = 240;

pub type Result<T> = std::result::Result<T, ConceptError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConceptError {
    #[error("invalid concept {field}: {reason}")]
    Validation { field: &'static str, reason: String },
    #[error("unknown concept: {id}")]
    NotFound { id: i64 },
    #[error("unknown concept facet: {id}")]
    FacetNotFound { id: i64 },
    #[error("unknown concept proposal: {id}")]
    ProposalNotFound { id: i64 },
    #[error("concept {operation} conflicted with existing state: {reason}")]
    Conflict {
        operation: &'static str,
        reason: String,
    },
    #[error("concept {operation} failed: {source}")]
    Database {
        operation: &'static str,
        #[source]
        source: sqlx::Error,
    },
    #[error("concept proposal contains invalid {field} JSON: {source}")]
    Json {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

pub struct ConceptStore<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ConceptStore<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: CreateConceptInput) -> Result<ConceptRecord> {
        let input = validate_concept_input(input)?;
        let mut transaction = begin_immediate(self.pool, "create").await?;
        let result = async {
            let now = Utc::now();
            let id = sqlx::query("INSERT INTO concepts(canonical_name, scope_type, scope_key, status, confidence, created_at, updated_at) VALUES (?, ?, ?, 'candidate', 0.2, ?, ?)")
                .bind(&input.canonical_name)
                .bind(&input.scope_type)
                .bind(&input.scope_key)
                .bind(now)
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(|source| database("create", source))?
                .last_insert_rowid();
            get_concept_on(&mut transaction, id, "create").await
        }
        .await;
        commit_write(transaction, "create", result).await
    }

    pub async fn get(&self, id: i64) -> Result<ConceptRecord> {
        sqlx::query_as::<_, ConceptRecord>(AssertSqlSafe(format!(
            "SELECT {CONCEPT_COLUMNS} FROM concepts WHERE id = ?"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await
        .map_err(|source| database("get", source))?
        .ok_or(ConceptError::NotFound { id })
    }

    pub async fn get_enriched(&self, id: i64) -> Result<Concept> {
        let records = hydrate_concepts(self.pool, vec![self.get(id).await?]).await?;
        records
            .into_iter()
            .next()
            .ok_or(ConceptError::NotFound { id })
    }

    pub async fn list(&self, filter: ConceptFilter) -> Result<Vec<Concept>> {
        validate_filter(&filter)?;
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {CONCEPT_COLUMNS} FROM concepts WHERE scope_type = "
        ));
        query
            .push_bind(&filter.scope_type)
            .push(" AND scope_key = ")
            .push_bind(&filter.scope_key);
        if let Some(status) = &filter.status {
            query.push(" AND status = ").push_bind(status);
        }
        query.push(" ORDER BY id");
        let records = query
            .build_query_as::<ConceptRecord>()
            .fetch_all(self.pool)
            .await
            .map_err(|source| database("list", source))?;
        hydrate_concepts(self.pool, records).await
    }

    pub async fn search(&self, query: &str, filter: ConceptFilter) -> Result<Vec<ConceptRecord>> {
        validate_filter(&filter)?;
        let expression = super::search_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, ConceptRecord>(AssertSqlSafe(format!(
            "SELECT DISTINCT {columns} FROM (SELECT c.* FROM concepts_fts JOIN concepts c ON c.id = concepts_fts.rowid WHERE concepts_fts MATCH ? UNION SELECT c.* FROM concept_facet_fts JOIN concept_facets f ON f.id = concept_facet_fts.rowid JOIN concepts c ON c.id = f.concept_id WHERE concept_facet_fts MATCH ? AND f.superseded_at IS NULL) AS concepts WHERE scope_type = ? AND scope_key = ? AND (? IS NULL OR status = ?) AND status NOT IN ('archived', 'merged') ORDER BY id",
            columns = CONCEPT_COLUMNS
        )))
        .bind(&expression)
        .bind(&expression)
        .bind(&filter.scope_type)
        .bind(&filter.scope_key)
        .bind(filter.status.as_deref())
        .bind(filter.status.as_deref())
        .fetch_all(self.pool)
        .await
        .map_err(|source| database("search", source))
    }

    pub async fn add_facet(
        &self,
        concept_id: i64,
        language: &str,
        facet_type: &str,
        value: &str,
    ) -> Result<ConceptFacetRecord> {
        let language = normalize_language(language)?;
        let facet_type = validate_facet_type(facet_type)?;
        let value = required("value", value)?;
        let mut transaction = begin_immediate(self.pool, "add facet").await?;
        let result = async {
            require_active_concept(&mut transaction, concept_id, "add facet").await?;
            let now = Utc::now();
            let id = sqlx::query("INSERT INTO concept_facets(concept_id, language, facet_type, value, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(concept_id).bind(&language).bind(&facet_type).bind(&value).bind(now).bind(now)
                .execute(&mut *transaction).await.map_err(|source| database("add facet", source))?.last_insert_rowid();
            if !language.is_empty() {
                sqlx::query("INSERT INTO concept_facet_language_tags(facet_id, language_tag) VALUES (?, ?)")
                    .bind(id).bind(&language).execute(&mut *transaction).await.map_err(|source| database("add facet", source))?;
            }
            get_facet_on(&mut transaction, id, "add facet").await
        }.await;
        commit_write(transaction, "add facet", result).await
    }

    pub async fn update_facet(&self, facet_id: i64, value: &str) -> Result<ConceptFacetRecord> {
        let value = required("value", value)?;
        let mut transaction = begin_immediate(self.pool, "update facet").await?;
        let result = async {
            let facet = get_facet_on(&mut transaction, facet_id, "update facet").await?;
            require_active_concept(&mut transaction, facet.concept_id, "update facet").await?;
            sqlx::query("UPDATE concept_facets SET value = ?, updated_at = ? WHERE id = ?")
                .bind(value)
                .bind(Utc::now())
                .bind(facet_id)
                .execute(&mut *transaction)
                .await
                .map_err(|source| database("update facet", source))?;
            get_facet_on(&mut transaction, facet_id, "update facet").await
        }
        .await;
        commit_write(transaction, "update facet", result).await
    }

    pub async fn list_facets(&self, concept_id: i64) -> Result<Vec<ConceptFacet>> {
        self.get(concept_id).await?;
        let rows = sqlx::query_as::<_, ConceptFacetRecord>(AssertSqlSafe(format!(
            "SELECT {FACET_COLUMNS} FROM concept_facets WHERE concept_id = ? AND superseded_at IS NULL ORDER BY id"
        )))
        .bind(concept_id)
        .fetch_all(self.pool)
        .await
        .map_err(|source| database("list facets", source))?;
        hydrate_facets(self.pool, rows).await
    }

    pub async fn set_canonical_facet(&self, facet_id: i64) -> Result<()> {
        let mut transaction = begin_immediate(self.pool, "set canonical facet").await?;
        let result = async {
            let facet = get_facet_on(&mut transaction, facet_id, "set canonical facet").await?;
            require_active_concept(&mut transaction, facet.concept_id, "set canonical facet").await?;
            sqlx::query("UPDATE concept_facets SET is_canonical = 0, updated_at = ? WHERE concept_id = ? AND facet_type = ? AND language = ? AND superseded_at IS NULL")
                .bind(Utc::now()).bind(facet.concept_id).bind(&facet.facet_type).bind(&facet.language)
                .execute(&mut *transaction).await.map_err(|source| database("set canonical facet", source))?;
            sqlx::query("UPDATE concept_facets SET is_canonical = 1, updated_at = ? WHERE id = ?")
                .bind(Utc::now()).bind(facet_id).execute(&mut *transaction).await.map_err(|source| database("set canonical facet", source))?;
            Ok(())
        }.await;
        commit_write(transaction, "set canonical facet", result).await
    }

    pub async fn rename_concept(
        &self,
        id: i64,
        new_name: &str,
        reason: &str,
    ) -> Result<ConceptRecord> {
        let new_name = required("canonical_name", new_name)?;
        let reason = reason.trim();
        let mut transaction = begin_immediate(self.pool, "rename").await?;
        let result = async {
            let concept = require_active_concept(&mut transaction, id, "rename").await?;
            if concept.canonical_name == new_name {
                return Ok(concept);
            }
            let duplicate: Option<i64> = sqlx::query_scalar("SELECT id FROM concepts WHERE id <> ? AND canonical_name = ? COLLATE NOCASE AND scope_type = ? AND scope_key = ? AND status NOT IN ('archived','merged') ORDER BY id LIMIT 1")
                .bind(id).bind(&new_name).bind(&concept.scope_type).bind(&concept.scope_key).fetch_optional(&mut *transaction).await.map_err(|source| database("rename", source))?;
            if duplicate.is_some() {
                return Err(conflict("rename", "an active concept already has that name in this scope"));
            }
            let now = Utc::now();
            ensure_former_label(
                &mut transaction,
                id,
                &concept.canonical_name,
                concept.confidence,
                now,
            )
            .await?;
            sqlx::query("UPDATE concepts SET canonical_name = ?, updated_at = ? WHERE id = ?")
                .bind(&new_name).bind(now).bind(id).execute(&mut *transaction).await.map_err(|source| database("rename", source))?;
            sqlx::query("INSERT INTO concept_renames(concept_id, old_name, new_name, reason, created_at) VALUES (?, ?, ?, ?, ?)")
                .bind(id).bind(&concept.canonical_name).bind(&new_name).bind(reason).bind(now).execute(&mut *transaction).await.map_err(|source| database("rename", source))?;
            get_concept_on(&mut transaction, id, "rename").await
        }.await;
        commit_write(transaction, "rename", result).await
    }

    pub async fn archive(&self, id: i64, _reason: &str) -> Result<()> {
        let mut transaction = begin_immediate(self.pool, "archive").await?;
        let result = async {
            let concept = get_concept_on(&mut transaction, id, "archive").await?;
            if concept.status == "merged" {
                return Err(conflict("archive", "merged concepts cannot be archived"));
            }
            sqlx::query("UPDATE concepts SET status = 'archived', updated_at = ? WHERE id = ?")
                .bind(Utc::now())
                .bind(id)
                .execute(&mut *transaction)
                .await
                .map_err(|source| database("archive", source))?;
            Ok(())
        }
        .await;
        commit_write(transaction, "archive", result).await
    }

    pub async fn reinforce(&self, id: i64) -> Result<()> {
        self.adjust_confidence(id, 0.15, "reinforce").await
    }

    pub async fn decay(&self, id: i64) -> Result<()> {
        self.adjust_confidence(id, -0.15, "decay").await
    }

    async fn adjust_confidence(&self, id: i64, delta: f64, operation: &'static str) -> Result<()> {
        let mut transaction = begin_immediate(self.pool, operation).await?;
        let result = async {
            require_active_concept(&mut transaction, id, operation).await?;
            sqlx::query(
                "UPDATE concepts SET confidence = min(1.0, max(0.0, confidence + ?)), updated_at = ? WHERE id = ?",
            )
            .bind(delta)
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(|source| database(operation, source))?;
            Ok(())
        }
        .await;
        commit_write(transaction, operation, result).await
    }

    pub async fn merge_concepts(&self, source: i64, target: i64, _reason: &str) -> Result<()> {
        if source == target {
            return Err(conflict("merge", "source and target must differ"));
        }
        let mut transaction = begin_immediate(self.pool, "merge").await?;
        let result = async {
            let source_row = require_active_concept(&mut transaction, source, "merge").await?;
            let target_row = require_active_concept(&mut transaction, target, "merge").await?;
            if source_row.scope_type != target_row.scope_type || source_row.scope_key != target_row.scope_key {
                return Err(conflict("merge", "source and target scopes must match"));
            }
            ensure_no_merge_cycle(&mut transaction, source, target).await?;
            let source_label_is_a_facet: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM concept_facets WHERE concept_id = ? AND value = ? AND superseded_at IS NULL)",
            )
            .bind(source)
            .bind(&source_row.canonical_name)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|source| database("merge", source))?;
            if !source_label_is_a_facet {
                ensure_former_label(
                    &mut transaction,
                    target,
                    &source_row.canonical_name,
                    source_row.confidence,
                    Utc::now(),
                )
                .await?;
            }
            merge_facets(&mut transaction, source, target).await?;
            sqlx::query("INSERT INTO concept_semantic_tags(concept_id, tag, confidence, created_at) SELECT ?, tag, confidence, created_at FROM concept_semantic_tags WHERE concept_id = ? ON CONFLICT(concept_id, tag) DO UPDATE SET confidence = max(concept_semantic_tags.confidence, excluded.confidence)")
                .bind(target).bind(source).execute(&mut *transaction).await.map_err(|source| database("merge", source))?;
            sqlx::query("INSERT INTO crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at) SELECT crystal_id, ?, link_type, confidence, created_at FROM crystal_concepts WHERE concept_id = ? ON CONFLICT(crystal_id, concept_id, link_type) DO UPDATE SET confidence = max(crystal_concepts.confidence, excluded.confidence)")
                .bind(target).bind(source).execute(&mut *transaction).await.map_err(|source| database("merge", source))?;
            sqlx::query("DELETE FROM crystal_concepts WHERE concept_id = ?").bind(source).execute(&mut *transaction).await.map_err(|source| database("merge", source))?;
            sqlx::query("UPDATE concepts SET status = 'merged', merged_into_concept_id = ?, updated_at = ? WHERE id = ?")
                .bind(target).bind(Utc::now()).bind(source).execute(&mut *transaction).await.map_err(|source| database("merge", source))?;
            Ok(())
        }.await;
        commit_write(transaction, "merge", result).await
    }

    pub async fn link_crystal(
        &self,
        crystal_id: i64,
        concept_id: i64,
        link_type: &str,
        confidence: f64,
    ) -> Result<()> {
        let link_type = required("link_type", link_type)?;
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(invalid("confidence", "must be finite and between 0 and 1"));
        }
        let mut transaction = begin_immediate(self.pool, "link crystal").await?;
        let result = async {
            require_active_concept(&mut transaction, concept_id, "link crystal").await?;
            let crystal_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM crystals WHERE id = ?)")
                .bind(crystal_id).fetch_one(&mut *transaction).await.map_err(|source| database("link crystal", source))?;
            if !crystal_exists {
                return Err(conflict("link crystal", "unknown crystal"));
            }
            sqlx::query("INSERT INTO crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(crystal_id, concept_id, link_type) DO UPDATE SET confidence = max(crystal_concepts.confidence, excluded.confidence)")
                .bind(crystal_id).bind(concept_id).bind(link_type).bind(confidence).bind(Utc::now())
                .execute(&mut *transaction).await.map_err(|source| database("link crystal", source))?;
            Ok(())
        }.await;
        commit_write(transaction, "link crystal", result).await
    }

    pub async fn concept_ids_for_crystal(&self, crystal_id: i64) -> Result<Vec<i64>> {
        sqlx::query_scalar("SELECT cc.concept_id FROM crystal_concepts cc JOIN concepts c ON c.id = cc.concept_id WHERE cc.crystal_id = ? AND c.status NOT IN ('archived','merged') ORDER BY cc.concept_id")
            .bind(crystal_id).fetch_all(self.pool).await.map_err(|source| database("list crystal concepts", source))
    }

    pub async fn set_semantic_tags(&self, concept_id: i64, tags: &[String]) -> Result<()> {
        let tags = clean_values(tags, false);
        let mut transaction = begin_immediate(self.pool, "set semantic tags").await?;
        let result = async {
            require_active_concept(&mut transaction, concept_id, "set semantic tags").await?;
            sqlx::query("DELETE FROM concept_semantic_tags WHERE concept_id = ?").bind(concept_id).execute(&mut *transaction).await.map_err(|source| database("set semantic tags", source))?;
            for tag in tags {
                sqlx::query("INSERT INTO concept_semantic_tags(concept_id, tag, confidence, created_at) VALUES (?, ?, 0.2, ?)")
                    .bind(concept_id).bind(tag).bind(Utc::now()).execute(&mut *transaction).await.map_err(|source| database("set semantic tags", source))?;
            }
            Ok(())
        }.await;
        commit_write(transaction, "set semantic tags", result).await
    }

    pub async fn recall_boosts(
        &self,
        crystal_ids: &[i64],
        query: &str,
        story_scopes: &[String],
    ) -> Result<HashMap<i64, f64>> {
        if crystal_ids.is_empty() || query.trim().is_empty() {
            return Ok(HashMap::new());
        }
        let expression = super::search_expression(query);
        let scopes = clean_values(story_scopes, false);
        let mut boosts = HashMap::<i64, f64>::new();
        let scope_chunks: Vec<&[String]> = if scopes.is_empty() {
            vec![&[]]
        } else {
            scopes.chunks(RECALL_IN_CHUNK_SIZE).collect()
        };
        for id_chunk in crystal_ids.chunks(RECALL_IN_CHUNK_SIZE) {
            for scope_chunk in &scope_chunks {
                let mut builder = QueryBuilder::<Sqlite>::new(
                    "SELECT DISTINCT cc.crystal_id, CASE WHEN EXISTS(SELECT 1 FROM concept_facets sf JOIN concept_facet_story_scopes ss ON ss.facet_id = sf.id WHERE sf.concept_id = c.id AND sf.superseded_at IS NULL AND ss.story_scope IN (",
                );
                push_strings(&mut builder, scope_chunk);
                builder.push(")) THEN 0.40 ELSE 0.15 END AS boost FROM crystal_concepts cc JOIN concepts c ON c.id = cc.concept_id LEFT JOIN concept_facets f ON f.concept_id = c.id AND f.superseded_at IS NULL WHERE cc.crystal_id IN (");
                push_ids(&mut builder, id_chunk);
                builder.push(") AND c.status NOT IN ('archived','merged') AND (c.canonical_name IN (SELECT canonical_name FROM concepts_fts JOIN concepts x ON x.id=concepts_fts.rowid WHERE concepts_fts MATCH ").push_bind(&expression).push(") OR f.value IN (SELECT value FROM concept_facet_fts JOIN concept_facets fx ON fx.id=concept_facet_fts.rowid WHERE concept_facet_fts MATCH ").push_bind(&expression).push(")) ORDER BY cc.crystal_id");
                let rows: Vec<(i64, f64)> = builder
                    .build_query_as()
                    .fetch_all(self.pool)
                    .await
                    .map_err(|source| database("recall boosts", source))?;
                for (id, boost) in rows {
                    boosts
                        .entry(id)
                        .and_modify(|current| *current = current.max(boost))
                        .or_insert(boost);
                }
            }
        }
        Ok(boosts)
    }
}

pub struct ConceptProposalStore<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ConceptProposalStore<'a> {
    #[must_use]
    pub const fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, input: CreateProposalInput) -> Result<i64> {
        let input = validate_proposal(input)?;
        let now = Utc::now();
        sqlx::query("INSERT INTO concept_proposals(series_slug, source_language, target_language, concept_text, source_form, canonical_rendering, approved_variants_json, forbidden_variants_json, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, '[]', '[]', 'pending', ?, ?)")
            .bind(input.series_slug).bind(input.source_language).bind(input.target_language).bind(input.concept_text).bind(input.source_form).bind(input.canonical_rendering).bind(now).bind(now)
            .execute(self.pool).await.map(|result| result.last_insert_rowid()).map_err(|source| database("create proposal", source))
    }

    pub async fn get(&self, id: i64) -> Result<ConceptProposal> {
        let row = sqlx::query_as::<_, ConceptProposalRecord>(AssertSqlSafe(format!(
            "SELECT {PROPOSAL_COLUMNS} FROM concept_proposals WHERE id = ?"
        )))
        .bind(id)
        .fetch_optional(self.pool)
        .await
        .map_err(|source| database("get proposal", source))?
        .ok_or(ConceptError::ProposalNotFound { id })?;
        decode_proposal(row)
    }

    pub async fn list_pending(&self) -> Result<Vec<ConceptProposal>> {
        let rows = sqlx::query_as::<_, ConceptProposalRecord>(AssertSqlSafe(format!(
            "SELECT {PROPOSAL_COLUMNS} FROM concept_proposals WHERE status = 'pending' ORDER BY id"
        )))
        .fetch_all(self.pool)
        .await
        .map_err(|source| database("list proposals", source))?;
        rows.into_iter().map(decode_proposal).collect()
    }

    pub async fn approve(&self, id: i64) -> Result<()> {
        self.set_status(id, "approved").await
    }
    pub async fn reject(&self, id: i64) -> Result<()> {
        self.set_status(id, "rejected").await
    }

    async fn set_status(&self, id: i64, status: &'static str) -> Result<()> {
        let mut transaction = begin_immediate(self.pool, "set proposal status").await?;
        let result = async {
            let row = sqlx::query_as::<_, ConceptProposalRecord>(AssertSqlSafe(format!(
                "SELECT {PROPOSAL_COLUMNS} FROM concept_proposals WHERE id = ?"
            )))
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|source| database("set proposal status", source))?
            .ok_or(ConceptError::ProposalNotFound { id })?;
            decode_proposal(row)?;
            let current: String =
                sqlx::query_scalar("SELECT status FROM concept_proposals WHERE id = ?")
                    .bind(id)
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(|source| database("set proposal status", source))?;
            if current == status {
                return Ok(());
            }
            if current != "pending" {
                return Err(conflict(
                    "set proposal status",
                    "proposal already reached a terminal state",
                ));
            }
            sqlx::query("UPDATE concept_proposals SET status = ?, updated_at = ? WHERE id = ?")
                .bind(status)
                .bind(Utc::now())
                .bind(id)
                .execute(&mut *transaction)
                .await
                .map_err(|source| database("set proposal status", source))?;
            Ok(())
        }
        .await;
        commit_write(transaction, "set proposal status", result).await
    }
}

async fn merge_facets(connection: &mut SqliteConnection, source: i64, target: i64) -> Result<()> {
    let facets = sqlx::query_as::<_, ConceptFacetRecord>(AssertSqlSafe(format!(
        "SELECT {FACET_COLUMNS} FROM concept_facets WHERE concept_id = ? ORDER BY id"
    )))
    .bind(source)
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| database("merge", source))?;
    for facet in facets {
        let duplicate: Option<i64> = sqlx::query_scalar("SELECT id FROM concept_facets WHERE concept_id = ? AND facet_type = ? AND value = ? AND language = ? AND superseded_at IS ? ORDER BY id LIMIT 1")
            .bind(target).bind(&facet.facet_type).bind(&facet.value).bind(&facet.language).bind(facet.superseded_at).fetch_optional(&mut *connection).await.map_err(|source| database("merge", source))?;
        if let Some(existing) = duplicate {
            for (table, column) in [
                ("concept_facet_language_tags", "language_tag"),
                ("concept_facet_story_scopes", "story_scope"),
                ("concept_facet_semantic_tags", "semantic_tag"),
            ] {
                let sql = format!(
                    "INSERT OR IGNORE INTO {table}(facet_id, {column}) SELECT ?, {column} FROM {table} WHERE facet_id = ?"
                );
                sqlx::query(AssertSqlSafe(sql))
                    .bind(existing)
                    .bind(facet.id)
                    .execute(&mut *connection)
                    .await
                    .map_err(|source| database("merge", source))?;
            }
            sqlx::query("DELETE FROM concept_facets WHERE id = ?")
                .bind(facet.id)
                .execute(&mut *connection)
                .await
                .map_err(|source| database("merge", source))?;
        } else {
            sqlx::query("UPDATE concept_facets SET concept_id = ?, is_canonical = 0, updated_at = ? WHERE id = ?")
                .bind(target)
                .bind(Utc::now())
                .bind(facet.id)
                .execute(&mut *connection)
                .await
                .map_err(|source| database("merge", source))?;
        }
    }
    Ok(())
}

async fn ensure_no_merge_cycle(
    connection: &mut SqliteConnection,
    source: i64,
    mut target: i64,
) -> Result<()> {
    let mut seen = HashSet::new();
    loop {
        if target == source || !seen.insert(target) {
            return Err(conflict("merge", "merge would create a cycle"));
        }
        let next: Option<i64> =
            sqlx::query_scalar("SELECT merged_into_concept_id FROM concepts WHERE id = ?")
                .bind(target)
                .fetch_one(&mut *connection)
                .await
                .map_err(|source| database("merge", source))?;
        let Some(next) = next else {
            return Ok(());
        };
        target = next;
    }
}

async fn ensure_former_label(
    connection: &mut SqliteConnection,
    concept_id: i64,
    value: &str,
    confidence: f64,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM concept_facets WHERE concept_id = ? AND value = ? AND superseded_at IS NULL)",
    )
    .bind(concept_id)
    .bind(value)
    .fetch_one(&mut *connection)
    .await
    .map_err(|source| database("preserve former label", source))?;
    if !exists {
        sqlx::query("INSERT INTO concept_facets(concept_id, language, facet_type, value, confidence, created_at, updated_at) VALUES (?, '', 'former_label', ?, ?, ?, ?)")
            .bind(concept_id)
            .bind(value)
            .bind(confidence)
            .bind(now)
            .bind(now)
            .execute(&mut *connection)
            .await
            .map_err(|source| database("preserve former label", source))?;
    }
    Ok(())
}

async fn hydrate_concepts(pool: &SqlitePool, rows: Vec<ConceptRecord>) -> Result<Vec<Concept>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let mut tags = Vec::<(i64, String)>::new();
    for chunk in ids.chunks(HYDRATION_CHUNK_SIZE) {
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT concept_id, tag FROM concept_semantic_tags WHERE concept_id IN (",
        );
        push_ids(&mut builder, chunk);
        builder.push(") ORDER BY concept_id, tag");
        tags.extend(
            builder
                .build_query_as()
                .fetch_all(pool)
                .await
                .map_err(|source| database("hydrate concepts", source))?,
        );
    }
    let mut grouped: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for (id, value) in tags {
        grouped.entry(id).or_default().push(value);
    }
    Ok(rows
        .into_iter()
        .map(|record| Concept {
            semantic_tags: grouped.remove(&record.id).unwrap_or_default(),
            record,
        })
        .collect())
}

async fn hydrate_facets(
    pool: &SqlitePool,
    rows: Vec<ConceptFacetRecord>,
) -> Result<Vec<ConceptFacet>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let languages =
        load_facet_texts(pool, &ids, "concept_facet_language_tags", "language_tag").await?;
    let stories = load_facet_texts(pool, &ids, "concept_facet_story_scopes", "story_scope").await?;
    let semantics =
        load_facet_texts(pool, &ids, "concept_facet_semantic_tags", "semantic_tag").await?;
    Ok(rows
        .into_iter()
        .map(|record| {
            let fallback = if record.language.is_empty() {
                Vec::new()
            } else {
                vec![record.language.clone()]
            };
            ConceptFacet {
                language_tags: languages.get(&record.id).cloned().unwrap_or(fallback),
                story_scopes: stories.get(&record.id).cloned().unwrap_or_default(),
                semantic_tags: semantics.get(&record.id).cloned().unwrap_or_default(),
                record,
            }
        })
        .collect())
}

async fn load_facet_texts(
    pool: &SqlitePool,
    ids: &[i64],
    table: &str,
    column: &str,
) -> Result<BTreeMap<i64, Vec<String>>> {
    let mut rows = Vec::<(i64, String)>::new();
    for chunk in ids.chunks(HYDRATION_CHUNK_SIZE) {
        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT facet_id, {column} FROM {table} WHERE facet_id IN ("
        ));
        push_ids(&mut builder, chunk);
        builder.push(format!(") ORDER BY facet_id, {column}"));
        rows.extend(
            builder
                .build_query_as()
                .fetch_all(pool)
                .await
                .map_err(|source| database("hydrate facets", source))?,
        );
    }
    let mut values = BTreeMap::new();
    for (id, value) in rows {
        values.entry(id).or_insert_with(Vec::new).push(value);
    }
    Ok(values)
}

fn decode_proposal(record: ConceptProposalRecord) -> Result<ConceptProposal> {
    let approved_variants = serde_json::from_str::<Vec<String>>(&record.approved_variants_json)
        .map_err(|source| ConceptError::Json {
            field: "approved_variants",
            source,
        })?;
    let forbidden_variants = serde_json::from_str::<Vec<String>>(&record.forbidden_variants_json)
        .map_err(|source| ConceptError::Json {
        field: "forbidden_variants",
        source,
    })?;
    Ok(ConceptProposal {
        record,
        approved_variants,
        forbidden_variants,
    })
}

fn validate_concept_input(mut input: CreateConceptInput) -> Result<CreateConceptInput> {
    input.canonical_name = required("canonical_name", &input.canonical_name)?;
    input.scope_type = required("scope_type", &input.scope_type)?;
    input.scope_key = input.scope_key.trim().to_owned();
    validate_scope(&input.scope_type, &input.scope_key)?;
    Ok(input)
}
fn validate_filter(filter: &ConceptFilter) -> Result<()> {
    validate_scope(&filter.scope_type, &filter.scope_key)?;
    if let Some(status) = &filter.status
        && !matches!(
            status.as_str(),
            "candidate" | "established" | "archived" | "merged"
        )
    {
        return Err(invalid("status", "is unknown"));
    }
    Ok(())
}
fn validate_proposal(mut input: CreateProposalInput) -> Result<CreateProposalInput> {
    input.series_slug = required("series_slug", &input.series_slug)?;
    input.source_language = normalize_language(&input.source_language)?;
    if input.source_language.is_empty() {
        return Err(invalid("source_language", "must not be empty"));
    }
    input.target_language = normalize_language(&input.target_language)?;
    if input.target_language.is_empty() {
        return Err(invalid("target_language", "must not be empty"));
    }
    input.concept_text = required("concept_text", &input.concept_text)?;
    input.source_form = required("source_form", &input.source_form)?;
    input.canonical_rendering = required("canonical_rendering", &input.canonical_rendering)?;
    Ok(input)
}
fn validate_scope(scope_type: &str, scope_key: &str) -> Result<()> {
    if (scope_type == "global") != scope_key.is_empty() {
        return Err(invalid(
            "scope",
            "global requires an empty key and non-global requires a key",
        ));
    }
    Ok(())
}
fn validate_facet_type(value: &str) -> Result<String> {
    let value = value.trim();
    if !matches!(
        value,
        "name" | "rendering" | "description" | "note" | "alias" | "former_label"
    ) {
        return Err(invalid("facet_type", "is unknown"));
    }
    Ok(value.to_owned())
}
fn normalize_language(value: &str) -> Result<String> {
    let value = value.trim().to_lowercase();
    if value.chars().any(char::is_whitespace) {
        return Err(invalid("language", "must be a compact language tag"));
    }
    Ok(value)
}
fn required(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(invalid(field, "must not be empty"))
    } else {
        Ok(value.to_owned())
    }
}
fn clean_values(values: &[String], lowercase: bool) -> Vec<String> {
    let values: Vec<String> = values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| {
            if lowercase {
                v.to_lowercase()
            } else {
                v.to_owned()
            }
        })
        .collect();
    crate::values::normalize_tuple(&values)
}
fn push_ids(builder: &mut QueryBuilder<Sqlite>, ids: &[i64]) {
    let mut values = builder.separated(", ");
    for id in ids {
        values.push_bind(*id);
    }
}
fn push_strings(builder: &mut QueryBuilder<Sqlite>, strings: &[String]) {
    if strings.is_empty() {
        builder.push("SELECT '' WHERE 0");
    } else {
        let mut values = builder.separated(", ");
        for value in strings {
            values.push_bind(value);
        }
    }
}

async fn begin_immediate(
    pool: &SqlitePool,
    operation: &'static str,
) -> Result<Transaction<'static, Sqlite>> {
    pool.begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|source| database(operation, source))
}
async fn commit_write<T>(
    transaction: Transaction<'static, Sqlite>,
    operation: &'static str,
    result: Result<T>,
) -> Result<T> {
    match result {
        Ok(value) => transaction
            .commit()
            .await
            .map(|()| value)
            .map_err(|source| database(operation, source)),
        Err(error) => Err(error),
    }
}
async fn get_concept_on(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<ConceptRecord> {
    sqlx::query_as::<_, ConceptRecord>(AssertSqlSafe(format!(
        "SELECT {CONCEPT_COLUMNS} FROM concepts WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|source| database(operation, source))?
    .ok_or(ConceptError::NotFound { id })
}
async fn require_active_concept(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<ConceptRecord> {
    let row = get_concept_on(connection, id, operation).await?;
    if matches!(row.status.as_str(), "archived" | "merged") {
        Err(conflict(operation, "concept is inactive"))
    } else {
        Ok(row)
    }
}
async fn get_facet_on(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<ConceptFacetRecord> {
    sqlx::query_as::<_, ConceptFacetRecord>(AssertSqlSafe(format!(
        "SELECT {FACET_COLUMNS} FROM concept_facets WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|source| database(operation, source))?
    .ok_or(ConceptError::FacetNotFound { id })
}
fn invalid(field: &'static str, reason: impl Into<String>) -> ConceptError {
    ConceptError::Validation {
        field,
        reason: reason.into(),
    }
}
fn conflict(operation: &'static str, reason: impl Into<String>) -> ConceptError {
    ConceptError::Conflict {
        operation,
        reason: reason.into(),
    }
}
fn database(operation: &'static str, source: sqlx::Error) -> ConceptError {
    ConceptError::Database { operation, source }
}
