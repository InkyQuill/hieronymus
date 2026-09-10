//! The normalized dream-output contract (port of the Python
//! `_NormalizedDreamOutput` sections the dreaming core applies): typed
//! records for concept proposals, concepts, facets, supersede actions, and
//! reinforce actions, plus the raw-JSON target guard for supersede actions.
//!
//! Binding rules (ADR 0011, ADR 0003):
//! - Generic Dream output may create concepts,
//!   facets, candidate rules (proposals), and graded-memory score deltas, but
//!   that path cannot activate, replace, or archive an active rule; the target
//!   guard rejects any supersede action touching an id outside the selected
//!   context or an active rule before any store call runs.
//! - Numeric inputs are finite and bounded; out-of-range or malformed values
//!   reject their entry individually into the durable audit channels
//!   (warnings / rejected_entries / skipped_candidates) and never silently
//!   drop a whole section.
//! - Rejection reasons name fields and ids, never provider credentials; the
//!   audit store redacts payloads again before persistence.

//! - Separate versioned correction decisions carry learned-policy drafts only.
//!   Trusted consolidation selection, origin, revisions and policy checks must
//!   succeed before these can change authority.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::dreaming::{
    MALFORMED_CONFIDENCE_PENALTY, MIN_NORMALIZED_CONFIDENCE, ParseWarning, append_parse_warning,
    optional_int, string_field, value_type_name,
};

/// Facet kinds a normalized facet may carry (Python `VALID_FACET_KINDS`).
pub const VALID_FACET_KINDS: [&str; 4] = ["name", "rendering", "description", "note"];

/// Bounds for graded-memory score deltas carried by reinforce actions
/// (scores live in `[0, 1]`, so a wider delta cannot be honored honestly).
pub const REINFORCE_DELTA_LIMIT: f64 = 1.0;

// ----------------------------------------------------------------------
// Target authorization
// ----------------------------------------------------------------------

/// The brief's raw-JSON guard: every `supersede_actions` entry must name two
/// integer crystal ids, both inside the selected context's allowed set and
/// neither an active rule. A malformed id is a rejection; an unauthorized id
/// fails closed.
pub fn validate_action_targets(
    value: &serde_json::Value,
    allowed: &std::collections::BTreeSet<i64>,
    active_rules: &std::collections::BTreeSet<i64>,
) -> Result<(), String> {
    if let Some(actions) = value.get("supersede_actions") {
        let actions = actions
            .as_array()
            .ok_or("supersede_actions must be an array")?;
        for action in actions {
            for field in ["old_crystal_id", "new_crystal_id"] {
                let id = action
                    .get(field)
                    .and_then(serde_json::Value::as_i64)
                    .ok_or_else(|| format!("{field} must be an integer"))?;
                if !allowed.contains(&id) || active_rules.contains(&id) {
                    return Err(format!("dream action is not authorized for crystal {id}"));
                }
            }
        }
    }
    Ok(())
}

/// The typed equivalent of [`validate_action_targets`] for parsed actions:
/// runs against the same context-derived sets immediately before the store
/// calls in the persistence transaction (the raw guard above only sees the
/// contract key; the parsed path also covers the Python wire key `supersede`).
pub(crate) fn validate_supersede_targets(
    actions: &[SupersedeAction],
    allowed: &BTreeSet<i64>,
    active_rules: &BTreeSet<i64>,
) -> Result<(), String> {
    for action in actions {
        for id in [action.old_crystal_id, action.new_crystal_id] {
            if !allowed.contains(&id) || active_rules.contains(&id) {
                return Err(format!("dream action is not authorized for crystal {id}"));
            }
        }
    }
    Ok(())
}

