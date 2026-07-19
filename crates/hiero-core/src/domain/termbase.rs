use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::Utc;
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection, SqlitePool, Transaction};

use super::{
    ContractTerm, TermProposal, TranslationContext, ValidationFinding,
    rule_parser::{ParsedRule, parse_rule},
};

const MIN_DETERMINISTIC_SCORE: f64 = 0.8;
const HYDRATION_CHUNK_SIZE: usize = 500;

pub type Result<T> = std::result::Result<T, TermbaseError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TermbaseError {
    #[error("invalid term {field}: {reason}")]
    Validation { field: &'static str, reason: String },
    #[error("unknown term: {id}")]
    NotFound { id: i64 },
    #[error("term {operation} conflicted with existing state: {reason}")]
    Conflict {
        operation: &'static str,
        reason: String,
    },
    #[error("term {operation} failed: {source}")]
    Database {
        operation: &'static str,
        #[source]
        source: sqlx::Error,
    },
    #[error("term tags could not be encoded: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

pub struct Termbase<'a> {
    pool: &'a SqlitePool,
    context: TranslationContext,
}

impl<'a> Termbase<'a> {
    #[must_use]
    pub fn new(pool: &'a SqlitePool, context: TranslationContext) -> Self {
        Self { pool, context }
    }

    pub async fn propose(&self, input: TermProposal) -> Result<i64> {
        let input = validate_proposal(input)?;
        if input.series_slug != self.context.series_slug
            || input.source_language != self.context.source_language
            || input.target_language != self.context.target_language
        {
            return Err(conflict(
                "propose",
                "proposal series and language pair must match the termbase context",
            ));
        }
        let tags_json =
            serde_json::to_string(&input.tags).map_err(|source| TermbaseError::Json { source })?;
        let text = rule_text(&input.source_text, &input.canonical_translation, None);
        let now = Utc::now();
        let mut transaction = begin_immediate(self.pool, "propose").await?;
        let result = async {
            let duplicate: Option<i64> = sqlx::query_scalar("SELECT id FROM crystals WHERE text = ? AND scope_type = 'series' AND scope_key = ? AND series_slug = ? AND source_language = ? AND target_language = ? AND trim(rule_intent) <> '' AND status IN ('active','candidate') ORDER BY id LIMIT 1")
                .bind(&text).bind(format!("series:{}", input.series_slug)).bind(&input.series_slug).bind(&input.source_language).bind(&input.target_language)
                .fetch_optional(&mut *transaction).await.map_err(|source| database("propose", source))?;
            if duplicate.is_some() { return Err(conflict("propose", "an equivalent active or candidate term exists")); }
            let id = sqlx::query("INSERT INTO crystals(crystal_type, text, title, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, source_credibility, rule_intent, status, created_at, updated_at) VALUES ('rule', ?, '', 'series', ?, ?, ?, ?, ?, 0.8, 0.95, 'user_rule', ?, 'candidate', ?, ?)")
                .bind(&text).bind(format!("series:{}", input.series_slug)).bind(&input.series_slug).bind(&input.source_language).bind(&input.target_language).bind(tags_json).bind(&input.category).bind(now).bind(now)
                .execute(&mut *transaction).await.map_err(|source| database("propose", source))?.last_insert_rowid();
            for tag in &input.tags {
                sqlx::query("INSERT INTO crystal_semantic_tags(crystal_id, tag, confidence, created_at) VALUES (?, ?, 0.95, ?)")
                    .bind(id).bind(tag).bind(now).execute(&mut *transaction).await.map_err(|source| database("propose", source))?;
            }
            Ok(id)
        }.await;
        commit_write(transaction, "propose", result).await
    }

    pub async fn approve_term(&self, id: i64) -> Result<()> {
        let mut transaction = begin_immediate(self.pool, "approve").await?;
        let result = async {
            let term = get_term_on(&mut transaction, id, "approve").await?;
            if !matches!(term.status.as_str(), "candidate" | "active") {
                return Err(conflict(
                    "approve",
                    "only candidate or active terms can be approved",
                ));
            }
            let parsed = parse_rule(&term.text)
                .ok_or_else(|| invalid("text", "does not use the supported rule grammar"))?;
            validate_term_context(&term, &self.context)?;
            if term.status == "active" {
                validate_existing_relationships(&mut transaction, &term, &parsed).await?;
                return Ok(());
            }
            ensure_approved_relationships(&mut transaction, &term, &parsed).await?;
            sqlx::query("UPDATE crystals SET status = 'active', updated_at = ? WHERE id = ?")
                .bind(Utc::now())
                .bind(id)
                .execute(&mut *transaction)
                .await
                .map_err(|source| database("approve", source))?;
            Ok(())
        }
        .await;
        commit_write(transaction, "approve", result).await
    }

    pub async fn contract(&self, raw_text: &str) -> Result<Vec<ContractTerm>> {
        let rules = load_active_rules(self.pool, &self.context).await?;
        Ok(resolve_rules(raw_text, &rules, &self.context)
            .matched
            .into_iter()
            .map(|matched| ContractTerm {
                crystal_id: matched.rule.id,
                source_text: matched.surface,
                canonical_translation: matched.rule.parsed.canonical.clone(),
            })
            .collect())
    }

    pub async fn validate(
        &self,
        translated: &str,
        raw: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<ValidationFinding>> {
        let source = match (raw, source) {
            (Some(_), Some(_)) => return Err(invalid("source", "pass raw or source, not both")),
            (None, None) => return Err(invalid("source", "raw or source is required")),
            (Some(value), None) | (None, Some(value)) => value,
        };
        let rules = load_active_rules(self.pool, &self.context).await?;
        let resolved = resolve_rules(source, &rules, &self.context);
        let mut findings = resolved.findings;
        for matched in resolved.matched {
            for forbidden in &matched.rule.parsed.forbidden {
                if translated.contains(forbidden) {
                    findings.push(ValidationFinding {
                        crystal_id: matched.rule.id,
                        kind: "forbidden_variant_used".into(),
                        severity: "high".into(),
                        expected: matched.rule.parsed.canonical.clone(),
                        observed: forbidden.clone(),
                        detail: "forbidden rendering was used for a contracted source form".into(),
                    });
                }
            }
            if !translated.contains(&matched.rule.parsed.canonical) {
                findings.push(ValidationFinding {
                    crystal_id: matched.rule.id,
                    kind: "canonical_missing".into(),
                    severity: "medium".into(),
                    expected: matched.rule.parsed.canonical.clone(),
                    observed: String::new(),
                    detail: "contracted source form is missing its canonical rendering".into(),
                });
            }
        }
        findings.sort_by(|left, right| {
            left.crystal_id
                .cmp(&right.crystal_id)
                .then_with(|| finding_order(&left.kind).cmp(&finding_order(&right.kind)))
        });
        Ok(findings)
    }
}

#[derive(Debug, Clone, FromRow)]
struct TermRow {
    id: i64,
    text: String,
    scope_type: String,
    scope_key: String,
    series_slug: String,
    source_language: String,
    target_language: String,
    tags_json: String,
    strength: f64,
    confidence: f64,
    rule_intent: String,
    status: String,
}

#[derive(Debug, Clone)]
struct ActiveRule {
    id: i64,
    parsed: ParsedRule,
    concept_ids: Vec<i64>,
    source_forms: Vec<String>,
    language_tags: Vec<String>,
    story_scopes: Vec<String>,
    semantic_tags: Vec<String>,
}

#[derive(Debug)]
struct MatchedRule<'a> {
    rule: &'a ActiveRule,
    surface: String,
}

#[derive(Debug, Clone, FromRow)]
struct FacetHydration {
    id: i64,
    concept_id: i64,
    language: String,
    facet_type: String,
    value: String,
}

struct ResolvedRules<'a> {
    matched: Vec<MatchedRule<'a>>,
    findings: Vec<ValidationFinding>,
}

