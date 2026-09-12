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
//! ADR0016 authority boundary: proposals remain advisory. Ordinary MCP
//! approval/archive wrappers report unverified origin; their historical names
//! and actor labels confer no human authority. Evidence-grounded learned work
//! uses `hieronymus_decide`, while user corrections use the guarded ingress.
//! Contract/validation keep coherent read context and accepted-receipt dependencies.
//! Proposals preserve legacy fields and add the existing typed `concept_id`.

use serde::Deserialize;
use serde_json::{Value, json};

use hieronymus::crystals::CrystalStore;
use hieronymus::memory_models::CrystalRecord;
use hieronymus::terminology::{ProposeFields, Source, TermRule, Termbase};

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
    #[serde(default)]
    concept_id: Option<i64>,
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
        concept_id: args.concept_id,
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
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Validate the retained legacy input shape before reporting the policy error.
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

/// Legacy compatibility surface: actor labels cannot bypass decision policy.
fn termbase_approve(
    _application: &Application,
    arguments: &Value,
    _actor: &str,
) -> Result<Value, AppError> {
    let _args = decode::<TermbaseApprove>(arguments)?;
    Err(legacy_authority_error())
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
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Validate the retained legacy input shape before reporting the policy error.
struct RuleCrystalArchive {
    crystal_id: i64,
}

/// Applies equally to decision-owned and migration-protected legacy global rules.
fn rule_crystal_archive(
    _application: &Application,
    arguments: &Value,
    _actor: &str,
) -> Result<Value, AppError> {
    let _args = decode::<RuleCrystalArchive>(arguments)?;
    Err(legacy_authority_error())
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
    story.apply(&mut context)?;
    Termbase::open(application.config(), &context).map_err(domain)
}

fn legacy_authority_error() -> AppError {
    AppError::Domain("unverified origin: legacy hard-rule mutation requires hieronymus_decide or trusted correction ingress".into())
}
