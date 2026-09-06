//! Behavior ported from `tests/test_dreaming.py` and
//! `tests/test_dream_evidence_passes.py`, adapted to the Rust dreaming core:
//! every dream cycle drives the seven evidence passes over one bounded
//! selection, applies one validated mutation batch, and audits inputs,
//! outputs, decisions, and mutations. The Python crystallize-only legacy
//! pipeline is intentionally not ported (see the dreaming design spec).

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{default_dream_config, save_dream_config};
use hieronymus::dream_locks::dream_cycle_lock;
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::dreaming::{DeterministicDreamProvider, DreamError, DreamProvider, DreamService};
use hieronymus::memory_models::{ShortTermMemoryRecord, TranslationContext};
use hieronymus::provider_config::{ProviderCatalog, ProviderProfile, save_provider_catalog};
use hieronymus::registry::Registry;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};

fn config(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path().join("hieronymus"))
}

fn context(series_slug: &str) -> TranslationContext {
    TranslationContext::new(series_slug, "ja", "ru", "translate")
        .volume("1")
        .chapter("2")
}

fn create_series(config: &HieronymusConfig, slug: &str) {
    Registry::open(config)
        .unwrap()
        .create_series(slug, "Only Sense Online", "ja", "ru", None)
        .unwrap();
}

fn add_memory(workspace: &WorkspaceStore, session_id: i64, kind: &str, text: &str) -> i64 {
    workspace
        .add_short_term_memory(session_id, &ShortTermMemoryInput::new(kind, text))
        .unwrap()
        .id
}

fn completed_session_with(
    config: &HieronymusConfig,
    slug: &str,
    build: impl FnOnce(&WorkspaceStore, i64) -> Vec<i64>,
) -> Vec<i64> {
    let workspace = WorkspaceStore::open(config).unwrap();
    let session = workspace.start_session(&context(slug)).unwrap();
    let ids = build(&workspace, session.id);
    workspace.complete_session(session.id).unwrap();
    ids
}

fn completed_session(config: &HieronymusConfig, slug: &str, texts: &[&str]) -> Vec<i64> {
    completed_session_with(config, slug, |workspace, session_id| {
        texts
            .iter()
            .map(|text| add_memory(workspace, session_id, "note", text))
            .collect()
    })
}

fn query(config: &HieronymusConfig, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Vec<Vec<Value>> {
    let connection = open_migrated(&config.database_path()).unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    let column_count = statement.column_count();
    let rows = statement
        .query_map(params, |row| {
            let mut values = Vec::with_capacity(column_count);
            for index in 0..column_count {
                let value: Value = match row.get_ref(index)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(value) => json!(value),
                    rusqlite::types::ValueRef::Real(value) => json!(value),
                    rusqlite::types::ValueRef::Text(text) => {
                        json!(String::from_utf8_lossy(text).into_owned())
                    }
                    rusqlite::types::ValueRef::Blob(blob) => {
                        json!(String::from_utf8_lossy(blob).into_owned())
                    }
                };
                values.push(value);
            }
            Ok(values)
        })
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

fn scalar(config: &HieronymusConfig, sql: &str) -> Value {
    query(config, sql, &[]).remove(0).remove(0)
}

fn phase_rows(config: &HieronymusConfig) -> Vec<(String, String, i64, i64)> {
    query(
        config,
        "select phase, status, input_count, output_count from dream_phase_runs order by id",
        &[],
    )
    .into_iter()
    .map(|row| {
        (
            row[0].as_str().unwrap().to_string(),
            row[1].as_str().unwrap().to_string(),
            row[2].as_i64().unwrap(),
            row[3].as_i64().unwrap(),
        )
    })
    .collect()
}

fn audit_event_summaries(config: &HieronymusConfig, run_id: i64) -> Vec<(String, String)> {
    query(
        config,
        "select event_type, summary from dream_audit_entries where dream_run_id = ?1 order by id",
        &[&run_id],
    )
    .into_iter()
    .map(|row| {
        (
            row[0].as_str().unwrap().to_string(),
            row[1].as_str().unwrap().to_string(),
        )
    })
    .collect()
}

fn last_phase_completed_payload(config: &HieronymusConfig, run_id: i64) -> Value {
    let connection = open_migrated(&config.database_path()).unwrap();
    let mut statement = connection
        .prepare(
            "select payload_json from dream_audit_entries
             where dream_run_id = ?1 and event_type = 'phase_completed'
             order by id desc limit 1",
        )
        .unwrap();
    let payload: String = statement.query_row([run_id], |row| row.get(0)).unwrap();
    serde_json::from_str(&payload).unwrap()
}

fn audit_payloads(config: &HieronymusConfig, run_id: i64, event_type: &str) -> Vec<Value> {
    query(
        config,
        "select payload_json from dream_audit_entries
         where dream_run_id = ?1 and event_type = ?2 order by id",
        &[&run_id, &event_type],
    )
    .into_iter()
    .map(|row| serde_json::from_str(row[0].as_str().unwrap()).unwrap())
    .collect()
}

fn sha256_hex(input: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(input.as_bytes()))
}