/// A reinforce target must live inside the selected context: an
/// unauthorized id fails the run closed before any score mutates.
pub(crate) fn validate_reinforce_targets(
    actions: &[ReinforceAction],
    allowed: &BTreeSet<i64>,
) -> Result<(), String> {
    for action in actions {
        if !allowed.contains(&action.crystal_id) {
            return Err(format!(
                "dream action is not authorized for crystal {}",
                action.crystal_id
            ));
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// Typed normalized records
// ----------------------------------------------------------------------

/// Port of `_NormalizedDreamConcept`.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedConcept {
    pub canonical_name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub confidence_delta: f64,
}

/// Port of `_NormalizedDreamFacet`.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedFacet {
    pub concept_name: String,
    pub value: String,
    pub kind: String,
    pub language_tags: Vec<String>,
    pub story_scopes: Vec<String>,
    pub semantic_tags: Vec<String>,
    pub confidence: f64,
    pub is_canonical: bool,
}

/// Port of `_DreamSupersedeAction`.
#[derive(Debug, Clone, PartialEq)]
pub struct SupersedeAction {
    pub old_crystal_id: i64,
    pub new_crystal_id: i64,
    pub reason: String,
}

/// Port of the `_normalize_score_action` tuple `(crystal_id, strength_delta,
/// confidence_delta)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReinforceAction {
    pub crystal_id: i64,
    pub strength_delta: f64,
    pub confidence_delta: f64,
}

/// Port of `DreamConceptProposal`: a strict concept proposal that becomes a
/// `pending` row in `strict_concept_proposals` — a candidate that only an
/// explicit human approval can act on, never an active rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ConceptProposal {
    pub series_slug: String,
    pub source_language: String,
    pub target_language: String,
    pub concept_text: String,
    pub source_form: String,
    pub canonical_rendering: String,
    pub approved_variants: Vec<String>,
    pub forbidden_variants: Vec<String>,
    pub rationale: String,
}

// ----------------------------------------------------------------------
// Numeric guards (ruling: finite, bounded numeric inputs)
// ----------------------------------------------------------------------

/// A JSON number that is finite and inside `min..=max`; anything else (wrong
/// type, NaN, infinite, out of range) is malformed.
pub(crate) fn bounded_number(value: Option<&Value>, min: f64, max: f64) -> Option<f64> {
    let parsed = value?.as_f64()?;
    if !parsed.is_finite() || parsed < min || parsed > max {
        return None;
    }
    Some(parsed)
}

// ----------------------------------------------------------------------
// Section parsers
// ----------------------------------------------------------------------

/// Port of `_normalize_dict_concept`. A structurally malformed entry is
/// rejected individually (`None` plus a durable rejected-entry record);
/// hard failures never kill the whole section.
pub(crate) fn normalize_concept_entry(
    item: &Value,
    entry_path: &str,
    warnings: &mut Vec<ParseWarning>,
    rejected_entries: &mut Vec<Value>,
) -> Option<NormalizedConcept> {
    let Some(payload) = item.as_object() else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "malformed_concept_entry",
            "candidate_type": value_type_name(item),
        }));
        return None;
    };
    let mut penalty = 0.0;
    let mut name = string_field(payload.get("canonical_name"))
        .trim()
        .to_string();
    if name.is_empty() {
        name = string_field(payload.get("name")).trim().to_string();
    }
    if name.is_empty() {
        name = string_field(payload.get("label")).trim().to_string();
        if !name.is_empty() {
            penalty += MALFORMED_CONFIDENCE_PENALTY;
            append_parse_warning(
                warnings,
                entry_path,
                "malformed_concept_name",
                "used fallback concept label",
                MALFORMED_CONFIDENCE_PENALTY,
            );
        }
    }
    if name.is_empty() {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "missing_concept_name",
        }));
        return None;
    }
    // Bounded numeric input: a present confidence must be a finite number in
    // [0, 1]; anything else rejects the entry (ruling: numeric validation).
    // Rejection records never echo offending values, so provider payloads
    // (and any credential material in them) cannot leak into the audit.
    let confidence = match bounded_number(payload.get("confidence"), 0.0, 1.0) {
        Some(confidence) => confidence,
        None if payload.get("confidence").is_none() => 0.2,
        None => {
            rejected_entries.push(json!({
                "entry_path": entry_path,
                "reason": "malformed_concept_confidence",
            }));
            return None;
        }
    };
    let confidence_delta = (confidence - penalty).clamp(MIN_NORMALIZED_CONFIDENCE, 1.0);
    Some(NormalizedConcept {
        canonical_name: name,
        description: string_field(payload.get("description")),
        tags: clean_string_tuple(&[payload.get("tags"), payload.get("semantic_tags")]),
        confidence_delta,
    })
}

