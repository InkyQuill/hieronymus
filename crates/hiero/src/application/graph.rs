//! The concept/facet/crystal graph tool family (plan M4):
//! `hieronymus_concept_create`, `hieronymus_concept_get`,
//! `hieronymus_concept_list`, `hieronymus_concept_update`,
//! `hieronymus_concept_archive`, `hieronymus_concept_merge`,
//! `hieronymus_concept_rename`, `hieronymus_concept_semantic_tags_set`,
//! `hieronymus_concept_facet_add`, `hieronymus_concept_facet_update`,
//! `hieronymus_concept_facet_list`, `hieronymus_concept_facet_set_canonical`,
//! `hieronymus_crystal_link_concept`, `hieronymus_crystal_story_scopes_set`,
//! `hieronymus_crystal_semantic_tags_set`, and
//! `hieronymus_concept_proposals_list`.
//!
//! Argument structs decode exactly the frozen `input_schema` rules from
//! `compatibility/snapshots/mcp.json` (required fields, defaults, and null
//! rules); decoding failures are [`AppError::Invalid`]. Store rejections are
//! [`AppError::Domain`]. All database work stays in the `hieronymus` store
//! APIs — no SQL lives here, and nothing in this family touches the ADR 0011
//! rule lifecycle (no rule activation, no authority mutation).
//!
//! Python parity notes (ADR 0003 graph, `mcp_server.py` as the fallback
//! reference):
//!
//! - Payloads are the Python `_concept_payload`/`_facet_payload`/
//!   `_crystal_payload` projections field for field (including the facet's
//!   public `kind` key next to the storage `facet_type`).
//! - `hieronymus_concept_facet_add` keeps the wrapper's type/kind precedence:
//!   an explicit `facet_type` replaces the default `name` kind, while an
//!   explicitly chosen `kind` is validated against `facet_type` by the store.
//! - `hieronymus_concept_facet_update` forwards every optional field through
//!   the presence-aware [`FacetPatch`]; the store maps an explicit JSON null
//!   to "unchanged" exactly like the Python wrapper (which cannot tell null
//!   from omitted) does.
//! - `hieronymus_concept_list` applies the series-scope filter in the
//!   wrapper (scope key match, plus global concepts when `include_global`).
//! - `hieronymus_concept_create` derives the series scope from
//!   `series_slug` after validating the series exists.
//! - `hieronymus_concept_proposals_list` returns the safe DTO projection of
//!   the pending `strict_concept_proposals` rows.

use serde::Deserialize;
use serde_json::{Value, json};

use hieronymus::concept_models::{ConceptFacetRecord, ConceptRecord};
use hieronymus::concepts::{ConceptStore, FacetFields, FacetPatch, NewConcept};
use hieronymus::crystals::CrystalStore;
use hieronymus::memory_models::CrystalRecord;
use hieronymus::registry::Registry;

use super::AppError;
use super::Application;
use super::decode;
use super::domain;

/// The family dispatcher: `None` means the tool is not ours.
pub(crate) fn dispatch(
    application: &Application,
    tool: &str,
    arguments: &Value,
    actor: &str,
) -> Option<Result<Value, AppError>> {
    let _ = actor; // No tool in this family is actor-scoped; the Python wrappers carry no actor argument.
    match tool {
        "hieronymus_concept_create" => Some(concept_create(application, arguments)),
        "hieronymus_concept_get" => Some(concept_get(application, arguments)),
        "hieronymus_concept_list" => Some(concept_list(application, arguments)),
        "hieronymus_concept_update" => Some(concept_update(application, arguments)),
        "hieronymus_concept_archive" => Some(concept_archive(application, arguments)),
        "hieronymus_concept_merge" => Some(concept_merge(application, arguments)),
        "hieronymus_concept_rename" => Some(concept_rename(application, arguments)),
        "hieronymus_concept_semantic_tags_set" => {
            Some(concept_semantic_tags_set(application, arguments))
        }
        "hieronymus_concept_facet_add" => Some(concept_facet_add(application, arguments)),
        "hieronymus_concept_facet_update" => Some(concept_facet_update(application, arguments)),
        "hieronymus_concept_facet_list" => Some(concept_facet_list(application, arguments)),
        "hieronymus_concept_facet_set_canonical" => {
            Some(concept_facet_set_canonical(application, arguments))
        }
        "hieronymus_crystal_link_concept" => Some(crystal_link_concept(application, arguments)),
        "hieronymus_crystal_story_scopes_set" => {
            Some(crystal_story_scopes_set(application, arguments))
        }
        "hieronymus_crystal_semantic_tags_set" => {
            Some(crystal_semantic_tags_set(application, arguments))
        }
        "hieronymus_concept_proposals_list" => Some(concept_proposals_list(application, arguments)),
        _ => None,
    }
}