// ---------------------------------------------------------------------------
// Test providers
// ---------------------------------------------------------------------------

/// Shared record of (pass name, selected memory ids) per provider call.
type CallLog = Arc<Mutex<Vec<(String, Vec<i64>)>>>;

/// Records every pass call; produces one valid crystal on knowledge_crystals
/// and the coverage list on coverage_audit. Constructed directly around a
/// shared call log so every fresh instance the resolver creates feeds the
/// same record.
struct EvidenceProvider {
    calls: CallLog,
    omit_coverage: bool,
}

impl DreamProvider for EvidenceProvider {
    fn name(&self) -> &str {
        "evidence-test"
    }

    fn run_pass(
        &self,
        pass_name: &str,
        _context: &TranslationContext,
        memories: &[ShortTermMemoryRecord],
    ) -> Result<Value, DreamError> {
        let ids: Vec<i64> = memories.iter().map(|memory| memory.id).collect();
        self.calls
            .lock()
            .unwrap()
            .push((pass_name.to_string(), ids.clone()));
        if pass_name == "coverage_audit" {
            let covered = if self.omit_coverage {
                ids[1..].to_vec()
            } else {
                ids
            };
            return Ok(json!({"covered_memory_ids": covered}));
        }
        if pass_name == "knowledge_crystals" {
            return Ok(json!({
                "crystals": [{
                    "crystal_type": "observation",
                    "title": "Evidence",
                    "text": "The selected memory is important.",
                    "strength": 0.6,
                    "confidence": 0.8,
                    "source_memory_ids": ids,
                }]
            }));
        }
        Ok(json!({}))
    }
}

struct FailingProvider {
    message: &'static str,
}

impl DreamProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    fn run_pass(
        &self,
        _pass_name: &str,
        _context: &TranslationContext,
        _memories: &[ShortTermMemoryRecord],
    ) -> Result<Value, DreamError> {
        Err(DreamError::Provider(self.message.to_string()))
    }
}

/// Emits provider payloads per pass from a fixed table.
struct ScriptedProvider {
    name: &'static str,
    passes: std::collections::HashMap<String, Value>,
}

impl ScriptedProvider {
    fn new(name: &'static str, passes: Vec<(&str, Value)>) -> Self {
        Self {
            name,
            passes: passes
                .into_iter()
                .map(|(pass, value)| (pass.to_string(), value))
                .collect(),
        }
    }
}

impl DreamProvider for ScriptedProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn run_pass(
        &self,
        pass_name: &str,
        _context: &TranslationContext,
        memories: &[ShortTermMemoryRecord],
    ) -> Result<Value, DreamError> {
        if pass_name == "coverage_audit" && !self.passes.contains_key(pass_name) {
            let ids: Vec<i64> = memories.iter().map(|memory| memory.id).collect();
            return Ok(json!({"covered_memory_ids": ids}));
        }
        Ok(self
            .passes
            .get(pass_name)
            .cloned()
            .unwrap_or_else(|| json!({})))
    }
}

