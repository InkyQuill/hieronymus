use std::path::PathBuf;

use chrono::Utc;
use rusqlite::Connection;
use serde_json::{Value, json};
use toml::Table;

use crate::concept_models::{ConceptFacetRecord, ConceptRecord};
use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::short_memory::search_expression;

pub const CONCEPT_CANDIDATE: &str = "candidate";
pub const CONCEPT_ESTABLISHED: &str = "established";
pub const CONCEPT_ARCHIVED: &str = "archived";
pub const CONCEPT_MERGED: &str = "merged";
const VALID_FACET_KINDS: [&str; 4] = ["name", "rendering", "description", "note"];
const COMPATIBILITY_FACET_TYPES: [&str; 2] = ["alias", "former_label"];
const LEGACY_PUBLIC_STATUSES: [(&str, &str); 2] =
    [("vague", CONCEPT_CANDIDATE), ("solid", CONCEPT_ESTABLISHED)];

/// Confidence at which a concept with enough linked evidence is established.
pub const ESTABLISHED_CONFIDENCE: f64 = 0.75;
/// Linked-evidence count required (together with confidence) for established.
pub const ESTABLISHED_EVIDENCE_COUNT: i64 = 2;

/// Recall-time boost for a crystal whose linked concept matches the query
/// text (Python `_CONCEPT_RECALL_TEXT_BOOST`).
pub const CONCEPT_RECALL_TEXT_BOOST: f64 = 0.15;
/// Recall-time boost for a crystal whose linked concept has a query-matching,
/// non-superseded facet scoped to a context story scope (Python
/// `_CONCEPT_RECALL_STORY_SCOPE_BOOST`).
pub const CONCEPT_RECALL_STORY_SCOPE_BOOST: f64 = 0.25;

#[derive(Debug, thiserror::Error)]
pub enum ConceptError {
    #[error("unknown concept: {0}")]
    UnknownConcept(i64),
    #[error("unknown concept facet: {0}")]
    UnknownFacet(i64),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}

pub fn public_status(status: &str) -> &str {
    for (legacy, current) in LEGACY_PUBLIC_STATUSES {
        if status == legacy {
            return current;
        }
    }
    status
}

fn storage_status(status: &str) -> Result<String, ConceptError> {
    let public = public_status(status).to_string();
    if [
        CONCEPT_CANDIDATE,
        CONCEPT_ESTABLISHED,
        CONCEPT_ARCHIVED,
        CONCEPT_MERGED,
    ]
    .contains(&public.as_str())
    {
        Ok(public)
    } else {
        Err(ConceptError::Invalid(format!(
            "unknown concept status: {status}"
        )))
    }
}

fn status_filter_values(status: &str) -> Result<Vec<String>, ConceptError> {
    let public = storage_status(status)?;
    let mut values = vec![public.clone()];
    if public == CONCEPT_CANDIDATE {
        values.push("vague".to_string());
    } else if public == CONCEPT_ESTABLISHED {
        values.push("solid".to_string());
    }
    Ok(values)
}

fn is_inactive_status(status: &str) -> bool {
    matches!(public_status(status), CONCEPT_ARCHIVED | CONCEPT_MERGED)
}

fn clamp_confidence(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

pub fn normalize_facet_kind(kind: &str) -> Result<String, ConceptError> {
    let clean = kind.trim();
    if VALID_FACET_KINDS.contains(&clean) {
        Ok(clean.to_string())
    } else {
        Err(ConceptError::Invalid(format!(
            "unknown concept facet kind: {kind}"
        )))
    }
}

fn normalize_facet_storage_kind(
    kind: Option<&str>,
    facet_type: Option<&str>,
) -> Result<String, ConceptError> {
    match kind {
        Some(kind) => {
            let clean = normalize_facet_kind(kind)?;
            if let Some(facet_type) = facet_type {
                let trimmed = facet_type.trim();
                if !trimmed.is_empty() && trimmed != clean {
                    return Err(ConceptError::Invalid(
                        "kind and facet_type must not conflict".to_string(),
                    ));
                }
            }
            Ok(clean)
        }
        None => match facet_type {
            None => Ok("name".to_string()),
            Some(facet_type) => {
                let clean = facet_type.trim();
                if VALID_FACET_KINDS.contains(&clean) || COMPATIBILITY_FACET_TYPES.contains(&clean)
                {
                    Ok(clean.to_string())
                } else {
                    Err(ConceptError::Invalid(format!(
                        "unknown concept facet kind: {facet_type}"
                    )))
                }
            }
        },
    }
}

fn clean_tags<'a>(tags: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut cleaned: Vec<String> = tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect();
    cleaned.sort();
    cleaned.dedup();
    cleaned
}

/// Parse a strict proposal's stored variant array; malformed JSON is a
/// rejection, never forwarded as raw text.
fn parse_variant_array(raw: &str) -> Result<Vec<String>, ConceptError> {
    serde_json::from_str(raw)
        .map_err(|error| ConceptError::Invalid(format!("malformed proposal variants: {error}")))
}

fn clean_language_tags<'a>(
    legacy_language: &'a str,
    language_tags: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut cleaned: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for tag in std::iter::once(legacy_language).chain(language_tags) {
        let clean_tag = tag.trim().to_lowercase();
        if !clean_tag.is_empty() && seen.insert(clean_tag.clone()) {
            cleaned.push(clean_tag);
        }
    }
    cleaned
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

/// Parameters for [`ConceptStore::create_concept`].
/// Parameters for [`ConceptStore::create_concept`]. The default status is
/// `candidate`.
#[derive(Debug, Clone)]
pub struct NewConcept {
    pub description: String,
    pub status: String,
    pub confidence: f64,
    pub scope_type: String,
    pub scope_key: String,
    pub semantic_tags: Vec<String>,
}

impl Default for NewConcept {
    fn default() -> Self {
        Self {
            description: String::new(),
            status: "candidate".to_string(),
            confidence: 0.2,
            scope_type: "global".to_string(),
            scope_key: String::new(),
            semantic_tags: Vec::new(),
        }
    }
}

/// Concept store over the data-root database.
pub struct ConceptStore {
    config: HieronymusConfig,
}

/// Optional facet parameters shared by add/update.
#[derive(Debug, Clone, Default)]
pub struct FacetFields {
    pub language: String,
    pub language_tags: Vec<String>,
    pub kind: Option<String>,
    pub facet_type: Option<String>,
    pub confidence: Option<f64>,
    pub source_crystal_id: Option<i64>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
}

/// Presence-aware facet patch for [`ConceptStore::update_facet`]: every field
/// mirrors one optional field of the frozen `hieronymus_concept_facet_update`
/// schema and preserves the difference between an omitted key (`None`) and an
/// explicit JSON `null` (`Some(None)`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct FacetPatch {
    #[serde(default, deserialize_with = "presence")]
    pub facet_type: Option<Option<String>>,
    #[serde(default, deserialize_with = "presence")]
    pub kind: Option<Option<String>>,
    #[serde(default, deserialize_with = "presence")]
    pub value: Option<Option<String>>,
    #[serde(default, deserialize_with = "presence")]
    pub language: Option<Option<String>>,
    #[serde(default, deserialize_with = "presence")]
    pub language_tags: Option<Option<Vec<String>>>,
    #[serde(default, deserialize_with = "presence")]
    pub story_scopes: Option<Option<Vec<String>>>,
    #[serde(default, deserialize_with = "presence")]
    pub semantic_tags: Option<Option<Vec<String>>>,
    #[serde(default, deserialize_with = "presence")]
    pub confidence: Option<Option<f64>>,
    #[serde(default, deserialize_with = "presence")]
    pub source_crystal_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "presence")]
    pub is_canonical: Option<Option<bool>>,
}

