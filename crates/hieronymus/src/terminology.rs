//! Deterministic terminology authority (ADR 0011): `term_rules`/
//! `term_rule_forms` own the enforceable rendering contract; fuzzy recall and
//! dreaming can only propose candidates.

use std::path::Path;

use chrono::Utc;
use rusqlite::Connection;

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;

#[derive(Debug, thiserror::Error)]
pub enum TermbaseError {
    #[error("unknown term rule: {0}")]
    UnknownRule(i64),
    #[error("rule crystals support at most one forbidden variant")]
    TooManyForbiddenVariants,
    #[error("approved variants that differ from canonical rendering are unsupported")]
    ApprovedVariantMismatch,
    #[error("rule crystal text cannot round-trip parsed fields")]
    RoundTripFailure,
    #[error("unknown concept: {0}")]
    UnknownConcept(i64),
    #[error("{0}")]
    Invalid(String),
    #[error("pass either raw_text or source_text, not both")]
    BothSources,
    #[error("validate requires raw_text or source_text")]
    MissingSource,
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(String),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}

/// One enforceable rule in the translation contract.
#[derive(Debug, Clone, PartialEq)]
pub struct ContractTerm {
    pub id: i64,
    pub category: String,
    pub source_text: String,
    pub canonical_translation: String,
    pub forbidden_variants: Vec<String>,
    pub tags: Vec<String>,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidationFinding {
    pub term_id: i64,
    pub kind: String,
    pub severity: String,
    pub expected: String,
    pub observed: String,
    pub message: String,
}

/// A stored rule with its forms hydrated.
#[derive(Debug, Clone, PartialEq)]
pub struct TermRule {
    pub id: i64,
    pub concept_id: Option<i64>,
    pub source_language: String,
    pub target_language: String,
    pub source_text: String,
    pub canonical_translation: String,
    pub forbidden_variants: Vec<String>,
    pub status: String,
    pub provenance: String,
    pub revision: i64,
    pub rule_crystal_id: Option<i64>,
    pub semantic_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub language_tags: Vec<String>,
}

/// The deterministic termbase over one translation context.
pub struct Termbase {
    config: HieronymusConfig,
    context: crate::memory_models::TranslationContext,
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

/// The canonical rule sentence: also the round-trip shape enforced on
/// propose (`<source> is translated as <target>[, not <forbidden>].`).
fn rule_text(
    source_text: &str,
    canonical_translation: &str,
    forbidden_variants: &[String],
) -> String {
    match forbidden_variants.first() {
        Some(forbidden) => {
            format!("{source_text} is translated as {canonical_translation}, not {forbidden}.")
        }
        None => format!("{source_text} is translated as {canonical_translation}."),
    }
}

fn validate_rule_shape(
    source_text: &str,
    canonical_translation: &str,
    approved_variants: &[String],
    forbidden_variants: &[String],
) -> Result<(), TermbaseError> {
    if forbidden_variants.len() > 1 {
        return Err(TermbaseError::TooManyForbiddenVariants);
    }
    if approved_variants
        .iter()
        .any(|variant| variant != canonical_translation)
    {
        return Err(TermbaseError::ApprovedVariantMismatch);
    }
    let text = rule_text(source_text, canonical_translation, forbidden_variants);
    let parsed = parse_rule_crystal(&text);
    match parsed {
        Some((parsed_source, parsed_canonical, parsed_forbidden))
            if parsed_source == source_text
                && parsed_canonical == canonical_translation
                && parsed_forbidden == forbidden_variants =>
        {
            Ok(())
        }
        _ => Err(TermbaseError::RoundTripFailure),
    }
}

/// Parse the canonical rule sentence. Returns
/// `(source_text, canonical_translation, forbidden_variants)`.
pub fn parse_rule_crystal(text: &str) -> Option<(String, String, Vec<String>)> {
    let text = text.strip_suffix('.')?;
    let (source_text, rest) = text.split_once(" is translated as ")?;
    let source_text = source_text.trim();
    if source_text.is_empty() {
        return None;
    }
    let (canonical_translation, forbidden) = match rest.split_once(", not ") {
        Some((canonical, forbidden)) => (canonical, Some(forbidden)),
        None => (rest, None),
    };
    if canonical_translation.trim().is_empty() {
        return None;
    }
    let forbidden_variants = match forbidden {
        Some(forbidden) => {
            let variants: Vec<String> = forbidden
                .split(" or ")
                .map(str::trim)
                .filter(|variant| !variant.is_empty())
                .map(String::from)
                .collect();
            if variants.is_empty() {
                return None;
            }
            variants
        }
        None => Vec::new(),
    };
    Some((
        source_text.to_string(),
        canonical_translation.trim().to_string(),
        forbidden_variants,
    ))
}

fn contains(raw_text: &str, text: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        raw_text.contains(text)
    } else {
        raw_text.to_lowercase().contains(&text.to_lowercase())
    }
}

struct ResolvedRule {
    rule: TermRule,
    source_surface: String,
}

/// Score one candidate against the translation context: +1 per intersecting
/// semantic tags, story scopes, and non-default language tags.
fn score_context_overlap<'a>(
    candidates: &[&'a TermRule],
    context: &crate::memory_models::TranslationContext,
) -> Vec<(i64, &'a TermRule)> {
    let context_semantic: std::collections::HashSet<&str> =
        context.semantic_tags.iter().map(String::as_str).collect();
    let context_scopes: std::collections::HashSet<&str> =
        context.story_scopes.iter().map(String::as_str).collect();
    let defaults: std::collections::HashSet<&str> = [
        context.source_language.as_str(),
        context.target_language.as_str(),
    ]
    .into_iter()
    .collect();
    let context_languages: std::collections::HashSet<&str> = context
        .language_tags
        .iter()
        .map(String::as_str)
        .filter(|tag| !defaults.contains(tag))
        .collect();

    candidates
        .iter()
        .map(|candidate| {
            let mut score = 0;
            if candidate
                .semantic_tags
                .iter()
                .any(|tag| context_semantic.contains(tag.as_str()))
            {
                score += 1;
            }
            if candidate
                .story_scopes
                .iter()
                .any(|scope| context_scopes.contains(scope.as_str()))
            {
                score += 1;
            }
            if candidate
                .language_tags
                .iter()
                .map(String::as_str)
                .any(|tag| !defaults.contains(tag) && context_languages.contains(tag))
            {
                score += 1;
            }
            (score, *candidate)
        })
        .collect()
}

/// Projection of a `term_rules` row for active-rule resolution.
struct TermRuleRow {
    id: i64,
    concept_id: Option<i64>,
    source_text: String,
    canonical_translation: String,
    forbidden_json: String,
    provenance: String,
    revision: i64,
    rule_crystal_id: Option<i64>,
}

impl Termbase {
    pub fn open(
        config: &HieronymusConfig,
        context: &crate::memory_models::TranslationContext,
    ) -> Result<Self, TermbaseError> {
        open_migrated(Path::new(&config.database_path()))?;
        Ok(Self {
            config: config.clone(),
            context: context.clone(),
        })
    }

