//! The memory tool family (plan M2): `hieronymus_memory_add`,
//! `hieronymus_memory_search`, `hieronymus_short_term_add`,
//! `hieronymus_short_term_add_batch`, `hieronymus_feedback`,
//! `hieronymus_recall`, `hieronymus_rag_import`, and `hieronymus_rag_search`.
//!
//! Argument structs decode exactly the frozen `input_schema` rules from
//! `compatibility/snapshots/mcp.json` (required fields, defaults, and null
//! rules); decoding failures are [`AppError::Invalid`]. Store rejections are
//! [`AppError::Domain`]. All database work stays in the `hieronymus` store
//! APIs — no SQL lives here.
//!
//! ADR 0011 shapes `hieronymus_recall`: the transport DTO serializes
//! `{recall_id, deterministic_contract, results, warnings}` — the normative
//! `results` key flattens the library's ranked hits, while the deterministic
//! contract is the section computed before any lane fusion and is serialized
//! whole (even when `limit` removed every advisory hit).
//!
//! Task C5 shapes the two retrieval tools' relationship to required
//! semantics: `hieronymus_rag_search` IS semantic RAG search, so it refuses a
//! semantic service that cannot serve rather than answering with the lexical
//! lane ([`Application::search_rag`]), while `hieronymus_recall` keeps serving
//! memory and deterministic terminology and reports the gap through
//! `warnings`. The legacy
//! `hieronymus_memory_add`/`hieronymus_memory_search` wrappers keep their
//! Python semantics: short-term session storage and legacy entry rows, not
//! crystal writes.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use hieronymus::crystals::CrystalStore;
use hieronymus::memory_models::{MetadataMap, ShortTermMemoryRecord};
use hieronymus::rag::{RagImport, RagStore};
use hieronymus::recall::{RecallHit, RecallResponse};
use hieronymus::registry::{Registry, Series};
use hieronymus::short_memory::search_expression;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};

use crate::daemon::semantic_worker::EMPTY_CORPUS_JOB_ID;

use super::AppError;
use super::Application;
use super::decode;
use super::domain;
use super::translation_context;

/// Python `MemoryStore._MAX_SEARCH_LIMIT`.
const LEGACY_SEARCH_LIMIT: usize = 50;
/// Python `MemoryStore._LEGACY_RECALL_OVERFETCH_FACTOR`.
const LEGACY_RECALL_OVERFETCH_FACTOR: usize = 2;
/// Python `_SHORT_TERM_REASON`.
const SHORT_TERM_REASON: &str = "active session short-term memory match";

/// The family dispatcher: `None` means the tool is not ours.
pub(crate) fn dispatch(
    application: &Application,
    tool: &str,
    arguments: &Value,
    actor: &str,
) -> Option<Result<Value, AppError>> {
    let _ = actor; // No tool in this family is actor-scoped yet.
    match tool {
        "hieronymus_memory_add" => Some(memory_add(application, arguments)),
        "hieronymus_memory_search" => Some(memory_search(application, arguments)),
        "hieronymus_short_term_add" => Some(short_term_add(application, arguments)),
        "hieronymus_short_term_add_batch" => Some(short_term_add_batch(application, arguments)),
        "hieronymus_feedback" => Some(feedback(application, arguments)),
        "hieronymus_recall" => Some(recall(application, arguments)),
        "hieronymus_rag_import" => Some(rag_import(application, arguments)),
        "hieronymus_rag_search" => Some(rag_search(application, arguments)),
        _ => None,
    }
}

fn workspace(application: &Application) -> Result<WorkspaceStore, AppError> {
    WorkspaceStore::open(application.config()).map_err(domain)
}

/// The Python `_series_context` rule: the registry is opened per request and
/// an unknown slug is a domain rejection. Shared with the terms family.
pub(crate) fn series_context(
    application: &Application,
    series_slug: &str,
) -> Result<Series, AppError> {
    let registry = Registry::open(application.config()).map_err(domain)?;
    registry.get_series(series_slug).map_err(domain)
}

