//! The term/rule-crystal tool family (plan M3): `hieronymus_termbase_propose`,
//! `hieronymus_termbase_approve`, `hieronymus_termbase_contract`,
//! `hieronymus_termbase_validate`, `hieronymus_rule_crystal_archive`,
//! `hieronymus_rule_crystal_validate`, and `hieronymus_rule_crystals_list`.
//!
//! Argument structs decode exactly the frozen `input_schema` rules from
//! `compatibility/snapshots/mcp.json` (required fields, defaults, and null
//! rules); decoding failures are [`AppError::Invalid`]. Store rejections are
//! [`AppError::Domain`]. All database work stays in the `hieronymus` store
//! APIs — no SQL lives here.
//!
//! ADR 0011 discipline, end to end:
//!
//! - A proposed term is a `candidate`: advisory, never enforced by
//!   `hieronymus_termbase_contract`, until an explicit approval runs
//!   through the audited lifecycle ([`Termbase::apply_action`]).
//! - The actor is ALWAYS the transport-authenticated credential holder
//!   passed to the dispatcher. The legacy wrappers carry no actor argument
//!   and the argument structs have no actor field, so an override cannot be
//!   decoded even in principle. Calling the explicitly named approval or
//!   archive operation IS the explicit user operation under the
//!   local-owner credential policy.
//! - `hieronymus_rule_crystal_archive` resolves the structured authority
//!   through `term_rules.rule_crystal_id` and archives authority AND
//!   projection in one transaction via [`Termbase::apply_action`]. Unknown
//!   or ambiguous links fail — the projection is never archived alone.
//! - `hieronymus_rule_crystal_validate` and `hieronymus_rule_crystals_list`
//!   are read-only advisory views over the projection.
//! - Where the Python wrapper shapes differ from the structured authority
//!   the public tool shapes are preserved: `hieronymus_termbase_propose`
//!   keeps the Python `term_id` key (plus the structured rule fields the
//!   Rust authority owns), `hieronymus_termbase_approve` keeps
//!   `{term_id, approved}`, and the legacy `category`/`tags`/`notes`
//!   arguments stay accepted (`category` has no structured column — the
//!   contract renders category `rule`; `tags` land in the structured
//!   semantic tags; `notes` lands in `term_rules.notes`).

use serde::Deserialize;
use serde_json::{Value, json};

use hieronymus::crystals::CrystalStore;
use hieronymus::memory_models::{CrystalRecord, TranslationContext};
use hieronymus::terminology::{
    ProposeFields, RuleAction, RuleActionRequest, Source, TermRule, Termbase, TermbaseError,
};

use super::AppError;
use super::Application;
use super::decode;
use super::domain;
use super::memory::series_context;
use super::translation_context;

/// The family dispatcher: `None` means the tool is not ours.
pub(crate) fn dispatch(
    application: &Application,
    tool: &str,
    arguments: &Value,
    actor: &str,
) -> Option<Result<Value, AppError>> {
    match tool {
        "hieronymus_termbase_propose" => Some(termbase_propose(application, arguments, actor)),
        "hieronymus_termbase_approve" => Some(termbase_approve(application, arguments, actor)),
        "hieronymus_termbase_contract" => Some(termbase_contract(application, arguments)),
        "hieronymus_termbase_validate" => Some(termbase_validate(application, arguments)),
        "hieronymus_rule_crystal_archive" => {
            Some(rule_crystal_archive(application, arguments, actor))
        }
        "hieronymus_rule_crystal_validate" => Some(rule_crystal_validate(application, arguments)),
        "hieronymus_rule_crystals_list" => Some(rule_crystals_list(application, arguments)),
        _ => None,
    }
}

fn termbase(
    application: &Application,
    series_slug: &str,
    source_language: Option<String>,
    target_language: Option<String>,
    volume: &str,
    chapter: &str,
) -> Result<Termbase, AppError> {
    let series = series_context(application, series_slug)?;
    let context = translation_context(
        &series,
        source_language,
        target_language,
        "translation",
        volume,
        chapter,
    )?;
    Termbase::open(application.config(), &context).map_err(domain)
}

/// The structured rule as a tool payload: the safe DTO projection plus the
/// legacy `term_id` key the Python wrapper returned.
fn rule_payload(rule: &TermRule) -> Value {
    json!({
        "id": rule.id,
        "term_id": rule.id,
        "concept_id": rule.concept_id,
        "source_language": rule.source_language,
        "target_language": rule.target_language,
        "source_text": rule.source_text,
        "canonical_translation": rule.canonical_translation,
        "forbidden_variants": rule.forbidden_variants,
        "status": rule.status,
        "provenance": rule.provenance,
        "revision": rule.revision,
        "rule_crystal_id": rule.rule_crystal_id,
        "semantic_tags": rule.semantic_tags,
        "story_scopes": rule.story_scopes,
        "language_tags": rule.language_tags,
    })
}

