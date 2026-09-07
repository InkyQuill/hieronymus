//! Deterministic terminology authority (ADR 0011): `term_rules`/
//! `term_rule_forms` own the enforceable rendering contract; fuzzy recall and
//! generic Dream mutations can only propose candidates. Validated learned and
//! explicit decisions use the authority module and the shared lifecycle transaction.

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
    #[error(
        "revision conflict for rule {rule_id}: expected revision {expected_revision}, current revision {current_revision}"
    )]
    RevisionConflict {
        rule_id: i64,
        expected_revision: i64,
        current_revision: i64,
    },
    #[error("idempotency key {0:?} was already used with a different request")]
    IdempotencyConflict(String),
    #[error("rule crystal {0} is linked to more than one term_rules row")]
    AmbiguousCrystalLink(i64),
    #[error(
        "rule crystal {0} has no linked term_rules authority row; archive the authority through the explicit rule lifecycle instead of the projection alone"
    )]
    UnlinkedCrystalLink(i64),
    #[error("rule-crystal projection {crystal_id} failed: {source}")]
    ProjectionFailure {
        crystal_id: i64,
        #[source]
        source: crate::crystals::CrystalError,
    },
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(String),
    #[error(transparent)]
    Open(#[from] crate::db::OpenMigratedError),
}

/// One enforceable rule in the translation contract. The serde projection is
/// the transport DTO for the recall response's `deterministic_contract`
/// section (ADR 0011); the field names are the frozen snake_case payload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContractTerm {
    pub id: i64,
    pub category: String,
    pub source_text: String,
    pub canonical_translation: String,
    pub forbidden_variants: Vec<String>,
    pub tags: Vec<String>,
    pub notes: String,
}

/// One validation finding; the serde projection is the transport DTO for the
/// `hieronymus_termbase_validate` payload (the Python `asdict` shape).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ValidationFinding {
    pub term_id: i64,
    pub kind: String,
    pub severity: String,
    pub expected: String,
    pub observed: String,
    pub message: String,
}

/// A stored rule with its forms hydrated. The serde projection is a safe DTO
/// for tool payloads and for the audited lifecycle's stored results
/// (`term_rule_actions.result_json`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// One explicit lifecycle transition over one structured rule (ADR 0011).
/// Only these actions may move a rule between statuses, and only
/// [`Termbase::apply_action`] executes them — audited, revision-checked, and
/// idempotent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    Approve,
    Archive,
    Replace { replacement_id: i64 },
}

impl RuleAction {
    /// The stable audit name recorded in `term_rule_actions.action`.
    pub fn name(&self) -> &'static str {
        match self {
            RuleAction::Approve => "approve",
            RuleAction::Archive => "archive",
            RuleAction::Replace { .. } => "replace",
        }
    }
}

/// One audited lifecycle request. `expected_revision` is the optimistic
/// concurrency token (the caller's observed `TermRule::revision`);
/// `idempotency_key` deduplicates retries: the same key with the same
/// canonical request replays its stored result, the same key with a
/// different request is a conflict. The transport layer supplies `actor`
/// from the authenticated credential — it is never accepted from raw tool
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleActionRequest {
    pub rule_id: i64,
    pub action: RuleAction,
    pub actor: String,
    pub reason: String,
    pub expected_revision: i64,
    pub idempotency_key: String,
}

/// The canonical serialization of a [`RuleActionRequest`] stored in
/// `term_rule_actions.request_canonical`. serde_json maps are key-sorted, so
/// equal requests always serialize identically; the idempotency key itself is
/// the lookup key and is not repeated in the payload.
fn canonical_request(request: &RuleActionRequest) -> Result<String, TermbaseError> {
    let action = match &request.action {
        RuleAction::Approve => serde_json::json!({ "kind": "approve" }),
        RuleAction::Archive => serde_json::json!({ "kind": "archive" }),
        RuleAction::Replace { replacement_id } => serde_json::json!({
            "kind": "replace",
            "replacement_id": replacement_id,
        }),
    };
    let payload = serde_json::json!({
        "action": action,
        "actor": request.actor,
        "expected_revision": request.expected_revision,
        "reason": request.reason,
        "rule_id": request.rule_id,
    });
    serde_json::to_string(&payload).map_err(|error| TermbaseError::Json(error.to_string()))
}