// ------------------------------------------------------- hieronymus_memory_add

#[derive(Deserialize)]
struct MemoryAdd {
    series_slug: String,
    kind: String,
    text: String,
    #[serde(default)]
    source_ref: String,
    #[serde(default = "default_importance")]
    importance: i64,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
}

fn default_importance() -> i64 {
    3
}

/// The legacy memory wrapper: user-authored material lands as a short-term
/// memory on the series' default session (`storage: short_term`), never as a
/// crystal write.
fn memory_add(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<MemoryAdd>(arguments)?;
    if args.kind.trim().is_empty() {
        return Err(AppError::Domain("kind must not be empty".to_string()));
    }
    let series = series_context(application, &args.series_slug)?;
    let context = translation_context(
        &series,
        args.source_language,
        args.target_language,
        "translation",
        "",
        "",
    )?;
    let store = workspace(application)?;
    let session = match store.active_default_session(&context).map_err(domain)? {
        Some(session) => session,
        None => store.start_session(&context).map_err(domain)?,
    };
    let kind = if args.kind == "rule" || args.kind == "correction" {
        "correction"
    } else {
        "note"
    };
    let metadata = MetadataMap::from([
        ("legacy_kind".to_string(), json!(args.kind)),
        ("importance".to_string(), json!(args.importance)),
    ]);
    let input = ShortTermMemoryInput {
        source_role: "user".to_string(),
        kind: kind.to_string(),
        text: args.text,
        source_ref: args.source_ref,
        metadata: Some(metadata),
        ..ShortTermMemoryInput::default()
    };
    let record = store
        .add_short_term_memory(session.id, &input)
        .map_err(domain)?;
    Ok(json!({"memory_id": record.id, "storage": "short_term"}))
}

// ---------------------------------------------------- hieronymus_memory_search

#[derive(Deserialize)]
struct MemorySearch {
    series_slug: String,
    query: String,
    #[serde(default = "default_search_limit")]
    limit: i64,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
}

fn default_search_limit() -> i64 {
    5
}

/// One legacy `MemoryEntry` row: `{id, kind, text, importance, source_ref}`
/// with the long-term provenance needed for id disambiguation.
struct LegacyEntry {
    id: i64,
    kind: String,
    text: String,
    importance: i64,
    source_ref: String,
    long_term: bool,
}

/// Python `_importance_from_metadata`: anything but a numeric importance is
/// the default 3; floats round.
fn importance_from_metadata(metadata: &MetadataMap) -> i64 {
    match metadata.get("importance") {
        Some(value) => match value.as_i64() {
            Some(importance) => importance,
            None => value
                .as_f64()
                .map(|importance| importance.round() as i64)
                .unwrap_or(3),
        },
        None => 3,
    }
}

/// Python `_entry_from_recall_result` short-term half: the legacy kind comes
/// from the memory metadata (`legacy_kind`) when present.
fn legacy_kind_for_memory(memory: &ShortTermMemoryRecord) -> String {
    match memory.metadata.get("legacy_kind") {
        Some(Value::String(legacy_kind)) if !legacy_kind.is_empty() => legacy_kind.clone(),
        _ => memory.kind.clone(),
    }
}

/// Python `_sort_legacy_entries`: importance descending, then smallest
/// absolute id, then real (non-negative) ids first.
fn sort_legacy_entries(entries: &mut [LegacyEntry]) {
    entries.sort_by(|left, right| {
        right
            .importance
            .cmp(&left.importance)
            .then(left.id.abs().cmp(&right.id.abs()))
            .then(left.id.is_negative().cmp(&right.id.is_negative()))
    });
}

