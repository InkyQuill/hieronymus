//! The durability contract for dream phases (task D3): deterministic phases
//! and the provider persistence phase commit their domain mutations, the
//! phase-completed status, and the redacted audit entry in ONE immediate
//! transaction. An audit or phase-status failure rolls the domain writes
//! back; the failure is still audited after the rollback, and a retry
//! applies every semantic effect exactly once.

use serde_json::{Value, json};

use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_audit::{DreamAuditStore, commit_audited};
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::dreaming::{DreamError, DreamProvider, DreamService};
use hieronymus::memory_models::{ShortTermMemoryRecord, TranslationContext};
use hieronymus::registry::Registry;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};
use rusqlite::Connection;

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

fn add_crystal(config: &HieronymusConfig, slug: &str, text: &str) -> i64 {
    CrystalStore::open(config)
        .unwrap()
        .add_crystal(&context(slug), "lesson", &NewCrystal::new("lesson", text))
        .unwrap()
}

fn add_memory(workspace: &WorkspaceStore, session_id: i64, kind: &str, text: &str) -> i64 {
    workspace
        .add_short_term_memory(session_id, &ShortTermMemoryInput::new(kind, text))
        .unwrap()
        .id
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

fn scalar_params(config: &HieronymusConfig, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Value {
    query(config, sql, params).remove(0).remove(0)
}

fn execute(config: &HieronymusConfig, sql: &str) {
    open_migrated(&config.database_path())
        .unwrap()
        .execute(sql, [])
        .unwrap();
}

/// Abort every `phase_completed` audit insert; the phase-status update and
/// all domain writes of the same transaction run first and must roll back.
fn block_phase_completed_audits(config: &HieronymusConfig, message: &str) {
    execute(
        config,
        &format!(
            "CREATE TRIGGER abort_phase_completed_audit BEFORE INSERT ON dream_audit_entries
             WHEN NEW.event_type = 'phase_completed'
             BEGIN SELECT RAISE(ABORT, '{message}'); END;"
        ),
    );
}

/// Abort every audit insert once one audit row exists: the first pair's
/// commit (including its own audit entry) succeeds, and the second pair's
/// in-transaction audit append fails — the injection lands INSIDE a pair
/// transaction (task D4's per-pair granularity).
fn block_all_audits_after_first(config: &HieronymusConfig, message: &str) {
    execute(
        config,
        &format!(
            "CREATE TRIGGER abort_audit BEFORE INSERT ON dream_audit_entries
             WHEN (SELECT count(*) FROM dream_audit_entries) >= 1
             BEGIN SELECT RAISE(ABORT, '{message}'); END;"
        ),
    );
}

/// Abort only the phase-completed transition; the failed transition stays
/// possible, so the run registry can still mark the phase failed.
fn block_phase_completion(config: &HieronymusConfig, message: &str) {
    execute(
        config,
        &format!(
            "CREATE TRIGGER abort_phase_completion BEFORE UPDATE OF status ON dream_phase_runs
             WHEN NEW.status = 'completed'
             BEGIN SELECT RAISE(ABORT, '{message}'); END;"
        ),
    );
}

fn phase_status(config: &HieronymusConfig, phase: &str) -> Vec<String> {
    query(
        config,
        "select status from dream_phase_runs where phase = ?1 order by id",
        &[&phase],
    )
    .into_iter()
    .map(|row| row[0].as_str().unwrap().to_string())
    .collect()
}

// ---------------------------------------------------------------------------
// The primitive
// ---------------------------------------------------------------------------

#[test]
fn audit_failure_rolls_back_mutation() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE domain(value INTEGER);
        CREATE TABLE audit(value INTEGER CHECK(value > 0));",
    )
    .unwrap();
    let result = commit_audited(&mut db, |tx| {
        tx.execute("INSERT INTO domain VALUES (1)", [])?;
        tx.execute("INSERT INTO audit VALUES (0)", [])?;
        Ok(())
    });
    assert!(result.is_err());
    let count: i64 = db
        .query_row("SELECT count(*) FROM domain", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn append_in_transaction_redacts_and_commits_with_the_callers_transaction() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    let mut connection = open_migrated(&config.database_path()).unwrap();
    connection
        .execute(
            "insert into dream_runs(cycle_id, status, provider, created_at)
             values (1, 'running', 'test', '2026-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
    let run_id = connection.last_insert_rowid();

    let audit_id = commit_audited(&mut connection, |tx| {
        DreamAuditStore::append_in_transaction(
            tx,
            run_id,
            None,
            "phase_completed",
            "info",
            "redaction check",
            &json!({"apikey": "secret-value", "model": "safe"}),
        )
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
    })
    .unwrap();

    let row: (String, String) = connection
        .query_row(
            "select event_type, payload_json from dream_audit_entries where id = ?1",
            [audit_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row.0, "phase_completed");
    assert!(row.1.contains("[REDACTED]"), "{}", row.1);
    assert!(!row.1.contains("secret-value"), "{}", row.1);
    assert!(row.1.contains("safe"), "{}", row.1);
}

// ---------------------------------------------------------------------------
// Deterministic phases under failure injection
// ---------------------------------------------------------------------------

/// The link phase commits each budgeted pair on its own (task D4 granularity,
/// superseding D3's whole-phase atomicity for this phase only). The audit
/// injection lands inside the SECOND pair's transaction: that pair rolls back
/// and stays queued, while the FIRST pair's committed link and audit entry
/// stay durable. A retry completes every remaining pair exactly once.
#[test]
fn audit_injection_inside_a_pair_transaction_keeps_committed_pairs_and_retry_completes_exactly_once()
 {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = WorkspaceStore::open(&config)
        .unwrap()
        .start_session(&context("only-sense-online"))
        .unwrap();
    // Three distinct crystals in one session: three unique unordered pairs.
    let first = add_crystal(
        &config,
        "only-sense-online",
        "Космический корабль летит к звёздам.",
    );
    let second = add_crystal(
        &config,
        "only-sense-online",
        "Рецепт хлеба требует тёплой воды.",
    );
    let third = add_crystal(
        &config,
        "only-sense-online",
        "Старый мост ведёт через реку.",
    );
    let created_at = "2026-01-01T00:00:00+00:00";
    for crystal_id in [first, second, third] {
        open_migrated(&config.database_path())
            .unwrap()
            .execute(
                "insert into crystal_activations(
                   crystal_id, session_id, recall_query, rank, score, outcome, created_at
                 )
                 values (?1, ?2, 'test', 0, 1.0, 'useful', ?3)",
                rusqlite::params![crystal_id, session.id, created_at],
            )
            .unwrap();
    }
    block_all_audits_after_first(&config, "audit blocked by test trigger");

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error.to_string().contains("audit blocked by test trigger"),
        "{error}"
    );

    // The first pair's atomic commit is durable: exactly one link exists
    // with its audit entry, and its pair row is applied. The second pair's
    // transaction (link + pair status + audit) rolled back as a unit, so it
    // retains its lazy cursor with no domain effect and no audit row. The batch is
    // still open: no activation was consumed.
    let links = query(
        &config,
        "select source_crystal_id, target_crystal_id, weight from crystal_links",
        &[],
    );
    assert_eq!(links.len(), 1, "committed pair stays committed: {links:?}");
    assert_eq!(links[0][0], json!(first));
    assert_eq!(links[0][1], json!(second));
    assert_eq!(links[0][2], json!(0.5));
    assert_eq!(
        query(
            &config,
            "select status from dream_link_pairs order by left_id, right_id",
            &[]
        ),
        vec![vec![json!("applied")]]
    );
    assert_eq!(
        query(
            &config,
            "select next_left_offset, next_right_offset from dream_link_batches",
            &[]
        ),
        vec![vec![json!(0), json!(2)]]
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_audit_entries where event_type = 'link_pair_completed'"
        ),
        json!(1)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from crystal_activations where cycle_id is null"
        ),
        json!(3)
    );
    assert_eq!(
        scalar(&config, "select completed_cycle from dream_link_batches"),
        Value::Null
    );
    assert_eq!(
        scalar(&config, "select applied_pair_count from dream_link_batches"),
        json!(1)
    );
    assert_eq!(
        scalar(&config, "select skipped_pair_count from dream_link_batches"),
        json!(0)
    );
    // The phase row stays untouched ('running' — deterministic phase rows are
    // never failed in bulk; the failed run row and, when writable, the
    // phase_failed record are the durable outcome).
    assert_eq!(phase_status(&config, "link_reinforcement"), vec!["running"]);
    let run_row = query(&config, "select status, error from dream_runs", &[]).remove(0);
    assert_eq!(run_row[0], json!("failed"));
    assert!(
        run_row[1].as_str().unwrap().contains("audit blocked"),
        "{}",
        run_row[1]
    );
    // The post-rollback phase_failed record was also blocked by the trigger;
    // the pair audit that committed before the injection is the only row.
    assert_eq!(
        scalar(&config, "select count(*) from dream_audit_entries"),
        json!(1)
    );

    // Retry without the trigger: every remaining pair applies exactly once.
    execute(&config, "DROP TRIGGER abort_audit");
    let retry = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let rerun = retry.run_cycle("manual", false).unwrap();
    assert_eq!(rerun.status, "completed");

    let links = query(
        &config,
        "select source_crystal_id, target_crystal_id, weight from crystal_links
         order by source_crystal_id, target_crystal_id",
        &[],
    );
    assert_eq!(links.len(), 3);
    // All three weights are the initial 0.5: no pair was applied twice.
    assert!(links.iter().all(|row| row[2] == json!(0.5)), "{links:?}");
    // Batch completion (last pair's commit): all activations consumed with
    // the retry run's cycle.
    assert_eq!(
        query(&config, "select cycle_id from crystal_activations", &[],),
        vec![
            vec![json!(rerun.cycle_id)],
            vec![json!(rerun.cycle_id)],
            vec![json!(rerun.cycle_id)]
        ]
    );
    assert_eq!(
        scalar(&config, "select completed_cycle from dream_link_batches"),
        json!(rerun.cycle_id)
    );
    assert_eq!(
        phase_status(&config, "link_reinforcement"),
        vec!["running", "completed"]
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_audit_entries where event_type = 'phase_completed'
             and summary = 'completed link_reinforcement phase'"
        ),
        json!(1)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_audit_entries where event_type = 'link_pair_completed'"
        ),
        json!(3)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_audit_entries where event_type = 'link_batch_completed'"
        ),
        json!(1)
    );
}