/// The Python `_concept_payload` projection (never a serialized store type).
fn concept_payload(concept: &ConceptRecord) -> Value {
    json!({
        "id": concept.id,
        "canonical_name": concept.canonical_name,
        "description": concept.description,
        "status": concept.status,
        "confidence": concept.confidence,
        "scope_type": concept.scope_type,
        "scope_key": concept.scope_key,
        "semantic_tags": concept.tags,
        "merged_into_concept_id": concept.merged_into_concept_id,
    })
}

/// The Python `_facet_payload` projection: the storage `facet_type` plus the
/// public `kind` (the compatibility kinds `alias`/`former_label` present as
/// `name`).
fn facet_payload(facet: &ConceptFacetRecord) -> Value {
    json!({
        "id": facet.id,
        "concept_id": facet.concept_id,
        "language": facet.language,
        "facet_type": facet.facet_type,
        "kind": facet.kind(),
        "value": facet.value,
        "confidence": facet.confidence,
        "source_crystal_id": facet.source_crystal_id,
        "language_tags": facet.language_tags,
        "story_scopes": facet.story_scopes,
        "semantic_tags": facet.semantic_tags,
        "is_canonical": facet.is_canonical,
    })
}

/// The Python `_crystal_payload` projection (same shape as the terms family).
fn crystal_payload(crystal: &CrystalRecord) -> Value {
    json!({
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

fn concepts(application: &Application) -> Result<ConceptStore, AppError> {
    ConceptStore::open(application.config()).map_err(domain)
}

// ----------------------------------------------------- hieronymus_concept_create

#[derive(Deserialize)]
struct ConceptCreate {
    canonical_name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_concept_status")]
    status: String,
    #[serde(default = "default_confidence")]
    confidence: f64,
    #[serde(default)]
    semantic_tags: Option<Vec<String>>,
    #[serde(default)]
    series_slug: String,
    #[serde(default = "default_scope_type")]
    scope_type: String,
    #[serde(default)]
    scope_key: String,
}

fn default_concept_status() -> String {
    "candidate".to_string()
}

fn default_confidence() -> f64 {
    0.2
}

fn default_scope_type() -> String {
    "global".to_string()
}

/// Create a concept; a non-empty `series_slug` is validated against the
/// registry and replaces the scope (Python wrapper behavior).
fn concept_create(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ConceptCreate>(arguments)?;
    let mut scope_type = args.scope_type;
    let mut scope_key = args.scope_key;
    if !args.series_slug.is_empty() {
        Registry::open(application.config())
            .map_err(domain)?
            .get_series(&args.series_slug)
            .map_err(domain)?;
        scope_type = "series".to_string();
        scope_key = format!("series:{}", args.series_slug);
    }
    let concept = concepts(application)?
        .create_concept(
            &args.canonical_name,
            &NewConcept {
                description: args.description,
                status: args.status,
                confidence: args.confidence,
                scope_type,
                scope_key,
                semantic_tags: args.semantic_tags.unwrap_or_default(),
            },
        )
        .map_err(domain)?;
    Ok(concept_payload(&concept))
}

// --------------------------------------------------------- hieronymus_concept_get

#[derive(Deserialize)]
struct ConceptGet {
    concept_id: i64,
}

fn concept_get(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ConceptGet>(arguments)?;
    let concept = concepts(application)?
        .get(args.concept_id)
        .map_err(domain)?;
    Ok(concept_payload(&concept))
}

// -------------------------------------------------------- hieronymus_concept_list

#[derive(Deserialize)]
struct ConceptList {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    semantic_tag: Option<String>,
    #[serde(default)]
    series_slug: Option<String>,
    #[serde(default = "default_true")]
    include_global: bool,
}

fn default_true() -> bool {
    true
}

/// List concepts with the optional status/tag filters applied by the store
/// and the series-scope filter applied here (Python wrapper behavior: exact
/// scope-key match, plus global concepts while `include_global`).
fn concept_list(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ConceptList>(arguments)?;
    let mut records = concepts(application)?
        .list_concepts(args.status.as_deref(), args.semantic_tag.as_deref())
        .map_err(domain)?;
    if let Some(series_slug) = &args.series_slug {
        let scope_key = format!("series:{series_slug}");
        records.retain(|concept| {
            concept.scope_key == scope_key
                || (args.include_global && concept.scope_type == "global")
        });
    }
    Ok(Value::Array(records.iter().map(concept_payload).collect()))
}

// ------------------------------------------------------ hieronymus_concept_update

#[derive(Deserialize)]
struct ConceptUpdate {
    concept_id: i64,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
}

/// Update mutable metadata; omitted and explicit-null fields stay unchanged
/// (the store's update_concept rule).
fn concept_update(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ConceptUpdate>(arguments)?;
    let concept = concepts(application)?
        .update_concept(
            args.concept_id,
            args.description.as_deref(),
            args.status.as_deref(),
            args.confidence,
        )
        .map_err(domain)?;
    Ok(concept_payload(&concept))
}

// ---------------------------------------------------- hieronymus_concept_archive

#[derive(Deserialize)]
struct ConceptArchive {
    concept_id: i64,
    #[serde(default)]
    reason: String,
}

fn concept_archive(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ConceptArchive>(arguments)?;
    let store = concepts(application)?;
    store
        .archive_concept(args.concept_id, &args.reason)
        .map_err(domain)?;
    let concept = store.get(args.concept_id).map_err(domain)?;
    Ok(concept_payload(&concept))
}

// ------------------------------------------------------ hieronymus_concept_merge

#[derive(Deserialize)]
struct ConceptMerge {
    source_concept_id: i64,
    target_concept_id: i64,
    #[serde(default)]
    reason: String,
}

/// One-way merge: the source becomes `merged` with a redirect to the target;
/// both resulting records are returned (Python wrapper shape).
fn concept_merge(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ConceptMerge>(arguments)?;
    let store = concepts(application)?;
    store
        .merge_concepts(args.source_concept_id, args.target_concept_id, &args.reason)
        .map_err(domain)?;
    Ok(json!({
        "source": concept_payload(&store.get(args.source_concept_id).map_err(domain)?),
        "target": concept_payload(&store.get(args.target_concept_id).map_err(domain)?),
    }))
}

// ----------------------------------------------------- hieronymus_concept_rename

#[derive(Deserialize)]
struct ConceptRename {
    concept_id: i64,
    new_label: String,
    #[serde(default)]
    source_crystal_id: Option<i64>,
}

fn concept_rename(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ConceptRename>(arguments)?;
    let concept = concepts(application)?
        .rename_concept(args.concept_id, &args.new_label, args.source_crystal_id)
        .map_err(domain)?;
    Ok(concept_payload(&concept))
}

// ------------------------------------------- hieronymus_concept_semantic_tags_set

#[derive(Deserialize)]
struct ConceptSemanticTagsSet {
    concept_id: i64,
    semantic_tags: Vec<String>,
}

fn concept_semantic_tags_set(
    application: &Application,
    arguments: &Value,
) -> Result<Value, AppError> {
    let args = decode::<ConceptSemanticTagsSet>(arguments)?;
    let store = concepts(application)?;
    store
        .set_semantic_tags(args.concept_id, &args.semantic_tags)
        .map_err(domain)?;
    let concept = store.get(args.concept_id).map_err(domain)?;
    Ok(concept_payload(&concept))
}

// ------------------------------------------------- hieronymus_concept_facet_add

#[derive(Deserialize)]
struct FacetAdd {
    concept_id: i64,
    value: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    language_tags: Option<Vec<String>>,
    #[serde(default = "default_facet_kind")]
    kind: Option<String>,
    #[serde(default)]
    facet_type: Option<String>,
    #[serde(default = "default_confidence")]
    confidence: f64,
    #[serde(default)]
    source_crystal_id: Option<i64>,
    #[serde(default)]
    is_canonical: bool,
    #[serde(default)]
    story_scopes: Option<Vec<String>>,
    #[serde(default)]
    semantic_tags: Option<Vec<String>>,
}

fn default_facet_kind() -> Option<String> {
    Some("name".to_string())
}

/// Add a facet. The Python wrapper's type/kind precedence: an explicit
/// non-empty `facet_type` replaces the default `name` kind; an explicitly
/// chosen `kind` is forwarded and the store rejects a conflicting pair.
fn concept_facet_add(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<FacetAdd>(arguments)?;
    let storage_kind = if matches!(args.facet_type.as_deref(), Some(facet_type) if !facet_type.is_empty())
        && args.kind.as_deref() == Some("name")
    {
        None
    } else {
        args.kind
    };
    let fields = FacetFields {
        language: args.language,
        language_tags: args.language_tags.unwrap_or_default(),
        kind: storage_kind,
        facet_type: args.facet_type,
        confidence: Some(args.confidence),
        source_crystal_id: args.source_crystal_id,
        story_scopes: args.story_scopes.unwrap_or_default(),
        semantic_tags: args.semantic_tags.unwrap_or_default(),
    };
    let facet = concepts(application)?
        .add_facet(
            args.concept_id,
            &args.value,
            &fields,
            args.confidence,
            args.is_canonical,
        )
        .map_err(domain)?;
    Ok(facet_payload(&facet))
}

// ---------------------------------------------- hieronymus_concept_facet_update

#[derive(Deserialize)]
struct FacetUpdate {
    facet_id: i64,
    #[serde(flatten)]
    patch: FacetPatch,
}

/// Update a facet through the presence-aware patch: omitted fields stay
/// unchanged, and an explicit JSON null also stays unchanged (Python parity —
/// the wrapper cannot tell null from omitted).
fn concept_facet_update(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<FacetUpdate>(arguments)?;
    let facet = concepts(application)?
        .update_facet(args.facet_id, &args.patch)
        .map_err(domain)?;
    Ok(facet_payload(&facet))
}

// ----------------------------------------------- hieronymus_concept_facet_list

#[derive(Deserialize)]
struct FacetList {
    concept_id: i64,
}

fn concept_facet_list(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<FacetList>(arguments)?;
    let facets = concepts(application)?
        .list_facets(args.concept_id)
        .map_err(domain)?;
    Ok(Value::Array(facets.iter().map(facet_payload).collect()))
}

// ---------------------------------------- hieronymus_concept_facet_set_canonical

#[derive(Deserialize)]
struct FacetSetCanonical {
    concept_id: i64,
    facet_id: i64,
}

fn concept_facet_set_canonical(
    application: &Application,
    arguments: &Value,
) -> Result<Value, AppError> {
    let args = decode::<FacetSetCanonical>(arguments)?;
    let store = concepts(application)?;
    store
        .set_canonical_facet(args.concept_id, args.facet_id)
        .map_err(domain)?;
    let facet =
        hieronymus::concepts::get_facet(application.config(), args.facet_id).map_err(domain)?;
    Ok(facet_payload(&facet))
}

// ------------------------------------------------ hieronymus_crystal_link_concept

#[derive(Deserialize)]
struct CrystalLinkConcept {
    crystal_id: i64,
    concept_id: i64,
    #[serde(default = "default_link_type")]
    link_type: String,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn default_link_type() -> String {
    "mentions".to_string()
}

/// Link a crystal to an active concept as evidence; the linked crystal is
/// returned (Python wrapper shape).
fn crystal_link_concept(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<CrystalLinkConcept>(arguments)?;
    concepts(application)?
        .link_crystal(
            args.crystal_id,
            args.concept_id,
            &args.link_type,
            args.confidence,
        )
        .map_err(domain)?;
    let crystal = CrystalStore::open(application.config())
        .map_err(domain)?
        .get(args.crystal_id)
        .map_err(domain)?;
    Ok(crystal_payload(&crystal))
}

// -------------------------------------------- hieronymus_crystal_story_scopes_set

#[derive(Deserialize)]
struct CrystalStoryScopesSet {
    crystal_id: i64,
    story_scopes: Vec<String>,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn crystal_story_scopes_set(
    application: &Application,
    arguments: &Value,
) -> Result<Value, AppError> {
    let args = decode::<CrystalStoryScopesSet>(arguments)?;
    let crystal = CrystalStore::open(application.config())
        .map_err(domain)?
        .set_story_scopes(args.crystal_id, &args.story_scopes, args.confidence)
        .map_err(domain)?;
    Ok(crystal_payload(&crystal))
}

// ------------------------------------------- hieronymus_crystal_semantic_tags_set

#[derive(Deserialize)]
struct CrystalSemanticTagsSet {
    crystal_id: i64,
    semantic_tags: Vec<String>,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn crystal_semantic_tags_set(
    application: &Application,
    arguments: &Value,
) -> Result<Value, AppError> {
    let args = decode::<CrystalSemanticTagsSet>(arguments)?;
    let crystal = CrystalStore::open(application.config())
        .map_err(domain)?
        .set_semantic_tags(args.crystal_id, &args.semantic_tags, args.confidence)
        .map_err(domain)?;
    Ok(crystal_payload(&crystal))
}

// -------------------------------------------- hieronymus_concept_proposals_list

#[derive(Deserialize)]
struct ConceptProposalsList {}

/// The pending strict concept proposals (safe DTO projection owned by the
/// store).
fn concept_proposals_list(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    decode::<ConceptProposalsList>(arguments)?;
    let proposals = concepts(application)?.list_proposals().map_err(domain)?;
    Ok(Value::Array(proposals))
}