/// Python `_disambiguate_entry_ids` + `_sort_legacy_entries` + truncation:
/// long-term ids colliding with any other entry become negative so both rows
/// stay addressable.
fn finish_legacy_entries(mut entries: Vec<LegacyEntry>, limit: usize) -> Value {
    let mut id_counts: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for entry in &entries {
        *id_counts.entry(entry.id).or_default() += 1;
    }
    for entry in &mut entries {
        if entry.long_term && id_counts.get(&entry.id).copied().unwrap_or(0) > 1 {
            entry.id = -entry.id.abs();
        }
    }
    sort_legacy_entries(&mut entries);
    Value::Array(
        entries
            .into_iter()
            .take(limit)
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "kind": entry.kind,
                    "text": entry.text,
                    "importance": entry.importance,
                    "source_ref": entry.source_ref,
                })
            })
            .collect(),
    )
}

/// The legacy memory search over one series: recall through the active
/// default session when one exists (dropping advisory RAG rows, keeping the
/// legacy entry shape), otherwise the direct context-scoped fallback.
fn memory_search(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<MemorySearch>(arguments)?;
    if args.limit < 1 {
        return Err(AppError::Domain("limit must be at least 1".to_string()));
    }
    if search_expression(&args.query).is_empty() {
        return Ok(json!([]));
    }
    let bounded_limit = (args.limit as usize).min(LEGACY_SEARCH_LIMIT);
    let series = series_context(application, &args.series_slug)?;
    let context = super::translation_context(
        &series,
        args.source_language,
        args.target_language,
        "translation",
        "",
        "",
    )?;
    let store = workspace(application)?;

    let entries: Vec<LegacyEntry> = match store.active_default_session(&context).map_err(domain)? {
        Some(session) => {
            let response = application
                .recall()
                .recall(
                    session.id,
                    &context,
                    &args.query,
                    bounded_limit * LEGACY_RECALL_OVERFETCH_FACTOR,
                )
                .map_err(domain)?;
            response
                .hits
                .iter()
                .filter(|hit| hit.source() != "rag")
                .map(|hit| match hit {
                    RecallHit::LongTerm { crystal, .. } => LegacyEntry {
                        id: crystal.id,
                        kind: if crystal.title.is_empty() {
                            crystal.crystal_type.clone()
                        } else {
                            crystal.title.clone()
                        },
                        text: crystal.text.clone(),
                        importance: (crystal.strength * 5.0).round() as i64,
                        source_ref: String::new(),
                        long_term: true,
                    },
                    RecallHit::ShortTerm { memory, .. } => LegacyEntry {
                        id: memory.id,
                        kind: legacy_kind_for_memory(memory),
                        text: memory.text.clone(),
                        importance: importance_from_metadata(&memory.metadata),
                        source_ref: memory.source_ref.clone(),
                        long_term: false,
                    },
                    RecallHit::Rag { .. } => {
                        unreachable!("RAG rows are filtered out above")
                    }
                })
                .collect()
        }
        None => {
            let memories = store
                .search_short_term_memories_for_context(&context, &args.query, bounded_limit)
                .map_err(domain)?;
            let mut entries: Vec<LegacyEntry> = memories
                .iter()
                .map(|memory| LegacyEntry {
                    id: memory.id,
                    kind: legacy_kind_for_memory(memory),
                    text: memory.text.clone(),
                    importance: importance_from_metadata(&memory.metadata),
                    source_ref: memory.source_ref.clone(),
                    long_term: false,
                })
                .collect();
            let crystals = CrystalStore::open(application.config())
                .map_err(domain)?
                .search_active(&context, &args.query, bounded_limit)
                .map_err(domain)?;
            entries.extend(crystals.iter().map(|crystal| LegacyEntry {
                id: crystal.id,
                kind: if crystal.title.is_empty() {
                    crystal.crystal_type.clone()
                } else {
                    crystal.title.clone()
                },
                text: crystal.text.clone(),
                importance: (crystal.strength * 5.0).round() as i64,
                source_ref: String::new(),
                long_term: true,
            }));
            entries
        }
    };
    Ok(finish_legacy_entries(entries, bounded_limit))
}

// -------------------------------------------------- hieronymus_short_term_add