/// Port of `_normalize_dict_facet`. Malformed metadata falls back with
/// confidence penalties and warnings; a facet without a usable value or
/// concept name, or with a non-bounded confidence, is rejected individually.
#[allow(clippy::too_many_lines)]
pub(crate) fn normalize_facet_entry(
    item: &Value,
    entry_path: &str,
    warnings: &mut Vec<ParseWarning>,
    rejected_entries: &mut Vec<Value>,
) -> Option<NormalizedFacet> {
    let Some(payload) = item.as_object() else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "malformed_facet_entry",
            "candidate_type": value_type_name(item),
        }));
        return None;
    };

    // Value: content/value/text are clean, body costs a penalty.
    let (value, mut metadata_penalty) = recover_facet_value(payload);
    let Some(value) = value else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "missing_facet_value",
        }));
        return None;
    };
    if metadata_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_value",
            "used fallback facet value field",
            metadata_penalty,
        );
    }

    let concept_name = facet_concept_name_from_payload(payload, entry_path, warnings);
    if concept_name.is_empty() {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "missing_facet_concept_name",
        }));
        return None;
    }

    let (kind, kind_penalty) = recover_facet_kind(payload);
    if kind_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_kind",
            "used fallback facet kind",
            kind_penalty,
        );
    }
    let (language_tags, language_penalty) =
        recover_facet_string_tuple(payload, &["language_tags", "language"]);
    if language_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_language_tags",
            "ignored malformed facet language metadata",
            language_penalty,
        );
    }
    let (story_scopes, story_scope_penalty) =
        recover_facet_string_tuple(payload, &["story_scopes", "story_scope"]);
    if story_scope_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_story_scopes",
            "ignored malformed facet story scope metadata",
            story_scope_penalty,
        );
    }
    let (semantic_tags, semantic_tag_penalty) =
        recover_facet_string_tuple(payload, &["semantic_tags", "tags"]);
    if semantic_tag_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_semantic_tags",
            "ignored malformed facet semantic tag metadata",
            semantic_tag_penalty,
        );
    }
    let (is_canonical, canonical_penalty) = recover_facet_canonical(payload);
    if canonical_penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_facet_canonical",
            "parsed non-boolean canonical metadata",
            canonical_penalty,
        );
    }
    metadata_penalty += kind_penalty
        + language_penalty
        + story_scope_penalty
        + semantic_tag_penalty
        + canonical_penalty;

    let confidence = match bounded_number(payload.get("confidence"), 0.0, 1.0) {
        Some(confidence) => confidence,
        None if payload.get("confidence").is_none() => 0.2,
        None => {
            rejected_entries.push(json!({
                "entry_path": entry_path,
                "reason": "malformed_facet_confidence",
            }));
            return None;
        }
    };
    Some(NormalizedFacet {
        concept_name,
        value,
        kind,
        language_tags,
        story_scopes,
        semantic_tags,
        confidence: (confidence - metadata_penalty).clamp(MIN_NORMALIZED_CONFIDENCE, 1.0),
        is_canonical,
    })
}

/// Port of `_normalize_supersede_action`: `None` marks a structurally
/// malformed action (the caller records it under `skipped_candidates`).
pub(crate) fn normalize_supersede_entry(item: &Value) -> Option<SupersedeAction> {
    let payload = item.as_object()?;
    let old_crystal_id = optional_int(payload.get("old_crystal_id"))?;
    let new_crystal_id = optional_int(payload.get("new_crystal_id"))?;
    Some(SupersedeAction {
        old_crystal_id,
        new_crystal_id,
        reason: string_field(payload.get("reason")),
    })
}

