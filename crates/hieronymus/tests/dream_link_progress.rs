//! Durable pair progress for the dream link-reinforcement phase (task D4):
//! eligible activations are snapshotted into one durable batch per session
//! (`dream_link_batches` / `dream_link_members`), every budgeted pair commits
//! its reinforcement, its pair-row terminalization, and its audit entry
//! atomically, and unprocessed pairs stay queued — so an exhausted budget or
//! a crash resumes on the next cycle instead of permanently dropping pairs
//! (the pre-D4 bug: a partial budget still stamped every activation
//! consumed). A batch's activations are stamped consumed only when its last
//! pair is applied or explicitly skipped.

use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_link_progress::LinkProgress;
use hieronymus::dream_link_progress::canonical_pairs;
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::dreaming::DreamService;
use hieronymus::memory_models::TranslationContext;
use hieronymus::registry::Registry;
use hieronymus::workspace::WorkspaceStore;
use serde_json::{Value, json};

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

/// A direct-drive audit context: audit entries require a dream run row.
fn create_run(config: &HieronymusConfig, cycle_id: i64) -> i64 {
    let connection = open_migrated(&config.database_path()).unwrap();
    connection
        .execute(
            "insert into dream_runs(cycle_id, status, provider, created_at)
             values (?1, 'running', 'test', '2026-01-01T00:00:00+00:00')",
            [cycle_id],
        )
        .unwrap();
    connection.last_insert_rowid()
}

/// Three clearly dissimilar crystals: co-activatable, never combinable.
fn dissimilar_texts() -> [&'static str; 3] {
    [
        "Quantum chalk experiments unfolded today.",
        "The harbour ledger closed before sunrise.",
        "Migrating geese crossed the northern valley.",
    ]
}

fn add_activation(config: &HieronymusConfig, session_id: i64, crystal_id: i64) -> i64 {
    let connection = open_migrated(&config.database_path()).unwrap();
    connection
        .execute(
            "insert into crystal_activations(
               crystal_id, session_id, recall_query, rank, score, outcome, created_at
             )
             values (?1, ?2, 'test', 0, 1.0, 'useful', '2026-01-01T00:00:00+00:00')",
            rusqlite::params![crystal_id, session_id],
        )
        .unwrap();
    connection.last_insert_rowid()
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

fn open_progress(config: &HieronymusConfig, run_id: i64) -> LinkProgress {
    let mut progress = LinkProgress::open(config).unwrap();
    progress.set_run_context(run_id, None);
    progress
}

fn start_session(config: &HieronymusConfig, slug: &str) -> i64 {
    WorkspaceStore::open(config)
        .unwrap()
        .start_session(&context(slug))
        .unwrap()
        .id
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
// Canonical pairs
// ---------------------------------------------------------------------------

#[test]
fn pairs_are_unique_and_resume_in_stable_order() {
    assert_eq!(canonical_pairs(&[3, 1, 2, 1]), vec![(1, 2), (1, 3), (2, 3)]);
}

// ---------------------------------------------------------------------------
// Durable per-batch pair progress
// ---------------------------------------------------------------------------

#[test]
fn pairs_are_terminalized_once_across_store_reopens() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let texts = dissimilar_texts();
    let crystals: Vec<i64> = texts
        .iter()
        .map(|text| add_crystal(&config, "only-sense-online", text))
        .collect();
    for crystal_id in &crystals {
        add_activation(&config, session, *crystal_id);
    }
    let run_id = create_run(&config, 1);

    // Budget one: the first pair applies, and the other activations stay
    // unconsumed (the pre-D4 bug stamped all of them consumed here).
    let mut first = open_progress(&config, run_id);
    assert_eq!(first.process(101, 1).unwrap(), 1);
    drop(first);
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(1)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from crystal_activations where cycle_id is null"
        ),
        json!(3),
        "activations are not consumed while the batch still has queued pairs"
    );
    let batch_id = scalar(&config, "select id from dream_link_batches")
        .as_i64()
        .unwrap();
    assert_eq!(
        query(
            &config,
            "select left_id, right_id, status from dream_link_pairs where batch_id = ?1 order by left_id, right_id",
            &[&batch_id]
        ),
        vec![
            vec![json!(crystals[0]), json!(crystals[1]), json!("applied")],
            vec![json!(crystals[0]), json!(crystals[2]), json!("queued")],
            vec![json!(crystals[1]), json!(crystals[2]), json!("queued")],
        ]
    );

    // Reopen the store and spend the budget one pair at a time.
    let mut second = open_progress(&config, run_id);
    assert_eq!(second.process(102, 1).unwrap(), 1);
    assert_eq!(second.process(103, 1).unwrap(), 1);
    drop(second);

    // Three distinct pair effects, and the batch completed only now.
    let links = query(
        &config,
        "select source_crystal_id, target_crystal_id, weight from crystal_links
         order by source_crystal_id, target_crystal_id",
        &[],
    );
    assert_eq!(links.len(), 3);
    assert_eq!(
        links,
        vec![
            vec![json!(crystals[0]), json!(crystals[1]), json!(0.5)],
            vec![json!(crystals[0]), json!(crystals[2]), json!(0.5)],
            vec![json!(crystals[1]), json!(crystals[2]), json!(0.5)],
        ]
    );
    assert_eq!(
        scalar(&config, "select completed_cycle from dream_link_batches"),
        json!(103)
    );
    assert_eq!(
        query(
            &config,
            "select distinct cycle_id from crystal_activations",
            &[]
        ),
        vec![vec![json!(103)]]
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

    // A fourth call has no work left and no further effect.
    let mut fourth = open_progress(&config, run_id);
    assert_eq!(fourth.process(104, 5).unwrap(), 0);
    drop(fourth);
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(3)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_audit_entries where event_type = 'link_pair_completed'"
        ),
        json!(3)
    );
}