async fn load_active_rules(
    pool: &SqlitePool,
    context: &TranslationContext,
) -> Result<Vec<ActiveRule>> {
    let rows = sqlx::query_as::<_, TermRow>("SELECT id, text, scope_type, scope_key, series_slug, source_language, target_language, tags_json, strength, confidence, rule_intent, status FROM crystals WHERE status = 'active' AND trim(rule_intent) <> '' AND strength >= ? AND confidence >= ? AND ((scope_type = 'global' AND scope_key = '' AND series_slug = '') OR (scope_type = 'series' AND series_slug = ? AND scope_key = ?)) AND (source_language = '' OR source_language = ?) AND (target_language = '' OR target_language = ?) AND NOT EXISTS (SELECT 1 FROM crystals successor WHERE successor.supersedes_crystal_id = crystals.id AND successor.status = 'active' AND trim(successor.rule_intent) <> '') ORDER BY id")
        .bind(MIN_DETERMINISTIC_SCORE)
        .bind(MIN_DETERMINISTIC_SCORE)
        .bind(&context.series_slug)
        .bind(&context.scope_key)
        .bind(&context.source_language)
        .bind(&context.target_language)
        .fetch_all(pool)
        .await
        .map_err(|source| database("load active rules", source))?;
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut concepts = Vec::<(i64, i64, String)>::new();
    for chunk in ids.chunks(HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT cc.crystal_id, cc.concept_id, c.canonical_name FROM crystal_concepts cc JOIN concepts c ON c.id = cc.concept_id WHERE c.status NOT IN ('archived','merged') AND cc.crystal_id IN (",
        );
        push_ids(&mut query, chunk);
        query.push(") ORDER BY cc.crystal_id, cc.concept_id");
        concepts.extend(
            query
                .build_query_as()
                .fetch_all(pool)
                .await
                .map_err(|source| database("load active rules", source))?,
        );
    }
    let mut concept_ids: Vec<i64> = concepts.iter().map(|(_, id, _)| *id).collect();
    concept_ids.sort_unstable();
    concept_ids.dedup();
    let mut facets = Vec::<FacetHydration>::new();
    if !concept_ids.is_empty() {
        for chunk in concept_ids.chunks(HYDRATION_CHUNK_SIZE) {
            let mut query = QueryBuilder::<Sqlite>::new(
                "SELECT id, concept_id, language, facet_type, value FROM concept_facets WHERE superseded_at IS NULL AND concept_id IN (",
            );
            push_ids(&mut query, chunk);
            query.push(") ORDER BY concept_id, id");
            facets.extend(
                query
                    .build_query_as()
                    .fetch_all(pool)
                    .await
                    .map_err(|source| database("load active rules", source))?,
            );
        }
    }
    let facet_ids: Vec<i64> = facets.iter().map(|facet| facet.id).collect();
    let crystal_languages = load_text_metadata(pool, &ids, MetadataTable::CrystalLanguage).await?;
    let crystal_stories = load_text_metadata(pool, &ids, MetadataTable::CrystalStory).await?;
    let crystal_semantics = load_text_metadata(pool, &ids, MetadataTable::CrystalSemantic).await?;
    let concept_semantics =
        load_text_metadata(pool, &concept_ids, MetadataTable::ConceptSemantic).await?;
    let facet_languages =
        load_text_metadata(pool, &facet_ids, MetadataTable::FacetLanguage).await?;
    let facet_stories = load_text_metadata(pool, &facet_ids, MetadataTable::FacetStory).await?;
    let facet_semantics =
        load_text_metadata(pool, &facet_ids, MetadataTable::FacetSemantic).await?;
    let mut facets_by_concept: HashMap<i64, Vec<&FacetHydration>> = HashMap::new();
    for facet in &facets {
        facets_by_concept
            .entry(facet.concept_id)
            .or_default()
            .push(facet);
    }
    let mut concepts_by_rule: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    for (rule_id, concept_id, name) in concepts {
        concepts_by_rule
            .entry(rule_id)
            .or_default()
            .push((concept_id, name));
    }
    let mut rules = Vec::new();
    for row in rows {
        let Some(parsed) = parse_rule(&row.text) else {
            continue;
        };
        let Some(concepts) = concepts_by_rule.remove(&row.id) else {
            continue;
        };
        if concepts.is_empty() {
            continue;
        }
        let mut forms = vec![parsed.source_text.clone()];
        let mut linked_ids = Vec::new();
        let mut language_tags = crystal_languages.get(&row.id).cloned().unwrap_or_default();
        language_tags.extend(
            [row.source_language.clone(), row.target_language.clone()]
                .into_iter()
                .filter(|value| !value.is_empty()),
        );
        let mut story_scopes = crystal_stories.get(&row.id).cloned().unwrap_or_default();
        let mut semantic_tags = crystal_semantics.get(&row.id).cloned().unwrap_or_default();
        for (concept_id, name) in concepts {
            linked_ids.push(concept_id);
            forms.push(name);
            semantic_tags.extend(
                concept_semantics
                    .get(&concept_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            for facet in facets_by_concept.get(&concept_id).into_iter().flatten() {
                let mut facet_language_tags =
                    facet_languages.get(&facet.id).cloned().unwrap_or_default();
                if facet_language_tags.is_empty() && !facet.language.is_empty() {
                    facet_language_tags.push(facet.language.clone());
                }
                language_tags.extend(facet_language_tags.iter().cloned());
                story_scopes.extend(facet_stories.get(&facet.id).cloned().unwrap_or_default());
                semantic_tags.extend(facet_semantics.get(&facet.id).cloned().unwrap_or_default());
                if is_source_facet(facet, &facet_language_tags, context) {
                    forms.push(facet.value.clone());
                }
            }
        }
        linked_ids.sort_unstable();
        linked_ids.dedup();
        let story_scopes = sorted_texts(story_scopes);
        if !story_scopes.is_empty()
            && !story_scopes
                .iter()
                .any(|scope| context.story_scopes.contains(scope))
        {
            continue;
        }
        rules.push(ActiveRule {
            id: row.id,
            parsed,
            concept_ids: linked_ids,
            source_forms: dedupe_folded(forms),
            language_tags: sorted_texts(language_tags),
            story_scopes,
            semantic_tags: sorted_texts(semantic_tags),
        });
    }
    Ok(rules)
}

fn resolve_rules<'a>(
    raw_text: &str,
    rules: &'a [ActiveRule],
    context: &TranslationContext,
) -> ResolvedRules<'a> {
    let folded = raw_text.to_lowercase();
    let mut occurrences = Vec::<(usize, usize, String, &'a ActiveRule)>::new();
    for rule in rules {
        for surface in &rule.source_forms {
            let needle = surface.to_lowercase();
            if needle.is_empty() {
                continue;
            }
            for (start, _) in folded.match_indices(&needle) {
                let end = start + needle.len();
                occurrences.push((start, end, surface.clone(), rule));
            }
        }
    }
    occurrences.sort_by(|left, right| {
        (right.1 - right.0)
            .cmp(&(left.1 - left.0))
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.3.id.cmp(&right.3.id))
    });
    let mut accepted = Vec::new();
    for occurrence in occurrences {
        if accepted.iter().any(|(start, end, _, _)| {
            *start <= occurrence.0
                && occurrence.1 <= *end
                && (*end - *start) > (occurrence.1 - occurrence.0)
        }) {
            continue;
        }
        accepted.push(occurrence);
    }
    let mut grouped: BTreeMap<String, (String, Vec<&ActiveRule>)> = BTreeMap::new();
    for (_, _, surface, rule) in accepted {
        let entry = grouped
            .entry(surface.to_lowercase())
            .or_insert_with(|| (surface, Vec::new()));
        if !entry.1.iter().any(|seen| seen.id == rule.id) {
            entry.1.push(rule);
        }
    }
    let mut selected = Vec::new();
    let mut findings = Vec::new();
    for (_, (surface, mut candidates)) in grouped {
        candidates.sort_by_key(|rule| rule.id);
        let mut concepts: BTreeSet<Vec<i64>> = candidates
            .iter()
            .map(|rule| rule.concept_ids.clone())
            .collect();
        let unresolved_candidates = candidates.clone();
        if concepts.len() > 1 {
            candidates = context_resolved_rules(&candidates, context);
            concepts = candidates
                .iter()
                .map(|rule| rule.concept_ids.clone())
                .collect();
        }
        let renderings: BTreeSet<&str> = candidates
            .iter()
            .map(|rule| rule.parsed.canonical.as_str())
            .collect();
        if concepts.len() != 1 {
            findings.push(ambiguity_finding(&surface, &unresolved_candidates));
            continue;
        }
        if renderings.len() != 1 {
            findings.push(conflict_finding(&surface, &candidates));
            continue;
        }
        for rule in candidates {
            selected.push(MatchedRule {
                rule,
                surface: surface.clone(),
            });
        }
    }
    selected.sort_by_key(|matched| matched.rule.id);
    findings.sort_by(|left, right| {
        left.crystal_id
            .cmp(&right.crystal_id)
            .then_with(|| finding_order(&left.kind).cmp(&finding_order(&right.kind)))
    });
    ResolvedRules {
        matched: selected,
        findings,
    }
}