/// Port of `_normalize_score_action` with the graded-memory numeric bounds:
/// deltas must be finite numbers within `[-1, 1]`, the crystal id must
/// resolve, and the provenance source ids must be a non-empty subset of the
/// selected memories. Malformed entries are rejected individually.
pub(crate) fn normalize_reinforce_entry(
    item: &Value,
    entry_path: &str,
    allowed_memory_ids: &std::collections::HashSet<i64>,
    rejected_entries: &mut Vec<Value>,
) -> Option<ReinforceAction> {
    let Some(payload) = item.as_object() else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "malformed_reinforce_action",
            "candidate_type": value_type_name(item),
        }));
        return None;
    };
    let Some(crystal_id) = optional_int(payload.get("crystal_id")) else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "reinforce_action_requires_crystal_id",
        }));
        return None;
    };
    let mut deltas = [0.0_f64, 0.0_f64];
    for (index, key) in ["strength_delta", "confidence_delta"]
        .into_iter()
        .enumerate()
    {
        match bounded_number(
            payload.get(key),
            -REINFORCE_DELTA_LIMIT,
            REINFORCE_DELTA_LIMIT,
        ) {
            Some(delta) => deltas[index] = delta,
            None if payload.get(key).is_none() => {}
            None => {
                rejected_entries.push(json!({
                    "entry_path": entry_path,
                    "reason": "reinforce_action_delta_out_of_range",
                    "field": key,
                }));
                return None;
            }
        }
    }
    // Provenance: reinforce actions must cite selected source memories.
    let Some(source_memory_ids) = payload.get("source_memory_ids").and_then(Value::as_array) else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "reinforce_action_requires_selected_source_memory_ids",
        }));
        return None;
    };
    if source_memory_ids.is_empty()
        || !source_memory_ids.iter().all(|id| {
            !id.is_boolean()
                && id
                    .as_i64()
                    .is_some_and(|id| allowed_memory_ids.contains(&id))
        })
    {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "reinforce_action_requires_selected_source_memory_ids",
        }));
        return None;
    }
    Some(ReinforceAction {
        crystal_id,
        strength_delta: deltas[0],
        confidence_delta: deltas[1],
    })
}

/// One strict concept proposal entry. Structurally malformed proposals are
/// rejected individually; context matching is validated later against the
/// pass context (a proposal for another series fails the run closed).
pub(crate) fn normalize_concept_proposal_entry(
    item: &Value,
    entry_path: &str,
    rejected_entries: &mut Vec<Value>,
) -> Option<ConceptProposal> {
    let Some(payload) = item.as_object() else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "malformed_concept_proposal",
            "candidate_type": value_type_name(item),
        }));
        return None;
    };
    let mut strings: Vec<String> = vec![String::new(); 6];
    for (index, key) in [
        "series_slug",
        "source_language",
        "target_language",
        "concept_text",
        "source_form",
        "canonical_rendering",
    ]
    .into_iter()
    .enumerate()
    {
        strings[index] = string_field(payload.get(key));
    }
    if strings[3].trim().is_empty() || strings[4].trim().is_empty() || strings[5].trim().is_empty()
    {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "invalid_concept_proposal",
        }));
        return None;
    }
    let variants = |key: &str| -> Option<Vec<String>> {
        let mut values = Vec::new();
        let fallback = Value::Array(Vec::new());
        for item in payload.get(key).unwrap_or(&fallback).as_array()? {
            values.push(item.as_str()?.to_string());
        }
        Some(values)
    };
    let (Some(approved_variants), Some(forbidden_variants)) = (
        variants("approved_variants"),
        variants("forbidden_variants"),
    ) else {
        rejected_entries.push(json!({
            "entry_path": entry_path,
            "reason": "invalid_concept_proposal_variants",
        }));
        return None;
    };
    Some(ConceptProposal {
        series_slug: strings[0].clone(),
        source_language: strings[1].clone(),
        target_language: strings[2].clone(),
        concept_text: strings[3].clone(),
        source_form: strings[4].clone(),
        canonical_rendering: strings[5].clone(),
        approved_variants,
        forbidden_variants,
        rationale: string_field(payload.get("rationale")),
    })
}