/// Serves a fixed prompt and endpoint so the audit contract can be pinned
/// exactly: the request and response audit entries must carry the SHA-256 of
/// the rendered prompt and the endpoint with credentials and query stripped.
struct PromptAuditProvider {
    prompt: &'static str,
    endpoint: &'static str,
}

impl DreamProvider for PromptAuditProvider {
    fn name(&self) -> &str {
        "prompt-audit"
    }

    fn endpoint(&self) -> &str {
        self.endpoint
    }

    fn render_pass_prompt(
        &self,
        _pass_name: &str,
        _context: &TranslationContext,
        _memories: &[ShortTermMemoryRecord],
    ) -> Result<String, DreamError> {
        Ok(self.prompt.to_string())
    }

    fn run_pass(
        &self,
        pass_name: &str,
        _context: &TranslationContext,
        memories: &[ShortTermMemoryRecord],
    ) -> Result<Value, DreamError> {
        let ids: Vec<i64> = memories.iter().map(|memory| memory.id).collect();
        if pass_name == "coverage_audit" {
            return Ok(json!({"covered_memory_ids": ids}));
        }
        if pass_name == "knowledge_crystals" {
            return Ok(json!({
                "crystals": [{
                    "crystal_type": "observation",
                    "title": "Prompt audit",
                    "text": "The audited memory is important.",
                    "strength": 0.6,
                    "confidence": 0.8,
                    "source_memory_ids": ids,
                }]
            }));
        }
        Ok(json!({}))
    }
}

// ---------------------------------------------------------------------------
// Run registry + persistence core
// ---------------------------------------------------------------------------

#[test]
fn dreaming_crystallizes_completed_short_term_memory() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let memory_ids =
        completed_session_with(&config, "only-sense-online", |workspace, session_id| {
            vec![add_memory(
                workspace,
                session_id,
                "correction",
                "Use Сенс, not Чувство, for Sense in UI references.",
            )]
        });

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.cycle_id, 1);
    assert_eq!(run.input_count, 1);
    assert_eq!(run.created_crystal_count, 1);
    let crystal = query(
        &config,
        "select crystal_type, title, text, source_credibility, created_cycle from crystals",
        &[],
    )
    .remove(0);
    assert_eq!(crystal[0], json!("concept"));
    assert_eq!(crystal[1], json!("Correction"));
    assert_eq!(
        crystal[2],
        json!("Use Сенс, not Чувство, for Sense in UI references.")
    );
    assert_eq!(crystal[4], json!(run.cycle_id));
    let sources = query(
        &config,
        "select short_term_memory_id from crystal_sources",
        &[],
    );
    assert_eq!(sources, vec![vec![json!(memory_ids[0])]]);
    let archived = scalar(
        &config,
        "select count(*) from short_term_memories where archived_at is not null",
    );
    assert_eq!(archived, json!(1));
}

#[test]
fn manual_dream_all_drains_small_batch_even_below_minimum() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(&config, "only-sense-online", &["A completed dream input."]);

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let run = service.run_all("admin", true, false).unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.input_count, 1);
    let pending = scalar(
        &config,
        "select count(*) from short_term_memories where archived_at is null",
    );
    assert_eq!(pending, json!(0));
}

#[test]
fn dreaming_ignores_active_sessions() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let workspace = WorkspaceStore::open(&config).unwrap();
    let session = workspace
        .start_session(&context("only-sense-online"))
        .unwrap();
    add_memory(
        &workspace,
        session.id,
        "correction",
        "Active notes should wait.",
    );

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("active"));
    assert_eq!(session_row[1], Value::Null);
}