    pub fn context(&self) -> &crate::memory_models::TranslationContext {
        &self.context
    }

    fn connection(&self) -> Result<Connection, TermbaseError> {
        let path: std::path::PathBuf = self.config.database_path();
        Ok(open_migrated(&path)?)
    }

    /// Register a candidate rule after deterministic shape validation.
    pub fn propose(
        &self,
        source_text: &str,
        canonical_translation: &str,
        fields: &ProposeFields,
    ) -> Result<TermRule, TermbaseError> {
        let concept_id = fields.concept_id;
        let approved_variants = &fields.approved_variants;
        let forbidden_variants = &fields.forbidden_variants;
        validate_rule_shape(
            source_text,
            canonical_translation,
            approved_variants,
            forbidden_variants,
        )?;
        if let Some(concept_id) = concept_id {
            let exists: Option<i64> = self
                .connection()?
                .query_row(
                    "select id from concepts where id = ?1",
                    [concept_id],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            if exists.is_none() {
                return Err(TermbaseError::UnknownConcept(concept_id));
            }
        }
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "insert into term_rules(
               concept_id, source_language, target_language, source_text,
               canonical_translation, forbidden_variants_json, status,
               provenance, created_at, updated_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, 'candidate', 'proposed', ?7, ?8)",
            rusqlite::params![
                concept_id,
                self.context.source_language,
                self.context.target_language,
                source_text,
                canonical_translation,
                serde_json::to_string(forbidden_variants)
                    .map_err(|error| TermbaseError::Json(error.to_string()))?,
                now,
                now,
            ],
        )?;
        let rule_id = transaction.last_insert_rowid();
        insert_form(
            &transaction,
            rule_id,
            "source",
            source_text,
            &self.context.source_language,
        )?;
        insert_form(
            &transaction,
            rule_id,
            "approved",
            canonical_translation,
            &self.context.target_language,
        )?;
        for variant in approved_variants {
            insert_form(
                &transaction,
                rule_id,
                "approved",
                variant,
                &self.context.target_language,
            )?;
        }
        for variant in forbidden_variants {
            insert_form(
                &transaction,
                rule_id,
                "forbidden",
                variant,
                &self.context.target_language,
            )?;
        }
        insert_side_values(
            &transaction,
            "term_rule_semantic_tags",
            "tag",
            rule_id,
            &fields.semantic_tags,
        )?;
        insert_side_values(
            &transaction,
            "term_rule_story_scopes",
            "story_scope",
            rule_id,
            &fields.story_scopes,
        )?;
        insert_side_values(
            &transaction,
            "term_rule_language_tags",
            "language_tag",
            rule_id,
            &fields.language_tags,
        )?;
        transaction.commit()?;
        self.get_rule(rule_id)
    }