#[test]
fn phase_completion_failure_rolls_back_reinforcement_and_audits_the_failure_after_rollback() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let crystal_id = add_crystal(&config, "only-sense-online", "Сохраняйте спокойствие.");
    open_migrated(&config.database_path())
        .unwrap()
        .execute(
            "insert into memory_events(
               crystal_id, session_id, event_type, source_role, evidence,
               strength_delta, confidence_delta, applied, created_at
             )
             values (?1, null, 'recalled_again', 'system', 'test', 0.1, 0.0, 0, ?2)",
            rusqlite::params![crystal_id, "2026-01-01T00:00:00+00:00"],
        )
        .unwrap();
    block_phase_completion(&config, "phase completion blocked");

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error.to_string().contains("phase completion blocked"),
        "{error}"
    );

    // The score delta and the event consumption rolled back with the phase
    // status update.
    assert_eq!(
        scalar_params(
            &config,
            "select strength from crystals where id = ?1",
            &[&crystal_id]
        ),
        json!(0.5)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from memory_events where applied = 0"
        ),
        json!(1)
    );
    assert_eq!(phase_status(&config, "reinforcement"), vec!["running"]);

    // The post-rollback failure record exists (the audit insert itself was
    // never blocked) and claims no domain effects.
    let failure = query(
        &config,
        "select event_type, severity, summary, payload_json from dream_audit_entries",
        &[],
    )
    .remove(0);
    assert_eq!(failure[0], json!("phase_failed"));
    assert_eq!(failure[1], json!("error"));
    assert_eq!(failure[2], json!("failed reinforcement phase"));
    let payload: Value = serde_json::from_str(failure[3].as_str().unwrap()).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .contains("phase completion blocked"),
        "{payload}"
    );
    assert!(
        payload["committed_domain_effects"]
            .as_str()
            .unwrap()
            .starts_with("none"),
        "{payload}"
    );
    assert!(payload.get("changed_crystals").is_none(), "{payload}");

    // Retry without the trigger: exactly one semantic effect.
    execute(&config, "DROP TRIGGER abort_phase_completion");
    let retry = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let rerun = retry.run_cycle("manual", false).unwrap();
    assert_eq!(rerun.status, "completed");

    assert_eq!(
        scalar_params(
            &config,
            "select strength from crystals where id = ?1",
            &[&crystal_id]
        ),
        json!(0.6)
    );
    let event = query(&config, "select applied, cycle_id from memory_events", &[]).remove(0);
    assert_eq!(event[0], json!(1));
    assert_eq!(event[1], json!(rerun.cycle_id));
    assert_eq!(
        phase_status(&config, "reinforcement"),
        vec!["running", "completed"]
    );
}