#[derive(Clone, Copy)]
enum MetadataTable {
    CrystalLanguage,
    CrystalStory,
    CrystalSemantic,
    ConceptSemantic,
    FacetLanguage,
    FacetStory,
    FacetSemantic,
}

impl MetadataTable {
    fn select_prefix(self) -> &'static str {
        match self {
            Self::CrystalLanguage => {
                "SELECT crystal_id, language_tag FROM crystal_language_tags WHERE crystal_id IN ("
            }
            Self::CrystalStory => {
                "SELECT crystal_id, scope FROM crystal_story_scopes WHERE crystal_id IN ("
            }
            Self::CrystalSemantic => {
                "SELECT crystal_id, tag FROM crystal_semantic_tags WHERE crystal_id IN ("
            }
            Self::ConceptSemantic => {
                "SELECT concept_id, tag FROM concept_semantic_tags WHERE concept_id IN ("
            }
            Self::FacetLanguage => {
                "SELECT facet_id, language_tag FROM concept_facet_language_tags WHERE facet_id IN ("
            }
            Self::FacetStory => {
                "SELECT facet_id, story_scope FROM concept_facet_story_scopes WHERE facet_id IN ("
            }
            Self::FacetSemantic => {
                "SELECT facet_id, semantic_tag FROM concept_facet_semantic_tags WHERE facet_id IN ("
            }
        }
    }
}