    /// The audited lifecycle transition: only an explicit approval activates a
    /// rule; passive processes cannot reach `active`.
    pub fn approve(&self, rule_id: i64, actor: &str, reason: &str) -> Result<(), TermbaseError> {
        let now = now_iso8601();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let status: String = transaction
            .query_row(
                "select status from term_rules where id = ?1",
                [rule_id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => TermbaseError::UnknownRule(rule_id),
                other => other.into(),
            })?;
        if status != "candidate" {
            return Err(TermbaseError::Invalid(format!(
                "only candidate rules can be approved; rule {rule_id} is {status}"
            )));
        }
        transaction.execute(
            "update term_rules set status = 'active', updated_at = ?2 where id = ?1",
            rusqlite::params![rule_id, now],
        )?;
        transaction.execute(
            "insert into term_rule_revisions(
               rule_id, actor, reason, prior_status, new_status, created_at
             )
             values (?1, ?2, ?3, 'candidate', 'active', ?4)",
            rusqlite::params![rule_id, actor, reason, now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// The deterministic contract for the raw source text: every active rule
    /// whose source surface occurs (case-insensitively, longest surface per
    /// rule wins).
    pub fn contract(&self, raw_text: &str) -> Result<Vec<ContractTerm>, TermbaseError> {
        let (resolved, _warnings) = self.resolve_active_rules(raw_text)?;
        Ok(resolved
            .into_iter()
            .map(|resolved| contract_term_for_rule(&resolved.rule, &resolved.source_surface))
            .collect())
    }

    /// Validate a translation against the deterministic contract:
    /// `forbidden_variant` (high) and `missing_canonical` (medium) findings,
    /// plus ambiguity warnings computed from the source.
    pub fn validate(
        &self,
        translated_text: &str,
        source: Source,
    ) -> Result<Vec<ValidationFinding>, TermbaseError> {
        let source_text = match source {
            Source::Raw(text) => text,
            Source::SourceText(text) => text,
        };
        let (resolved, mut findings) = self.resolve_active_rules(&source_text)?;
        let terms: Vec<ContractTerm> = resolved
            .into_iter()
            .map(|resolved| contract_term_for_rule(&resolved.rule, &resolved.source_surface))
            .collect();
        for term in terms {
            for forbidden_variant in &term.forbidden_variants {
                if !contains(translated_text, forbidden_variant, true) {
                    continue;
                }
                findings.push(ValidationFinding {
                    term_id: term.id,
                    kind: "forbidden_variant".to_string(),
                    severity: "high".to_string(),
                    expected: term.canonical_translation.clone(),
                    observed: forbidden_variant.clone(),
                    message: format!(
                        "Use {:?} for {:?}; {:?} is forbidden.",
                        term.canonical_translation, term.source_text, forbidden_variant
                    ),
                });
            }
            if !translated_text.contains(&term.canonical_translation) {
                findings.push(ValidationFinding {
                    term_id: term.id,
                    kind: "missing_canonical".to_string(),
                    severity: "medium".to_string(),
                    expected: term.canonical_translation.clone(),
                    observed: String::new(),
                    message: format!(
                        "Raw text contains {:?}, but translation does not contain approved form {:?}.",
                        term.source_text, term.canonical_translation
                    ),
                });
            }
        }
        Ok(findings)
    }

    /// Resolve active rules whose source surface occurs in the raw text.
    /// Multiple active rules for one surface with different concept sets are
    /// ambiguous and surface a warning instead of an enforceable term.
    fn resolve_active_rules(
        &self,
        raw_text: &str,
    ) -> Result<(Vec<ResolvedRule>, Vec<ValidationFinding>), TermbaseError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "select id, concept_id, source_text, canonical_translation,
                    forbidden_variants_json, provenance, revision, rule_crystal_id
             from term_rules
             where status = 'active'
             order by length(source_text) desc, id",
        )?;
        let mut active: Vec<TermRule> = statement
            .query_map([], |row| {
                Ok(TermRuleRow {
                    id: row.get(0)?,
                    concept_id: row.get(1)?,
                    source_text: row.get(2)?,
                    canonical_translation: row.get(3)?,
                    forbidden_json: row.get(4)?,
                    provenance: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    revision: row.get::<_, i64>(6)?,
                    rule_crystal_id: row.get::<_, Option<i64>>(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|row: TermRuleRow| {
                let forbidden_variants: Vec<String> = serde_json::from_str(&row.forbidden_json)
                    .map_err(|error| TermbaseError::Json(error.to_string()))?;
                Ok(TermRule {
                    id: row.id,
                    concept_id: row.concept_id,
                    source_language: self.context.source_language.clone(),
                    target_language: self.context.target_language.clone(),
                    source_text: row.source_text,
                    canonical_translation: row.canonical_translation,
                    forbidden_variants,
                    status: "active".to_string(),
                    provenance: row.provenance,
                    revision: row.revision,
                    rule_crystal_id: row.rule_crystal_id,
                    semantic_tags: rule_side_values(
                        &connection,
                        "term_rule_semantic_tags",
                        "tag",
                        row.id,
                    )?,
                    story_scopes: rule_side_values(
                        &connection,
                        "term_rule_story_scopes",
                        "story_scope",
                        row.id,
                    )?,
                    language_tags: rule_side_values(
                        &connection,
                        "term_rule_language_tags",
                        "language_tag",
                        row.id,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, TermbaseError>>()?;
        drop(statement);
        active.sort_by(|left, right| {
            right
                .source_text
                .len()
                .cmp(&left.source_text.len())
                .then(left.id.cmp(&right.id))
        });

        let mut resolved: Vec<ResolvedRule> = Vec::new();
        let mut warnings: Vec<ValidationFinding> = Vec::new();
        let mut candidates_by_surface: std::collections::BTreeMap<String, Vec<&TermRule>> =
            std::collections::BTreeMap::new();
        for rule in &active {
            if contains(raw_text, &rule.source_text, false) {
                candidates_by_surface
                    .entry(rule.source_text.clone())
                    .or_default()
                    .push(rule);
            }
        }
        // Longer surfaces win: skip a surface fully contained in an already
        // resolved longer surface.
        for (surface, candidates) in candidates_by_surface {
            let covered = resolved.iter().any(|resolved: &ResolvedRule| {
                surface.len() < resolved.source_surface.len()
                    && contains(&resolved.source_surface, &surface, false)
            });
            if covered {
                continue;
            }
            let concept_sets: std::collections::HashSet<Option<i64>> = candidates
                .iter()
                .map(|candidate| candidate.concept_id)
                .collect();
            if concept_sets.len() == 1 {
                let mut renderings: Vec<&str> = candidates
                    .iter()
                    .map(|candidate| candidate.canonical_translation.as_str())
                    .collect();
                renderings.sort();
                renderings.dedup();
                if renderings.len() > 1 {
                    warnings.push(finding_conflicting(&surface, &renderings, &candidates));
                    continue;
                }
                for candidate in candidates {
                    resolved.push(ResolvedRule {
                        rule: candidate.clone(),
                        source_surface: surface.clone(),
                    });
                }
                continue;
            }
            // Different concepts: best context overlap disambiguates; zero or
            // tied overlap stays ambiguous.
            let scored = score_context_overlap(&candidates, &self.context);
            let best = scored.iter().map(|(score, _)| *score).max().unwrap_or(0);
            let _ = &candidates;
            if best <= 0 {
                warnings.push(finding_ambiguous(&surface, &candidates));
                continue;
            }
            let scored_pairs = score_context_overlap(&candidates, &self.context);
            let mut winners: Vec<&TermRule> = scored_pairs
                .iter()
                .filter(|(score, _)| *score == best)
                .map(|(_, candidate)| *candidate)
                .collect();
            winners.sort_by_key(|candidate| candidate.id);
            let winner_sets: std::collections::HashSet<Option<i64>> = winners
                .iter()
                .map(|candidate| candidate.concept_id)
                .collect();
            let mut renderings: Vec<&str> = winners
                .iter()
                .map(|candidate| candidate.canonical_translation.as_str())
                .collect();
            renderings.sort();
            renderings.dedup();
            if winner_sets.len() == 1 && renderings.len() == 1 {
                for candidate in winners {
                    resolved.push(ResolvedRule {
                        rule: candidate.clone(),
                        source_surface: surface.clone(),
                    });
                }
            } else {
                warnings.push(finding_ambiguous(&surface, &candidates));
            }
        }
        Ok((resolved, warnings))
    }

    /// Read one rule with its forms hydrated.
    pub fn get_rule(&self, rule_id: i64) -> Result<TermRule, TermbaseError> {
        let connection = self.connection()?;
        hydrate_rule(&connection, rule_id)
    }
}

/// Which source text to resolve the contract against.
#[derive(Debug, Clone)]
pub enum Source {
    Raw(String),
    SourceText(String),
}

/// Optional context metadata attached to a proposed rule; used to resolve
/// ambiguous source surfaces (best context overlap wins, zero overlap stays
/// ambiguous).
#[derive(Debug, Clone, Default)]
pub struct ProposeFields {
    pub concept_id: Option<i64>,
    pub approved_variants: Vec<String>,
    pub forbidden_variants: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub language_tags: Vec<String>,
}

fn finding_ambiguous(surface: &str, candidates: &[&TermRule]) -> ValidationFinding {
    let mut renderings: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.canonical_translation.as_str())
        .filter(|rendering| !rendering.is_empty())
        .collect();
    renderings.sort();
    renderings.dedup();
    let term_id = candidates
        .iter()
        .map(|candidate| candidate.id)
        .min()
        .unwrap_or(0);
    ValidationFinding {
        term_id,
        kind: "ambiguous_source".to_string(),
        severity: "warning".to_string(),
        expected: renderings.join(", "),
        observed: surface.to_string(),
        message: format!(
            "Source form {surface:?} maps to multiple active rule concepts; add concept, \
             semantic tag, or story scope context before enforcing a rendering."
        ),
    }
}

fn finding_conflicting(
    surface: &str,
    renderings: &[&str],
    candidates: &[&TermRule],
) -> ValidationFinding {
    let term_id = candidates
        .iter()
        .map(|candidate| candidate.id)
        .min()
        .unwrap_or(0);
    ValidationFinding {
        term_id,
        kind: "conflicting_active_rules".to_string(),
        severity: "warning".to_string(),
        expected: renderings.join(", "),
        observed: surface.to_string(),
        message: format!(
            "Source form {surface:?} has conflicting active rules for the same concept; \
             supersede, archive, or consolidate one rule before enforcing a rendering."
        ),
    }
}

fn contract_term_for_rule(rule: &TermRule, source_surface: &str) -> ContractTerm {
    ContractTerm {
        id: rule.rule_crystal_id.unwrap_or(rule.id),
        category: "rule".to_string(),
        source_text: source_surface.to_string(),
        canonical_translation: rule.canonical_translation.clone(),
        forbidden_variants: rule.forbidden_variants.clone(),
        tags: Vec::new(),
        notes: rule_text(
            source_surface,
            &rule.canonical_translation,
            &rule.forbidden_variants,
        ),
    }
}

fn hydrate_rule(connection: &Connection, rule_id: i64) -> Result<TermRule, TermbaseError> {
    let (
        concept_id,
        source_language,
        target_language,
        source_text,
        canonical_translation,
        forbidden_json,
        status,
        provenance,
        revision,
        rule_crystal_id,
    ) = connection
        .query_row(
            "select concept_id, source_language, target_language, source_text,
                    canonical_translation, forbidden_variants_json, status,
                    provenance, revision, rule_crystal_id
             from term_rules where id = ?1",
            [rule_id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                ))
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => TermbaseError::UnknownRule(rule_id),
            other => other.into(),
        })?;
    let forbidden_variants: Vec<String> = serde_json::from_str(&forbidden_json)
        .map_err(|error| TermbaseError::Json(error.to_string()))?;
    let rule = TermRule {
        id: rule_id,
        concept_id,
        source_language,
        target_language,
        source_text,
        canonical_translation,
        forbidden_variants,
        status,
        provenance: provenance.unwrap_or_default(),
        revision,
        rule_crystal_id,
        semantic_tags: rule_side_values(connection, "term_rule_semantic_tags", "tag", rule_id)?,
        story_scopes: rule_side_values(
            connection,
            "term_rule_story_scopes",
            "story_scope",
            rule_id,
        )?,
        language_tags: rule_side_values(
            connection,
            "term_rule_language_tags",
            "language_tag",
            rule_id,
        )?,
    };
    Ok(rule)
}

fn rule_side_values(
    connection: &Connection,
    table: &str,
    column: &str,
    rule_id: i64,
) -> Result<Vec<String>, TermbaseError> {
    let mut statement = connection.prepare(&format!(
        "select {column} from {table} where rule_id = ?1 order by {column}"
    ))?;
    let rows = statement.query_map([rule_id], |row| row.get::<_, String>(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn insert_side_values(
    transaction: &rusqlite::Transaction<'_>,
    table: &str,
    column: &str,
    rule_id: i64,
    values: &[String],
) -> Result<(), rusqlite::Error> {
    for value in values {
        transaction.execute(
            &format!("insert or ignore into {table}(rule_id, {column}) values (?1, ?2)"),
            rusqlite::params![rule_id, value],
        )?;
    }
    Ok(())
}

fn insert_form(
    transaction: &rusqlite::Transaction<'_>,
    rule_id: i64,
    form_kind: &str,
    surface: &str,
    language: &str,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "insert into term_rule_forms(rule_id, form_kind, surface, language)
         values (?1, ?2, ?3, ?4)",
        rusqlite::params![rule_id, form_kind, surface, language],
    )?;
    Ok(())
}
