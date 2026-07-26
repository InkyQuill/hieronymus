use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolContract {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeriesCreateInput {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub source_language: String,
    #[serde(default)]
    pub target_language: String,
    #[serde(default)]
    pub language_tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeriesTagsInput {
    pub series_id: i64,
    pub language_tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptListInput {
    pub status: Option<String>,
    pub semantic_tag: Option<String>,
    pub series_slug: Option<String>,
    #[serde(default = "yes")]
    pub include_global: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptIdInput {
    pub concept_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptCreateInput {
    pub canonical_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "candidate")]
    pub status: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub semantic_tags: Vec<String>,
    #[serde(default)]
    pub series_slug: String,
    #[serde(default = "global")]
    pub scope_type: String,
    #[serde(default)]
    pub scope_key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptUpdateInput {
    pub concept_id: i64,
    pub description: Option<String>,
    pub status: Option<String>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptArchiveInput {
    pub concept_id: i64,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptMergeInput {
    pub source_concept_id: i64,
    pub target_concept_id: i64,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptRenameInput {
    pub concept_id: i64,
    pub new_label: String,
    pub source_crystal_id: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FacetAddInput {
    pub concept_id: i64,
    pub value: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub language_tags: Vec<String>,
    pub kind: Option<String>,
    pub facet_type: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    pub source_crystal_id: Option<i64>,
    #[serde(default)]
    pub is_canonical: bool,
    #[serde(default)]
    pub story_scopes: Vec<String>,
    #[serde(default)]
    pub semantic_tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FacetUpdateInput {
    pub facet_id: i64,
    pub value: Option<String>,
    pub language: Option<String>,
    pub language_tags: Option<Vec<String>>,
    pub kind: Option<String>,
    pub facet_type: Option<String>,
    pub confidence: Option<f64>,
    pub source_crystal_id: Option<i64>,
    pub is_canonical: Option<bool>,
    pub story_scopes: Option<Vec<String>>,
    pub semantic_tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FacetCanonicalInput {
    pub concept_id: i64,
    pub facet_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptTagsInput {
    pub concept_id: i64,
    pub semantic_tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrystalLinkInput {
    pub crystal_id: i64,
    pub concept_id: i64,
    #[serde(default = "mentions")]
    pub link_type: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrystalScopesInput {
    pub crystal_id: i64,
    pub story_scopes: Vec<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrystalTagsInput {
    pub crystal_id: i64,
    pub semantic_tags: Vec<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleListInput {
    pub status: Option<String>,
    pub series_slug: Option<String>,
    #[serde(default = "fifty")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrystalIdInput {
    pub crystal_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextTextInput {
    pub series_slug: String,
    pub raw_text: String,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub chapter: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValidateTextInput {
    pub series_slug: String,
    pub raw_text: String,
    pub translated_text: String,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub chapter: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TermProposeInput {
    pub series_slug: String,
    pub category: String,
    pub source_text: String,
    pub canonical_translation: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub chapter: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TermApproveInput {
    pub series_slug: String,
    pub term_id: i64,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub chapter: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemorySearchInput {
    pub series_slug: String,
    pub query: String,
    #[serde(default = "five")]
    pub limit: usize,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RagImportInput {
    pub series_slug: String,
    pub path: std::path::PathBuf,
    pub source_ref: Option<String>,
    #[serde(default = "auto")]
    pub source_type: String,
    #[serde(default)]
    pub language_tags: Vec<String>,
    #[serde(default)]
    pub story_scopes: Vec<String>,
    #[serde(default)]
    pub semantic_tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RagSearchInput {
    pub series_slug: String,
    pub query: String,
    #[serde(default = "ten")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryAddInput {
    pub series_slug: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub source_ref: String,
    #[serde(default = "three")]
    pub importance: i64,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionStartInput {
    pub series_slug: String,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    #[serde(default = "translation")]
    pub task_type: String,
    #[serde(default)]
    pub volume: String,
    #[serde(default)]
    pub chapter: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionIdInput {
    pub session_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShortAddInput {
    pub session_id: i64,
    pub kind: String,
    pub text: String,
    #[serde(default = "agent")]
    pub source_role: String,
    #[serde(default)]
    pub source_ref: String,
    #[serde(default = "empty_object")]
    pub metadata: Value,
    #[serde(default)]
    pub language_tags: Vec<String>,
    #[serde(default)]
    pub story_scopes: Vec<String>,
    #[serde(default)]
    pub semantic_tags: Vec<String>,
    #[serde(default = "observation")]
    pub source_credibility: String,
    #[serde(default)]
    pub rule_intent: String,
    #[serde(default)]
    pub soft_origin: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShortBatchInput {
    pub session_id: i64,
    pub items: Vec<ShortItemInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShortItemInput {
    pub kind: String,
    pub text: String,
    #[serde(default = "agent")]
    pub source_role: String,
    #[serde(default)]
    pub source_ref: String,
    #[serde(default = "empty_object")]
    pub metadata: Value,
    #[serde(default)]
    pub language_tags: Vec<String>,
    #[serde(default)]
    pub story_scopes: Vec<String>,
    #[serde(default)]
    pub semantic_tags: Vec<String>,
    #[serde(default = "observation")]
    pub source_credibility: String,
    #[serde(default)]
    pub rule_intent: String,
    #[serde(default)]
    pub soft_origin: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecallInput {
    pub session_id: i64,
    pub series_slug: String,
    pub query: String,
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    pub task_type: Option<String>,
    pub volume: Option<String>,
    pub chapter: Option<String>,
    #[serde(default = "ten")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FeedbackInput {
    pub session_id: i64,
    pub correction_text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DreamInput {
    pub provider: Option<String>,
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecallFeedbackInput {
    pub session_id: i64,
    #[serde(default)]
    pub useful: Vec<i64>,
    #[serde(default)]
    pub miss: Vec<i64>,
}

pub fn tool_catalog() -> Vec<ToolContract> {
    const COMPATIBILITY: &str = "Compatibility wrapper. New workflows should use concept, facet, short-term memory, and rule-crystal primitives.";
    let mut tools = vec![
        contract::<NoArgs>(
            "hieronymus_status",
            "Report MCP adapter mode and discovered local service status.",
        ),
        contract::<SeriesCreateInput>(
            "hieronymus_series_create",
            "Create or update a language-neutral series.",
        ),
        contract::<SeriesCreateInput>("hieronymus_series_init", COMPATIBILITY),
        contract::<NoArgs>(
            "hieronymus_series_list",
            "List registered series with language tags.",
        ),
        contract::<SeriesTagsInput>(
            "hieronymus_series_set_language_tags",
            "Replace language tags for a series without changing compatibility fields.",
        ),
        contract::<ConceptListInput>(
            "hieronymus_concept_list",
            "List concepts with optional status, tag, and series-scope filters.",
        ),
        contract::<ConceptIdInput>("hieronymus_concept_get", "Get one concept by id."),
        contract::<ConceptCreateInput>("hieronymus_concept_create", "Create a concept primitive."),
        contract::<ConceptUpdateInput>(
            "hieronymus_concept_update",
            "Update concept mutable metadata.",
        ),
        contract::<ConceptArchiveInput>(
            "hieronymus_concept_archive",
            "Archive a concept so recall and strict rule logic stop using it.",
        ),
        contract::<ConceptMergeInput>(
            "hieronymus_concept_merge",
            "Merge one concept into another active concept.",
        ),
        contract::<ConceptRenameInput>(
            "hieronymus_concept_rename",
            "Rename a concept while retaining the old name as a former-label facet.",
        ),
        contract::<FacetAddInput>(
            "hieronymus_concept_facet_add",
            "Add a multilingual concept facet.",
        ),
        contract::<FacetUpdateInput>(
            "hieronymus_concept_facet_update",
            "Update a concept facet primitive.",
        ),
        contract::<ConceptIdInput>(
            "hieronymus_concept_facet_list",
            "List active facets for a concept.",
        ),
        contract::<FacetCanonicalInput>(
            "hieronymus_concept_facet_set_canonical",
            "Set one concept facet as canonical for its concept.",
        ),
        contract::<ConceptTagsInput>(
            "hieronymus_concept_semantic_tags_set",
            "Replace concept semantic tags.",
        ),
        contract::<CrystalLinkInput>(
            "hieronymus_crystal_link_concept",
            "Link a long-term crystal to a concept.",
        ),
        contract::<CrystalScopesInput>(
            "hieronymus_crystal_story_scopes_set",
            "Replace crystal story scopes.",
        ),
        contract::<CrystalTagsInput>(
            "hieronymus_crystal_semantic_tags_set",
            "Replace crystal semantic tags.",
        ),
        contract::<RuleListInput>(
            "hieronymus_rule_crystals_list",
            "List rule crystals for review.",
        ),
        contract::<CrystalIdInput>("hieronymus_rule_crystal_archive", "Archive a rule crystal."),
        contract::<CrystalIdInput>(
            "hieronymus_rule_crystal_validate",
            "Validate rule-crystal shape and deterministic enforceability.",
        ),
        contract::<ContextTextInput>("hieronymus_termbase_contract", COMPATIBILITY),
        contract::<ValidateTextInput>("hieronymus_termbase_validate", COMPATIBILITY),
        contract::<TermProposeInput>("hieronymus_termbase_propose", COMPATIBILITY),
        contract::<TermApproveInput>("hieronymus_termbase_approve", COMPATIBILITY),
        contract::<MemorySearchInput>("hieronymus_memory_search", COMPATIBILITY),
        contract::<RagImportInput>(
            "hieronymus_rag_import",
            "Import a text, markdown, or glossary file into the project RAG store.",
        ),
        contract::<RagSearchInput>(
            "hieronymus_rag_search",
            "Search project RAG chunks for a series.",
        ),
        contract::<MemoryAddInput>("hieronymus_memory_add", COMPATIBILITY),
        contract::<SessionStartInput>(
            "hieronymus_session_start",
            "Start an agent workflow session for a translation context.",
        ),
        contract::<SessionIdInput>(
            "hieronymus_session_complete",
            "Complete an agent workflow session so it can be dreamed.",
        ),
        contract::<ShortAddInput>(
            "hieronymus_short_term_add",
            "Add a short-term memory to an active session.\n\nsource_role is an optional freeform provenance label. It does not control dreaming's\ncrystal type or confidence; use source_credibility and rule_intent for those signals.",
        ),
        contract::<ShortBatchInput>(
            "hieronymus_short_term_add_batch",
            "Atomically add up to 500 short-term memories to one active session.\n\nEach item requires kind and text. source_role is optional freeform provenance metadata and\ndefaults to agent; it does not control dreaming's categorization or confidence.",
        ),
        contract::<RecallInput>(
            "hieronymus_recall",
            "Recall long-term crystals and active short-term memories for a stored session.",
        ),
        contract::<FeedbackInput>(
            "hieronymus_feedback",
            "Record user correction feedback as short-term memory.",
        ),
        contract::<DreamInput>(
            "hieronymus_dream",
            "Run dreaming over all pending completed-session memories.",
        ),
        contract::<NoArgs>("hieronymus_concept_proposals_list", COMPATIBILITY),
        contract::<RecallFeedbackInput>(
            "hieronymus_recall_feedback",
            "After acting on hieronymus_recall results, report crystal ids as {useful: [...], miss: [...]} to record activation outcomes.",
        ),
    ];
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    tools
}

fn contract<T: JsonSchema>(name: &str, description: &str) -> ToolContract {
    ToolContract {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: serde_json::to_value(schema_for!(T)).expect("JSON Schema should serialize"),
    }
}

fn yes() -> bool {
    true
}
fn default_confidence() -> f64 {
    0.2
}
fn candidate() -> String {
    "candidate".into()
}
fn global() -> String {
    "global".into()
}
fn mentions() -> String {
    "mentions".into()
}
fn fifty() -> usize {
    50
}
fn five() -> usize {
    5
}
fn ten() -> usize {
    10
}
fn three() -> i64 {
    3
}
fn auto() -> String {
    "auto".into()
}
fn translation() -> String {
    "translation".into()
}
fn agent() -> String {
    "agent".into()
}
fn observation() -> String {
    "observation".into()
}
fn empty_object() -> Value {
    serde_json::json!({})
}