#[test]
fn budget_zero_consumes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let left = add_crystal(&config, "only-sense-online", dissimilar_texts()[0]);
    let right = add_crystal(&config, "only-sense-online", dissimilar_texts()[1]);
    add_activation(&config, session, left);
    add_activation(&config, session, right);
    let run_id = create_run(&config, 1);

    let mut progress = open_progress(&config, run_id);
    assert_eq!(progress.process(201, 0).unwrap(), 0);

    // Zero mutation: no batch, no pairs, no members, no consumed
    // activations, no links, no audit.
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_batches"),
        json!(0)
    );
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_pairs"),
        json!(0)
    );
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_members"),
        json!(0)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from crystal_activations where cycle_id is null"
        ),
        json!(2)
    );
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(0)
    );
    assert_eq!(
        scalar(&config, "select count(*) from dream_audit_entries"),
        json!(0)
    );
}

#[test]
fn no_activations_creates_no_batch() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let _session = start_session(&config, "only-sense-online");
    let run_id = create_run(&config, 1);

    let mut progress = open_progress(&config, run_id);
    assert_eq!(progress.process(301, 5).unwrap(), 0);
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_batches"),
        json!(0)
    );
}

#[test]
fn empty_batch_consumes_its_single_activation() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let crystal_id = add_crystal(&config, "only-sense-online", dissimilar_texts()[0]);
    add_activation(&config, session, crystal_id);
    let run_id = create_run(&config, 1);

    // One crystal forms no pair: an empty (pair-less) batch is snapshotted
    // and immediately completed, consuming the activation without effects —
    // the durable equivalent of the old in-pass consumption.
    let mut progress = open_progress(&config, run_id);
    assert_eq!(progress.process(401, 5).unwrap(), 0);
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_batches"),
        json!(1)
    );
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_pairs"),
        json!(0)
    );
    assert_eq!(
        scalar(&config, "select completed_cycle from dream_link_batches"),
        json!(401)
    );
    assert_eq!(
        query(&config, "select cycle_id from crystal_activations", &[]),
        vec![vec![json!(401)]]
    );
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(0)
    );
}

#[test]
fn duplicate_activations_pair_once_and_consume_once() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let left = add_crystal(&config, "only-sense-online", dissimilar_texts()[0]);
    let right = add_crystal(&config, "only-sense-online", dissimilar_texts()[1]);
    // The same crystal activated twice (duplicate), plus a second crystal.
    add_activation(&config, session, left);
    add_activation(&config, session, left);
    add_activation(&config, session, right);
    let run_id = create_run(&config, 1);

    let mut progress = open_progress(&config, run_id);
    assert_eq!(progress.process(501, 10).unwrap(), 1);

    // UNIQUE membership keeps every activation in the batch; the duplicate
    // crystal still yields exactly one pair.
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_batches"),
        json!(1)
    );
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_members"),
        json!(3)
    );
    assert_eq!(
        scalar(&config, "select count(*) from dream_link_pairs"),
        json!(1)
    );
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(1)
    );
    assert_eq!(
        query(
            &config,
            "select distinct cycle_id from crystal_activations",
            &[]
        ),
        vec![vec![json!(501)]],
        "all member activations — duplicates included — are consumed at completion"
    );
}

