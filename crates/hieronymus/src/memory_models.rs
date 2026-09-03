use std::collections::BTreeMap;

/// Order-preserving normalization: trim, optional lowercase, drop empties,
/// deduplicate on first occurrence. Mirrors the Python
/// `normalize_string_tuple` (registry tag seeding uses a different, sorted
/// normalizer — the difference is intentional).
pub fn normalize_string_tuple<'a>(
    values: impl IntoIterator<Item = &'a str>,
    lowercase: bool,
) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for value in values {
        let mut item = value.trim().to_string();
        if lowercase {
            item = item.to_lowercase();
        }
        if item.is_empty() || !seen.insert(item.clone()) {
            continue;
        }
        normalized.push(item);
    }
    normalized
}

/// The translation task context. Constructing one normalizes typed metadata:
/// story scopes seed from `volume:`/`chapter:` when not given explicitly,
/// language tags seed from the default directions, semantic tags from `tags`.
#[derive(Debug, Clone, Default)]
pub struct TranslationContext {
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    pub task_type: String,
    pub volume: String,
    pub chapter: String,
    pub tags: Vec<String>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    language_tags_explicit: bool,
    story_scopes_explicit: bool,
    semantic_tags_explicit: bool,
}

/// Equality compares the observable fields; the explicitness flags only steer
/// seed normalization while building.
impl PartialEq for TranslationContext {
    fn eq(&self, other: &Self) -> bool {
        self.series_slug == other.series_slug
            && self.source_language == other.source_language
            && self.target_language == other.target_language
            && self.task_type == other.task_type
            && self.volume == other.volume
            && self.chapter == other.chapter
            && self.tags == other.tags
            && self.language_tags == other.language_tags
            && self.story_scopes == other.story_scopes
            && self.semantic_tags == other.semantic_tags
    }
}

impl Eq for TranslationContext {}

impl TranslationContext {
    pub fn new(
        series_slug: impl Into<String>,
        source_language: impl Into<String>,
        target_language: impl Into<String>,
        task_type: impl Into<String>,
    ) -> Self {
        Self {
            series_slug: series_slug.into(),
            source_language: source_language.into(),
            target_language: target_language.into(),
            task_type: task_type.into(),
            volume: String::new(),
            chapter: String::new(),
            tags: Vec::new(),
            language_tags: Vec::new(),
            story_scopes: Vec::new(),
            semantic_tags: Vec::new(),
            language_tags_explicit: false,
            story_scopes_explicit: false,
            semantic_tags_explicit: false,
        }
        .normalize_seeds()
    }

    /// Setting volume/chapter re-seeds story scopes unless explicit scopes
    /// were provided.
    pub fn volume(mut self, volume: impl Into<String>) -> Self {
        self.volume = volume.into();
        self = self.normalize_seeds();
        self
    }

    pub fn chapter(mut self, chapter: impl Into<String>) -> Self {
        self.chapter = chapter.into();
        self = self.normalize_seeds();
        self
    }

    fn normalize_seeds(mut self) -> Self {
        if !self.language_tags_explicit {
            self.language_tags = normalize_string_tuple(
                [self.source_language.as_str(), self.target_language.as_str()],
                true,
            );
        }
        if !self.story_scopes_explicit {
            let mut seeds: Vec<String> = Vec::new();
            if !self.volume.trim().is_empty() {
                seeds.push(format!("volume:{}", self.volume.trim()));
            }
            if !self.chapter.trim().is_empty() {
                seeds.push(format!("chapter:{}", self.chapter.trim()));
            }
            self.story_scopes = seeds;
        }
        if !self.semantic_tags_explicit {
            self.semantic_tags =
                normalize_string_tuple(self.tags.iter().map(String::as_str), false);
        }
        self
    }

    /// Typed metadata overrides; `None` falls back to the compatibility seeds.
    pub fn with_metadata(
        mut self,
        language_tags: Option<Vec<String>>,
        story_scopes: Option<Vec<String>>,
        semantic_tags: Option<Vec<String>>,
    ) -> Self {
        if let Some(tags) = language_tags {
            self.language_tags = normalize_string_tuple(tags.iter().map(String::as_str), true);
            self.language_tags_explicit = true;
        }
        if let Some(scopes) = story_scopes {
            self.story_scopes = normalize_string_tuple(scopes.iter().map(String::as_str), false);
            self.story_scopes_explicit = true;
        }
        if let Some(tags) = semantic_tags {
            self.semantic_tags = normalize_string_tuple(tags.iter().map(String::as_str), false);
            self.semantic_tags_explicit = true;
        }
        self
    }

    pub fn scope_key(&self) -> String {
        format!("series:{}", self.series_slug)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSessionRecord {
    pub id: i64,
    pub context: TranslationContext,
    pub status: String,
    pub cycle_id: Option<i64>,
}

/// Metadata values are JSON scalars/arrays; sorted-key serialization matches
/// the Python `json.dumps(..., sort_keys=True)` contract.
pub type MetadataMap = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, PartialEq)]
pub struct ShortTermMemoryRecord {
    pub id: i64,
    pub session_id: i64,
    pub source_role: String,
    pub kind: String,
    pub text: String,
    pub source_ref: String,
    pub metadata: MetadataMap,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub source_credibility: String,
    pub rule_intent: String,
    pub soft_origin: String,
}

/// Long-term crystal: the advisory memory projection (ADR 0011). A rule
/// crystal's deterministic authority lives in `term_rules`, not here.
#[derive(Debug, Clone, PartialEq)]
pub struct CrystalRecord {
    pub id: i64,
    pub crystal_type: String,
    pub text: String,
    pub title: String,
    pub scope_type: String,
    pub scope_key: String,
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    pub strength: f64,
    pub confidence: f64,
    pub status: String,
    pub source_credibility: String,
    pub rule_intent: String,
    pub malformed_penalty: f64,
    pub supersedes_crystal_id: Option<i64>,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub soft_origin: String,
    pub is_inferred: bool,
    pub concept_ids: Vec<i64>,
}