#[test]
fn dreaming_marks_completed_sessions_as_dreamed_with_cycle() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(
        &config,
        "only-sense-online",
        &["Keep item crafting notes concise."],
    );

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("dreamed"));
    assert_eq!(session_row[1], json!(run.cycle_id));
}

#[test]
fn dreaming_creates_next_cycle_id() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(&config, "only-sense-online", &["First note."]);

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let first_run = service.run_cycle("manual", false).unwrap();
    completed_session(&config, "only-sense-online", &["Second note."]);
    let second_run = service.run_cycle("manual", false).unwrap();

    assert_eq!(first_run.cycle_id, 1);
    assert_eq!(second_run.cycle_id, 2);
}

// ---------------------------------------------------------------------------
// Locking
// ---------------------------------------------------------------------------

#[test]
fn dreaming_rejects_second_cycle_while_lock_is_active() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");

    let held = dream_cycle_lock(&config, "manual").unwrap();
    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    drop(held);

    assert!(
        error.to_string().contains("dream cycle already running"),
        "{error}"
    );
}

#[test]
fn dreaming_releases_lock_after_provider_exception() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(&config, "only-sense-online", &["A completed dream input."]);

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(FailingProvider {
                message: "provider failed",
            })
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(error.to_string().contains("provider failed"), "{error}");

    let retry = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let run = retry.run_cycle("manual", false).unwrap();
    assert_eq!(run.status, "completed");
}

#[test]
fn dreaming_records_locked_skip_without_consuming_cycle_id() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(&config, "only-sense-online", &["A completed dream input."]);

    let held = dream_cycle_lock(&config, "manual").unwrap();
    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let skipped = service.run_cycle("manual", true).unwrap();
    drop(held);

    assert_eq!(skipped.status, "skipped");
    assert_eq!(skipped.cycle_id, -1);
    assert_eq!(skipped.error, "dream cycle already running");

    let run = service.run_cycle("manual", false).unwrap();
    assert_eq!(run.status, "completed");
    assert_eq!(run.cycle_id, 1);
    let rows = query(
        &config,
        "select cycle_id, status from dream_runs order by id",
        &[],
    );
    assert_eq!(
        rows,
        vec![
            vec![json!(-1), json!("skipped")],
            vec![json!(1), json!("completed")]
        ]
    );
}

// ---------------------------------------------------------------------------
// Failed runs leave no partial mutations
// ---------------------------------------------------------------------------

#[test]
fn dreaming_records_failed_run_for_unparsable_provider_output() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(&config, "only-sense-online", &["Valid input."]);

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(ScriptedProvider::new(
                "empty-text",
                vec![(
                    "knowledge_crystals",
                    json!({"crystals": [{"crystal_type": "lesson", "text": ""}]}),
                )],
            ))
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error.to_string().contains("candidate content is required"),
        "{error}"
    );

    let run_row = query(
        &config,
        "select status, error, completed_at from dream_runs",
        &[],
    )
    .remove(0);
    assert_eq!(run_row[0], json!("failed"));
    assert!(run_row[1].as_str().unwrap().contains("candidate content"));
    assert!(!run_row[2].is_null());
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("completed"));
    assert_eq!(session_row[1], Value::Null);
}

#[test]
fn dreaming_records_failed_run_when_provider_returns_non_object() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(&config, "only-sense-online", &["Valid input."]);

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(ScriptedProvider::new(
                "null-pass",
                vec![("knowledge_crystals", json!(null))],
            ))
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error.to_string().contains("output must be an object"),
        "{error}"
    );

    let run_row = query(&config, "select status, completed_at from dream_runs", &[]).remove(0);
    assert_eq!(run_row[0], json!("failed"));
    assert!(!run_row[1].is_null());
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
}