#[derive(Deserialize)]
struct ShortTermAdd {
    session_id: i64,
    kind: String,
    text: String,
    #[serde(default = "default_source_role")]
    source_role: String,
    #[serde(default)]
    source_ref: String,
    #[serde(default)]
    metadata: Option<MetadataMap>,
    #[serde(default)]
    language_tags: Option<Vec<String>>,
    #[serde(default)]
    story_scopes: Option<Vec<String>>,
    #[serde(default)]
    semantic_tags: Option<Vec<String>>,
    #[serde(default = "default_credibility")]
    source_credibility: String,
    #[serde(default)]
    rule_intent: String,
    #[serde(default)]
    soft_origin: String,
}

fn default_source_role() -> String {
    "agent".to_string()
}

fn default_credibility() -> String {
    "observation".to_string()
}

impl ShortTermAdd {
    fn into_input(self) -> ShortTermMemoryInput {
        ShortTermMemoryInput {
            source_role: self.source_role,
            kind: self.kind,
            text: self.text,
            source_ref: self.source_ref,
            metadata: self.metadata,
            language_tags: self.language_tags.unwrap_or_default(),
            story_scopes: self.story_scopes.unwrap_or_default(),
            semantic_tags: self.semantic_tags.unwrap_or_default(),
            source_credibility: self.source_credibility,
            rule_intent: self.rule_intent,
            soft_origin: self.soft_origin,
        }
    }
}

fn short_term_add(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ShortTermAdd>(arguments)?;
    let store = workspace(application)?;
    let record = store
        .add_short_term_memory(args.session_id, &args.into_input())
        .map_err(domain)?;
    Ok(json!({"memory_id": record.id}))
}

// --------------------------------------------- hieronymus_short_term_add_batch

#[derive(Deserialize)]
struct ShortTermAddBatch {
    session_id: i64,
    items: Vec<BatchItem>,
}

/// One batch item: the same shape as a single short-term add, minus the
/// session (the batch is session-scoped).
#[derive(Deserialize)]
struct BatchItem {
    kind: String,
    text: String,
    #[serde(default = "default_source_role")]
    source_role: String,
    #[serde(default)]
    source_ref: String,
    #[serde(default)]
    metadata: Option<MetadataMap>,
    #[serde(default)]
    language_tags: Option<Vec<String>>,
    #[serde(default)]
    story_scopes: Option<Vec<String>>,
    #[serde(default)]
    semantic_tags: Option<Vec<String>>,
    #[serde(default = "default_credibility")]
    source_credibility: String,
    #[serde(default)]
    rule_intent: String,
    #[serde(default)]
    soft_origin: String,
}

impl BatchItem {
    fn into_input(self) -> ShortTermMemoryInput {
        ShortTermMemoryInput {
            source_role: self.source_role,
            kind: self.kind,
            text: self.text,
            source_ref: self.source_ref,
            metadata: self.metadata,
            language_tags: self.language_tags.unwrap_or_default(),
            story_scopes: self.story_scopes.unwrap_or_default(),
            semantic_tags: self.semantic_tags.unwrap_or_default(),
            source_credibility: self.source_credibility,
            rule_intent: self.rule_intent,
            soft_origin: self.soft_origin,
        }
    }
}

/// Atomic batch insertion: every item is validated before anything is
/// written, and the store inserts the whole batch in one transaction — a
/// single rejection leaves no partial memories behind.
fn short_term_add_batch(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<ShortTermAddBatch>(arguments)?;
    let store = workspace(application)?;
    let inputs: Vec<ShortTermMemoryInput> = args
        .items
        .into_iter()
        .map(|item| item.into_input())
        .collect();
    let records = store
        .add_short_term_memories_batch(args.session_id, &inputs)
        .map_err(domain)?;
    let memory_ids: Vec<i64> = records.iter().map(|record| record.id).collect();
    Ok(json!({"memory_ids": memory_ids, "count": memory_ids.len()}))
}

// -------------------------------------------------------- hieronymus_feedback

#[derive(Deserialize)]
struct Feedback {
    session_id: i64,
    correction_text: String,
}