/// The deterministic termbase over one translation context.
pub struct Termbase {
    config: HieronymusConfig,
    context: crate::memory_models::TranslationContext,
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339()
}

/// Defensive `if not exists` ensure for the audited lifecycle ledger
/// (`term_rule_actions`), following the code-side pattern of
/// `semantic_store::ensure_semantic_schema`/`SEMANTIC_SCHEMA_SQL`.
///
/// The canonical definition of this table is now the runtime plan's schema-v2
/// migration (`migrations/002-durable-work.sql`): every database opened
/// through `db::open_migrated` already carries it, so this ensure is a
/// deliberate no-op there. The column set, the `rule_id` foreign key, and the
/// partial unique index on `idempotency_key` are kept byte-consistent with
/// that migration so the two never diverge; [`Termbase::apply_action`] writes
/// `expected_revision` / `request_canonical` and reads back by
/// `idempotency_key`.
const TERM_RULE_ACTIONS_SCHEMA_SQL: &str = "
create table if not exists term_rule_actions (
  id integer primary key,
  idempotency_key text not null default '',
  rule_id integer not null references term_rules(id),
  actor text not null,
  reason text not null,
  action text not null,
  expected_revision integer not null,
  resulting_revision integer not null,
  request_canonical text not null,
  result_json text not null,
  created_at text not null
);
create unique index if not exists term_rule_actions_idempotency_key_idx
  on term_rule_actions(idempotency_key) where idempotency_key <> '';
create index if not exists term_rule_actions_rule_idx
  on term_rule_actions(rule_id, id);
";

fn ensure_term_rule_actions_schema(connection: &Connection) -> Result<(), TermbaseError> {
    connection.execute_batch(TERM_RULE_ACTIONS_SCHEMA_SQL)?;
    Ok(())
}