async fn load_text_metadata(
    pool: &SqlitePool,
    ids: &[i64],
    table: MetadataTable,
) -> Result<HashMap<i64, Vec<String>>> {
    let mut grouped = HashMap::<i64, Vec<String>>::new();
    for chunk in ids.chunks(HYDRATION_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(table.select_prefix());
        push_ids(&mut query, chunk);
        query.push(") ORDER BY 1, 2");
        let rows: Vec<(i64, String)> = query
            .build_query_as()
            .fetch_all(pool)
            .await
            .map_err(|source| database("hydrate active rules", source))?;
        for (id, value) in rows {
            grouped.entry(id).or_default().push(value);
        }
    }
    Ok(grouped)
}

fn is_source_facet(
    facet: &FacetHydration,
    language_tags: &[String],
    context: &TranslationContext,
) -> bool {
    if !matches!(facet.facet_type.as_str(), "name" | "alias" | "former_label") {
        return false;
    }
    language_tags.is_empty()
        || language_tags.contains(&context.source_language)
        || !language_tags.contains(&context.target_language)
}

fn sorted_texts(values: Vec<String>) -> Vec<String> {
    let mut values: Vec<String> = values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

fn context_resolved_rules<'a>(
    candidates: &[&'a ActiveRule],
    context: &TranslationContext,
) -> Vec<&'a ActiveRule> {
    let default_languages = BTreeSet::from([
        context.source_language.as_str(),
        context.target_language.as_str(),
    ]);
    let extra_languages: BTreeSet<&str> = context
        .language_tags
        .iter()
        .map(String::as_str)
        .filter(|tag| !default_languages.contains(tag))
        .collect();
    let mut scored = Vec::new();
    for candidate in candidates {
        let mut score = 0_u8;
        if candidate
            .semantic_tags
            .iter()
            .any(|tag| context.semantic_tags.contains(tag))
        {
            score += 1;
        }
        if candidate
            .story_scopes
            .iter()
            .any(|scope| context.story_scopes.contains(scope))
        {
            score += 1;
        }
        if candidate
            .language_tags
            .iter()
            .map(String::as_str)
            .any(|tag| extra_languages.contains(tag))
        {
            score += 1;
        }
        scored.push((score, *candidate));
    }
    let best = scored.iter().map(|(score, _)| *score).max().unwrap_or(0);
    if best == 0 {
        Vec::new()
    } else {
        scored
            .into_iter()
            .filter_map(|(score, candidate)| (score == best).then_some(candidate))
            .collect()
    }
}

