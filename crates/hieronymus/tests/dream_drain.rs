//! Task D5, Step 1: the pure drain regression. `run_all` used to perform one
//! capped selection, so a backlog larger than the cap stayed pending (Astra
//! 17); `drain_batches` is the bounded batch loop the production drain is
//! built on.

use hieronymus::dreaming::drain_batches;
use std::sync::atomic::AtomicBool;
#[test]
fn dream_all_drains_more_than_one_batch() {
    let mut batches = [2_usize, 2, 1, 0].into_iter();
    let count = drain_batches(|| Ok(batches.next().unwrap_or(0)), &AtomicBool::new(false)).unwrap();
    assert_eq!(count, 5);
}

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{default_dream_config, save_dream_config};
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::dreaming::{DreamService, MAX_DRAIN_BATCHES};

fn seed(config: &HieronymusConfig, members: i64) {
    let connection = open_migrated(&config.database_path()).unwrap();
    connection.execute_batch("insert into series(slug,title,default_source_language,default_target_language,created_at,updated_at) values('book','Book','ja','ru','now','now'); insert into task_sessions(id, series_slug, source_language, target_language, status, task_type, created_at, last_activity_at) values(1, 'book', 'ja', 'ru', 'completed', 'translate', 'now', 'now');").unwrap();
    connection.execute("with recursive n(x) as (values(1) union all select x+1 from n where x<?1)
      insert into crystals(id, text, crystal_type, scope_type, strength, confidence, status, soft_origin, created_at, updated_at)
      select x, 'text ' || x, 'lesson', 'global', 0.5, 0.5, 'active', '', 'now', 'now' from n", [members]).unwrap();
}

#[test]
fn feedback_only_drain_counts_consumed_events() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed(&config, 1);
    let connection = open_migrated(&config.database_path()).unwrap();
    connection.execute_batch("insert into memory_events(crystal_id, event_type, source_role, strength_delta, confidence_delta, applied, created_at)
      values(1, 'recalled_again', 'system', 0.1, 0.0, 0, 'now'),
            (1, 'recalled_again', 'system', 0.1, 0.0, 0, 'now'),
            (1, 'recalled_again', 'system', 0.1, 0.0, 0, 'now');").unwrap();
    let mut settings = default_dream_config();
    settings.max_total_affected_crystals = 1;
    save_dream_config(&config, &settings).unwrap();
    let result = DreamService::open(&config, WorkflowResolver::deterministic())
        .unwrap()
        .run_all("manual", true, false)
        .unwrap();
    assert_eq!(result.batches, 3);
    assert_eq!(result.outcome, "completed");
    assert_eq!(result.progress.feedback_events, 3);
    assert_eq!(result.input_count, 0);
}

#[test]
fn bounded_pair_drain_reports_pending_and_restarts_without_replay() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed(&config, 17);
    let connection = open_migrated(&config.database_path()).unwrap();
    connection.execute_batch("insert into crystal_activations(crystal_id, session_id, recall_query, rank, score, outcome, created_at)
      select id, 1, 'test', 0, 1, 'useful', 'now' from crystals;").unwrap();
    let mut settings = default_dream_config();
    settings.max_relation_records_per_pass = 1;
    save_dream_config(&config, &settings).unwrap();
    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let result = service.run_all("manual", true, false).unwrap();
    assert_eq!(result.batches, MAX_DRAIN_BATCHES);
    assert_eq!(result.outcome, "pending");
    assert_eq!(result.record.status, "completed");
    assert_eq!(result.progress.terminalized_pairs, 128);
    let result = service.run_all("manual", true, false).unwrap();
    assert_eq!(result.outcome, "completed");
    assert_eq!(result.progress.terminalized_pairs, 8);
}

#[test]
fn cancellation_after_pair_commit_reports_interrupted() {
    use std::sync::{Arc, atomic::Ordering};
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed(&config, 3);
    let connection = open_migrated(&config.database_path()).unwrap();
    connection.execute_batch("insert into crystal_activations(crystal_id, session_id, recall_query, rank, score, outcome, created_at)
      select id, 1, 'test', 0, 1, 'useful', 'now' from crystals;").unwrap();
    let mut settings = default_dream_config();
    settings.max_relation_records_per_pass = 1;
    save_dream_config(&config, &settings).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let mut service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    service.set_phase_observer(Arc::new(move |_, _, phase| {
        if phase == "link_reinforcement" {
            signal.store(true, Ordering::SeqCst);
        }
    }));
    let result = service.run_draining("manual", true, &cancelled).unwrap();
    assert_eq!(result.outcome, "interrupted");
    assert_eq!(result.progress.terminalized_pairs, 1);
}

#[test]
fn working_copy_only_drain_counts_archives() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    seed(&config, 1);
    let connection = open_migrated(&config.database_path()).unwrap();
    connection.execute_batch("insert into short_term_memories(session_id,source_role,kind,text,source_crystal_id,created_at)
      values(1,'system','note','text 1',1,'now'),(1,'system','note','text 1',1,'now'),(1,'system','note','text 1',1,'now');").unwrap();
    let mut settings = default_dream_config();
    settings.max_long_term_records_affected_per_run = 1;
    save_dream_config(&config, &settings).unwrap();
    let result = DreamService::open(&config, WorkflowResolver::deterministic())
        .unwrap()
        .run_all("manual", true, false)
        .unwrap();
    assert_eq!(result.batches, 3);
    assert_eq!(result.outcome, "completed");
    assert_eq!(result.progress.archived_inputs, 3);
}