/// The canonical rule sentence: also the round-trip shape enforced on
/// propose (`<source> is translated as <target>[, not <forbidden>].`).
/// The upgrade converter builds the same sentence when linking legacy rule
/// crystals, so the format lives in exactly one place.
pub(crate) fn rule_text(
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

pub(crate) fn validate_rule_shape(
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

/// Deterministic re-validation of one stored rule's structured forms before
/// an activation transition (ADR 0011: a lifecycle action "succeeds only
/// after deterministic validation of the structured rule"). Runs inside the
/// caller's transaction.
fn validate_structured_rule(connection: &Connection, rule_id: i64) -> Result<(), TermbaseError> {
    let (source_text, canonical_translation, forbidden_json): (String, String, String) = connection
        .query_row(
            "select source_text, canonical_translation, forbidden_variants_json
                 from term_rules where id = ?1",
            [rule_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => TermbaseError::UnknownRule(rule_id),
            other => other.into(),
        })?;
    let forbidden_variants: Vec<String> = serde_json::from_str(&forbidden_json)
        .map_err(|error| TermbaseError::Json(error.to_string()))?;
    validate_rule_shape(
        &source_text,
        &canonical_translation,
        &[],
        &forbidden_variants,
    )
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
               provenance, notes, created_at, updated_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, 'candidate', 'proposed', ?7, ?8, ?9)",
            rusqlite::params![
                concept_id,
                self.context.source_language,
                self.context.target_language,
                source_text,
                canonical_translation,
                serde_json::to_string(forbidden_variants)
                    .map_err(|error| TermbaseError::Json(error.to_string()))?,
                fields.notes,
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

    /// The legacy approval entry (compatibility wrapper): an explicit
    /// candidate→active transition delegated to [`Termbase::apply_action`]
    /// with a deterministic internal idempotency key derived from the
    /// action, the rule, and the currently pending transition, so legacy
    /// retries can never double-apply. The transition itself stays strict:
    /// approving a rule that is no longer a candidate is a rejection (a
    /// retry after a committed approval reports the transition error instead
    /// of silently re-approving).
    pub fn approve(&self, rule_id: i64, actor: &str, reason: &str) -> Result<(), TermbaseError> {
        let rule = self.get_rule(rule_id)?;
        let request = RuleActionRequest {
            rule_id,
            action: RuleAction::Approve,
            actor: actor.to_string(),
            reason: reason.to_string(),
            expected_revision: rule.revision,
            idempotency_key: format!("legacy-approve:{rule_id}:{}@{}", rule.status, rule.revision),
        };
        self.apply_action(&request)?;
        Ok(())
    }

    /// Trusted local compatibility lifecycle. Decision-owned rules require the
    /// authority module; ordinary transports must use verified ingress. One SQLite transaction
    ///
    /// 1. replays the stored result when the idempotency key matches the
    ///    canonical request (retries are safe), or rejects a key reuse with a
    ///    different request;
    /// 2. checks `expected_revision` against the stored revision (stale
    ///    callers get [`TermbaseError::RevisionConflict`]);
    /// 3. re-validates the structured forms of every rule the action
    ///    activates (deterministic round-trip, never a guess);
    /// 4. mutates `term_rules`/`term_rule_revisions` and, when a projection
    ///    is linked, the rule-crystal projection IN THE SAME transaction;
    /// 5. records the `term_rule_actions` row (actor, reason, expected and
    ///    resulting revision, canonical request, resulting state) inside the
    ///    same commit — a failure anywhere rolls the whole action back.
    ///
    /// The returned rule is the resulting authority: the target rule for
    /// approve/archive, the activated replacement for
    /// [`RuleAction::Replace`]. The `actor` must be the authenticated
    /// local caller. This legacy string does not prove user authorship; transport
    /// adapters must route autonomous decisions through the authority module.
    pub fn apply_action(&self, request: &RuleActionRequest) -> Result<TermRule, TermbaseError> {
        if request.actor.trim().is_empty() {
            return Err(TermbaseError::Invalid(
                "actor must not be empty: lifecycle transitions require an authenticated actor"
                    .to_string(),
            ));
        }
        if request.idempotency_key.trim().is_empty() {
            return Err(TermbaseError::Invalid(
                "idempotency_key must not be empty".to_string(),
            ));
        }
        let mut connection = self.connection()?;
        ensure_term_rule_actions_schema(&connection)?;
        let transaction = connection.transaction()?;

        let result = apply_lifecycle_tx(&transaction, request, false)?;
        transaction.commit()?;
        Ok(result)
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
        let connection = self.connection()?;
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
            let mut approved=connection.prepare("select surface,case_sensitive from term_rule_forms where rule_id=?1 and form_kind='approved'")?;
            let approved = approved
                .query_map([term.id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if !translated_text.contains(&term.canonical_translation)
                && !approved
                    .iter()
                    .any(|(form, case)| contains(translated_text, form, *case))
            {
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
        let mut matching = Vec::new();
        for rule in active {
            if !crate::authority_applicability::rule_identity_in_context(
                &connection,
                rule.id,
                &self.context,
            )
            .map_err(|error| TermbaseError::Invalid(error.to_string()))?
            {
                continue;
            }
            let mut forms=connection.prepare("select surface,case_sensitive from term_rule_forms where rule_id=?1 and form_kind='source' order by length(surface) desc,id")?;
            let forms = forms
                .query_map([rule.id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if let Some((surface, _)) = forms
                .iter()
                .find(|(surface, case)| contains(raw_text, surface, *case))
            {
                let mut matched = rule;
                matched.source_text = surface.clone();
                matching.push(matched);
            } else if forms.is_empty() {
                matching.push(rule);
            }
        }
        active = matching;
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
                // Identity is established before authority/applicability removes
                // historical renderings; outside identities were retained above.
                let candidates = current_rule_candidates(&connection, &self.context, candidates)?;
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
            if winner_sets.len() != 1 {
                warnings.push(finding_ambiguous(&surface, &candidates));
                continue;
            }
            let identity = winners[0].concept_id;
            // Context chooses an identity, not a historical rendering row.
            winners = current_rule_candidates(
                &connection,
                &self.context,
                candidates
                    .iter()
                    .copied()
                    .filter(|candidate| candidate.concept_id == identity)
                    .collect(),
            )?;
            let mut renderings: Vec<&str> = winners
                .iter()
                .map(|candidate| candidate.canonical_translation.as_str())
                .collect();
            renderings.sort();
            renderings.dedup();
            if winner_sets.len() == 1 && renderings.len() <= 1 {
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

    /// Resolve the single authority rule a rule-crystal projection is linked
    /// to (`term_rules.rule_crystal_id`). ADR 0011: the projection is
    /// derived, so lifecycle operations against a crystal must resolve the
    /// authority exactly — an unknown link and an ambiguous link (multiple
    /// rules claiming one crystal) both fail instead of guessing.
    pub fn rule_id_for_crystal(
        config: &HieronymusConfig,
        crystal_id: i64,
    ) -> Result<Option<i64>, TermbaseError> {
        let connection = open_migrated(Path::new(&config.database_path()))?;
        let ids: Vec<i64> = {
            let mut statement = connection
                .prepare("select id from term_rules where rule_crystal_id = ?1 order by id")?;
            let rows = statement.query_map([crystal_id], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        match ids.as_slice() {
            [] => Ok(None),
            [rule_id] => Ok(Some(*rule_id)),
            _ => Err(TermbaseError::AmbiguousCrystalLink(crystal_id)),
        }
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
/// ambiguous). `notes` lands in `term_rules.notes` (legacy
/// `hieronymus_termbase_propose` notes argument).
#[derive(Debug, Clone, Default)]
pub struct ProposeFields {
    pub concept_id: Option<i64>,
    pub approved_variants: Vec<String>,
    pub forbidden_variants: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub language_tags: Vec<String>,
    pub notes: String,
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

pub(crate) fn hydrate_rule(
    connection: &Connection,
    rule_id: i64,
) -> Result<TermRule, TermbaseError> {
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

/// Shared lifecycle and projection write. Caller owns transaction and authority validation.
pub(crate) fn apply_lifecycle_tx(
    transaction: &rusqlite::Transaction<'_>,
    request: &RuleActionRequest,
    validated_authority: bool,
) -> Result<TermRule, TermbaseError> {
    let canonical = canonical_request(request)?;
    let now = now_iso8601();
    // Idempotency first: the lookup key is the idempotency key.
    let existing: Option<(String, String)> = transaction
        .query_row(
            "select request_canonical, result_json from term_rule_actions
                 where idempotency_key = ?1",
            [&request.idempotency_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map(Some)
        .or_else(|error: rusqlite::Error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(TermbaseError::Database(other)),
        })?;
    if let Some((stored_canonical, stored_result)) = existing {
        if stored_canonical == canonical {
            // Same key, same request: replay the stored result instead
            // of re-executing the transition.
            return serde_json::from_str(&stored_result)
                .map_err(|error| TermbaseError::Json(error.to_string()));
        }
        return Err(TermbaseError::IdempotencyConflict(
            request.idempotency_key.clone(),
        ));
    }

    let owned: bool = transaction.query_row("select exists(select 1 from rule_authority where rule_id=?1 and (decision_id is not null or consolidation_result_id is not null))", [request.rule_id], |r|r.get(0))?;
    if owned && !validated_authority {
        return Err(TermbaseError::Invalid(
            "validated authority decision required".into(),
        ));
    }
    let (status, revision, rule_crystal_id): (String, i64, Option<i64>) = transaction
        .query_row(
            "select status, revision, rule_crystal_id from term_rules where id = ?1",
            [request.rule_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => TermbaseError::UnknownRule(request.rule_id),
            other => other.into(),
        })?;
    if revision != request.expected_revision {
        return Err(TermbaseError::RevisionConflict {
            rule_id: request.rule_id,
            expected_revision: request.expected_revision,
            current_revision: revision,
        });
    }
    let resulting_revision = revision + 1;

    let result_rule_id = match &request.action {
        RuleAction::Approve => {
            if status != "candidate" {
                return Err(TermbaseError::Invalid(format!(
                    "only candidate rules can be approved; rule {rule_id} is {status}",
                    rule_id = request.rule_id
                )));
            }
            if !validated_authority {
                validate_structured_rule(transaction, request.rule_id)?;
            }
            transaction.execute(
                "update term_rules set status = 'active', revision = ?2, updated_at = ?3
                     where id = ?1",
                rusqlite::params![request.rule_id, resulting_revision, now],
            )?;
            transaction.execute(
                "insert into term_rule_revisions(
                       rule_id, actor, reason, prior_status, new_status, created_at
                     )
                     values (?1, ?2, ?3, 'candidate', 'active', ?4)",
                rusqlite::params![request.rule_id, request.actor, request.reason, now],
            )?;
            request.rule_id
        }
        RuleAction::Archive => {
            if !matches!(status.as_str(), "candidate" | "active") {
                return Err(TermbaseError::Invalid(format!(
                    "only candidate or active rules can be archived; rule {rule_id} is {status}",
                    rule_id = request.rule_id
                )));
            }
            transaction.execute(
                "update term_rules set status = 'archived', revision = ?2, updated_at = ?3
                     where id = ?1",
                rusqlite::params![request.rule_id, resulting_revision, now],
            )?;
            transaction.execute(
                "insert into term_rule_revisions(
                       rule_id, actor, reason, prior_status, new_status, created_at
                     )
                     values (?1, ?2, ?3, ?4, 'archived', ?5)",
                rusqlite::params![request.rule_id, request.actor, request.reason, status, now],
            )?;
            if let Some(crystal_id) = rule_crystal_id {
                // The projection is archived in the SAME transaction as
                // the authority: an archive never leaves an active
                // advisory projection behind, and a projection failure
                // rolls the authority back.
                crate::crystals::archive_rule_crystal_in_transaction(transaction, crystal_id, &now)
                    .map_err(|source| TermbaseError::ProjectionFailure { crystal_id, source })?;
            }
            request.rule_id
        }
        RuleAction::Replace { replacement_id } => {
            if *replacement_id == request.rule_id {
                return Err(TermbaseError::Invalid(
                    "a rule cannot replace itself".to_string(),
                ));
            }
            if status != "active" {
                return Err(TermbaseError::Invalid(format!(
                    "only active rules can be replaced; rule {rule_id} is {status}",
                    rule_id = request.rule_id
                )));
            }
            let replacement = hydrate_rule(transaction, *replacement_id)?;
            if replacement.status != "candidate" {
                return Err(TermbaseError::Invalid(format!(
                    "only candidate rules can activate as replacements; rule {replacement_id} is {status}",
                    replacement_id = replacement.id,
                    status = replacement.status,
                )));
            }
            if replacement.rule_crystal_id.is_some() {
                return Err(TermbaseError::Invalid(format!(
                    "replacement rule {replacement_id} already links a rule-crystal projection; \
                         the replaced rule's projection cannot move there without ambiguity",
                    replacement_id = replacement.id,
                )));
            }
            if !validated_authority {
                validate_structured_rule(transaction, replacement.id)?;
            }
            // Supersede the replaced rule (revision-checked above) and
            // release its projection link: the link moves to the
            // replacement below, so exactly one authority row ever
            // claims the crystal.
            transaction.execute(
                "update term_rules set status = 'superseded', revision = ?2,
                     rule_crystal_id = null, updated_at = ?3 where id = ?1",
                rusqlite::params![request.rule_id, resulting_revision, now],
            )?;
            transaction.execute(
                "insert into term_rule_revisions(
                       rule_id, actor, reason, prior_status, new_status, created_at
                     )
                     values (?1, ?2, ?3, 'active', 'superseded', ?4)",
                rusqlite::params![request.rule_id, request.actor, request.reason, now],
            )?;
            // Activate the replacement; when the replaced rule owned a
            // crystal projection, the link and the derived crystal text
            // move to the replacement in the same transaction — the
            // projection stays the derived rendering of the authority.
            let replacement_revision = replacement.revision + 1;
            match rule_crystal_id {
                Some(crystal_id) => {
                    let sentence = rule_text(
                        &replacement.source_text,
                        &replacement.canonical_translation,
                        &replacement.forbidden_variants,
                    );
                    transaction.execute(
                        "update term_rules set status = 'active', revision = ?2,
                             rule_crystal_id = ?3, updated_at = ?4 where id = ?1",
                        rusqlite::params![replacement.id, replacement_revision, crystal_id, now],
                    )?;
                    transaction.execute(
                        "update crystals_fts set text = ?2 where rowid = ?1",
                        rusqlite::params![crystal_id, sentence],
                    )?;
                    transaction.execute(
                        "update crystals set text = ?2, updated_at = ?3 where id = ?1",
                        rusqlite::params![crystal_id, sentence, now],
                    )?;
                }
                None => {
                    transaction.execute(
                        "update term_rules set status = 'active', revision = ?2,
                             updated_at = ?3 where id = ?1",
                        rusqlite::params![replacement.id, replacement_revision, now],
                    )?;
                }
            }
            transaction.execute(
                "insert into term_rule_revisions(
                       rule_id, actor, reason, prior_status, new_status, created_at
                     )
                     values (?1, ?2, ?3, 'candidate', 'active', ?4)",
                rusqlite::params![replacement.id, request.actor, request.reason, now],
            )?;
            replacement.id
        }
    };

    let result = hydrate_rule(transaction, result_rule_id)?;
    if validated_authority
        && result.status == "active"
        && let Some(crystal_id) = result.rule_crystal_id
    {
        let projection = structured_projection(transaction, result.id)?;
        transaction.execute(
            "update crystals_fts set text=?2 where rowid=?1",
            rusqlite::params![crystal_id, projection],
        )?;
        transaction.execute(
            "update crystals set text=?2,updated_at=?3 where id=?1",
            rusqlite::params![crystal_id, projection, now],
        )?;
    }

    transaction.execute(
        "insert into term_rule_actions(
               rule_id, action, actor, reason, expected_revision, resulting_revision,
               idempotency_key, request_canonical, result_json, created_at
             )
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            request.rule_id,
            request.action.name(),
            request.actor,
            request.reason,
            request.expected_revision,
            resulting_revision,
            request.idempotency_key,
            canonical,
            serde_json::to_string(&result)
                .map_err(|error| TermbaseError::Json(error.to_string()))?,
            now,
        ],
    )?;
    Ok(result)
}

/// Lossless derived projection for validated authority. Structured tables remain
/// the sole rule truth; this JSON text is regenerated inside lifecycle writes.
fn structured_projection(db: &Connection, id: i64) -> Result<String, TermbaseError> {
    let rule = hydrate_rule(db, id)?;
    let mut s=db.prepare("select form_kind,surface,language,case_sensitive from term_rule_forms where rule_id=? order by id")?;
    let forms=s.query_map([id],|r|Ok(serde_json::json!({"kind":r.get::<_,String>(0)?,"surface":r.get::<_,String>(1)?,"language":r.get::<_,String>(2)?,"case_sensitive":r.get::<_,bool>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(serde_json::json!({"version":1,"rule_id":id,"concept_id":rule.concept_id,"canonical":rule.canonical_translation,"forms":forms}).to_string())
}

/// Call only once the source occurrence has one resolved concept identity.
fn current_rule_candidates<'a>(
    db: &Connection,
    context: &crate::memory_models::TranslationContext,
    candidates: Vec<&'a TermRule>,
) -> Result<Vec<&'a TermRule>, TermbaseError> {
    let mut current = Vec::new();
    for rule in candidates {
        if crate::authority_applicability::rule_is_current(db, rule.id, context)
            .map_err(|error| TermbaseError::Invalid(error.to_string()))?
        {
            current.push(rule);
        }
    }
    Ok(current)
}