/// The correction-text feedback tool: user correction prose recorded as a
/// short-term memory. This is distinct from the correlated recall-outcome
/// feedback (`POST /recall/feedback` + `hiero recall-feedback`), which
/// addresses a `recall_id` by activation ids.
fn feedback(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<Feedback>(arguments)?;
    let store = workspace(application)?;
    let input = ShortTermMemoryInput {
        source_role: "user".to_string(),
        kind: "correction".to_string(),
        text: args.correction_text,
        ..ShortTermMemoryInput::default()
    };
    let record = store
        .add_short_term_memory(args.session_id, &input)
        .map_err(domain)?;
    Ok(json!({"memory_id": record.id}))
}

// ---------------------------------------------------------- hieronymus_recall

#[derive(Deserialize)]
struct RecallArgs {
    session_id: i64,
    series_slug: String,
    query: String,
    #[serde(default)]
    source_language: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    task_type: Option<String>,
    #[serde(default)]
    volume: Option<String>,
    #[serde(default)]
    chapter: Option<String>,
    #[serde(default = "default_recall_limit")]
    limit: i64,
}

fn default_recall_limit() -> i64 {
    10
}

/// Recall for a stored session: the session's own context rules (cross-series
/// arguments and override mismatches are rejections), and the ADR 0011 DTO —
/// `{recall_id, deterministic_contract, results, warnings}` where the
/// contract is returned whole even when `limit` removed all advisory hits.
///
/// Mixed recall never hard-fails on missing semantics (that is the strict
/// `hieronymus_rag_search` contract instead): memory rows and the
/// deterministic terminology contract still run and still return. What it
/// does do since task C5 is say so — `warnings` carries
/// `semantic_lane_unavailable` whenever required semantics did not run over
/// the current corpus, whether the lane is unarmed, degraded, or the shared
/// semantic service reports it cannot serve yet.
fn recall(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<RecallArgs>(arguments)?;
    let series = series_context(application, &args.series_slug)?;
    let store = workspace(application)?;
    let session = store.get_session(args.session_id).map_err(domain)?;
    let context = &session.context;
    if context.series_slug != series.slug {
        return Err(AppError::Domain("session context mismatch".to_string()));
    }
    let overrides: [(&str, Option<String>, &String); 5] = [
        (
            "source_language",
            args.source_language,
            &context.source_language,
        ),
        (
            "target_language",
            args.target_language,
            &context.target_language,
        ),
        ("task_type", args.task_type, &context.task_type),
        ("volume", args.volume, &context.volume),
        ("chapter", args.chapter, &context.chapter),
    ];
    for (field_name, override_value, context_value) in overrides {
        if let Some(value) = override_value
            && &value != context_value
        {
            return Err(AppError::Domain(format!(
                "session context mismatch: {field_name}"
            )));
        }
    }
    if args.limit < 1 {
        return Err(AppError::Domain("limit must be at least 1".to_string()));
    }
    let response = application
        .recall()
        .recall(args.session_id, context, &args.query, args.limit as usize)
        .map_err(domain)?;
    Ok(recall_payload(&response))
}

/// The transport DTO (ADR 0011): the internal hit field stays `hits`; the
/// normative JSON key is `results`. Every contract term is serialized, even
/// when the ranked list is empty.
fn recall_payload(response: &RecallResponse) -> Value {
    json!({
        "recall_id": response.recall_id,
        "deterministic_contract": response.deterministic_contract,
        "results": response
            .hits
            .iter()
            .enumerate()
            .map(|(index, hit)| recall_result_row(hit, index + 1))
            .collect::<Vec<Value>>(),
        "warnings": response.warnings,
    })
}