// ---------------------------------------------------------------------------
// The provider persistence phase under failure injection
// ---------------------------------------------------------------------------

/// One valid crystal citing the selected memories, full coverage: the minimal
/// provider output whose application is the persistence phase's domain work.
struct PersistenceProvider;

impl DreamProvider for PersistenceProvider {
    fn name(&self) -> &str {
        "persistence-test"
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
                    "title": "Atomic persistence",
                    "text": "The persisted memory survives rollback tests.",
                    "strength": 0.6,
                    "confidence": 0.8,
                    "source_memory_ids": ids,
                }]
            }));
        }
        Ok(json!({}))
    }
}

#[test]
fn persistence_audit_failure_rolls_back_crystallization_and_retry_applies_exactly_once() {
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
        "note",
        "A memory destined for crystals.",
    );
    workspace.complete_session(session.id).unwrap();
    block_phase_completed_audits(&config, "persistence audit blocked");

    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(|| Box::new(PersistenceProvider)),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error.to_string().contains("persistence audit blocked"),
        "{error}"
    );

    // The whole application transaction rolled back: no crystal, no archived
    // memory, no dreamed session, no completed persistence phase. The provider
    // pass phases of this failed run are marked failed in bulk (pre-existing
    // run-failure bookkeeping: no phase may look like applied work).
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories where archived_at is not null"
        ),
        json!(0)
    );
    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("completed"));
    assert_eq!(session_row[1], Value::Null);
    assert_eq!(phase_status(&config, "persistence"), vec!["failed"]);
    assert_eq!(phase_status(&config, "knowledge_crystals"), vec!["failed"]);
    let failure = query(
        &config,
        "select summary, payload_json from dream_audit_entries
         where event_type = 'phase_failed'",
        &[],
    )
    .remove(0);
    assert_eq!(failure[0], json!("failed persistence phase"));
    let payload: Value = serde_json::from_str(failure[1].as_str().unwrap()).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .contains("persistence audit blocked"),
        "{payload}"
    );

    // Retry without the trigger: exactly one semantic effect, and the
    // successful write exposes domain row, phase completion, and audit
    // together.
    execute(&config, "DROP TRIGGER abort_phase_completed_audit");
    let retry = DreamService::open(
        &config,
        WorkflowResolver::serving(|| Box::new(PersistenceProvider)),
    )
    .unwrap();
    let rerun = retry.run_cycle("manual", false).unwrap();
    assert_eq!(rerun.status, "completed");

    let crystals = query(&config, "select title, created_cycle from crystals", &[]);
    assert_eq!(crystals.len(), 1);
    assert_eq!(crystals[0][1], json!(rerun.cycle_id));
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories where archived_at is not null"
        ),
        json!(1)
    );
    let session_row = query(&config, "select status, cycle_id from task_sessions", &[]).remove(0);
    assert_eq!(session_row[0], json!("dreamed"));
    assert_eq!(session_row[1], json!(rerun.cycle_id));
    assert_eq!(
        phase_status(&config, "persistence"),
        vec!["failed", "completed"]
    );
    let completed = query(
        &config,
        "select summary from dream_audit_entries
         where event_type = 'phase_completed' and phase_run_id in
           (select id from dream_phase_runs where phase = 'persistence')",
        &[],
    );
    assert_eq!(completed, vec![vec![json!("completed persistence phase")]]);
}