/// Port of `_concept_names_from_payload`: `concept_names`/`concepts` lists,
/// a bare `concept_name`, and nested `{canonical_name|name|label}` objects.
/// Malformed entries cost a confidence penalty and one aggregate warning.
pub(crate) fn concept_names_from_payload(
    payload: &serde_json::Map<String, Value>,
    entry_path: &str,
    warnings: &mut Vec<ParseWarning>,
) -> (Vec<String>, f64) {
    let mut values: Vec<Value> = Vec::new();
    for key in ["concept_names", "concepts"] {
        match payload.get(key) {
            Some(Value::Array(items)) => values.extend(items.iter().cloned()),
            Some(value) if !value.is_null() => values.push(value.clone()),
            _ => {}
        }
    }
    if let Some(name) = payload.get("concept_name").filter(|value| !value.is_null()) {
        values.push(name.clone());
    }

    let mut names: Vec<String> = Vec::new();
    let mut penalty = 0.0;
    for value in &values {
        match value {
            Value::String(text) => {
                if text.trim().is_empty() {
                    penalty += MALFORMED_CONFIDENCE_PENALTY;
                } else {
                    names.push(text.clone());
                }
            }
            Value::Object(nested) => {
                let (name, name_penalty) = recover_optional_concept_name(nested);
                penalty += name_penalty;
                if !name.is_empty() {
                    names.push(name);
                }
            }
            _ => penalty += MALFORMED_CONFIDENCE_PENALTY,
        }
    }
    if penalty > 0.0 {
        append_parse_warning(
            warnings,
            entry_path,
            "malformed_crystal_concept_metadata",
            "ignored malformed crystal concept metadata",
            penalty,
        );
    }
    (crate::dreaming::clean_text_tuple(names), penalty)
}

// ----------------------------------------------------------------------
// Facet recovery helpers (ports)
// ----------------------------------------------------------------------

/// Port of `_recover_facet_value`.
fn recover_facet_value(payload: &serde_json::Map<String, Value>) -> (Option<String>, f64) {
    for key in ["content", "value", "text"] {
        if let Some(Value::String(value)) = payload.get(key)
            && !value.trim().is_empty()
        {
            return (Some(crate::dreaming::collapse_whitespace(value)), 0.0);
        }
    }
    if let Some(Value::String(value)) = payload.get("body")
        && !value.trim().is_empty()
    {
        return (
            Some(crate::dreaming::collapse_whitespace(value)),
            MALFORMED_CONFIDENCE_PENALTY,
        );
    }
    (None, 0.0)
}

/// Port of `_recover_facet_kind`.
fn recover_facet_kind(payload: &serde_json::Map<String, Value>) -> (String, f64) {
    if !payload.contains_key("kind") && !payload.contains_key("facet_type") {
        return ("note".to_string(), 0.0);
    }
    let value = payload.get("kind").or_else(|| payload.get("facet_type"));
    let Some(Value::String(value)) = value else {
        return ("note".to_string(), MALFORMED_CONFIDENCE_PENALTY);
    };
    if value.trim().is_empty() {
        return ("note".to_string(), MALFORMED_CONFIDENCE_PENALTY);
    }
    let clean_kind = value.trim().to_lowercase().replace('-', "_");
    if VALID_FACET_KINDS.contains(&clean_kind.as_str()) {
        return (clean_kind, 0.0);
    }
    if clean_kind == "alias" || clean_kind == "former_label" {
        return ("name".to_string(), MALFORMED_CONFIDENCE_PENALTY);
    }
    ("note".to_string(), MALFORMED_CONFIDENCE_PENALTY)
}