/// Flatten one ranked hit to the agreed results row: tier, stable id,
/// activation id where relevant, score/rank/reason, text, scopes/tags,
/// concept references, credibility, and rule/thought markers.
fn recall_result_row(hit: &RecallHit, rank: usize) -> Value {
    let mut row = json!({
        "tier": hit.source(),
        "rank": rank,
        "score": hit.score(),
    });
    match hit {
        RecallHit::LongTerm {
            crystal,
            reason,
            activation_id,
            ..
        } => {
            row["id"] = json!(crystal.id);
            row["activation_id"] = json!(activation_id);
            row["title"] = json!(if crystal.title.is_empty() {
                crystal.crystal_type.clone()
            } else {
                crystal.title.clone()
            });
            row["kind"] = json!(crystal.crystal_type);
            row["crystal_type"] = json!(crystal.crystal_type);
            row["text"] = json!(crystal.text);
            row["rank_reason"] = json!(reason);
            row["language_tags"] = json!(crystal.language_tags);
            row["story_scopes"] = json!(crystal.story_scopes);
            row["semantic_tags"] = json!(crystal.semantic_tags);
            row["concept_ids"] = json!(crystal.concept_ids);
            row["source_credibility"] = json!(crystal.source_credibility);
            row["rule_intent"] = json!(crystal.rule_intent);
            row["is_rule"] = json!(crystal.crystal_type == "rule" && crystal.status == "active");
            row["is_thought"] = json!(crystal.crystal_type == "thought" || crystal.is_inferred);
        }
        RecallHit::ShortTerm { memory, .. } => {
            row["id"] = json!(memory.id);
            row["activation_id"] = Value::Null;
            row["title"] = json!(memory.kind);
            row["kind"] = json!(memory.kind);
            row["crystal_type"] = Value::Null;
            row["text"] = json!(memory.text);
            row["rank_reason"] = json!(SHORT_TERM_REASON);
            row["language_tags"] = json!(memory.language_tags);
            row["story_scopes"] = json!(memory.story_scopes);
            row["semantic_tags"] = json!(memory.semantic_tags);
            row["concept_ids"] = json!([] as [i64; 0]);
            row["source_credibility"] = json!(memory.source_credibility);
            row["rule_intent"] = json!(memory.rule_intent);
            row["is_rule"] = json!(!memory.rule_intent.trim().is_empty());
            row["is_thought"] = json!(memory.source_credibility == "thought");
        }
        RecallHit::Rag {
            chunk,
            reason,
            conflicts_with_rule_ids,
            ..
        } => {
            row["id"] = json!(chunk.id);
            row["activation_id"] = Value::Null;
            row["title"] = json!(chunk.title());
            row["kind"] = json!(chunk.kind());
            row["crystal_type"] = Value::Null;
            row["text"] = json!(chunk.text);
            row["rank_reason"] = json!(reason);
            row["language_tags"] = json!(chunk.language_tags);
            row["story_scopes"] = json!(chunk.story_scopes);
            row["semantic_tags"] = json!(chunk.semantic_tags);
            row["concept_ids"] = json!([] as [i64; 0]);
            row["source_credibility"] = json!("");
            row["rule_intent"] = json!("");
            row["is_rule"] = json!(false);
            row["is_thought"] = json!(false);
            row["conflicts_with_rule_ids"] = json!(conflicts_with_rule_ids);
            row["source_ref"] = json!(chunk.source_ref);
            row["chunk_kind"] = json!(chunk.chunk_kind);
            row["location"] = json!(chunk.location);
            row["metadata"] = json!(chunk.metadata);
        }
    }
    row
}

// ------------------------------------------------------- hieronymus_rag_import

#[derive(Deserialize)]
struct RagImportArgs {
    series_slug: String,
    path: String,
    #[serde(default)]
    source_ref: Option<String>,
    #[serde(default = "default_source_type")]
    source_type: String,
    #[serde(default)]
    language_tags: Option<Vec<String>>,
    #[serde(default)]
    story_scopes: Option<Vec<String>>,
    #[serde(default)]
    semantic_tags: Option<Vec<String>>,
}

fn default_source_type() -> String {
    "auto".to_string()
}

/// The three honest answers to "is the new text semantically indexed?".
/// Serialized as `semantic_indexing` so no caller has to infer it from the
/// shape of another field.
const INDEXING_QUEUED: &str = "queued";
const INDEXING_OWED: &str = "owed";
const INDEXING_NOT_REQUIRED: &str = "not-required";