struct RevisionChangingProvider {
    config: HieronymusConfig,
}
impl DreamProvider for RevisionChangingProvider {
    fn name(&self) -> &str {
        "revision-changing-test"
    }
    fn run_pass(
        &self,
        pass: &str,
        context: &TranslationContext,
        memories: &[ShortTermMemoryRecord],
    ) -> Result<Value, DreamError> {
        if pass == "knowledge_crystals" {
            use hieronymus::{authority::DecisionStore, authority_models::*};
            use sha2::{Digest, Sha256};
            let claim = &memories[0].claim_annotation.claims[0];
            let mut db = open_migrated(&self.config.database_path())?;
            let request = DecisionRequestV1 {
                version: 1,
                decision_id: "10000000-0000-4000-8000-000000004004".into(),
                expected_revision: hieronymus::coherent_reads::revision(&db, &context.series_slug)?,
                actor_kind: ActorKind::ExplicitUser,
                origin: OriginReceiptId("20000000-0000-4000-8000-000000004004".into()),
                evidence_refs: vec![],
                series_id: claim.applicability.series_id,
                concept_id: claim.concept_id,
                source_language: context.source_language.clone(),
                target_language: None,
                applicability: claim.applicability.clone(),
                operation: OperationV1::Correct {
                    intent: CorrectionIntentV1::Fact {
                        claim_id: claim.claim_id,
                        claim_revision: claim.revision,
                        effect: FactEffect::Invalidate,
                    },
                },
            };
            // Private stored-origin fixture; this is not a host authority claim.
            let binding=json!({"decision_id":request.decision_id,"expected_revision":request.expected_revision,"selected_source":null,"series_id":request.series_id,"concept_id":request.concept_id,"source_language":request.source_language,"target_language":request.target_language,"applicability":request.applicability,"evidence_ids":[],"operation":request.operation}).to_string();
            let text = "that memory is wrong";
            let hash = format!("{:x}", Sha256::digest(format!("{text}\n{binding}")));
            db.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,'console_user','fixture',?1,?2,?3,?4,'now')",rusqlite::params![request.origin.0,text,binding,hash])?;
            let result = DecisionStore::new(&mut db).apply(&request)?;
            assert!(
                matches!(result, DecisionResultV1::Applied { .. }),
                "{result:?}"
            );
        }
        PersistenceProvider.run_pass(pass, context, memories)
    }
}
#[test]
fn inflight_authority_change_discards_dream_output_and_retains_pending_inputs() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    current_story::register(&config, "book");
    let mut story = context("book").volume("I").chapter("Opening");
    story.story_viewpoint = hieronymus::story_applicability::Viewpoint::Narrator;
    let ws = WorkspaceStore::open(&config).unwrap();
    let session = ws.start_session(&story).unwrap();
    open_migrated(&config.database_path()).unwrap().execute_batch("insert into concepts(id,canonical_name,scope_type,scope_key,created_at,updated_at) values(1,'Mira','series','series:book','now','now')").unwrap();
    let mut claim = current_story::claim(
        &config,
        "book",
        "Mira knows a selected assertion before correction.",
    );
    claim.concept_id = Some(1);
    let mut input = ShortTermMemoryInput::new("note", &claim.text);
    input.claims = vec![claim];
    ws.add_short_term_memory(session.id, &input).unwrap();
    ws.complete_session(session.id).unwrap();
    let provider_config = config.clone();
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(move || {
            Box::new(RevisionChangingProvider {
                config: provider_config.clone(),
            })
        }),
    )
    .unwrap();
    let result = service.run_cycle("manual", false);
    assert!(
        matches!(result, Err(DreamError::StaleAuthority)),
        "stale output must be retryable with a typed conflict: {result:?}"
    );
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(0));
    assert_eq!(
        scalar(
            &config,
            "select count(*) from claim_effects where effect='invalid'"
        ),
        json!(1)
    );
    assert_eq!(ws.get_session(session.id).unwrap().status, "completed");
    assert_eq!(ws.list_short_term_memories(session.id).unwrap().len(), 1);
    let retry = DreamService::open(
        &config,
        WorkflowResolver::serving(|| Box::new(PersistenceProvider)),
    )
    .unwrap();
    assert_eq!(
        retry.run_cycle("manual", false).unwrap().status,
        "completed"
    );
    let recalled = hieronymus::recall::RecallService::open(&config)
        .unwrap()
        .recall_context(&story, "persisted", 10)
        .unwrap();
    assert!(recalled.hits.is_empty());
    assert_eq!(recalled.non_current.len(), 1);
    assert!(matches!(
        recalled.non_current[0].claim_annotation().disposition,
        hieronymus::claim_reads::ClaimDisposition::Invalid
    ));
}

#[test]
fn dream_crystal_preserves_all_selected_claim_ids() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let ws = WorkspaceStore::open(&config).unwrap();
    let session = ws.start_session(&context("book")).unwrap();
    add_memory(&ws, session.id, "note", "First compound source assertion.");
    add_memory(&ws, session.id, "note", "Second source assertion.");
    let before = query(
        &config,
        "select claim_id from claim_bindings where short_term_id is not null order by claim_id",
        &[],
    );
    assert_eq!(before.len(), 2);
    ws.complete_session(session.id).unwrap();
    DreamService::open(
        &config,
        WorkflowResolver::serving(|| Box::new(PersistenceProvider)),
    )
    .unwrap()
    .run_cycle("manual", false)
    .unwrap();
    assert_eq!(
        query(
            &config,
            "select claim_id from claim_bindings where crystal_id is not null order by claim_id",
            &[]
        ),
        before
    );
}

#[path = "support/current_story.rs"]
mod current_story;