fn ambiguity_finding(surface: &str, candidates: &[&ActiveRule]) -> ValidationFinding {
    ValidationFinding {
        crystal_id: candidates.iter().map(|rule| rule.id).min().unwrap_or(0),
        kind: "ambiguous_source".into(),
        severity: "warning".into(),
        expected: renderings(candidates),
        observed: surface.to_owned(),
        detail: "source form maps to multiple active rule concepts; add context before enforcing a rendering".into(),
    }
}

fn conflict_finding(surface: &str, candidates: &[&ActiveRule]) -> ValidationFinding {
    ValidationFinding {
        crystal_id: candidates.iter().map(|rule| rule.id).min().unwrap_or(0),
        kind: "conflicting_active_rules".into(),
        severity: "warning".into(),
        expected: renderings(candidates),
        observed: surface.to_owned(),
        detail: "source form has conflicting active rules for the same concept; consolidate before enforcing a rendering".into(),
    }
}

fn renderings(candidates: &[&ActiveRule]) -> String {
    let values: BTreeSet<&str> = candidates
        .iter()
        .map(|rule| rule.parsed.canonical.as_str())
        .filter(|value| !value.is_empty())
        .collect();
    values.into_iter().collect::<Vec<_>>().join(", ")
}

async fn ensure_approved_relationships(
    connection: &mut SqliteConnection,
    term: &TermRow,
    parsed: &ParsedRule,
) -> Result<()> {
    validate_persisted_term(term)?;
    let tags: Vec<String> = serde_json::from_str(&term.tags_json)
        .map_err(|_| invalid("tags_json", "must be an array of strings"))?;
    let tags = clean_values(&tags);
    let concept_id = find_or_create_concept(connection, term, parsed, &tags).await?;
    ensure_facet(
        connection,
        concept_id,
        &term.source_language,
        "name",
        &parsed.source_text,
        true,
    )
    .await?;
    ensure_facet(
        connection,
        concept_id,
        &term.target_language,
        "rendering",
        &parsed.canonical,
        false,
    )
    .await?;
    for tag in &tags {
        sqlx::query("INSERT INTO concept_semantic_tags(concept_id, tag, confidence, created_at) VALUES (?, ?, 0.95, ?) ON CONFLICT(concept_id, tag) DO UPDATE SET confidence=max(concept_semantic_tags.confidence,excluded.confidence)").bind(concept_id).bind(tag).bind(Utc::now()).execute(&mut *connection).await.map_err(|source|database("approve",source))?;
    }
    sqlx::query("INSERT INTO crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at) VALUES (?, ?, 'defines', 0.95, ?) ON CONFLICT(crystal_id,concept_id,link_type) DO UPDATE SET confidence=max(crystal_concepts.confidence,excluded.confidence)").bind(term.id).bind(concept_id).bind(Utc::now()).execute(&mut *connection).await.map_err(|source|database("approve",source))?;
    for (table, target) in [("concepts", concept_id), ("crystals", term.id)] {
        sqlx::query("INSERT INTO migration_ledger(source_table, source_id, target_table, target_id) VALUES ('term_proposals', ?, ?, ?) ON CONFLICT(source_table,source_id,target_table) DO UPDATE SET target_id=excluded.target_id").bind(term.id.to_string()).bind(table).bind(target).execute(&mut *connection).await.map_err(|source|database("approve",source))?;
    }
    Ok(())
}