/// Import a text/markdown/glossary file through the RAG store: the store
/// resolves the actual parser and source type from the file, persists chunks
/// with their typed tags, and treats identical checksum re-imports as
/// metadata refreshes. Unsupported types are clean tool errors.
///
/// The authoritative transaction also records the corpus-revision bump and the
/// durable semantic work intent (`RagStore::import_file`), so the owed
/// indexing is committed with the chunks that owe it. Queueing the rebuild
/// through the daemon's semantic controller afterwards is a best-effort
/// *notification* — a fast path, not the record.
///
/// Task C4 (review finding A4) is about what this call then TELLS the caller.
/// The pre-C4 payload put an enqueue failure into `semantic_rebuild_job` as
/// `{"error": ...}` on an otherwise-successful result, which reads as success
/// to anything that does not inspect the field's type — the exact shape that
/// let unindexed text look indexed. Now `semantic_indexing` states the outcome
/// outright: `queued` (the rebuild is in the durable queue, `semantic_rebuild_job`
/// carries its id), `owed` (the intent is durably recorded but this call could
/// not queue it — no daemon owns this application, or the enqueue failed with
/// `semantic_indexing_error`; reconciliation will queue it), or `not-required`
/// (there is no indexing to do — an identical-checksum metadata refresh
/// changed no indexable text, or there were no chunks to index at all).
///
/// `semantic_rebuild_job` is a job id or `null`, never anything else, and it
/// is a string exactly when `semantic_indexing` is `queued`. The controller's
/// internal `rebuild:empty-corpus` marker is therefore mapped to
/// `not-required` with a `null` job rather than leaked: it is the worker's own
/// no-op signal, not a durable job a caller could ever look up.
fn rag_import(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<RagImportArgs>(arguments)?;
    let import = RagImport {
        source_ref: args.source_ref,
        source_type: args.source_type,
        language_tags: args.language_tags.unwrap_or_default(),
        story_scopes: args.story_scopes.unwrap_or_default(),
        semantic_tags: args.semantic_tags.unwrap_or_default(),
    };
    let result = RagStore::open(application.config())
        .map_err(domain)?
        .import_file(&args.series_slug, Path::new(&args.path), &import)
        .map_err(domain)?;
    // Notify only when indexable text actually changed. The gate is exactly
    // the one the store used to bump the corpus revision: a skipped
    // (identical-checksum) import refreshed metadata tags only, everything
    // else moved the revision.
    //
    // The pre-C4 gate also tested `chunk_count == 0`, which was dead code:
    // `load_rag_file` rejects a source that parses to zero chunks before the
    // import transaction is even opened, so a committed non-skipped import
    // always added at least one chunk.
    let (rebuild_job, indexing, indexing_error) = if result.skipped {
        (Value::Null, INDEXING_NOT_REQUIRED, None)
    } else {
        match application.request_rebuild(&args.series_slug) {
            // The controller found no chunks to index and answered with its
            // internal no-op marker, which is not a durable job id and must
            // never be handed to a caller as one. Unreachable through this
            // path today (see above), so this is the belt-and-braces half of
            // the "a job id or null, never anything else" contract rather than
            // a scenario — if the corpus can ever be emptied out from under an
            // import, the payload stays honest instead of advertising a job
            // nobody can look up.
            Some(Ok(job_id)) if job_id == EMPTY_CORPUS_JOB_ID => {
                (Value::Null, INDEXING_NOT_REQUIRED, None)
            }
            Some(Ok(job_id)) => (json!(job_id), INDEXING_QUEUED, None),
            // The import is committed and the intent is durable; what must not
            // happen is reporting this as a queued job.
            Some(Err(error)) => (Value::Null, INDEXING_OWED, Some(error)),
            None => (Value::Null, INDEXING_OWED, None),
        }
    };
    let mut payload = json!({
        "source_id": result.source.id,
        "series_slug": result.source.series_slug,
        "source_ref": result.source.source_ref,
        "source_type": result.source.source_type,
        "content_type": result.source.content_type,
        "checksum": result.source.checksum,
        "metadata": result.source.metadata,
        "chunk_count": result.chunk_count,
        "skipped": result.skipped,
        "normalized_path": result.normalized_path,
        "normalized_format": result.normalized_format,
        "semantic_rebuild_job": rebuild_job,
        "semantic_indexing": indexing,
    });
    if let Some(error) = indexing_error {
        payload["semantic_indexing_error"] = json!(error);
    }
    Ok(payload)
}