/// The Python `_crystal_payload` projection (never a serialized store type).
fn crystal_payload(crystal: &CrystalRecord) -> Value {
    json!({
        "claim_annotation": crystal.claim_annotation,
        "id": crystal.id,
        "crystal_type": crystal.crystal_type,
        "text": crystal.text,
        "title": crystal.title,
        "confidence": crystal.confidence,
        "strength": crystal.strength,
        "status": crystal.status,
        "source_credibility": crystal.source_credibility,
        "rule_intent": crystal.rule_intent,
        "story_scopes": crystal.story_scopes,
        "semantic_tags": crystal.semantic_tags,
        "concept_ids": crystal.concept_ids,
    })
}

// --------------------------------------------------- hieronymus_termbase_propose

#[derive(Deserialize)]
#[allow(dead_code)] // `category` is required by the frozen schema but has no structured column (documented below).
struct TermbasePropose {
    series_slug: String,
    category: String,
    source_text: String,
    canonical_translation: String,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    volume: String,
    #[serde(default)]
    chapter: String,
}

/// Propose a candidate term: deterministic shape validation at the store,
/// but the rule stays advisory (`candidate`) until an explicit approval.
fn termbase_propose(
    application: &Application,
    arguments: &Value,
    _actor: &str,
) -> Result<Value, AppError> {
    let args = decode::<TermbasePropose>(arguments)?;
    if args.source_text.trim().is_empty() {
        return Err(AppError::Domain(
            "source_text must not be empty".to_string(),
        ));
    }
    if args.canonical_translation.trim().is_empty() {
        return Err(AppError::Domain(
            "canonical_translation must not be empty".to_string(),
        ));
    }
    let termbase = termbase(
        application,
        &args.series_slug,
        args.source_language,
        args.target_language,
        &args.volume,
        &args.chapter,
    )?;
    let fields = ProposeFields {
        // The legacy tags argument is the structured semantic-tag set (used
        // for ambiguity resolution); the legacy category argument has no
        // structured column — the authority renders category `rule`.
        semantic_tags: args.tags.unwrap_or_default(),
        notes: args.notes,
        ..ProposeFields::default()
    };
    let rule = termbase
        .propose(&args.source_text, &args.canonical_translation, &fields)
        .map_err(domain)?;
    Ok(rule_payload(&rule))
}

// --------------------------------------------------- hieronymus_termbase_approve

#[derive(Deserialize)]
struct TermbaseApprove {
    series_slug: String,
    term_id: i64,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    volume: String,
    #[serde(default)]
    chapter: String,
}

/// The explicit approval operation: the authenticated transport actor runs
/// the audited candidate→active transition (legacy arguments preserved; the
/// idempotency key and expected revision are derived internally from the
/// currently pending transition).
fn termbase_approve(
    application: &Application,
    arguments: &Value,
    actor: &str,
) -> Result<Value, AppError> {
    let args = decode::<TermbaseApprove>(arguments)?;
    let termbase = termbase(
        application,
        &args.series_slug,
        args.source_language,
        args.target_language,
        &args.volume,
        &args.chapter,
    )?;
    termbase
        .approve(
            args.term_id,
            actor,
            "approved via hieronymus_termbase_approve",
        )
        .map_err(domain)?;
    Ok(json!({"term_id": args.term_id, "approved": true}))
}

// -------------------------------------------------- hieronymus_termbase_contract

#[derive(Deserialize)]
struct TermbaseContract {
    #[serde(flatten)]
    story: super::StoryReadArgs,
    series_slug: String,
    raw_text: String,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    volume: String,
    #[serde(default)]
    chapter: String,
}

/// The deterministic contract for the raw source text: only ACTIVE rules
/// surface, before any advisory evidence.
fn termbase_contract(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<TermbaseContract>(arguments)?;
    let termbase = read_termbase(
        application,
        &args.series_slug,
        args.source_language,
        args.target_language,
        &args.volume,
        &args.chapter,
        &args.story,
    )?;
    let observed = termbase
        .contract_observed(&args.raw_text, args.story.required_decision_id.as_deref())
        .map_err(super::term_read_error)?;
    Ok(json!({"resulting_revision":observed.resulting_revision,"results":observed.value}))
}

// -------------------------------------------------- hieronymus_termbase_validate

#[derive(Deserialize)]
struct TermbaseValidate {
    #[serde(flatten)]
    story: super::StoryReadArgs,
    series_slug: String,
    raw_text: String,
    translated_text: String,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    volume: String,
    #[serde(default)]
    chapter: String,
}