#[test]
fn incomplete_coverage_rolls_back_all_dream_mutations() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let memory_ids = completed_session(&config, "book", &["Conclusion one.", "Conclusion two."]);
    assert_eq!(memory_ids.len(), 2);

    let calls: CallLog = Arc::new(Mutex::new(Vec::new()));
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving({
            let calls = Arc::clone(&calls);
            move || {
                Box::new(EvidenceProvider {
                    calls: Arc::clone(&calls),
                    omit_coverage: true,
                })
            }
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(error.to_string().contains("coverage_incomplete"), "{error}");

    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    let workspace = WorkspaceStore::open(&config).unwrap();
    let session_id: i64 = query(&config, "select id from task_sessions", &[])
        .remove(0)
        .remove(0)
        .as_i64()
        .unwrap();
    assert_eq!(
        workspace
            .list_short_term_memories(session_id)
            .unwrap()
            .len(),
        2,
        "memories must stay pending when coverage is incomplete"
    );
}

#[test]
fn dreaming_applies_the_previously_unsupported_concept_section() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["Valid input."]);

    // Task D2 closed the gap this run used to fail closed on: the
    // `concepts` section now normalizes, applies, and audits cleanly
    // alongside the crystal that was always supported.
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(ScriptedProvider::new(
                "concept-sections",
                vec![(
                    "knowledge_crystals",
                    json!({
                        "crystals": [{
                            "crystal_type": "observation",
                            "title": "Applied crystal",
                            "text": "This crystal applies with its concept.",
                            "confidence": 0.8
                        }],
                        "concepts": [{"name": "Concept application is no longer a later slice"}]
                    }),
                )],
            ))
        }),
    )
    .unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.created_crystal_count, 1);
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(1));
    let concept = query(
        &config,
        "select canonical_name, status, scope_type from concepts",
        &[],
    )
    .remove(0);
    assert_eq!(
        concept[0],
        json!("Concept application is no longer a later slice")
    );
    assert_eq!(concept[1], json!("candidate"));
    assert_eq!(concept[2], json!("global"));
    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("dreamed"));
    let audited_events = query(
        &config,
        "select count(*) from dream_audit_entries where dream_run_id = ?1",
        &[&run.id],
    )
    .remove(0)
    .remove(0);
    assert!(
        audited_events.as_i64().unwrap() > 0,
        "the completed run must keep its audit entries"
    );
}

#[test]
fn dreaming_fails_closed_when_pass_output_exceeds_max_records_per_pass() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["One.", "Two.", "Three."]);

    let mut dream_config = default_dream_config();
    dream_config
        .workflows
        .get_mut("knowledge_crystals")
        .unwrap()
        .max_records_per_pass = 1;
    save_dream_config(&config, &dream_config).unwrap();

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(ScriptedProvider::new(
                "over-pass-cap",
                vec![(
                    "knowledge_crystals",
                    json!({"crystals": [
                        {"crystal_type": "observation", "text": "First conclusion."},
                        {"crystal_type": "observation", "text": "Second conclusion."}
                    ]}),
                )],
            ))
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("knowledge_crystals output exceeds max_records_per_pass"),
        "{error}"
    );

    let run_row = query(&config, "select status from dream_runs", &[]).remove(0);
    assert_eq!(run_row[0], json!("failed"));
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("completed"));
    assert_eq!(session_row[1], Value::Null);
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories where archived_at is not null"
        ),
        json!(0)
    );
}

#[test]
fn dreaming_fails_closed_when_batch_exceeds_max_long_term_records_affected_per_run() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["Valid input."]);

    // Three staged crystals across passes: under every per-pass cap but
    // over the run cap.
    let mut dream_config = default_dream_config();
    dream_config.max_long_term_records_affected_per_run = 2;
    save_dream_config(&config, &dream_config).unwrap();

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(ScriptedProvider::new(
                "over-run-cap",
                vec![
                    (
                        "rule_crystals",
                        json!({"rule_crystals": [
                            {"crystal_type": "rule", "text": "Rule conclusion."}
                        ]}),
                    ),
                    (
                        "knowledge_crystals",
                        json!({"crystals": [
                            {"crystal_type": "observation", "text": "First conclusion."},
                            {"crystal_type": "observation", "text": "Second conclusion."}
                        ]}),
                    ),
                ],
            ))
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exceeds max_long_term_records_affected_per_run"),
        "{error}"
    );

    let run_row = query(&config, "select status from dream_runs", &[]).remove(0);
    assert_eq!(run_row[0], json!("failed"));
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("completed"));
    assert_eq!(session_row[1], Value::Null);
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories where archived_at is not null"
        ),
        json!(0)
    );
}