#[test]
fn sessions_never_share_a_batch() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session_one = start_session(&config, "only-sense-online");
    let session_two = start_session(&config, "only-sense-online");
    let texts = dissimilar_texts();
    let first_left = add_crystal(&config, "only-sense-online", texts[0]);
    let first_right = add_crystal(&config, "only-sense-online", texts[1]);
    let second_left = add_crystal(&config, "only-sense-online", texts[0]);
    let second_right = add_crystal(&config, "only-sense-online", texts[2]);
    add_activation(&config, session_one, first_left);
    add_activation(&config, session_one, first_right);
    add_activation(&config, session_two, second_left);
    add_activation(&config, session_two, second_right);
    let run_id = create_run(&config, 1);

    // Budget one: session one's pair (lowest session id first) consumes the
    // budget, so session two is not snapshotted yet — no queued work is
    // created ahead of budget. Its activations stay unconsumed.
    let mut progress = open_progress(&config, run_id);
    assert_eq!(progress.process(601, 1).unwrap(), 1);
    let batches = query(
        &config,
        "select b.id, b.session_id, b.completed_cycle,
                (select count(*) from dream_link_members m
                 where m.batch_id = b.id
                   and m.activation_id in (
                     select id from crystal_activations where session_id = b.session_id
                   )) as own_members,
                (select count(*) from dream_link_members m where m.batch_id = b.id) as members
         from dream_link_batches b order by b.id",
        &[],
    );
    assert_eq!(
        batches.len(),
        1,
        "session two has no batch while the budget was spent"
    );
    assert_eq!(batches[0][1], json!(session_one));
    assert_eq!(
        batches[0][2],
        json!(601),
        "session one's single-pair batch completed"
    );
    assert_eq!(
        batches[0][3], batches[0][4],
        "batch one holds only its session's activations"
    );
    assert_eq!(
        scalar_params(
            &config,
            "select count(*) from crystal_activations where session_id = ?1 and cycle_id is null",
            &[&session_two]
        ),
        json!(2),
        "session two's activations stay unconsumed and unsnapshotted"
    );

    // The next call snapshots session two into its own batch and completes it.
    assert_eq!(progress.process(602, 10).unwrap(), 1);
    let second_batch = query(
        &config,
        "select session_id, completed_cycle,
                (select count(*) from dream_link_members m
                 where m.batch_id = b.id
                   and m.activation_id in (
                     select id from crystal_activations where session_id = b.session_id
                   )) as own_members,
                (select count(*) from dream_link_members m where m.batch_id = b.id) as members
         from dream_link_batches b where session_id = ?1",
        &[&session_two],
    )
    .remove(0);
    assert_eq!(second_batch[0], json!(session_two));
    assert_eq!(second_batch[1], json!(602));
    assert_eq!(
        second_batch[2], second_batch[3],
        "batch two holds only its session's activations"
    );
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(2)
    );
    assert_eq!(
        query(
            &config,
            "select distinct session_id, cycle_id from crystal_activations order by session_id",
            &[]
        ),
        vec![
            vec![json!(session_one), json!(601)],
            vec![json!(session_two), json!(602)],
        ],
        "each session's activations are stamped with their own batch's completion cycle"
    );
    // No cross-session link ever exists.
    assert_eq!(
        scalar_params(
            &config,
            "select count(*) from crystal_links
             where (source_crystal_id in (?, ?) and target_crystal_id in (?, ?))
                or (source_crystal_id in (?, ?) and target_crystal_id in (?, ?))",
            &[
                &first_left,
                &first_right,
                &second_left,
                &second_right, //
                &second_left,
                &second_right,
                &first_left,
                &first_right,
            ]
        ),
        json!(0)
    );
}