/// Validate a translation against the deterministic contract: the findings
/// are computed from the active rules (and their ambiguity warnings) BEFORE
/// any advisory input — never from ranked recall results.
fn termbase_validate(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<TermbaseValidate>(arguments)?;
    let termbase = read_termbase(
        application,
        &args.series_slug,
        args.source_language,
        args.target_language,
        &args.volume,
        &args.chapter,
        &args.story,
    )?;
    let observed = termbase
        .validate_observed(
            &args.translated_text,
            Source::Raw(args.raw_text),
            args.story.required_decision_id.as_deref(),
        )
        .map_err(super::term_read_error)?;
    Ok(json!({"resulting_revision":observed.resulting_revision,"results":observed.value}))
}

// ---------------------------------------------- hieronymus_rule_crystal_archive

#[derive(Deserialize)]
struct RuleCrystalArchive {
    crystal_id: i64,
}

/// Archive a rule crystal THROUGH the structured authority: resolve the
/// linked `term_rules` row, then run the audited archive action so authority
/// and projection transition in one transaction. Unknown or ambiguous links
/// fail — a projection is never archived alone (that would leave the
/// authority active and enforced while its advisory rendering disappears).
fn rule_crystal_archive(
    application: &Application,
    arguments: &Value,
    actor: &str,
) -> Result<Value, AppError> {
    let args = decode::<RuleCrystalArchive>(arguments)?;
    let config = application.config();
    let crystals = CrystalStore::open(config).map_err(domain)?;
    let crystal = crystals.get(args.crystal_id).map_err(domain)?;
    if crystal.crystal_type != "rule" {
        return Err(AppError::Domain(
            "crystal is not a rule crystal".to_string(),
        ));
    }
    let rule_id = match Termbase::rule_id_for_crystal(config, args.crystal_id).map_err(domain)? {
        Some(rule_id) => rule_id,
        // Unknown or ambiguous links fail — never guess.
        None => return Err(domain(TermbaseError::UnlinkedCrystalLink(args.crystal_id))),
    };
    let context = TranslationContext::new(
        crystal.series_slug.as_str(),
        crystal.source_language.as_str(),
        crystal.target_language.as_str(),
        "translation",
    );
    let termbase = Termbase::open(config, &context).map_err(domain)?;
    let rule = termbase.get_rule(rule_id).map_err(domain)?;
    if rule.status == "archived" && crystal.status == "archived" {
        // Safe retry: this exact transition already committed.
        return Ok(crystal_payload(&crystal));
    }
    termbase
        .apply_action(&RuleActionRequest {
            rule_id,
            action: RuleAction::Archive,
            actor: actor.to_string(),
            reason: "archived via hieronymus_rule_crystal_archive".to_string(),
            expected_revision: rule.revision,
            idempotency_key: format!(
                "mcp-rule-crystal-archive:{}:{}@{}",
                args.crystal_id, rule.status, rule.revision
            ),
        })
        .map_err(domain)?;
    let archived = crystals.get(args.crystal_id).map_err(domain)?;
    Ok(crystal_payload(&archived))
}

// --------------------------------------------- hieronymus_rule_crystal_validate

#[derive(Deserialize)]
struct RuleCrystalValidate {
    crystal_id: i64,
}

/// Read-only rule-crystal validation: shape plus the deterministic
/// enforceability thresholds over the advisory projection.
fn rule_crystal_validate(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<RuleCrystalValidate>(arguments)?;
    let report = CrystalStore::open(application.config())
        .map_err(domain)?
        .validate_rule_crystal(args.crystal_id)
        .map_err(domain)?;
    serde_json::to_value(&report).map_err(|error| AppError::Domain(error.to_string()))
}

// ------------------------------------------------ hieronymus_rule_crystals_list

#[derive(Deserialize)]
struct RuleCrystalsList {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    series_slug: Option<String>,
    #[serde(default = "default_crystals_list_limit")]
    limit: i64,
}

fn default_crystals_list_limit() -> i64 {
    50
}

/// List rule crystals for review (newest first, optional status and series
/// filters). Advisory review surface only — statuses here never change the
/// structured authority.
fn rule_crystals_list(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<RuleCrystalsList>(arguments)?;
    if args.limit < 1 {
        return Err(AppError::Domain("limit must be at least 1".to_string()));
    }
    let crystals = CrystalStore::open(application.config())
        .map_err(domain)?
        .list_rule_crystals(
            args.status.as_deref(),
            args.series_slug.as_deref(),
            args.limit as usize,
        )
        .map_err(domain)?;
    Ok(Value::Array(crystals.iter().map(crystal_payload).collect()))
}

#[allow(clippy::too_many_arguments)]
fn read_termbase(
    application: &Application,
    series_slug: &str,
    source_language: Option<String>,
    target_language: Option<String>,
    volume: &str,
    chapter: &str,
    story: &super::StoryReadArgs,
) -> Result<Termbase, AppError> {
    let series = series_context(application, series_slug)?;
    let mut context = translation_context(
        &series,
        source_language,
        target_language,
        "translation",
        volume,
        chapter,
    )?;
    story.apply(&mut context);
    Termbase::open(application.config(), &context).map_err(domain)
}