// ---------------------------------------------------------------------------
// Evidence passes over one bounded selection
// ---------------------------------------------------------------------------

#[test]
fn evidence_dream_runs_all_passes_over_the_same_selection() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["Conclusion one.", "Conclusion two."]);
    let extra_ids = completed_session(&config, "book", &["A conclusion from the next session."]);
    let call_log: CallLog = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::clone(&call_log);
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(move || {
            Box::new(EvidenceProvider {
                calls: Arc::clone(&calls),
                omit_coverage: false,
            })
        }),
    )
    .unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    let calls = call_log.lock().unwrap().clone();
    let expected_passes = [
        "concepts",
        "terminology_candidates",
        "rule_crystals",
        "knowledge_crystals",
        "relations",
        "reinforcement",
        "coverage_audit",
    ];
    assert_eq!(
        calls
            .iter()
            .map(|(pass, _)| pass.as_str())
            .collect::<Vec<_>>(),
        expected_passes
    );
    let distinct_selections: std::collections::HashSet<Vec<i64>> =
        calls.iter().map(|(_pass, ids)| ids.clone()).collect();
    assert_eq!(distinct_selections.len(), 1);
    let selection = calls[0].1.clone();
    assert_eq!(selection.len(), 3);
    assert!(selection.contains(&extra_ids[0]));

    let mut expected_phase_rows: Vec<(String, String, i64, i64)> = expected_passes
        .iter()
        .map(|pass| (pass.to_string(), "completed".to_string(), 3, 0))
        .collect();
    // knowledge_crystals staged one crystal; coverage_audit accounted for
    // all three selected memories.
    expected_phase_rows[3].3 = 1;
    expected_phase_rows[6].3 = 3;
    // The persistence phase applies the one validated mutation batch.
    expected_phase_rows.push(("persistence".to_string(), "completed".to_string(), 3, 1));
    assert_eq!(phase_rows(&config), expected_phase_rows);
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(1));

    // Audit: per-pass input/output summaries plus one persistence summary
    // carrying the mutation record.
    let summaries = audit_event_summaries(&config, run.id);
    let mut expected_summaries: Vec<(String, String)> = Vec::new();
    for pass in expected_passes {
        expected_summaries.push((
            "provider_request".to_string(),
            format!("sent {pass} request"),
        ));
        expected_summaries.push((
            "provider_response".to_string(),
            format!("received {pass} response"),
        ));
    }
    expected_summaries.push((
        "phase_completed".to_string(),
        "completed persistence phase".to_string(),
    ));
    assert_eq!(summaries, expected_summaries);
}

#[test]
fn audit_records_prompt_hash_and_redacted_endpoint_on_request_and_response() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["The audited memory is important."]);

    const RENDERED_PROMPT: &str = "rendered dream prompt for the audit hash";
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(PromptAuditProvider {
                prompt: RENDERED_PROMPT,
                endpoint: "https://editor:secret@api.example.com:8443/v1?api_key=topsecret#frag",
            })
        }),
    )
    .unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    let expected_hash = sha256_hex(RENDERED_PROMPT);
    for event_type in ["provider_request", "provider_response"] {
        let payloads = audit_payloads(&config, run.id, event_type);
        assert_eq!(payloads.len(), 7, "{event_type}: one entry per pass");
        for payload in &payloads {
            assert_eq!(
                payload["prompt_sha256"],
                json!(expected_hash),
                "{event_type}: the hash must match the rendered prompt"
            );
            assert_eq!(
                payload["endpoint"],
                json!("https://api.example.com:8443/v1"),
                "{event_type}: only scheme, host, port, and path survive"
            );
        }
    }
    // Raw endpoint credentials and query strings never reach any audit
    // record of the run.
    for row in query(
        &config,
        "select payload_json from dream_audit_entries where dream_run_id = ?1",
        &[&run.id],
    ) {
        let payload = row[0].as_str().unwrap();
        assert!(!payload.contains("editor:secret"), "{payload}");
        assert!(!payload.contains("topsecret"), "{payload}");
    }
}