#[test]
fn deleted_crystal_pairs_are_skipped_with_audited_reasons() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let texts = dissimilar_texts();
    let left = add_crystal(&config, "only-sense-online", texts[0]);
    let middle = add_crystal(&config, "only-sense-online", texts[1]);
    let doomed = add_crystal(&config, "only-sense-online", texts[2]);
    for crystal_id in [left, middle, doomed] {
        add_activation(&config, session, crystal_id);
    }
    let run_id = create_run(&config, 1);

    // Budget one applies the first pair, leaving two queued.
    let mut progress = open_progress(&config, run_id);
    assert_eq!(progress.process(701, 1).unwrap(), 1);

    // Delete the third crystal (its activation cascades away): the queued
    // pairs survive as snapshot rows and must become audited skips, not
    // silent drops or failures.
    execute(
        &config,
        &format!("delete from crystals where id = {doomed}"),
    );
    assert_eq!(progress.process(702, 10).unwrap(), 2);

    let pairs = query(
        &config,
        "select left_id, right_id, status, applied_cycle, result_json
         from dream_link_pairs order by left_id, right_id",
        &[],
    );
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[0][0], json!(left));
    assert_eq!(pairs[0][1], json!(middle));
    assert_eq!(pairs[0][2], json!("applied"));
    assert_eq!(pairs[0][3], json!(701));
    for pair in &pairs[1..] {
        assert_eq!(pair[2], json!("skipped"), "{}", pair[4]);
        assert_eq!(pair[3], json!(702));
        let result: Value = serde_json::from_str(pair[4].as_str().unwrap()).unwrap();
        assert_eq!(result["reason"], json!("crystal_missing"));
        assert_eq!(result["missing_crystal_ids"], json!([doomed]));
    }
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(1)
    );
    assert_eq!(
        scalar(&config, "select completed_cycle from dream_link_batches"),
        json!(702)
    );
    assert_eq!(
        query(
            &config,
            "select distinct cycle_id from crystal_activations",
            &[]
        ),
        vec![vec![json!(702)]]
    );
    // The skips are audited.
    let skips = query(
        &config,
        "select severity, payload_json from dream_audit_entries
         where event_type = 'link_pair_completed' and payload_json like '%skipped%'",
        &[],
    );
    assert_eq!(skips.len(), 2);
    let payload: Value = serde_json::from_str(skips[0][1].as_str().unwrap()).unwrap();
    assert_eq!(payload["status"], json!("skipped"));
    assert_eq!(payload["result"]["reason"], json!("crystal_missing"));
}

#[test]
fn crash_between_pair_and_batch_completion_resumes_to_completion() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let texts = dissimilar_texts();
    let left = add_crystal(&config, "only-sense-online", texts[0]);
    let right = add_crystal(&config, "only-sense-online", texts[1]);
    add_activation(&config, session, left);
    add_activation(&config, session, right);
    let run_id = create_run(&config, 1);

    let mut progress = open_progress(&config, run_id);
    assert_eq!(progress.process(801, 5).unwrap(), 1);

    // Simulate the crash window: the last pair committed, but the batch
    // completion (activation stamping + completed_cycle) never landed.
    execute(
        &config,
        "update dream_link_batches set completed_cycle = null",
    );
    execute(&config, "update crystal_activations set cycle_id = null");
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_link_pairs where status = 'queued'"
        ),
        json!(0)
    );

    // The resume heals the batch: no new pair effects, but the batch
    // completes and the activations are stamped exactly once.
    let mut restarted = open_progress(&config, run_id);
    assert_eq!(restarted.process(802, 5).unwrap(), 0);
    assert_eq!(
        scalar(&config, "select completed_cycle from dream_link_batches"),
        json!(802)
    );
    assert_eq!(
        query(
            &config,
            "select distinct cycle_id from crystal_activations",
            &[]
        ),
        vec![vec![json!(802)]]
    );
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(1)
    );
    let weights: Vec<Value> = query(&config, "select weight from crystal_links", &[]).remove(0);
    assert_eq!(
        weights,
        vec![json!(0.5)],
        "resume never re-applies a committed pair"
    );
}

// ---------------------------------------------------------------------------
// Phase integration under failure injection
// ---------------------------------------------------------------------------

/// Abort the pair's status terminalization: the link mutation of the same
/// pair transaction has already run and must roll back with it.
fn block_pair_status(config: &HieronymusConfig, message: &str) {
    execute(
        config,
        &format!(
            "CREATE TRIGGER abort_pair_status BEFORE UPDATE OF status ON dream_link_pairs
             WHEN NEW.status = 'applied'
             BEGIN SELECT RAISE(ABORT, '{message}'); END;"
        ),
    );
}

