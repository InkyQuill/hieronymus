use std::ops::Deref;

use serde::{Deserialize, Serialize};

use crate::db::CrystalRecord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslationContext {
    pub series_slug: String,
    pub scope_key: String,
    pub source_language: String,
    pub target_language: String,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub tags: Vec<String>,
}

impl TranslationContext {
    #[must_use]
    pub fn new(
        series_slug: impl Into<String>,
        source_language: impl Into<String>,
        target_language: impl Into<String>,
    ) -> Self {
        let series_slug = series_slug.into().trim().to_owned();
        let source_language = source_language.into().trim().to_lowercase();
        let target_language = target_language.into().trim().to_lowercase();
        let language_tags =
            normalize_texts(&[source_language.clone(), target_language.clone()], true);
        Self {
            scope_key: format!("series:{series_slug}"),
            series_slug,
            source_language,
            target_language,
            language_tags,
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
            tags: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_metadata(
        mut self,
        language_tags: &[String],
        story_scopes: &[String],
        semantic_tags: &[String],
        tags: &[String],
    ) -> Self {
        self.language_tags = normalize_texts(language_tags, true);
        self.story_scopes = normalize_texts(story_scopes, false);
        self.semantic_tags = normalize_texts(semantic_tags, false);
        self.tags = normalize_texts(tags, false);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    LongTerm,
    ShortTerm,
    Rag,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallResult {
    pub source: MemorySource,
    pub rank: usize,
    pub score: f64,
    pub text: String,
    pub reason: String,
    pub id: i64,
    pub metadata: serde_json::Value,
}

/// A persisted crystal enriched with its normalized public metadata.
///
/// The raw schema row remains [`CrystalRecord`]; stores return this domain view
/// so callers never need side-table queries or N+1 hydration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Crystal {
    pub record: CrystalRecord,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub concept_ids: Vec<i64>,
}

impl Deref for Crystal {
    type Target = CrystalRecord;

    fn deref(&self) -> &Self::Target {
        &self.record
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddCrystalInput {
    pub crystal_type: String,
    pub title: String,
    pub text: String,
    pub scope_type: String,
    pub scope_key: String,
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    pub source_credibility: String,
    pub rule_intent: String,
    pub strength: f64,
    pub confidence: f64,
    pub tags: Vec<String>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub soft_origin: Option<String>,
    pub is_inferred: bool,
    pub malformed_penalty: f64,
    pub supersedes_crystal_id: Option<i64>,
    pub status: String,
}

impl Default for AddCrystalInput {
    fn default() -> Self {
        Self {
            crystal_type: "observation".to_owned(),
            title: String::new(),
            text: String::new(),
            scope_type: "global".to_owned(),
            scope_key: String::new(),
            series_slug: String::new(),
            source_language: String::new(),
            target_language: String::new(),
            source_credibility: "observation".to_owned(),
            rule_intent: String::new(),
            strength: 0.5,
            confidence: 0.5,
            tags: Vec::new(),
            language_tags: Vec::new(),
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
            soft_origin: None,
            is_inferred: false,
            malformed_penalty: 0.0,
            supersedes_crystal_id: None,
            status: "active".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleFilter {
    pub status: Option<String>,
    pub series_slug: Option<String>,
    pub limit: usize,
}

impl Default for RuleFilter {
    fn default() -> Self {
        Self {
            status: None,
            series_slug: None,
            limit: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub findings: Vec<String>,
}

pub(super) fn normalize_texts(values: &[String], lowercase: bool) -> Vec<String> {
    let values: Vec<String> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| {
            if lowercase {
                value.to_lowercase()
            } else {
                value.to_owned()
            }
        })
        .collect();
    crate::values::normalize_tuple(&values)
}