#[test]
fn book_scale_batch_is_covered_by_every_dream_pass() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let memory_ids = completed_session_with(&config, "book", |workspace, session_id| {
        (0..500)
            .map(|index| {
                add_memory(
                    workspace,
                    session_id,
                    "reading",
                    &format!("Distinct reading conclusion {index}."),
                )
            })
            .collect()
    });

    let call_log: CallLog = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::clone(&call_log);
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(move || {
            Box::new(EvidenceProvider {
                calls: Arc::clone(&calls),
                omit_coverage: false,
            })
        }),
    )
    .unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    let calls = call_log.lock().unwrap().clone();
    assert_eq!(calls.len(), 7);
    assert!(
        calls.iter().all(|(_pass, ids)| ids == &memory_ids),
        "every pass must see the same bounded selection"
    );
    let workspace = WorkspaceStore::open(&config).unwrap();
    let session_id: i64 = query(&config, "select id from task_sessions", &[])
        .remove(0)
        .remove(0)
        .as_i64()
        .unwrap();
    assert!(
        workspace
            .list_short_term_memories(session_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn provider_sees_only_the_selection_bounded_by_max_short_term_memories_per_run() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let memory_ids = completed_session_with(&config, "book", |workspace, session_id| {
        (0..5)
            .map(|index| {
                add_memory(
                    workspace,
                    session_id,
                    "note",
                    &format!("Distinct reading conclusion {index}."),
                )
            })
            .collect()
    });

    let mut dream_config = default_dream_config();
    dream_config.max_short_term_memories_per_run = 2;
    save_dream_config(&config, &dream_config).unwrap();

    let call_log: CallLog = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::clone(&call_log);
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(move || {
            Box::new(EvidenceProvider {
                calls: Arc::clone(&calls),
                omit_coverage: false,
            })
        }),
    )
    .unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.input_count, 2);
    let calls = call_log.lock().unwrap().clone();
    assert_eq!(calls.len(), 7);
    let bounded = memory_ids[..2].to_vec();
    assert!(
        calls.iter().all(|(_pass, ids)| ids == &bounded),
        "every pass must see only the bounded selection, saw {calls:?}"
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories where archived_at is null"
        ),
        json!(3),
        "memories beyond the run cap stay pending for the next run"
    );
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(1));
}

// ---------------------------------------------------------------------------
// Parse decisions are audited with reasons
// ---------------------------------------------------------------------------

#[test]
fn skipped_provider_candidates_are_audited_with_reasons() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["A completed dream input."]);

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(ScriptedProvider::new(
                "invalid-source",
                vec![(
                    "knowledge_crystals",
                    json!({
                        "crystals": [
                            {
                                "content": "This candidate cites an unknown source memory.",
                                "source_memory_ids": [999999]
                            },
                            123
                        ]
                    }),
                )],
            ))
        }),
    )
    .unwrap();
    let run = service.run_cycle("manual", false).unwrap();

    assert_eq!(run.status, "completed");
    let payload = last_phase_completed_payload(&config, run.id);
    assert_eq!(payload["created_crystals"], json!([]));
    assert_eq!(
        payload["skipped_candidates"],
        json!([
            {
                "entry_path": "crystals[0]",
                "reason": "invalid_source_memory_ids",
                "source_memory_ids": [999999]
            },
            {
                "entry_path": "crystals[1]",
                "reason": "malformed_candidate",
                "candidate_type": "integer"
            }
        ])
    );
}