/// Port of `_recover_facet_string_tuple` (string or list of strings).
pub(crate) fn recover_facet_string_tuple(
    payload: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> (Vec<String>, f64) {
    let mut values: Vec<String> = Vec::new();
    let mut penalty = 0.0;
    for key in keys {
        let Some(value) = payload.get(*key) else {
            continue;
        };
        match value {
            Value::String(text) => {
                if text.trim().is_empty() {
                    penalty += MALFORMED_CONFIDENCE_PENALTY;
                } else {
                    values.push(text.clone());
                }
            }
            Value::Array(items) => {
                for item in items {
                    match item {
                        Value::String(text) if !text.trim().is_empty() => {
                            values.push(text.clone());
                        }
                        _ => penalty += MALFORMED_CONFIDENCE_PENALTY,
                    }
                }
            }
            _ => penalty += MALFORMED_CONFIDENCE_PENALTY,
        }
    }
    (crate::dreaming::clean_text_tuple(values), penalty)
}

/// Port of `_recover_facet_canonical`.
fn recover_facet_canonical(payload: &serde_json::Map<String, Value>) -> (bool, f64) {
    if let Some(value) = payload.get("is_canonical") {
        return parse_facet_canonical(value);
    }
    if let Some(value) = payload.get("canonical") {
        return parse_facet_canonical(value);
    }
    (false, 0.0)
}

/// Port of `_parse_facet_canonical`.
fn parse_facet_canonical(value: &Value) -> (bool, f64) {
    match value {
        Value::Bool(parsed) => (*parsed, 0.0),
        Value::String(text) => {
            let clean = text.trim().to_lowercase();
            match clean.as_str() {
                "true" | "yes" | "1" => (true, MALFORMED_CONFIDENCE_PENALTY),
                "false" | "no" | "0" => (false, MALFORMED_CONFIDENCE_PENALTY),
                _ => (false, MALFORMED_CONFIDENCE_PENALTY),
            }
        }
        Value::Number(number) if number.as_i64() == Some(0) || number.as_i64() == Some(1) => {
            (number.as_i64() == Some(1), MALFORMED_CONFIDENCE_PENALTY)
        }
        _ => (false, MALFORMED_CONFIDENCE_PENALTY),
    }
}

/// Port of `_facet_concept_name_from_payload`.
fn facet_concept_name_from_payload(
    payload: &serde_json::Map<String, Value>,
    entry_path: &str,
    warnings: &mut Vec<ParseWarning>,
) -> String {
    for key in ["concept_name", "concept_label", "canonical_name"] {
        if let Some(Value::String(value)) = payload.get(key)
            && !value.trim().is_empty()
        {
            return value.trim().to_string();
        }
    }
    if let Some(Value::String(concept)) = payload.get("concept")
        && !concept.trim().is_empty()
    {
        return concept.trim().to_string();
    }
    if let Some(nested) = payload.get("concept").and_then(Value::as_object) {
        let (name, penalty) = recover_optional_concept_name(nested);
        if penalty > 0.0 {
            append_parse_warning(
                warnings,
                &format!("{entry_path}.concept"),
                "malformed_concept_name",
                "used fallback concept label",
                penalty,
            );
        }
        if !name.is_empty() {
            return name;
        }
    }
    String::new()
}

/// Port of `_recover_optional_concept_name`.
fn recover_optional_concept_name(payload: &serde_json::Map<String, Value>) -> (String, f64) {
    for (key, penalty) in [
        ("canonical_name", 0.0),
        ("name", 0.0),
        ("label", MALFORMED_CONFIDENCE_PENALTY),
    ] {
        if let Some(Value::String(value)) = payload.get(key)
            && !value.trim().is_empty()
        {
            return (value.trim().to_string(), penalty);
        }
    }
    (String::new(), MALFORMED_CONFIDENCE_PENALTY)
}

/// Port of `_clean_string_tuple`: strings and lists of strings, trimmed,
/// deduplicated, ordered.
pub(crate) fn clean_string_tuple(values: &[Option<&Value>]) -> Vec<String> {
    let mut strings: Vec<String> = Vec::new();
    for value in values.iter().flatten() {
        match value {
            Value::String(text) => strings.push(text.clone()),
            Value::Array(items) => {
                for item in items {
                    if let Value::String(text) = item {
                        strings.push(text.clone());
                    }
                }
            }
            _ => {}
        }
    }
    crate::dreaming::clean_text_tuple(strings)
}

// ----------------------------------------------------------------------
// Typed output validation (fail-closed invariants over accepted entries)
// ----------------------------------------------------------------------

/// Facet invariants (Python `_validate_normalized_output` facet half): names
/// and values are non-empty, kinds are known, confidence is bounded.
pub(crate) fn validate_facets(facets: &[NormalizedFacet]) -> Result<(), String> {
    for facet in facets {
        if facet.concept_name.trim().is_empty() {
            return Err("facet concept_name must not be empty".to_string());
        }
        if facet.value.trim().is_empty() {
            return Err("facet value must not be empty".to_string());
        }
        if !VALID_FACET_KINDS.contains(&facet.kind.as_str()) {
            return Err(format!("unknown facet kind: {}", facet.kind));
        }
        if !facet.confidence.is_finite() || !(0.0..=1.0).contains(&facet.confidence) {
            return Err("facet confidence must be between 0 and 1".to_string());
        }
    }
    Ok(())
}

/// Concept invariants: names are non-empty, deltas are bounded.
pub(crate) fn validate_concepts(concepts: &[NormalizedConcept]) -> Result<(), String> {
    for concept in concepts {
        if concept.canonical_name.trim().is_empty() {
            return Err("concept canonical_name must not be empty".to_string());
        }
        if !concept.confidence_delta.is_finite() || !(0.0..=1.0).contains(&concept.confidence_delta)
        {
            return Err("concept confidence_delta must be between 0 and 1".to_string());
        }
    }
    Ok(())
}

/// Proposal context binding: a proposal asserts another series' language
/// pair at most — a mismatch fails the run closed.
pub(crate) fn validate_proposal_contexts(
    proposals: &[ConceptProposal],
    series_slug: &str,
    source_language: &str,
    target_language: &str,
) -> Result<(), String> {
    for proposal in proposals {
        if proposal.series_slug != series_slug {
            return Err("proposal series_slug must match context".to_string());
        }
        if proposal.source_language != source_language {
            return Err("proposal source_language must match context".to_string());
        }
        if proposal.target_language != target_language {
            return Err("proposal target_language must match context".to_string());
        }
    }
    Ok(())
}

/// Reinforce invariants over the accepted actions: bounded deltas.
pub(crate) fn validate_reinforce_actions(actions: &[ReinforceAction]) -> Result<(), String> {
    for action in actions {
        for (name, delta) in [
            ("strength_delta", action.strength_delta),
            ("confidence_delta", action.confidence_delta),
        ] {
            if !delta.is_finite()
                || !(-REINFORCE_DELTA_LIMIT..=REINFORCE_DELTA_LIMIT).contains(&delta)
            {
                return Err(format!("reinforce {name} must be between -1 and 1"));
            }
        }
    }
    Ok(())
}

/// Separate provider draft: trusted actor, identity and selection are never wire fields.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionsDraftV1 {
    pub version: u8,
    pub mutations: Vec<crate::consolidation::DerivedMutationV1>,
}
/// Parse the correction lane without accepting generic Dream actions or authority claims.
pub fn parse_decisions(value: Value) -> Result<DecisionsDraftV1, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        decisions: DecisionsDraftV1,
    }
    let envelope: Envelope =
        serde_json::from_value(value).map_err(|_| "invalid_decisions_schema")?;
    let draft = envelope.decisions;
    if draft.version != 1 || draft.mutations.len() > 100 {
        return Err("invalid_decisions_bounds".into());
    }
    Ok(draft)
}