// ------------------------------------------------------- hieronymus_rag_search

#[derive(Deserialize)]
struct RagSearchArgs {
    series_slug: String,
    query: String,
    #[serde(default = "default_rag_search_limit")]
    limit: i64,
}

fn default_rag_search_limit() -> i64 {
    10
}

/// The public semantic RAG search tool: argument decoding plus the strict
/// semantic gate, then the shared hybrid retrieval in
/// [`Application::search_rag`].
///
/// Argument validation stays ahead of the gate so a malformed call keeps its
/// frozen diagnostic ("limit must be at least 1") whatever the semantic
/// service is doing.
fn rag_search(application: &Application, arguments: &Value) -> Result<Value, AppError> {
    let args = decode::<RagSearchArgs>(arguments)?;
    if args.limit < 1 {
        return Err(AppError::Domain("limit must be at least 1".to_string()));
    }
    application.search_rag(&args.series_slug, &args.query, args.limit as usize)
}

impl Application {
    /// Semantic (hybrid) RAG search over one series, with NO synthetic task
    /// session: the shared armed semantic lane fused with the FTS chunk lane
    /// by reciprocal rank, exactly as `hieronymus_recall` fuses them
    /// (`RecallService::search_series`).
    ///
    /// Task C5 (review finding A5) is about what this call refuses to do.
    /// Before it, `hieronymus_rag_search` opened `RagStore` and ran the
    /// lexical FTS query directly: it neither consumed the semantic lane nor
    /// required readiness, so a missing, unconfigured, or failed semantic
    /// runtime produced ordinary successful search results and the caller had
    /// no way to tell that the semantic half never ran. Since both working
    /// memory and semantic RAG are mandatory — FTS-only is not a completion
    /// or release alternative — that is exactly the shape of degradation that
    /// must never be silent.
    ///
    /// So the required service is gated first
    /// (`Application::require_semantic_service`): a service that is absent,
    /// acquiring, rebuilding, or failed is an error carrying its own reason,
    /// not a lexical answer. A ready service with an empty series is a
    /// success with zero rows (ready-for-ingest), and a ready service whose
    /// lane then cannot execute over a series that HAS chunks is an error
    /// too.
    ///
    /// The response stays the bare row array the frozen `outputSchema`
    /// describes; the accepted delta is what the rows now mean — see
    /// `compatibility/rust/rag-search-v2.json`.
    pub fn search_rag(&self, series: &str, query: &str, limit: usize) -> Result<Value, AppError> {
        self.require_semantic_service()?;
        let hits = self
            .recall()
            .search_series(series, query, limit)
            .map_err(domain)?;
        Ok(Value::Array(
            hits.iter()
                .map(|hit| {
                    json!({
                        "source": "rag",
                        "id": hit.chunk.id,
                        "title": hit.chunk.title(),
                        "kind": hit.chunk.kind(),
                        "text": hit.chunk.text,
                        "display_text": hit.chunk.display_text,
                        "source_ref": hit.chunk.source_ref,
                        "chunk_kind": hit.chunk.chunk_kind,
                        "location": hit.chunk.location,
                        "metadata": hit.chunk.metadata,
                        "language_tags": hit.chunk.language_tags,
                        "story_scopes": hit.chunk.story_scopes,
                        "semantic_tags": hit.chunk.semantic_tags,
                        "score": hit.score,
                        "rank_reason": hit.reason,
                    })
                })
                .collect(),
        ))
    }
}