#[test]
fn malformed_rule_crystal_gets_penalties_and_parse_warnings() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["Cooking term guidance."]);

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(ScriptedProvider::new(
                "malformed-dict",
                vec![(
                    "rule_crystals",
                    json!({
                        "rule_crystals": [{
                            "body": "Keep cooking terminology practical and concrete.",
                            "kind": "rule_crystal",
                            "source_credibility": "user_rule",
                            "rule_intent": "terminology",
                            "concept_names": ["Cooking"]
                        }]
                    }),
                )],
            ))
        }),
    )
    .unwrap();
    let run = service.run_all("manual", true, false).unwrap();

    assert_eq!(run.status, "completed");
    let crystal = query(
        &config,
        "select crystal_type, text, confidence, source_credibility, rule_intent,
                malformed_penalty
         from crystals",
        &[],
    )
    .remove(0);
    assert_eq!(crystal[0], json!("rule"));
    assert_eq!(
        crystal[1],
        json!("Keep cooking terminology practical and concrete.")
    );
    assert!(crystal[2].as_f64().unwrap() < 0.9);
    assert_eq!(crystal[3], json!("user_rule"));
    assert_eq!(crystal[4], json!("terminology"));
    assert!(crystal[5].as_f64().unwrap() > 0.0);

    let parse_warnings = query(
        &config,
        "select summary from dream_audit_entries
         where dream_run_id = ?1 and event_type = 'parse_warnings'",
        &[&run.id],
    );
    assert!(
        !parse_warnings.is_empty(),
        "malformed parse must be audited"
    );
}

#[test]
fn deterministic_provider_uses_credibility_not_source_role_for_crystal_type() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let workspace = WorkspaceStore::open(&config).unwrap();
    let session = workspace
        .start_session(&context("only-sense-online"))
        .unwrap();
    let user_id = {
        let mut input = ShortTermMemoryInput::new("correction", "User lesson.");
        input.source_role = "editorial-review".to_string();
        input.source_credibility = "user_rule".to_string();
        input.rule_intent = "correction".to_string();
        workspace
            .add_short_term_memory(session.id, &input)
            .unwrap()
            .id
    };
    let mentor_id = add_memory(&workspace, session.id, "note", "Mentor lore.");
    let mundane_id = add_memory(&workspace, session.id, "tm", "Mundane term.");
    let system_id = add_memory(&workspace, session.id, "trace", "System trace.");
    let memories = workspace.list_short_term_memories(session.id).unwrap();

    let output = DeterministicDreamProvider
        .run_pass(
            "knowledge_crystals",
            &context("only-sense-online"),
            &memories,
        )
        .unwrap();

    let types: Vec<(String, Vec<i64>)> = output["crystals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| {
            (
                candidate["crystal_type"].as_str().unwrap().to_string(),
                candidate["source_memory_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_i64().unwrap())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        types,
        vec![
            ("rule".to_string(), vec![user_id]),
            ("concept".to_string(), vec![mentor_id]),
            ("concept".to_string(), vec![mundane_id]),
            ("concept".to_string(), vec![system_id]),
        ]
    );
}

// ---------------------------------------------------------------------------
// Error redaction
// ---------------------------------------------------------------------------

#[test]
fn dream_error_records_redact_configured_api_key_value() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    completed_session(&config, "only-sense-online", &["A completed dream input."]);

    let catalog = ProviderCatalog::default().with_provider(
        "openai",
        ProviderProfile::new(
            "OpenAI",
            "openai",
            "https://api.openai.com/v1",
            "raw-secret-value",
            30.0,
        ),
    );
    save_provider_catalog(&config, &catalog).unwrap();

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| {
            Box::new(FailingProvider {
                message: "provider rejected raw-secret-value",
            })
        }),
    )
    .unwrap();
    assert!(service.run_cycle("manual", false).is_err());

    let stored_error = scalar(&config, "select error from dream_runs");
    assert_eq!(stored_error, json!("provider rejected [redacted]"));
}