/// Deserialize one optional patch field, preserving explicit nulls: an
/// omitted key defaults to `None`, and a present key decodes as
/// `Some(Option::<T>)` so a JSON `null` becomes `Some(None)` instead of
/// collapsing into "omitted".
fn presence<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(<Option<T> as serde::Deserialize>::deserialize(
        deserializer,
    )?))
}

impl ConceptStore {
    pub fn open(config: &HieronymusConfig) -> Result<Self, ConceptError> {
        open_migrated(&config.database_path())?;
        Ok(Self {
            config: config.clone(),
        })
    }

    pub fn create_concept(
        &self,
        canonical_name: &str,
        fields: &NewConcept,
    ) -> Result<ConceptRecord, ConceptError> {
        let name = canonical_name.trim();
        if name.is_empty() {
            return Err(ConceptError::Invalid(
                "concept canonical_name must not be empty".to_string(),
            ));
        }
        validate_scope(&fields.scope_type, &fields.scope_key)?;
        let clean_description = fields.description.trim();
        let storage = storage_status(&fields.status)?;
        if is_inactive_status(&storage) {
            return Err(ConceptError::Invalid(
                "concept_create cannot set inactive status".to_string(),
            ));
        }
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "insert into concepts(
               canonical_name, description, scope_type, scope_key,
               status, confidence, created_at, updated_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                name,
                clean_description,
                fields.scope_type,
                fields.scope_key,
                storage,
                clamp_confidence(fields.confidence),
                now,
                now,
            ],
        )?;
        let concept_id = transaction.last_insert_rowid();
        set_semantic_tags(&transaction, concept_id, &fields.semantic_tags, &now)?;
        transaction.commit()?;
        drop(connection);
        self.get(concept_id)
    }

    pub fn get(&self, concept_id: i64) -> Result<ConceptRecord, ConceptError> {
        let connection = self.connection()?;
        let concept_row = require_concept_fields(&connection, concept_id)?;
        concept_record_from_row(&connection, &concept_row)
    }

    pub fn list_concepts(
        &self,
        status: Option<&str>,
        semantic_tag: Option<&str>,
    ) -> Result<Vec<ConceptRecord>, ConceptError> {
        let mut query = String::from("select distinct c.* from concepts c");
        let mut params: Vec<String> = Vec::new();
        let mut where_clauses: Vec<String> = Vec::new();
        if let Some(semantic_tag) = semantic_tag {
            let clean = semantic_tag.trim();
            if !clean.is_empty() {
                query.push_str(" join concept_semantic_tags t on t.concept_id = c.id");
                where_clauses.push("t.tag = ?".to_string());
                params.push(clean.to_string());
            }
        }
        if let Some(status) = status {
            let values = status_filter_values(status)?;
            let placeholders = values.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            where_clauses.push(format!("c.status in ({placeholders})"));
            params.extend(values);
        }
        if !where_clauses.is_empty() {
            query.push_str(" where ");
            query.push_str(&where_clauses.join(" and "));
        }
        query.push_str(" order by c.id");

        let connection = self.connection()?;
        let mut statement = connection.prepare(&query)?;
        let rows = statement.query_map(rusqlite::params_from_iter(params.iter()), |row| {
            row_ref_from_statement_row(row)
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(concept_record_from_row(&connection, &row?)?);
        }
        Ok(records)
    }

    /// Add a facet to an active concept, with typed language tags, story
    /// scopes, and semantic tags; `is_canonical` moves the canonical flag.
    #[allow(clippy::too_many_arguments)]
    pub fn add_facet(
        &self,
        concept_id: i64,
        value: &str,
        fields: &FacetFields,
        confidence: f64,
        is_canonical: bool,
    ) -> Result<ConceptFacetRecord, ConceptError> {
        let clean_value = value.trim();
        if clean_value.is_empty() {
            return Err(ConceptError::Invalid(
                "concept facet value must not be empty".to_string(),
            ));
        }
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        require_active_concept(&transaction, concept_id)?;
        let facet_id = add_facet_with_connection(
            &transaction,
            concept_id,
            clean_value,
            fields,
            confidence,
            is_canonical,
            &now,
        )?;
        transaction.commit()?;
        drop(connection);
        get_facet(&self.config, facet_id)
    }

    pub fn list_facets(&self, concept_id: i64) -> Result<Vec<ConceptFacetRecord>, ConceptError> {
        let connection = self.connection()?;
        require_concept_fields(&connection, concept_id)?;
        let ids: Vec<i64> = {
            let mut statement = connection.prepare(
                "select id from concept_facets
                 where concept_id = ?1 and superseded_at is null
                 order by is_canonical desc, id",
            )?;
            let rows = statement.query_map([concept_id], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut facets = Vec::with_capacity(ids.len());
        for id in ids {
            facets.push(get_facet(&self.config, id)?);
        }
        Ok(facets)
    }

    /// Move the canonical flag of an active concept to `facet_id`.
    pub fn set_canonical_facet(&self, concept_id: i64, facet_id: i64) -> Result<(), ConceptError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        require_active_concept(&transaction, concept_id)?;
        set_canonical_facet_with_connection(&transaction, concept_id, facet_id)?;
        transaction.commit()?;
        Ok(())
    }

    /// Update one live facet of an active concept from a presence-aware
    /// [`FacetPatch`]. The Python `update_facet` reference cannot distinguish
    /// an explicit JSON `null` from an omitted field and treats both as
    /// "unchanged" for every optional field, so both patch states map to
    /// "keep the stored value" here; a present value replaces it (an empty
    /// list clears a list field, an explicitly empty value is rejected). The
    /// row update, side-table replacements, and the requested canonical move
    /// commit in one transaction, so the FTS triggers keep the facet
    /// projection consistent with the row or nothing at all.
    pub fn update_facet(
        &self,
        facet_id: i64,
        patch: &FacetPatch,
    ) -> Result<ConceptFacetRecord, ConceptError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let row = require_live_facet_row(&transaction, facet_id)?;
        let concept_id = row.concept_id;
        require_active_concept(&transaction, concept_id)?;

        let next_value = match &patch.value {
            None | Some(None) => row.value.clone(),
            Some(Some(value)) => {
                let clean = value.trim();
                if clean.is_empty() {
                    return Err(ConceptError::Invalid(
                        "concept facet value must not be empty".to_string(),
                    ));
                }
                clean.to_string()
            }
        };
        let kind = patch.kind.as_ref().and_then(|value| value.as_deref());
        let facet_type = patch.facet_type.as_ref().and_then(|value| value.as_deref());
        let next_kind = match (kind, facet_type) {
            // Both absent (or both null): the stored kind stays.
            (None, None) => row.facet_type.clone(),
            (kind, facet_type) => normalize_facet_storage_kind(kind, facet_type)?,
        };
        let language = patch.language.as_ref().and_then(|value| value.as_deref());
        let language_tags = patch
            .language_tags
            .as_ref()
            .and_then(|value| value.as_deref());
        let next_language_tags: Option<Vec<String>> = match (language, language_tags) {
            (None, None) => None,
            (language, language_tags) => Some(clean_language_tags(
                language.unwrap_or(""),
                language_tags.unwrap_or(&[]).iter().map(String::as_str),
            )),
        };
        let next_language = match &next_language_tags {
            None => row.language.clone(),
            Some(tags) => tags.first().cloned().unwrap_or_default(),
        };
        let next_confidence = match patch.confidence {
            None | Some(None) => row.confidence,
            Some(Some(confidence)) => clamp_confidence(confidence),
        };
        let next_source_crystal_id = match patch.source_crystal_id {
            None | Some(None) => row.source_crystal_id,
            Some(Some(source_crystal_id)) => Some(source_crystal_id),
        };
        let next_is_canonical = match patch.is_canonical {
            None | Some(None) => row.is_canonical,
            Some(Some(is_canonical)) => is_canonical,
        };
        transaction.execute(
            "update concept_facets
             set language = ?1, facet_type = ?2, value = ?3, source_crystal_id = ?4,
                 confidence = ?5, is_canonical = ?6, updated_at = ?7
             where id = ?8",
            rusqlite::params![
                next_language,
                next_kind,
                next_value,
                next_source_crystal_id,
                next_confidence,
                next_is_canonical as i64,
                now,
                facet_id,
            ],
        )?;
        if let Some(tags) = &next_language_tags {
            set_facet_language_tags(&transaction, facet_id, tags)?;
        }
        if let Some(Some(story_scopes)) = &patch.story_scopes {
            set_facet_story_scopes(&transaction, facet_id, story_scopes)?;
        }
        if let Some(Some(semantic_tags)) = &patch.semantic_tags {
            set_facet_semantic_tags(&transaction, facet_id, semantic_tags)?;
        }
        if next_is_canonical {
            set_canonical_facet_with_connection(&transaction, concept_id, facet_id)?;
        }
        transaction.commit()?;
        drop(connection);
        get_facet(&self.config, facet_id)
    }

    pub fn set_semantic_tags(&self, concept_id: i64, tags: &[String]) -> Result<(), ConceptError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        require_active_concept(&transaction, concept_id)?;
        set_semantic_tags(&transaction, concept_id, tags, &now)?;
        transaction.commit()?;
        Ok(())
    }

    /// The pending strict concept proposals as safe DTO payloads (Python
    /// `ConceptProposalStore.list_pending`'s strict half, projected through
    /// `StrictConceptProposal`): the explicit column list never forwards
    /// internal rows wholesale (`dream_run_id` and timestamps stay
    /// unprojected) and the variant arrays are parsed, not carried as raw
    /// JSON text — a malformed row is a loud rejection, not leaked SQL
    /// output.
    pub fn list_proposals(&self) -> Result<Vec<Value>, ConceptError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select id, series_slug, source_language, target_language, concept_text,
                    source_form, canonical_rendering, approved_variants_json,
                    forbidden_variants_json, rationale, status
             from strict_concept_proposals
             where status = 'pending'
             order by id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
            ))
        })?;
        let mut payloads = Vec::new();
        for row in rows {
            let (
                id,
                series_slug,
                source_language,
                target_language,
                concept_text,
                source_form,
                canonical_rendering,
                approved_variants_json,
                forbidden_variants_json,
                rationale,
                status,
            ) = row?;
            payloads.push(json!({
                "id": id,
                "series_slug": series_slug,
                "source_language": source_language,
                "target_language": target_language,
                "concept_text": concept_text,
                "source_form": source_form,
                "canonical_rendering": canonical_rendering,
                "approved_variants": parse_variant_array(&approved_variants_json)?,
                "forbidden_variants": parse_variant_array(&forbidden_variants_json)?,
                "rationale": rationale,
                "status": status,
            }));
        }
        Ok(payloads)
    }

    pub fn update_concept(
        &self,
        concept_id: i64,
        description: Option<&str>,
        status: Option<&str>,
        confidence: Option<f64>,
    ) -> Result<ConceptRecord, ConceptError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let active = require_active_concept(&transaction, concept_id)?;
        let current_status = active.status.clone();
        let current_description = active.description.clone();
        let current_confidence = active.confidence;
        let next_description = match description {
            None => current_description.clone(),
            Some(description) => description.trim().to_string(),
        };
        let next_status = match status {
            None => current_status,
            Some(status) => storage_status(status)?,
        };
        let _ = current_description.clone();
        if is_inactive_status(&next_status) {
            return Err(ConceptError::Invalid(
                "concept_update cannot set inactive status".to_string(),
            ));
        }
        let next_confidence = match confidence {
            None => current_confidence,
            Some(confidence) => clamp_confidence(confidence),
        };
        transaction.execute(
            "update concepts
             set description = ?1, status = ?2, confidence = ?3, updated_at = ?4
             where id = ?5",
            rusqlite::params![
                next_description,
                next_status,
                next_confidence,
                now,
                concept_id
            ],
        )?;
        transaction.commit()?;
        drop(connection);
        self.get(concept_id)
    }

    /// Archive an active concept. Archived concepts cannot be mutated but stay
    /// searchable as history.
    pub fn archive_concept(&self, concept_id: i64, _reason: &str) -> Result<(), ConceptError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        require_active_concept(&transaction, concept_id)?;
        transaction.execute(
            "update concepts set status = ?1, updated_at = ?2 where id = ?3",
            rusqlite::params![CONCEPT_ARCHIVED, now, concept_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// FTS-backed concept search: exact name/facet matches outrank FTS
    /// matches, semantic-tag filter intersects, story scopes boost.
    pub fn search(
        &self,
        query: &str,
        semantic_tag: Option<&str>,
        story_scopes: &[String],
    ) -> Result<Vec<ConceptRecord>, ConceptError> {
        let clean_query = query.trim();
        let clean_semantic_tag = semantic_tag.map(str::trim).unwrap_or("");
        let clean_story_scopes = clean_tags(story_scopes.iter().map(String::as_str));
        if clean_query.is_empty() && clean_semantic_tag.is_empty() {
            return Ok(Vec::new());
        }

        let connection = self.connection()?;
        let mut scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
        if !clean_query.is_empty() {
            let mut statement = connection.prepare(
                "select distinct c.id
                 from concepts c
                 left join concept_facets f
                   on f.concept_id = c.id
                  and f.superseded_at is null
                 where c.status not in (?, ?)
                   and (
                     c.canonical_name = ? collate nocase
                     or f.value = ? collate nocase
                   )",
            )?;
            let exact_ids: Vec<i64> = statement
                .query_map(
                    rusqlite::params![CONCEPT_ARCHIVED, CONCEPT_MERGED, clean_query, clean_query],
                    |row| row.get(0),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            for id in exact_ids {
                scores
                    .entry(id)
                    .and_modify(|s| *s = s.max(100.0))
                    .or_insert(100.0);
            }

            let expression = search_expression(clean_query);
            if !expression.is_empty() {
                for (sql, score) in [
                    (
                        "select c.id
                         from concepts_fts
                         join concepts c on c.id = concepts_fts.rowid
                         where concepts_fts match ?1
                           and c.status not in (?2, ?3)",
                        60.0,
                    ),
                    (
                        "select distinct c.id
                         from concept_facet_fts
                         join concept_facets f on f.id = concept_facet_fts.rowid
                         join concepts c on c.id = f.concept_id
                         where concept_facet_fts match ?1
                           and f.superseded_at is null
                           and c.status not in (?2, ?3)",
                        55.0,
                    ),
                ] {
                    let mut statement = connection.prepare(sql)?;
                    let ids: Vec<i64> = statement
                        .query_map(
                            rusqlite::params![expression, CONCEPT_ARCHIVED, CONCEPT_MERGED],
                            |row| row.get(0),
                        )?
                        .collect::<Result<Vec<_>, _>>()?;
                    for id in ids {
                        scores
                            .entry(id)
                            .and_modify(|s| *s = s.max(score))
                            .or_insert(score);
                    }
                }
            }
        }

        if !clean_semantic_tag.is_empty() {
            let tagged_ids = concept_ids_for_semantic_tag(&connection, clean_semantic_tag)?;
            if !clean_query.is_empty() {
                scores.retain(|concept_id, _| tagged_ids.contains(concept_id));
                for score in scores.values_mut() {
                    *score += 50.0;
                }
            } else {
                scores = tagged_ids.iter().map(|id| (*id, 50.0)).collect();
            }
        }

        if !clean_story_scopes.is_empty() && !scores.is_empty() {
            for concept_id in concept_ids_for_story_scopes(&connection, &clean_story_scopes)? {
                if let Some(score) = scores.get_mut(&concept_id) {
                    *score += 10.0;
                }
            }
        }

        if scores.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = scores.keys().map(|_| "?").collect::<Vec<_>>().join(", ");
        let mut statement = connection.prepare(&format!(
            "select * from concepts c where c.id in ({placeholders})"
        ))?;
        let ids: Vec<i64> = scores.keys().copied().collect();
        let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            row_ref_from_statement_row(row)
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(concept_record_from_row(&connection, &row?)?);
        }
        records.sort_by(|left, right| {
            let left_score = scores[&left.id];
            let right_score = scores[&right.id];
            right_score
                .partial_cmp(&left_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.id.cmp(&right.id))
        });
        Ok(records)
    }

    fn connection(&self) -> Result<Connection, ConceptError> {
        let path: PathBuf = self.config.database_path();
        Ok(open_migrated(&path)?)
    }
}

fn validate_scope(scope_type: &str, scope_key: &str) -> Result<(), ConceptError> {
    match scope_type {
        "global" if !scope_key.is_empty() => Err(ConceptError::Invalid(
            "global concept scope requires an empty key".to_string(),
        )),
        "global" => Ok(()),
        _ if scope_key.is_empty() => Err(ConceptError::Invalid(
            "non-global concept scope requires a key".to_string(),
        )),
        _ => Ok(()),
    }
}

fn require_concept_fields(
    connection: &Connection,
    concept_id: i64,
) -> Result<ConceptRow, ConceptError> {
    connection
        .query_row(
            "select id, canonical_name, description, scope_type, scope_key,
                    status, confidence, merged_into_concept_id
             from concepts where id = ?1",
            [concept_id],
            |row| {
                Ok(ConceptRow {
                    id: row.get("id")?,
                    canonical_name: row.get("canonical_name")?,
                    description: row.get("description")?,
                    status: row.get("status")?,
                    confidence: row.get("confidence")?,
                    scope_type: row.get("scope_type")?,
                    scope_key: row.get("scope_key")?,
                    merged_into_concept_id: row.get("merged_into_concept_id")?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ConceptError::UnknownConcept(concept_id),
            other => other.into(),
        })
}

fn require_active_concept(
    connection: &Connection,
    concept_id: i64,
) -> Result<ConceptRow, ConceptError> {
    let row = require_concept_fields(connection, concept_id)?;
    if is_inactive_status(&row.status) {
        return Err(ConceptError::Invalid(
            "cannot mutate inactive concept".to_string(),
        ));
    }
    Ok(row)
}

/// The live (non-superseded) facet row; a superseded or unknown facet id is
/// the same [`ConceptError::UnknownFacet`] rejection (Python treats both as
/// unknown).
fn require_live_facet_row(
    connection: &Connection,
    facet_id: i64,
) -> Result<FacetRow, ConceptError> {
    connection
        .query_row(
            "select id, concept_id, language, facet_type, value, confidence,
                    source_crystal_id, is_canonical
             from concept_facets where id = ?1 and superseded_at is null",
            [facet_id],
            facet_row,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ConceptError::UnknownFacet(facet_id),
            other => other.into(),
        })
}

fn concept_ids_for_semantic_tag(
    connection: &Connection,
    tag: &str,
) -> Result<Vec<i64>, ConceptError> {
    let mut statement = connection.prepare(
        "select concept_id from concept_semantic_tags where tag = ?1 order by concept_id",
    )?;
    let rows = statement.query_map([tag], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn concept_ids_for_story_scopes(
    connection: &Connection,
    story_scopes: &[String],
) -> Result<Vec<i64>, ConceptError> {
    let placeholders = story_scopes
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = connection.prepare(&format!(
        "select distinct c.id
         from concepts c
         join concept_facets f on f.concept_id = c.id
         join concept_facet_story_scopes s on s.facet_id = f.id
         where s.story_scope in ({placeholders})
           and f.superseded_at is null
         order by c.id"
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(story_scopes.iter()), |row| {
        row.get(0)
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn set_semantic_tags(
    connection: &Connection,
    concept_id: i64,
    tags: &[String],
    now: &str,
) -> Result<(), ConceptError> {
    connection.execute(
        "delete from concept_semantic_tags where concept_id = ?1",
        [concept_id],
    )?;
    for tag in clean_tags(tags.iter().map(String::as_str)) {
        connection.execute(
            "insert into concept_semantic_tags(concept_id, tag, created_at)
             values (?1, ?2, ?3)",
            rusqlite::params![concept_id, tag, now],
        )?;
    }
    Ok(())
}

fn set_canonical_facet_with_connection(
    connection: &Connection,
    concept_id: i64,
    facet_id: i64,
) -> Result<(), ConceptError> {
    let exists: Option<i64> = connection
        .query_row(
            "select id from concept_facets
             where id = ?1 and concept_id = ?2 and superseded_at is null",
            rusqlite::params![facet_id, concept_id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    if exists.is_none() {
        return Err(ConceptError::UnknownFacet(facet_id));
    }
    let now = now_iso8601();
    connection.execute(
        "update concept_facets set is_canonical = 0, updated_at = ?1 where concept_id = ?2",
        rusqlite::params![now, concept_id],
    )?;
    connection.execute(
        "update concept_facets set is_canonical = 1, updated_at = ?1 where id = ?2",
        rusqlite::params![now, facet_id],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn add_facet_with_connection(
    connection: &Connection,
    concept_id: i64,
    clean_value: &str,
    fields: &FacetFields,
    confidence: f64,
    is_canonical: bool,
    now: &str,
) -> Result<i64, ConceptError> {
    let clean_kind =
        normalize_facet_storage_kind(fields.kind.as_deref(), fields.facet_type.as_deref())?;
    let clean_language_tags = clean_language_tags(
        &fields.language,
        fields.language_tags.iter().map(String::as_str),
    );
    let legacy_language = clean_language_tags.first().cloned().unwrap_or_default();
    require_active_concept(connection, concept_id)?;
    connection.execute(
        "insert into concept_facets(
           concept_id, language, facet_type, value, source_crystal_id,
           confidence, is_canonical, created_at, updated_at
         )
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            concept_id,
            legacy_language,
            clean_kind,
            clean_value,
            fields.source_crystal_id,
            clamp_confidence(confidence),
            is_canonical as i64,
            now,
            now,
        ],
    )?;
    let facet_id = connection.last_insert_rowid();
    set_facet_language_tags(connection, facet_id, &clean_language_tags)?;
    set_facet_story_scopes(connection, facet_id, &fields.story_scopes)?;
    set_facet_semantic_tags(connection, facet_id, &fields.semantic_tags)?;
    if is_canonical {
        set_canonical_facet_with_connection(connection, concept_id, facet_id)?;
    }
    Ok(facet_id)
}

fn set_facet_language_tags(
    connection: &Connection,
    facet_id: i64,
    language_tags: &[String],
) -> Result<(), ConceptError> {
    connection.execute(
        "delete from concept_facet_language_tags where facet_id = ?1",
        [facet_id],
    )?;
    for language_tag in clean_language_tags("", language_tags.iter().map(String::as_str)) {
        connection.execute(
            "insert into concept_facet_language_tags(facet_id, language_tag)
             values (?1, ?2)",
            rusqlite::params![facet_id, language_tag],
        )?;
    }
    Ok(())
}

fn set_facet_story_scopes(
    connection: &Connection,
    facet_id: i64,
    story_scopes: &[String],
) -> Result<(), ConceptError> {
    connection.execute(
        "delete from concept_facet_story_scopes where facet_id = ?1",
        [facet_id],
    )?;
    for story_scope in clean_tags(story_scopes.iter().map(String::as_str)) {
        connection.execute(
            "insert into concept_facet_story_scopes(facet_id, story_scope)
             values (?1, ?2)",
            rusqlite::params![facet_id, story_scope],
        )?;
    }
    Ok(())
}

fn set_facet_semantic_tags(
    connection: &Connection,
    facet_id: i64,
    semantic_tags: &[String],
) -> Result<(), ConceptError> {
    connection.execute(
        "delete from concept_facet_semantic_tags where facet_id = ?1",
        [facet_id],
    )?;
    for semantic_tag in clean_tags(semantic_tags.iter().map(String::as_str)) {
        connection.execute(
            "insert into concept_facet_semantic_tags(facet_id, semantic_tag)
             values (?1, ?2)",
            rusqlite::params![facet_id, semantic_tag],
        )?;
    }
    Ok(())
}

pub fn get_facet(
    config: &HieronymusConfig,
    facet_id: i64,
) -> Result<ConceptFacetRecord, ConceptError> {
    let path: PathBuf = config.database_path();
    let connection = open_migrated(&path)?;
    let row = connection
        .query_row(
            "select id, concept_id, language, facet_type, value, confidence,
                    source_crystal_id, is_canonical
             from concept_facets where id = ?1",
            [facet_id],
            facet_row,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ConceptError::UnknownFacet(facet_id),
            other => other.into(),
        })?;
    facet_record_from_parts(&connection, &row)
}

/// Column projection of a `concept_facets` row before side-table hydration.
struct FacetRow {
    id: i64,
    concept_id: i64,
    language: String,
    facet_type: String,
    value: String,
    confidence: f64,
    source_crystal_id: Option<i64>,
    is_canonical: bool,
}

fn facet_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FacetRow> {
    Ok(FacetRow {
        id: row.get(0)?,
        concept_id: row.get(1)?,
        language: row.get(2)?,
        facet_type: row.get(3)?,
        value: row.get(4)?,
        confidence: row.get(5)?,
        source_crystal_id: row.get(6)?,
        is_canonical: row.get::<_, i64>(7)? != 0,
    })
}

fn facet_record_from_parts(
    connection: &Connection,
    parts: &FacetRow,
) -> Result<ConceptFacetRecord, ConceptError> {
    let language_tags: Vec<String> = {
        let mut statement = connection.prepare(
            "select language_tag from concept_facet_language_tags
             where facet_id = ?1
             order by case when language_tag = ?2 then 0 else 1 end, language_tag",
        )?;
        let rows = statement.query_map(rusqlite::params![parts.id, parts.language], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let language_tags = if language_tags.is_empty() && !parts.language.is_empty() {
        vec![parts.language.clone()]
    } else {
        language_tags
    };
    let story_scopes: Vec<String> = {
        let mut statement = connection.prepare(
            "select story_scope from concept_facet_story_scopes
             where facet_id = ?1 order by story_scope",
        )?;
        let rows = statement.query_map([parts.id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let semantic_tags: Vec<String> = {
        let mut statement = connection.prepare(
            "select semantic_tag from concept_facet_semantic_tags
             where facet_id = ?1 order by semantic_tag",
        )?;
        let rows = statement.query_map([parts.id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    Ok(ConceptFacetRecord {
        id: parts.id,
        concept_id: parts.concept_id,
        language: parts.language.clone(),
        facet_type: parts.facet_type.clone(),
        value: parts.value.clone(),
        confidence: parts.confidence,
        source_crystal_id: parts.source_crystal_id,
        language_tags,
        story_scopes,
        semantic_tags,
        is_canonical: parts.is_canonical,
    })
}

fn row_ref_from_statement_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConceptRow> {
    Ok(ConceptRow {
        id: row.get("id")?,
        canonical_name: row.get("canonical_name")?,
        description: row.get("description")?,
        status: row.get("status")?,
        confidence: row.get("confidence")?,
        scope_type: row.get("scope_type")?,
        scope_key: row.get("scope_key")?,
        merged_into_concept_id: row.get("merged_into_concept_id")?,
    })
}

/// Column projection of a `concepts` row before hydration.
pub struct ConceptRow {
    pub id: i64,
    pub canonical_name: String,
    pub description: String,
    pub status: String,
    pub confidence: f64,
    pub scope_type: String,
    pub scope_key: String,
    pub merged_into_concept_id: Option<i64>,
}

fn concept_record_from_row(
    connection: &Connection,
    row: &ConceptRow,
) -> Result<ConceptRecord, ConceptError> {
    let tags: Vec<String> = {
        let mut statement = connection
            .prepare("select tag from concept_semantic_tags where concept_id = ?1 order by tag")?;
        let rows = statement.query_map([row.id], |tag_row| tag_row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    Ok(ConceptRecord {
        id: row.id,
        canonical_name: row.canonical_name.clone(),
        description: row.description.clone(),
        status: public_status(&row.status).to_string(),
        confidence: row.confidence,
        scope_type: row.scope_type.clone(),
        scope_key: row.scope_key.clone(),
        tags,
        merged_into_concept_id: row.merged_into_concept_id,
    })
}

/// Re-exported for tests that exercise scoped stores against temp tables.
pub fn concept_row_guard(_: &Table) {}

impl ConceptStore {
    /// Rename an active concept: the old label stays searchable as a
    /// `former_label` facet (created only when not already present) and the
    /// rename is recorded in `concept_renames`.
    pub fn rename_concept(
        &self,
        concept_id: i64,
        new_label: &str,
        source_crystal_id: Option<i64>,
    ) -> Result<ConceptRecord, ConceptError> {
        let clean_label = new_label.trim();
        if clean_label.is_empty() {
            return Err(ConceptError::Invalid(
                "concept canonical_name must not be empty".to_string(),
            ));
        }
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let active = require_active_concept(&transaction, concept_id)?;
        let old_label = active.canonical_name.clone();
        if old_label == clean_label {
            transaction.commit()?;
            return self.get(concept_id);
        }

        if !facet_value_exists(&transaction, concept_id, &old_label)? {
            transaction.execute(
                "insert into concept_facets(
                   concept_id, language, facet_type, value, source_crystal_id,
                   confidence, created_at, updated_at
                 )
                 values (?1, '', 'former_label', ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    concept_id,
                    old_label,
                    source_crystal_id,
                    active.confidence,
                    now,
                    now,
                ],
            )?;
        }
        transaction.execute(
            "insert into concept_renames(concept_id, old_name, new_name, created_at)
             values (?1, ?2, ?3, ?4)",
            rusqlite::params![concept_id, old_label, clean_label, now],
        )?;
        transaction.execute(
            "update concepts set canonical_name = ?1, updated_at = ?2 where id = ?3",
            rusqlite::params![clean_label, now, concept_id],
        )?;
        transaction.commit()?;
        self.get(concept_id)
    }

    /// One-way merge: source facets, crystal links, and semantic tags move to
    /// the active target; the source becomes `merged` and fails closed on
    /// further mutation.
    pub fn merge_concepts(
        &self,
        source_concept_id: i64,
        target_concept_id: i64,
        _reason: &str,
    ) -> Result<(), ConceptError> {
        if source_concept_id == target_concept_id {
            return Err(ConceptError::Invalid(
                "source and target concepts must differ".to_string(),
            ));
        }
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let source = require_active_concept(&transaction, source_concept_id)?;
        let target = require_concept_fields(&transaction, target_concept_id)?;
        if is_inactive_status(&target.status) {
            return Err(ConceptError::Invalid(
                "merge target concept must be active".to_string(),
            ));
        }

        if !facet_value_exists(&transaction, source_concept_id, &source.canonical_name)? {
            ensure_facet(
                &transaction,
                target_concept_id,
                &source.canonical_name,
                "former_label",
                source.confidence,
                &now,
            )?;
        }

        let links: Vec<(i64, String, f64, String)> = {
            let mut statement = transaction.prepare(
                "select crystal_id, link_type, confidence, created_at
                 from crystal_concepts where concept_id = ?1",
            )?;
            let rows = statement.query_map([source_concept_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (crystal_id, link_type, confidence, created_at) in links {
            transaction.execute(
                "insert into crystal_concepts(
                   crystal_id, concept_id, link_type, confidence, created_at
                 )
                 values (?1, ?2, ?3, ?4, ?5)
                 on conflict(crystal_id, concept_id, link_type) do update set
                   confidence = max(crystal_concepts.confidence, excluded.confidence)",
                rusqlite::params![
                    crystal_id,
                    target_concept_id,
                    link_type,
                    confidence,
                    created_at
                ],
            )?;
        }
        transaction.execute(
            "delete from crystal_concepts where concept_id = ?1",
            [source_concept_id],
        )?;
        move_facets_to_target(&transaction, source_concept_id, target_concept_id, &now)?;

        let tags: Vec<(String, f64, String)> = {
            let mut statement = transaction.prepare(
                "select tag, confidence, created_at
                 from concept_semantic_tags where concept_id = ?1",
            )?;
            let rows = statement.query_map([source_concept_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (tag, confidence, created_at) in tags {
            transaction.execute(
                "insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
                 values (?1, ?2, ?3, ?4)
                 on conflict(concept_id, tag) do update set
                   confidence = max(concept_semantic_tags.confidence, excluded.confidence)",
                rusqlite::params![target_concept_id, tag, confidence, created_at],
            )?;
        }
        transaction.execute(
            "delete from concept_semantic_tags where concept_id = ?1",
            [source_concept_id],
        )?;
        transaction.execute(
            "update concepts
             set status = ?1, merged_into_concept_id = ?2, updated_at = ?3
             where id = ?4",
            rusqlite::params![CONCEPT_MERGED, target_concept_id, now, source_concept_id],
        )?;
        refresh_concept_status(&transaction, target_concept_id, &now)?;
        transaction.commit()?;
        Ok(())
    }

    /// Attach a crystal as linked evidence of an active concept; relinking
    /// with higher confidence keeps the maximum. Re-evaluates the concept's
    /// established status.
    pub fn link_crystal(
        &self,
        crystal_id: i64,
        concept_id: i64,
        link_type: &str,
        confidence: f64,
    ) -> Result<(), ConceptError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let concept = require_concept_fields(&transaction, concept_id)?;
        if is_inactive_status(&concept.status) {
            return Err(ConceptError::Invalid(
                "cannot link crystal to inactive concept".to_string(),
            ));
        }
        transaction.execute(
            "insert into crystal_concepts(
               crystal_id, concept_id, link_type, confidence, created_at
             )
             values (?1, ?2, ?3, ?4, ?5)
             on conflict(crystal_id, concept_id, link_type) do update set
               confidence = max(crystal_concepts.confidence, excluded.confidence)",
            rusqlite::params![
                crystal_id,
                concept_id,
                link_type,
                clamp_confidence(confidence),
                now
            ],
        )?;
        refresh_concept_status(&transaction, concept_id, &now)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn concept_ids_for_crystal(&self, crystal_id: i64) -> Result<Vec<i64>, ConceptError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select distinct concept_id from crystal_concepts
             where crystal_id = ?1 order by concept_id",
        )?;
        let rows = statement.query_map([crystal_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Recall-time ranking boosts (Python
    /// `ConceptStore.recall_boosts_for_crystals`): crystals whose linked
    /// concept matches the query text gain [`CONCEPT_RECALL_TEXT_BOOST`],
    /// and crystals whose linked concept carries a query-matching,
    /// non-superseded facet scoped to one of `story_scopes` additionally
    /// gain [`CONCEPT_RECALL_STORY_SCOPE_BOOST`]. Empty crystal ids or a
    /// blank query yield an empty map; zero-boost entries are filtered.
    pub fn recall_boosts_for_crystals(
        &self,
        crystal_ids: &[i64],
        query: &str,
        story_scopes: &[String],
    ) -> Result<std::collections::HashMap<i64, f64>, ConceptError> {
        let mut clean_crystal_ids = crystal_ids.to_vec();
        clean_crystal_ids.sort_unstable();
        clean_crystal_ids.dedup();
        let clean_query = query.trim();
        if clean_crystal_ids.is_empty() || clean_query.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let clean_story_scopes = clean_tags(story_scopes.iter().map(String::as_str));

        let mut boosts: std::collections::HashMap<i64, f64> = clean_crystal_ids
            .iter()
            .map(|crystal_id| (*crystal_id, 0.0))
            .collect();
        let connection = self.connection()?;

        let matching_concept_ids = concept_ids_matching_recall_query(&connection, clean_query)?;
        if !matching_concept_ids.is_empty() {
            let crystal_placeholders = placeholders(clean_crystal_ids.len());
            let concept_placeholders = placeholders(matching_concept_ids.len());
            let mut statement = connection.prepare(&format!(
                "select distinct crystal_id
                 from crystal_concepts
                 where crystal_id in ({crystal_placeholders})
                   and concept_id in ({concept_placeholders})"
            ))?;
            let rows = statement.query_map(
                rusqlite::params_from_iter(
                    clean_crystal_ids.iter().chain(matching_concept_ids.iter()),
                ),
                |row| row.get(0),
            )?;
            for row in rows {
                *boosts.entry(row?).or_insert(0.0) += CONCEPT_RECALL_TEXT_BOOST;
            }
        }

        if !clean_story_scopes.is_empty() {
            let matching_facet_ids = facet_ids_matching_query(&connection, clean_query)?;
            if matching_facet_ids.is_empty() {
                return Ok(positive_boosts(boosts));
            }
            let facet_placeholders = placeholders(matching_facet_ids.len());
            let crystal_placeholders = placeholders(clean_crystal_ids.len());
            let scope_placeholders = placeholders(clean_story_scopes.len());
            let mut statement = connection.prepare(&format!(
                "select distinct cc.crystal_id
                 from crystal_concepts cc
                 join concept_facets f
                   on f.concept_id = cc.concept_id
                  and f.superseded_at is null
                 join concept_facet_story_scopes s
                   on s.facet_id = f.id
                 where f.id in ({facet_placeholders})
                   and cc.crystal_id in ({crystal_placeholders})
                   and s.story_scope in ({scope_placeholders})"
            ))?;
            let rows = statement.query_map(
                rusqlite::params_from_iter(
                    matching_facet_ids
                        .iter()
                        .map(|facet_id| rusqlite::types::Value::Integer(*facet_id))
                        .chain(
                            clean_crystal_ids
                                .iter()
                                .map(|crystal_id| rusqlite::types::Value::Integer(*crystal_id)),
                        )
                        .chain(
                            clean_story_scopes
                                .iter()
                                .map(|scope| rusqlite::types::Value::Text(scope.clone())),
                        ),
                ),
                |row| row.get(0),
            )?;
            for row in rows {
                *boosts.entry(row?).or_insert(0.0) += CONCEPT_RECALL_STORY_SCOPE_BOOST;
            }
        }

        Ok(positive_boosts(boosts))
    }
}

/// Comma-separated `?` placeholders for `in (...)` clauses.
fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn positive_boosts(
    boosts: std::collections::HashMap<i64, f64>,
) -> std::collections::HashMap<i64, f64> {
    boosts
        .into_iter()
        .filter(|(_, boost)| *boost > 0.0)
        .collect()
}

/// Python `_facet_ids_matching_query_with_connection`: live facets whose
/// value equals the query (case-insensitively) plus FTS matches, in id order.
fn facet_ids_matching_query(
    connection: &Connection,
    query: &str,
) -> Result<Vec<i64>, ConceptError> {
    let mut facet_ids: std::collections::HashSet<i64> = {
        let mut statement = connection.prepare(
            "select id
             from concept_facets
             where value = ? collate nocase
               and superseded_at is null",
        )?;
        let rows = statement.query_map([query], |row| row.get(0))?;
        rows.collect::<Result<std::collections::HashSet<i64>, _>>()?
    };
    let expression = search_expression(query);
    if !expression.is_empty() {
        let mut statement = connection.prepare(
            "select f.id
             from concept_facet_fts
             join concept_facets f on f.id = concept_facet_fts.rowid
             where concept_facet_fts match ?1
               and f.superseded_at is null",
        )?;
        let rows = statement.query_map([expression], |row| row.get(0))?;
        for facet_id in rows {
            facet_ids.insert(facet_id?);
        }
    }
    let mut sorted: Vec<i64> = facet_ids.into_iter().collect();
    sorted.sort_unstable();
    Ok(sorted)
}

/// Python `_concept_ids_matching_recall_query_with_connection`: active
/// (non-archived, non-merged) concepts whose canonical name equals the query
/// (case-insensitively) plus the concepts behind query-matching facets.
fn concept_ids_matching_recall_query(
    connection: &Connection,
    query: &str,
) -> Result<Vec<i64>, ConceptError> {
    let mut concept_ids: std::collections::HashSet<i64> = {
        let mut statement = connection.prepare(
            "select id
             from concepts
             where canonical_name = ? collate nocase
               and status not in (?, ?)",
        )?;
        let rows = statement.query_map(
            rusqlite::params![query, CONCEPT_ARCHIVED, CONCEPT_MERGED],
            |row| row.get(0),
        )?;
        rows.collect::<Result<std::collections::HashSet<i64>, _>>()?
    };
    let facet_ids = facet_ids_matching_query(connection, query)?;
    if !facet_ids.is_empty() {
        let facet_placeholders = placeholders(facet_ids.len());
        let mut statement = connection.prepare(&format!(
            "select distinct c.id
             from concepts c
             join concept_facets f on f.concept_id = c.id
             where f.id in ({facet_placeholders})
               and c.status not in (?, ?)"
        ))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(
                facet_ids
                    .iter()
                    .map(|facet_id| rusqlite::types::Value::Integer(*facet_id))
                    .chain(
                        [CONCEPT_ARCHIVED, CONCEPT_MERGED]
                            .iter()
                            .map(|status| rusqlite::types::Value::Text((*status).to_string())),
                    ),
            ),
            |row| row.get(0),
        )?;
        for concept_id in rows {
            concept_ids.insert(concept_id?);
        }
    }
    let mut sorted: Vec<i64> = concept_ids.into_iter().collect();
    sorted.sort_unstable();
    Ok(sorted)
}

/// Established status requires both the confidence threshold and the linked
/// evidence count; an explicit terminal/established status is preserved.
fn concept_status(
    confidence: f64,
    linked_evidence_count: i64,
    current_status: Option<&str>,
) -> String {
    if let Some(current) = current_status {
        let public = public_status(current);
        if public == CONCEPT_ESTABLISHED || public == CONCEPT_ARCHIVED || public == CONCEPT_MERGED {
            return public.to_string();
        }
    }
    if confidence >= ESTABLISHED_CONFIDENCE && linked_evidence_count >= ESTABLISHED_EVIDENCE_COUNT {
        CONCEPT_ESTABLISHED.to_string()
    } else {
        CONCEPT_CANDIDATE.to_string()
    }
}

fn linked_evidence_count(connection: &Connection, concept_id: i64) -> Result<i64, ConceptError> {
    let count: i64 = connection.query_row(
        "select count(distinct crystal_id) from crystal_concepts where concept_id = ?1",
        [concept_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn refresh_concept_status(
    connection: &Connection,
    concept_id: i64,
    now: &str,
) -> Result<(), ConceptError> {
    let row = require_concept_fields(connection, concept_id)?;
    let evidence = linked_evidence_count(connection, concept_id)?;
    let status = concept_status(row.confidence, evidence, Some(&row.status));
    if status != row.status {
        connection.execute(
            "update concepts set status = ?1, updated_at = ?2 where id = ?3",
            rusqlite::params![status, now, concept_id],
        )?;
    }
    Ok(())
}

fn facet_value_exists(
    connection: &Connection,
    concept_id: i64,
    value: &str,
) -> Result<bool, ConceptError> {
    let found: Option<i64> = connection
        .query_row(
            "select 1 from concept_facets
             where concept_id = ?1 and value = ?2 and superseded_at is null
             limit 1",
            rusqlite::params![concept_id, value],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(found.is_some())
}

fn facet_by_identity(
    connection: &Connection,
    concept_id: i64,
    value: &str,
    facet_type: &str,
    language: &str,
) -> Result<Option<i64>, ConceptError> {
    let found: Option<i64> = connection
        .query_row(
            "select id from concept_facets
             where concept_id = ?1 and value = ?2 and facet_type = ?3
               and language = ?4 and superseded_at is null
             order by id limit 1",
            rusqlite::params![concept_id, value, facet_type, language],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(found)
}

/// Idempotently ensure a facet with the given value/type exists (matching by
/// identity, then by value) and return its id.
fn ensure_facet(
    connection: &Connection,
    concept_id: i64,
    value: &str,
    facet_type: &str,
    confidence: f64,
    now: &str,
) -> Result<i64, ConceptError> {
    let clean_value = value.trim();
    if clean_value.is_empty() {
        return Err(ConceptError::Invalid(
            "concept facet value must not be empty".to_string(),
        ));
    }
    if let Some(existing) = facet_by_identity(connection, concept_id, clean_value, facet_type, "")?
    {
        return Ok(existing);
    }
    if let Some(existing) = facet_by_value_first_id(connection, concept_id, clean_value)? {
        return Ok(existing);
    }
    connection.execute(
        "insert into concept_facets(
           concept_id, language, facet_type, value, source_crystal_id,
           confidence, created_at, updated_at
         )
         values (?1, '', ?2, ?3, NULL, ?4, ?5, ?6)",
        rusqlite::params![
            concept_id,
            facet_type,
            clean_value,
            clamp_confidence(confidence),
            now,
            now,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn facet_by_value_first_id(
    connection: &Connection,
    concept_id: i64,
    value: &str,
) -> Result<Option<i64>, ConceptError> {
    let found: Option<i64> = connection
        .query_row(
            "select id from concept_facets
             where concept_id = ?1 and value = ?2 and superseded_at is null
             order by id limit 1",
            rusqlite::params![concept_id, value],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(found)
}

// ----------------------------------------------------------------------
// Dream graph application (transaction-aware primitives, task D2)
// ----------------------------------------------------------------------

/// The candidate row a reinforcement delta applies to (Python
/// `_reinforcement_target_with_connection`): the single active concept with
/// this identity in scope; among several, tag matching disambiguates.
struct ReinforcementTarget {
    id: i64,
    confidence: f64,
    status: String,
}

fn reinforcement_target(
    connection: &Connection,
    canonical_name: &str,
    scope_type: &str,
    scope_key: &str,
    tags: &[String],
) -> Result<Option<ReinforcementTarget>, ConceptError> {
    let mut statement = connection.prepare(
        "select id, confidence, status
         from concepts
         where scope_type = ?1 and scope_key = ?2 and canonical_name = ?3
           and status not in (?, ?)
         order by id",
    )?;
    let rows = statement.query_map(
        rusqlite::params![
            scope_type,
            scope_key,
            canonical_name,
            CONCEPT_ARCHIVED,
            CONCEPT_MERGED
        ],
        |row| {
            Ok(ReinforcementTarget {
                id: row.get(0)?,
                confidence: row.get(1)?,
                status: row.get(2)?,
            })
        },
    )?;
    let mut rows: Vec<ReinforcementTarget> = rows.collect::<Result<Vec<_>, _>>()?;
    if rows.len() <= 1 {
        return Ok(rows.pop());
    }
    if tags.is_empty() {
        return Ok(None);
    }
    let requested: std::collections::HashSet<&str> = tags.iter().map(String::as_str).collect();
    let mut tagged: Vec<(ReinforcementTarget, std::collections::HashSet<String>)> =
        Vec::with_capacity(rows.len());
    for row in rows {
        let mut statement = connection
            .prepare("select tag from concept_semantic_tags where concept_id = ?1 order by tag")?;
        let tag_rows = statement.query_map([row.id], |tag_row| tag_row.get::<_, String>(0))?;
        let row_tags: std::collections::HashSet<String> = tag_rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .collect();
        tagged.push((row, row_tags));
    }
    let requested_strings: std::collections::HashSet<String> =
        requested.into_iter().map(String::from).collect();
    let exact: Vec<usize> = tagged
        .iter()
        .enumerate()
        .filter(|(_, (_, row_tags))| *row_tags == requested_strings)
        .map(|(index, _)| index)
        .collect();
    if exact.len() == 1 {
        return Ok(Some(tagged.swap_remove(exact[0]).0));
    }
    let overlapping: Vec<usize> = tagged
        .iter()
        .enumerate()
        .filter(|(_, (_, row_tags))| row_tags.iter().any(|tag| requested_strings.contains(tag)))
        .map(|(index, _)| index)
        .collect();
    if overlapping.len() == 1 {
        return Ok(Some(tagged.swap_remove(overlapping[0]).0));
    }
    Ok(None)
}

/// Port of `_create_or_reinforce_with_connection` for dream graph
/// application: create the concept as a candidate or reinforce the existing
/// one (confidence delta, non-empty description, tags), refreshing its
/// established status from linked evidence. Runs inside the caller's
/// transaction; dream output can only ever create or update candidates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_or_reinforce_concept_in_transaction(
    connection: &Connection,
    canonical_name: &str,
    description: &str,
    tags: &[String],
    confidence_delta: f64,
    scope_type: &str,
    scope_key: &str,
    now: &str,
) -> Result<i64, ConceptError> {
    let name = canonical_name.trim();
    if name.is_empty() {
        return Err(ConceptError::Invalid(
            "concept canonical_name must not be empty".to_string(),
        ));
    }
    validate_scope(scope_type, scope_key)?;
    let clean_description = description.trim();
    let clean_tags = clean_tags(tags.iter().map(String::as_str));

    let (concept_id, confidence) =
        match reinforcement_target(connection, name, scope_type, scope_key, &clean_tags)? {
            None => {
                let confidence = clamp_confidence(confidence_delta);
                let status = concept_status(confidence, 0, None);
                connection.execute(
                    "insert into concepts(
                   canonical_name, description, scope_type, scope_key,
                   status, confidence, created_at, updated_at
                 )
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    rusqlite::params![
                        name,
                        clean_description,
                        scope_type,
                        scope_key,
                        status,
                        confidence,
                        now
                    ],
                )?;
                (connection.last_insert_rowid(), confidence)
            }
            Some(target) => {
                let confidence = clamp_confidence(target.confidence + confidence_delta);
                let evidence = linked_evidence_count(connection, target.id)?;
                let status = concept_status(confidence, evidence, Some(&target.status));
                connection.execute(
                    "update concepts
                 set description = case when ?2 != '' then ?2 else description end,
                     confidence = ?3,
                     status = ?4,
                     updated_at = ?5
                 where id = ?1",
                    rusqlite::params![target.id, clean_description, confidence, status, now],
                )?;
                (target.id, confidence)
            }
        };
    for tag in clean_tags {
        connection.execute(
            "insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
             values (?1, ?2, ?3, ?4)
             on conflict(concept_id, tag) do update set
               confidence = max(concept_semantic_tags.confidence, excluded.confidence)",
            rusqlite::params![concept_id, tag, confidence, now],
        )?;
    }
    Ok(concept_id)
}

/// Insert one pending strict concept proposal inside the caller's
/// transaction (Python `ConceptProposalStore._create_with_connection`).
/// Proposals are candidates: only the explicit approval operation can act
/// on them, so dream output can never activate a rule through this path.
pub(crate) fn create_concept_proposal_in_transaction(
    connection: &Connection,
    dream_run_id: i64,
    proposal: &crate::dream_output::ConceptProposal,
    now: &str,
) -> Result<i64, ConceptError> {
    let approved_variants_json = serde_json::to_string(&proposal.approved_variants)
        .map_err(|error| ConceptError::Invalid(error.to_string()))?;
    let forbidden_variants_json = serde_json::to_string(&proposal.forbidden_variants)
        .map_err(|error| ConceptError::Invalid(error.to_string()))?;
    connection.execute(
        "insert into strict_concept_proposals(
           dream_run_id, series_slug, source_language, target_language,
           concept_text, source_form, canonical_rendering,
           approved_variants_json, forbidden_variants_json, rationale,
           status, created_at, updated_at
         )
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11, ?11)",
        rusqlite::params![
            dream_run_id,
            proposal.series_slug,
            proposal.source_language,
            proposal.target_language,
            proposal.concept_text,
            proposal.source_form,
            proposal.canonical_rendering,
            approved_variants_json,
            forbidden_variants_json,
            proposal.rationale,
            now
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

/// Move the source concept's live facets onto the target: identical facets
/// (value/type/language) merge their side-table metadata, others are
/// re-parented and lose their canonical flag.
fn move_facets_to_target(
    connection: &Connection,
    source_concept_id: i64,
    target_concept_id: i64,
    now: &str,
) -> Result<(), ConceptError> {
    let source_facet_ids: Vec<i64> = {
        let mut statement = connection.prepare(
            "select id from concept_facets
             where concept_id = ?1 and superseded_at is null order by id",
        )?;
        let rows = statement.query_map([source_concept_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for source_facet_id in source_facet_ids {
        let facet = get_facet_row(connection, source_facet_id)?;
        let existing = facet_by_identity(
            connection,
            target_concept_id,
            &facet.value,
            &facet.facet_type,
            &facet.language,
        )?;
        match existing {
            None => {
                connection.execute(
                    "update concept_facets
                     set concept_id = ?1, is_canonical = 0, updated_at = ?2
                     where id = ?3",
                    rusqlite::params![target_concept_id, now, source_facet_id],
                )?;
            }
            Some(target_facet_id) => {
                copy_facet_metadata(connection, source_facet_id, target_facet_id)?;
                connection.execute(
                    "delete from concept_facets where id = ?1",
                    [source_facet_id],
                )?;
            }
        }
    }
    Ok(())
}

fn get_facet_row(connection: &Connection, facet_id: i64) -> Result<FacetRow, ConceptError> {
    connection
        .query_row(
            "select id, concept_id, language, facet_type, value, confidence,
                    source_crystal_id, is_canonical
             from concept_facets where id = ?1",
            [facet_id],
            facet_row,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ConceptError::UnknownFacet(facet_id),
            other => other.into(),
        })
}

fn copy_facet_metadata(
    connection: &Connection,
    source_facet_id: i64,
    target_facet_id: i64,
) -> Result<(), ConceptError> {
    for (table, column) in [
        ("concept_facet_language_tags", "language_tag"),
        ("concept_facet_story_scopes", "story_scope"),
        ("concept_facet_semantic_tags", "semantic_tag"),
    ] {
        let sql = format!(
            "insert or ignore into {table}(facet_id, {column})
             select ?1, {column}
             from {table} where facet_id = ?2"
        );
        connection.execute(&sql, rusqlite::params![target_facet_id, source_facet_id])?;
    }
    Ok(())
}