fn validate_term_context(term: &TermRow, context: &TranslationContext) -> Result<()> {
    if term.scope_type != "series"
        || term.series_slug != context.series_slug
        || term.scope_key != context.scope_key
        || term.source_language != context.source_language
        || term.target_language != context.target_language
    {
        return Err(conflict(
            "approve",
            "term series and language pair do not match the termbase context",
        ));
    }
    Ok(())
}

async fn validate_existing_relationships(
    connection: &mut SqliteConnection,
    term: &TermRow,
    parsed: &ParsedRule,
) -> Result<()> {
    validate_persisted_term(term)
        .map_err(|_| conflict("approve", "active term has invalid persisted fields"))?;
    serde_json::from_str::<Vec<String>>(&term.tags_json)
        .map_err(|_| conflict("approve", "active term has invalid tag metadata"))?;

    let concepts: Vec<(i64, String)> = sqlx::query_as(
        "SELECT c.id, c.status FROM crystal_concepts cc JOIN concepts c ON c.id = cc.concept_id WHERE cc.crystal_id = ? AND cc.link_type = 'defines' ORDER BY c.id",
    )
    .bind(term.id)
    .fetch_all(&mut *connection)
    .await
    .map_err(|source| database("validate active approval", source))?;
    let [(concept_id, status)] = concepts.as_slice() else {
        return Err(conflict(
            "approve",
            "active term must define exactly one concept",
        ));
    };
    if matches!(status.as_str(), "archived" | "merged") {
        return Err(conflict(
            "approve",
            "active term points to an inactive concept",
        ));
    }

    let source_id = term.id.to_string();
    for (target_table, expected_id) in [("crystals", term.id), ("concepts", *concept_id)] {
        let target_id: Option<i64> = sqlx::query_scalar(
            "SELECT target_id FROM migration_ledger WHERE source_table = 'term_proposals' AND source_id = ? AND target_table = ?",
        )
        .bind(&source_id)
        .bind(target_table)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|source| database("validate active approval", source))?;
        if target_id != Some(expected_id) {
            return Err(conflict(
                "approve",
                "active term migration ledger is missing or inconsistent",
            ));
        }
    }

    for (language, facet_type, value) in [
        (&term.source_language, "name", &parsed.source_text),
        (&term.target_language, "rendering", &parsed.canonical),
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM concept_facets f WHERE f.concept_id = ? AND f.language = ? AND f.facet_type = ? AND f.value = ? AND f.superseded_at IS NULL AND EXISTS(SELECT 1 FROM concept_facet_language_tags lt WHERE lt.facet_id = f.id AND lt.language_tag = ?))",
        )
        .bind(concept_id)
        .bind(language)
        .bind(facet_type)
        .bind(value)
        .bind(language)
        .fetch_one(&mut *connection)
        .await
        .map_err(|source| database("validate active approval", source))?;
        if !exists {
            return Err(conflict(
                "approve",
                "active term concept facets are missing or inconsistent",
            ));
        }
    }
    Ok(())
}