#[test]
fn pair_status_failure_rolls_back_the_pair_and_retry_applies_exactly_once() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let left = add_crystal(&config, "only-sense-online", dissimilar_texts()[0]);
    let right = add_crystal(&config, "only-sense-online", dissimilar_texts()[1]);
    add_activation(&config, session, left);
    add_activation(&config, session, right);
    block_pair_status(&config, "pair status blocked by test trigger");

    let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pair status blocked by test trigger"),
        "{error}"
    );

    // The pair transaction rolled back as a unit: no link, the pair stays
    // queued, the batch stays open, no activation was consumed.
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(0)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_link_pairs where status = 'queued'"
        ),
        json!(1)
    );
    assert_eq!(
        scalar(&config, "select completed_cycle from dream_link_batches"),
        Value::Null
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from crystal_activations where cycle_id is null"
        ),
        json!(2)
    );
    assert_eq!(phase_status(&config, "link_reinforcement"), vec!["running"]);

    // The post-rollback failure record exists and claims no domain effects.
    let failure = query(
        &config,
        "select event_type, severity, summary, payload_json from dream_audit_entries
         where event_type = 'phase_failed'",
        &[],
    )
    .remove(0);
    assert_eq!(failure[1], json!("error"));
    assert_eq!(failure[2], json!("failed link_reinforcement phase"));
    let payload: Value = serde_json::from_str(failure[3].as_str().unwrap()).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .contains("pair status blocked"),
        "{payload}"
    );
    assert_eq!(payload["committed_link_pairs"], json!(0));
    assert!(
        payload["committed_domain_effects"]
            .as_str()
            .unwrap()
            .starts_with("none"),
        "{payload}"
    );

    // Retry without the trigger: exactly one semantic effect.
    execute(&config, "DROP TRIGGER abort_pair_status");
    let retry = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let rerun = retry.run_cycle("manual", false).unwrap();
    assert_eq!(rerun.status, "completed");
    assert_eq!(
        query(&config, "select weight from crystal_links", &[]),
        vec![vec![json!(0.5)]]
    );
    assert_eq!(
        scalar(&config, "select status from dream_link_pairs"),
        json!("applied")
    );
    assert_eq!(
        query(
            &config,
            "select distinct cycle_id from crystal_activations",
            &[]
        ),
        vec![vec![json!(rerun.cycle_id)]]
    );
    assert_eq!(
        phase_status(&config, "link_reinforcement"),
        vec!["running", "completed"]
    );
}

#[test]
fn budgeted_cycles_execute_each_pair_exactly_once() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "only-sense-online");
    let session = start_session(&config, "only-sense-online");
    let texts = dissimilar_texts();
    let crystals: Vec<i64> = texts
        .iter()
        .map(|text| add_crystal(&config, "only-sense-online", text))
        .collect();
    for crystal_id in &crystals {
        add_activation(&config, session, *crystal_id);
    }
    // The phase budget (max_relation_records_per_pass) of one: every cycle
    // terminalizes exactly one pair.
    std::fs::write(
        config.dream_config_path(),
        "[dreaming]\nmax_relation_records_per_pass = 1\n",
    )
    .unwrap();

    let mut cycles: Vec<i64> = Vec::new();
    for expected_links in 1..=3 {
        let service = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
        let run = service.run_cycle("manual", false).unwrap();
        cycles.push(run.cycle_id);
        assert_eq!(run.status, "completed");
        assert_eq!(
            scalar(&config, "select count(*) from crystal_links"),
            json!(expected_links)
        );
        if expected_links < 3 {
            // Not all pairs are terminal: the activations stay unconsumed
            // and the batch stays open for the next cycle.
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
                scalar(
                    &config,
                    "select count(*) from dream_link_pairs where status = 'queued'"
                ),
                json!(3 - expected_links)
            );
        }
    }

    // The third pair completed the batch: everything consumed exactly once.
    assert_eq!(
        query(
            &config,
            "select distinct cycle_id from crystal_activations",
            &[]
        ),
        vec![vec![json!(cycles[2])]]
    );
    assert_eq!(
        query(
            &config,
            "select weight from crystal_links order by source_crystal_id, target_crystal_id",
            &[]
        ),
        vec![vec![json!(0.5)], vec![json!(0.5)], vec![json!(0.5)]],
        "each pair applied exactly once (a replay would read 0.6)"
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from dream_link_pairs where status = 'applied'"
        ),
        json!(3)
    );

    // A further cycle has nothing left to do.
    let extra = DreamService::open(&config, WorkflowResolver::deterministic()).unwrap();
    let extra_run = extra.run_cycle("manual", false).unwrap();
    assert_eq!(extra_run.status, "completed");
    assert_eq!(
        scalar(&config, "select count(*) from crystal_links"),
        json!(3)
    );
}