async fn find_or_create_concept(
    connection: &mut SqliteConnection,
    term: &TermRow,
    parsed: &ParsedRule,
    tags: &[String],
) -> Result<i64> {
    let candidates:Vec<i64>=sqlx::query_scalar("SELECT id FROM concepts WHERE canonical_name = ? AND scope_type = ? AND scope_key = ? AND status NOT IN ('archived','merged') ORDER BY id").bind(&parsed.source_text).bind(&term.scope_type).bind(&term.scope_key).fetch_all(&mut *connection).await.map_err(|source|database("approve",source))?;
    let selected = if candidates.len() == 1 {
        let candidate = candidates[0];
        let renderings: Vec<String> = sqlx::query_scalar(
            "SELECT value FROM concept_facets WHERE concept_id = ? AND language = ? AND facet_type = 'rendering' AND superseded_at IS NULL ORDER BY id",
        )
        .bind(candidate)
        .bind(&term.target_language)
        .fetch_all(&mut *connection)
        .await
        .map_err(|source| database("approve", source))?;
        (renderings.is_empty() || renderings.iter().any(|value| value == &parsed.canonical))
            .then_some(candidate)
    } else if candidates.len() > 1 && !tags.is_empty() {
        let mut scored = Vec::new();
        for id in &candidates {
            let count:i64=sqlx::query_scalar("SELECT count(*) FROM concept_semantic_tags WHERE concept_id = ? AND tag IN (SELECT value FROM json_each(?))").bind(id).bind(serde_json::to_string(tags).map_err(|source|TermbaseError::Json{source})?).fetch_one(&mut *connection).await.map_err(|source|database("approve",source))?;
            scored.push((count, *id));
        }
        scored.sort_by(|a, b| b.cmp(a));
        if scored.first().is_some_and(|x| x.0 > 0)
            && scored.get(1).is_none_or(|x| x.0 < scored[0].0)
        {
            Some(scored[0].1)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(id) = selected {
        return Ok(id);
    }
    let now = Utc::now();
    sqlx::query("INSERT INTO concepts(canonical_name,scope_type,scope_key,status,confidence,created_at,updated_at) VALUES(?,?,?,'established',0.95,?,?)").bind(&parsed.source_text).bind(&term.scope_type).bind(&term.scope_key).bind(now).bind(now).execute(&mut *connection).await.map(|result|result.last_insert_rowid()).map_err(|source|database("approve",source))
}
async fn ensure_facet(
    connection: &mut SqliteConnection,
    concept_id: i64,
    language: &str,
    facet_type: &str,
    value: &str,
    canonical: bool,
) -> Result<()> {
    let existing:Option<i64>=sqlx::query_scalar("SELECT id FROM concept_facets WHERE concept_id=? AND language=? AND facet_type=? AND value=? AND superseded_at IS NULL ORDER BY id LIMIT 1").bind(concept_id).bind(language).bind(facet_type).bind(value).fetch_optional(&mut *connection).await.map_err(|source|database("approve",source))?;
    let id = if let Some(id) = existing {
        id
    } else {
        let now = Utc::now();
        sqlx::query("INSERT INTO concept_facets(concept_id,language,facet_type,value,confidence,is_canonical,created_at,updated_at) VALUES(?,?,?,?,0.95,?,?,?)").bind(concept_id).bind(language).bind(facet_type).bind(value).bind(canonical).bind(now).bind(now).execute(&mut *connection).await.map_err(|source|database("approve",source))?.last_insert_rowid()
    };
    if !language.is_empty() {
        sqlx::query(
            "INSERT OR IGNORE INTO concept_facet_language_tags(facet_id,language_tag) VALUES(?,?)",
        )
        .bind(id)
        .bind(language)
        .execute(&mut *connection)
        .await
        .map_err(|source| database("approve", source))?;
    }
    Ok(())
}

fn validate_proposal(mut input: TermProposal) -> Result<TermProposal> {
    input.series_slug = required("series_slug", &input.series_slug)?;
    if !valid_slug(&input.series_slug) {
        return Err(invalid("series_slug", "must be a canonical slug"));
    }
    input.source_language = language("source_language", &input.source_language)?;
    input.target_language = language("target_language", &input.target_language)?;
    input.category = required("category", &input.category)?;
    input.source_text = required("source_text", &input.source_text)?;
    input.canonical_translation = required("canonical_translation", &input.canonical_translation)?;
    if input.source_text.contains(" is translated as ") || input.source_text.contains(", not ") {
        return Err(invalid("source_text", "contains a reserved rule delimiter"));
    }
    if input.canonical_translation.contains(" is translated as ")
        || input.canonical_translation.contains(", not ")
    {
        return Err(invalid(
            "canonical_translation",
            "contains a reserved rule delimiter",
        ));
    }
    input.tags = clean_values(&input.tags);
    Ok(input)
}
fn validate_persisted_term(term: &TermRow) -> Result<()> {
    if term.rule_intent.trim().is_empty() {
        return Err(invalid("rule_intent", "must not be empty"));
    }
    if !term.strength.is_finite() || !term.confidence.is_finite() {
        return Err(invalid("score", "must be finite"));
    }
    if term.scope_type == "series"
        && (term.series_slug.is_empty() || term.scope_key != format!("series:{}", term.series_slug))
    {
        return Err(invalid("scope", "is incoherent"));
    }
    if term.source_language.trim().is_empty() || term.target_language.trim().is_empty() {
        return Err(invalid("languages", "must not be empty"));
    }
    Ok(())
}
fn rule_text(source: &str, canonical: &str, forbidden: Option<&str>) -> String {
    match forbidden {
        Some(value) => format!("{source} is translated as {canonical}, not {value}."),
        None => format!("{source} is translated as {canonical}."),
    }
}
fn dedupe_folded(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim().to_owned();
            if value.is_empty() || !seen.insert(value.to_lowercase()) {
                None
            } else {
                Some(value)
            }
        })
        .collect()
}
fn clean_values(values: &[String]) -> Vec<String> {
    let values: Vec<String> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    crate::values::normalize_tuple(&values)
}
fn language(field: &'static str, value: &str) -> Result<String> {
    let value = required(field, value)?.to_lowercase();
    if value.chars().any(char::is_whitespace) {
        Err(invalid(field, "must be a compact language tag"))
    } else {
        Ok(value)
    }
}
fn required(field: &'static str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(invalid(field, "must not be empty"))
    } else {
        Ok(value.to_owned())
    }
}
fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}
fn finding_order(kind: &str) -> u8 {
    match kind {
        "ambiguous_source" => 0,
        "conflicting_active_rules" => 1,
        "forbidden_variant_used" => 2,
        "canonical_missing" => 3,
        _ => 4,
    }
}
fn push_ids(builder: &mut QueryBuilder<Sqlite>, ids: &[i64]) {
    let mut values = builder.separated(", ");
    for id in ids {
        values.push_bind(*id);
    }
}
async fn get_term_on(
    connection: &mut SqliteConnection,
    id: i64,
    operation: &'static str,
) -> Result<TermRow> {
    sqlx::query_as::<_,TermRow>("SELECT id,text,scope_type,scope_key,series_slug,source_language,target_language,tags_json,strength,confidence,rule_intent,status FROM crystals WHERE id=? AND trim(rule_intent)<>''").bind(id).fetch_optional(&mut *connection).await.map_err(|source|database(operation,source))?.ok_or(TermbaseError::NotFound{id})
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
fn invalid(field: &'static str, reason: impl Into<String>) -> TermbaseError {
    TermbaseError::Validation {
        field,
        reason: reason.into(),
    }
}
fn conflict(operation: &'static str, reason: impl Into<String>) -> TermbaseError {
    TermbaseError::Conflict {
        operation,
        reason: reason.into(),
    }
}
fn database(operation: &'static str, source: sqlx::Error) -> TermbaseError {
    TermbaseError::Database { operation, source }
}
