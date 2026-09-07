//! Plan W3: the typed, audited admin action surface
//! ([`hiero::application::admin::run_action`] /
//! [`hiero::application::admin::validate_action_request`]).
//!
//! Every one of the 13 canonical actions gets a success case and an
//! invalid-input case; failed actions must leave the DB and the audit log
//! consistent; rule crystals (any status) reject `delete`/`merge`/`split`;
//! duplicate/missing ids and unconfirmed destructive requests are rejected.

use hiero::application::Application;
use hiero::application::admin::{ACTION_NAMES, run_action, validate_action_request};
use hiero::daemon::dream_worker::DreamController;
use hiero::daemon::workers::WorkerGroup;
use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_config::{default_dream_config, save_dream_config};
use hieronymus::dream_workflows::WorkflowResolver;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const ACTOR: &str = "test-actor";
const TS: &str = "2026-02-03T04:05:06Z";

// --------------------------------------------------------------------------
// Step 1 regression (from the plan)
// --------------------------------------------------------------------------

#[test]
fn split_requires_content_for_both_resulting_crystals() {
    assert!(
        validate_action_request("split_crystal", &json!({ "id": 7, "confirmed": true })).is_err()
    );
    assert!(
        validate_action_request(
            "split_crystal",
            &json!({
                "id": 7, "confirmed": true, "parts": ["First memory", "Second memory"]
            })
        )
        .is_ok()
    );
}

// --------------------------------------------------------------------------
// Fixtures
// --------------------------------------------------------------------------

struct Fixture {
    _root: tempfile::TempDir,
    config: HieronymusConfig,
    app: Application,
}

fn setup() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let app = Application::open(&config).unwrap();
    seed(&open_migrated(&config.database_path()).unwrap());
    Fixture {
        _root: root,
        config,
        app,
    }
}

/// Seed a realistic small world:
///   * series `s1` (ja -> en) and a foreign series `s2`;
///   * crystal 1 (active), crystal 2 (active), lesson 3 (active);
///   * an ACTIVE rule crystal 4;
///   * a short-term memory + `crystal_sources` link + a recall activation for
///     crystal 1;
///   * concept 1;
///   * a finished dream run 1 (cycle 1) with one phase and one pending
///     strict-concept proposal 1.
fn seed(db: &Connection) {
    db.execute_batch(
        "
        insert into series(slug, title, default_source_language, default_target_language,
                           created_at, updated_at)
          values ('s1', 'Series One', 'ja', 'en', '2026-02-03T04:05:06Z', '2026-02-03T04:05:06Z');
        insert into series(slug, title, default_source_language, default_target_language,
                           created_at, updated_at)
          values ('s2', 'Series Two', 'ja', 'en', '2026-02-03T04:05:06Z', '2026-02-03T04:05:06Z');
        ",
    )
    .unwrap();

    let crystal = |db: &Connection, id: i64, kind: &str, text: &str, title: &str, series: &str| {
        db.execute(
            "insert into crystals(
               id, crystal_type, text, title, scope_type, scope_key, series_slug,
               source_language, target_language, tags_json, strength, confidence, status,
               created_cycle, created_at, updated_at
             )
             values (?1, ?2, ?3, ?4, 'series', ?5, ?5, 'ja', 'en', '[]', 0.8, 0.9, 'active',
                     0, ?6, ?6)",
            rusqlite::params![id, kind, text, title, series, TS],
        )
        .unwrap();
        db.execute(
            "insert into crystals_fts(rowid, title, text) values (?1, ?2, ?3)",
            rusqlite::params![id, title, text],
        )
        .unwrap();
    };
    crystal(
        db,
        1,
        "crystal",
        "The hero's name is Alto.",
        "Hero name",
        "s1",
    );
    crystal(
        db,
        2,
        "crystal",
        "The city is called Verel.",
        "City name",
        "s1",
    );
    crystal(
        db,
        3,
        "lesson",
        "Prefer terse dialogue tags.",
        "Dialogue lesson",
        "s1",
    );
    crystal(db, 4, "rule", "Use Sense, not Feeling.", "Sense rule", "s1");

    db.execute(
        "insert into task_sessions(
           id, series_slug, source_language, target_language, task_type, status, cycle_id,
           volume, chapter, created_at, last_activity_at
         )
         values (1, 's1', 'ja', 'en', 'translation', 'completed', 1, '1', '1', ?1, ?1)",
        rusqlite::params![TS],
    )
    .unwrap();
    db.execute(
        "insert into short_term_memories(
           id, session_id, source_role, kind, text, source_ref, created_at
         )
         values (1, 1, 'user', 'note', 'Alto is the protagonist.', 'ch1:p3', ?1)",
        rusqlite::params![TS],
    )
    .unwrap();
    db.execute(
        "insert into crystal_sources(crystal_id, short_term_memory_id) values (1, 1)",
        [],
    )
    .unwrap();
    db.execute(
        "insert into crystal_activations(
           crystal_id, session_id, recall_query, rank, score, reason, recall_id, created_at
         )
         values (1, 1, 'who is the hero', 1, 0.87421, 'lexical + concept', 'rec-1', ?1)",
        rusqlite::params![TS],
    )
    .unwrap();

    db.execute(
        "insert into concepts(id, canonical_name, description, scope_type, scope_key, status,
                              confidence, created_at, updated_at)
         values (1, 'Alto', 'The protagonist', 'series', 'series:s1', 'active', 0.6, ?1, ?1)",
        rusqlite::params![TS],
    )
    .unwrap();

    db.execute(
        "insert into dream_runs(id, cycle_id, status, provider, input_count,
                                created_crystal_count, proposal_count, error, created_at)
         values (1, 1, 'completed', 'deterministic', 2, 1, 1, '', ?1)",
        rusqlite::params![TS],
    )
    .unwrap();
    db.execute(
        "insert into dream_phase_runs(
           dream_run_id, phase, provider_profile, provider_type, model, status,
           input_count, output_count, created_at
         )
         values (1, 'knowledge_crystals', 'det', 'det', 'det', 'completed', 2, 1, ?1)",
        rusqlite::params![TS],
    )
    .unwrap();
    db.execute(
        "insert into strict_concept_proposals(
           id, dream_run_id, series_slug, source_language, target_language, concept_text,
           source_form, canonical_rendering, rationale, status, created_at, updated_at
         )
         values (1, 1, 's1', 'ja', 'en', 'Verel', 'ヴェレル', 'Verel', 'City name proposal',
                 'pending', ?1, ?1)",
        rusqlite::params![TS],
    )
    .unwrap();
}

fn db(fixture: &Fixture) -> Connection {
    open_migrated(&fixture.config.database_path()).unwrap()
}

fn count(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn run(
    fixture: &Fixture,
    action: &str,
    args: Value,
) -> Result<Value, hiero::application::AppError> {
    run_action(&fixture.app, ACTOR, action, &args)
}

// --------------------------------------------------------------------------
// The 13-action table: success + invalid-input for each
// --------------------------------------------------------------------------

#[test]
fn action_catalog_has_thirteen_named_actions() {
    assert_eq!(ACTION_NAMES.len(), 13);
}

#[test]
fn add_memory_creates_a_crystal_and_audits() {
    let fx = setup();
    let before = count(&db(&fx), "select count(*) from crystals");
    let out = run(
        &fx,
        "add_memory",
        json!({ "series": "s1", "text": "The moon has two names.", "view": "Crystals" }),
    )
    .unwrap();
    assert_eq!(out["result"]["action"], "add");
    assert_eq!(out["result"]["entity_type"], "crystal");
    let db = db(&fx);
    assert_eq!(count(&db, "select count(*) from crystals"), before + 1);
    assert_eq!(
        count(&db, "select count(*) from audit_log where action = 'add'"),
        1
    );
    // FTS stays in sync so the new row is searchable.
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals_fts where crystals_fts match 'moon'"
        ),
        1
    );
}

#[test]
fn add_memory_rejects_an_unknown_series() {
    let fx = setup();
    let error = run(
        &fx,
        "add_memory",
        json!({ "series": "nope", "text": "orphan" }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
    assert_eq!(
        count(&db(&fx), "select count(*) from audit_log"),
        0,
        "a failed add writes no audit row"
    );
}

#[test]
fn edit_memory_replaces_text_and_records_before_after() {
    let fx = setup();
    let out = run(
        &fx,
        "edit_memory",
        json!({ "id": 1, "text": "The hero's name is Alto Verren." }),
    )
    .unwrap();
    assert_eq!(out["result"]["message"], "Crystal edited");
    let db = db(&fx);
    assert_eq!(
        db.query_row("select text from crystals where id = 1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "The hero's name is Alto Verren."
    );
    // Old FTS token gone, new token present.
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals_fts where crystals_fts match 'Verren'"
        ),
        1
    );
    let (before, after): (String, String) = db
        .query_row(
            "select before_json, after_json from audit_log where action = 'edit'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(before.contains("Alto."));
    assert!(after.contains("Alto Verren."));
}

#[test]
fn edit_memory_rejects_empty_text() {
    let fx = setup();
    let error = run(&fx, "edit_memory", json!({ "id": 1, "text": "   " })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
    assert_eq!(
        db(&fx)
            .query_row("select text from crystals where id = 1", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "The hero's name is Alto."
    );
}

#[test]
fn delete_selected_archives_crystals_with_a_feedback_event() {
    let fx = setup();
    let out = run(
        &fx,
        "delete_selected",
        json!({ "ids": [1, 2], "view": "Crystals", "confirmed": true }),
    )
    .unwrap();
    assert_eq!(out["result"]["action"], "delete");
    let db = db(&fx);
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals where id in (1, 2) and status = 'archived'"
        ),
        2
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from memory_events where event_type = 'deleted_by_user'"
        ),
        2
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from audit_log where action = 'delete'"
        ),
        2
    );
}

#[test]
fn delete_selected_without_confirmation_posts_nothing() {
    let fx = setup();
    let error = run(
        &fx,
        "delete_selected",
        json!({ "ids": [1], "view": "Crystals" }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
    let db = db(&fx);
    assert_eq!(
        db.query_row("select status from crystals where id = 1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "active"
    );
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
}

#[test]
fn delete_selected_rejects_an_active_rule_crystal_and_leaves_it_untouched() {
    let fx = setup();
    let error = run(
        &fx,
        "delete_selected",
        json!({ "ids": [2, 4], "view": "Crystals", "confirmed": true }),
    )
    .unwrap_err();
    match error {
        hiero::application::AppError::Domain(message) => {
            assert!(message.contains("rule"), "{message}");
        }
        other => panic!("expected a domain rejection, got {other:?}"),
    }
    let db = db(&fx);
    // The pre-flight rejects the whole batch: crystal 2 is still active too.
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals where id in (2, 4) and status = 'active'"
        ),
        2
    );
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
    assert_eq!(count(&db, "select count(*) from memory_events"), 0);
}

#[test]
fn delete_selected_rejects_a_missing_row() {
    let fx = setup();
    let error = run(
        &fx,
        "delete_selected",
        json!({ "ids": [999], "view": "Crystals", "confirmed": true }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
    assert_eq!(count(&db(&fx), "select count(*) from audit_log"), 0);
}

/// A malformed id in the batch is rejected outright — a destructive action
/// never proceeds on the integer subset.
#[test]
fn delete_selected_rejects_a_non_integer_id_in_the_batch() {
    let fx = setup();
    let error = run(
        &fx,
        "delete_selected",
        json!({ "ids": [1, "oops", 2], "view": "Crystals", "confirmed": true }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
    let db = db(&fx);
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals where id in (1, 2) and status = 'active'"
        ),
        2
    );
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
}

fn seed_extra_concepts(db: &Connection) {
    db.execute(
        "insert into concepts(id, canonical_name, description, scope_type, scope_key, status,
                              confidence, created_at, updated_at)
         values (2, 'Verel', 'The city', 'series', 'series:s1', 'active', 0.5, ?1, ?1),
                (3, 'Kess', 'The river', 'series', 'series:s1', 'active', 0.4, ?1, ?1)",
        rusqlite::params![TS],
    )
    .unwrap();
}

#[test]
fn delete_selected_archives_a_concept_batch_atomically() {
    let fx = setup();
    seed_extra_concepts(&db(&fx));
    let out = run(
        &fx,
        "delete_selected",
        json!({ "ids": [2, 3], "view": "Concepts", "confirmed": true }),
    )
    .unwrap();
    assert_eq!(out["result"]["action"], "delete");
    let db = db(&fx);
    assert_eq!(
        count(
            &db,
            "select count(*) from concepts where id in (2, 3) and status = 'archived'"
        ),
        2
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from audit_log where action = 'archive' and entity_type = 'concept'"
        ),
        2
    );
}

/// The Concepts branch pre-flights every id: a missing (or already-inactive)
/// id fails the whole batch, so no concept is left half-archived and no
/// audit row is written.
#[test]
fn delete_selected_concept_batch_is_all_or_nothing() {
    let fx = setup();
    seed_extra_concepts(&db(&fx));
    let error = run(
        &fx,
        "delete_selected",
        json!({ "ids": [2, 999], "view": "Concepts", "confirmed": true }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
    let db = db(&fx);
    assert_eq!(
        db.query_row("select status from concepts where id = 2", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "active",
        "concept 2 must not be archived when the batch fails"
    );
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
}

#[test]
fn merge_selected_merges_concepts_and_audits() {
    let fx = setup();
    seed_extra_concepts(&db(&fx));
    let out = run(
        &fx,
        "merge_selected",
        json!({ "ids": [2, 3], "view": "Concepts", "confirmed": true }),
    )
    .unwrap();
    assert_eq!(out["result"]["entity_id"], 2);
    let db = db(&fx);
    assert_eq!(
        db.query_row("select status from concepts where id = 3", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "merged"
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from audit_log where action = 'merge' and entity_type = 'concept'"
        ),
        1
    );
}

/// The Concepts merge branch pre-flights every id before touching any
/// concept, so an unknown id fails before the first merge commits.
#[test]
fn merge_selected_concept_batch_preflights_every_id() {
    let fx = setup();
    seed_extra_concepts(&db(&fx));
    let error = run(
        &fx,
        "merge_selected",
        json!({ "ids": [2, 3, 999], "view": "Concepts", "confirmed": true }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
    let db = db(&fx);
    assert_eq!(
        count(
            &db,
            "select count(*) from concepts where id in (2, 3) and status = 'active'"
        ),
        2,
        "no concept is merged when the batch pre-flight fails"
    );
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
}

#[test]
fn merge_selected_builds_a_new_crystal_and_archives_the_sources() {
    let fx = setup();
    let out = run(
        &fx,
        "merge_selected",
        json!({
            "ids": [1, 2], "view": "Crystals", "confirmed": true,
            "title": "Names", "text": "The hero is Alto; the city is Verel."
        }),
    )
    .unwrap();
    let merged_id = out["result"]["entity_id"].as_i64().unwrap();
    let db = db(&fx);
    assert_eq!(
        db.query_row(
            "select status from crystals where id = ?1",
            [merged_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "active"
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals where id in (1, 2) and status = 'archived'"
        ),
        2
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from crystal_links where link_type = 'merged_from'"
        ),
        2
    );
    assert_eq!(
        count(&db, "select count(*) from audit_log where action = 'merge'"),
        1
    );
}

#[test]
fn merge_selected_rejects_duplicate_ids() {
    let fx = setup();
    let error = run(
        &fx,
        "merge_selected",
        json!({ "ids": [1, 1], "view": "Crystals", "confirmed": true, "text": "x" }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
    assert_eq!(count(&db(&fx), "select count(*) from audit_log"), 0);
}

#[test]
fn merge_selected_rejects_mismatched_contexts() {
    let fx = setup();
    // Move crystal 2 to the foreign series.
    db(&fx)
        .execute(
            "update crystals set series_slug = 's2', scope_key = 's2' where id = 2",
            [],
        )
        .unwrap();
    let error = run(
        &fx,
        "merge_selected",
        json!({ "ids": [1, 2], "view": "Crystals", "confirmed": true, "text": "x" }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
    let db = db(&fx);
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals where id in (1, 2) and status = 'active'"
        ),
        2
    );
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
}

#[test]
fn split_crystal_creates_the_parts_and_archives_the_source() {
    let fx = setup();
    let out = run(
        &fx,
        "split_crystal",
        json!({
            "id": 2, "confirmed": true,
            "parts": [
                { "title": "City", "text": "The city is Verel." },
                "Verel sits on the river."
            ]
        }),
    )
    .unwrap();
    let new_ids = out["result"]["new_crystal_ids"].as_array().unwrap();
    assert_eq!(new_ids.len(), 2);
    let db = db(&fx);
    assert_eq!(
        db.query_row("select status from crystals where id = 2", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "archived"
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from crystal_links where link_type = 'split_from'"
        ),
        2
    );
    assert_eq!(
        count(&db, "select count(*) from audit_log where action = 'split'"),
        1
    );
}

#[test]
fn split_crystal_rejects_a_single_part() {
    let fx = setup();
    let error = run(
        &fx,
        "split_crystal",
        json!({ "id": 2, "confirmed": true, "parts": ["only one"] }),
    )
    .unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
    assert_eq!(
        db(&fx)
            .query_row("select status from crystals where id = 2", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "active"
    );
}

#[test]
fn split_crystal_rejects_an_active_rule_crystal() {
    let fx = setup();
    let error = run(
        &fx,
        "split_crystal",
        json!({ "id": 4, "confirmed": true, "parts": ["a", "b"] }),
    )
    .unwrap_err();
    match error {
        hiero::application::AppError::Domain(message) => assert!(message.contains("rule")),
        other => panic!("expected domain rejection, got {other:?}"),
    }
    assert_eq!(count(&db(&fx), "select count(*) from audit_log"), 0);
}

/// ADR 0011: `merge`/`split` copy `crystal_type` and always land
/// `status='active'`, so a *candidate* rule crystal must be rejected too —
/// otherwise a W3 action would mint an active rule projection outside M3.
#[test]
fn merge_and_split_reject_a_candidate_rule_crystal() {
    let fx = setup();
    db(&fx)
        .execute("update crystals set status = 'candidate' where id = 4", [])
        .unwrap();

    let merge_error = run(
        &fx,
        "merge_selected",
        json!({ "ids": [2, 4], "view": "Crystals", "confirmed": true, "text": "x" }),
    )
    .unwrap_err();
    match merge_error {
        hiero::application::AppError::Domain(message) => {
            assert!(message.contains("rule"), "{message}")
        }
        other => panic!("expected domain rejection, got {other:?}"),
    }

    let split_error = run(
        &fx,
        "split_crystal",
        json!({ "id": 4, "confirmed": true, "parts": ["a", "b"] }),
    )
    .unwrap_err();
    match split_error {
        hiero::application::AppError::Domain(message) => {
            assert!(message.contains("rule"), "{message}")
        }
        other => panic!("expected domain rejection, got {other:?}"),
    }

    let db = db(&fx);
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
    assert_eq!(
        count(
            &db,
            "select count(*) from crystals where crystal_type = 'rule'"
        ),
        1,
        "no new rule crystal was minted"
    );
}

#[test]
fn reinforce_crystal_raises_scores_and_audits() {
    let fx = setup();
    let out = run(&fx, "reinforce_crystal", json!({ "id": 1 })).unwrap();
    assert_eq!(out["result"]["message"], "Crystal reinforced");
    let db = db(&fx);
    let (strength, confidence): (f64, f64) = db
        .query_row(
            "select strength, confidence from crystals where id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((strength - 0.95).abs() < 1e-9);
    assert!((confidence - 1.0).abs() < 1e-9);
    assert_eq!(
        count(
            &db,
            "select count(*) from memory_events where event_type = 'confirmed_by_user'"
        ),
        1
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from audit_log where action = 'reinforce'"
        ),
        1
    );
}

#[test]
fn reinforce_crystal_rejects_a_missing_crystal() {
    let fx = setup();
    let error = run(&fx, "reinforce_crystal", json!({ "id": 404 })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
    let db = db(&fx);
    assert_eq!(count(&db, "select count(*) from memory_events"), 0);
    assert_eq!(count(&db, "select count(*) from audit_log"), 0);
}

#[test]
fn decay_crystal_lowers_scores_and_audits() {
    let fx = setup();
    run(&fx, "decay_crystal", json!({ "id": 1 })).unwrap();
    let db = db(&fx);
    let (strength, confidence): (f64, f64) = db
        .query_row(
            "select strength, confidence from crystals where id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!((strength - 0.6).abs() < 1e-9);
    assert!((confidence - 0.65).abs() < 1e-9);
    assert_eq!(
        count(&db, "select count(*) from audit_log where action = 'decay'"),
        1
    );
}

#[test]
fn decay_crystal_rejects_a_non_integer_id() {
    let fx = setup();
    let error = run(&fx, "decay_crystal", json!({ "id": "abc" })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
}

#[test]
fn approve_proposal_creates_a_concept_and_marks_it_approved() {
    let fx = setup();
    let out = run(
        &fx,
        "approve_proposal",
        json!({ "id": 1, "reason": "verified" }),
    )
    .unwrap();
    let concept_id = out["result"]["concept_id"].as_i64().unwrap();
    let db = db(&fx);
    assert_eq!(
        db.query_row(
            "select status from strict_concept_proposals where id = 1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "approved"
    );
    assert_eq!(
        count(
            &db,
            &format!("select count(*) from concepts where id = {concept_id}")
        ),
        1
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from audit_log where action = 'approve'"
        ),
        1
    );
}

#[test]
fn approve_proposal_rejects_a_non_pending_proposal() {
    let fx = setup();
    db(&fx)
        .execute(
            "update strict_concept_proposals set status = 'approved' where id = 1",
            [],
        )
        .unwrap();
    let error = run(&fx, "approve_proposal", json!({ "id": 1 })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
    assert_eq!(
        count(&db(&fx), "select count(*) from concepts"),
        1,
        "no concept is created for a rejected approval"
    );
}

/// The whole approval — concept row, semantic tag, rendering facet, proposal
/// status flip, audit — is one transaction. A failure after the concept
/// insert rolls everything back: the proposal stays `pending`, and no orphan
/// concept/facet/tag or audit row survives. The failure is injected with a
/// trigger that aborts the audit insert (migrations recreate dropped tables,
/// so a trigger is the reliable seam).
#[test]
fn approve_proposal_rolls_back_completely_when_a_later_write_fails() {
    let fx = setup();
    db(&fx)
        .execute_batch(
            "create trigger boom_audit before insert on audit_log
             begin select raise(abort, 'injected failure'); end;",
        )
        .unwrap();

    let error = run(&fx, "approve_proposal", json!({ "id": 1 })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));

    let db = db(&fx);
    assert_eq!(
        db.query_row(
            "select status from strict_concept_proposals where id = 1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending",
        "the proposal must remain pending"
    );
    assert_eq!(
        count(&db, "select count(*) from concepts"),
        1,
        "only the seeded concept survives — no orphan concept"
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from concept_semantic_tags where tag = 'concept-proposal'"
        ),
        0,
        "no orphan semantic tag"
    );
    assert_eq!(
        count(&db, "select count(*) from concept_facets"),
        0,
        "no orphan rendering facet"
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from audit_log where action = 'approve'"
        ),
        0,
        "no audit row"
    );
}

#[test]
fn reject_proposal_marks_it_rejected_with_a_reason() {
    let fx = setup();
    run(
        &fx,
        "reject_proposal",
        json!({ "id": 1, "reason": "duplicate of an existing concept" }),
    )
    .unwrap();
    let db = db(&fx);
    assert_eq!(
        db.query_row(
            "select status from strict_concept_proposals where id = 1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "rejected"
    );
    assert_eq!(
        db.query_row(
            "select note from audit_log where action = 'reject'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "duplicate of an existing concept"
    );
}

#[test]
fn reject_proposal_requires_a_reason() {
    let fx = setup();
    let error = run(&fx, "reject_proposal", json!({ "id": 1 })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
    assert_eq!(
        db(&fx)
            .query_row(
                "select status from strict_concept_proposals where id = 1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "pending"
    );
}

#[test]
fn inspect_provenance_returns_the_source_memories_without_mutating() {
    let fx = setup();
    let audit_before = count(&db(&fx), "select count(*) from audit_log");
    let out = run(&fx, "inspect_provenance", json!({ "id": 1 })).unwrap();
    assert_eq!(out["provenance"]["title"], "Hero name");
    let sources = out["provenance"]["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["text"], "Alto is the protagonist.");
    assert_eq!(sources[0]["session_id"], "1");
    assert_eq!(
        count(&db(&fx), "select count(*) from audit_log"),
        audit_before,
        "a read-only inspection writes no audit row"
    );
}

#[test]
fn inspect_provenance_rejects_a_missing_crystal() {
    let fx = setup();
    let error = run(&fx, "inspect_provenance", json!({ "id": 77 })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
}

#[test]
fn inspect_recall_reasons_returns_activations_formatted() {
    let fx = setup();
    let out = run(&fx, "inspect_recall_reasons", json!({ "id": 1 })).unwrap();
    let reasons = out["reasons"].as_array().unwrap();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0]["query"], "who is the hero");
    assert_eq!(reasons[0]["rank"], "1");
    assert_eq!(reasons[0]["score"], "0.874");
    assert_eq!(reasons[0]["reason"], "lexical + concept");
}

#[test]
fn inspect_recall_reasons_filters_by_recall_id() {
    let fx = setup();
    let out = run(
        &fx,
        "inspect_recall_reasons",
        json!({ "id": 1, "recall_id": "other" }),
    )
    .unwrap();
    assert_eq!(out["reasons"].as_array().unwrap().len(), 0);
}

#[test]
fn inspect_recall_reasons_rejects_a_non_integer_id() {
    let fx = setup();
    let error = run(&fx, "inspect_recall_reasons", json!({})).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
}

/// The D5 test injection seam for the console action: a supervised dream
/// controller on the deterministic lane, wired into the fixture's
/// application exactly as `Daemon::start` wires the production controller.
fn install_deterministic_controller(fixture: &Fixture) {
    let mut dream_config = default_dream_config();
    for name in ["coverage_audit", "knowledge_crystals"] {
        let workflow = dream_config.workflows.get_mut(name).unwrap();
        workflow.provider = "deterministic".to_string();
        workflow.model = "deterministic".to_string();
        workflow.enabled = true;
    }
    save_dream_config(&fixture.config, &dream_config).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let mut workers = WorkerGroup::new(Arc::clone(&stop));
    let controller = DreamController::start_with_provider_source(
        fixture.config.clone(),
        &mut workers,
        Arc::new(WorkflowResolver::deterministic),
    )
    .unwrap();
    fixture.app.install_dream_controller(controller);
}

#[test]
fn run_manual_dreaming_drains_through_the_controller_and_audits() {
    let fx = setup();
    install_deterministic_controller(&fx);
    let out = run(&fx, "run_manual_dreaming", json!({ "all": true })).unwrap();
    assert_eq!(out["result"]["action"], "run");
    assert!(out["run"]["cycle_id"].is_number());
    // Two honest audit surfaces: the D5 controller writes one `run` row per
    // completed manual run, and the W3 action attributes its own row to the
    // authenticated actor.
    assert_eq!(
        count(
            &db(&fx),
            "select count(*) from audit_log where action = 'run' and entity_type = 'dream'"
        ),
        2
    );
    assert_eq!(
        count(
            &db(&fx),
            "select count(*) from audit_log where action = 'run' and entity_type = 'dream' \
             and actor = 'test-actor'"
        ),
        1
    );
}

#[test]
fn run_manual_dreaming_ignores_a_provider_key() {
    // The DTO only accepts `all`; a `provider` key is silently ignored and
    // the run uses the configured lanes (here the deterministic injection).
    let fx = setup();
    install_deterministic_controller(&fx);
    let out = run(&fx, "run_manual_dreaming", json!({ "provider": "openai" }));
    assert!(
        out.is_ok(),
        "an unknown key is ignored; the configured lanes serve the run"
    );
    assert_eq!(out.unwrap()["run"]["provider"], "deterministic");
}

#[test]
fn run_manual_dreaming_fails_closed_without_a_daemon_controller() {
    // D5: no direct provider construction outside the daemon's controller.
    let fx = setup();
    let error = run(&fx, "run_manual_dreaming", json!({ "all": true })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
}

#[test]
fn review_dream_output_builds_the_review_payload() {
    let fx = setup();
    let out = run(&fx, "review_dream_output", json!({ "run_id": 1 })).unwrap();
    let review = &out["review"];
    assert_eq!(review["run_id"], 1);
    assert_eq!(review["source_sessions"], json!([1]));
    assert_eq!(
        review["consumed_memories"],
        json!(["Alto is the protagonist."])
    );
    assert_eq!(review["strict_proposals"], json!(["Verel"]));
    assert_eq!(review["passes"].as_array().unwrap().len(), 1);
    assert_eq!(review["passes"][0]["name"], "knowledge_crystals");
}

#[test]
fn review_dream_output_rejects_a_missing_run() {
    let fx = setup();
    let error = run(&fx, "review_dream_output", json!({ "run_id": 999 })).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Domain(_)));
}

#[test]
fn an_unknown_action_is_not_implemented() {
    let fx = setup();
    let error = run(&fx, "teleport_crystal", json!({ "id": 1 })).unwrap_err();
    assert!(matches!(
        error,
        hiero::application::AppError::NotImplemented(_)
    ));
}

fn seed_merge_evidence(db: &Connection) {
    seed_extra_concepts(db);
    for (id, value) in [(1, "Alto"), (2, "Verel"), (3, "Kess")] {
        db.execute(
            "insert into concept_facets(id, concept_id, language, facet_type, value,
            confidence, is_canonical, created_at, updated_at)
            values (?1, ?1, 'en', 'rendering', ?2, 0.7, 1, ?3, ?3)",
            rusqlite::params![id, value, TS],
        )
        .unwrap();
        db.execute("insert into concept_facet_story_scopes(facet_id, story_scope) values (?1, 'chapter:1')", [id]).unwrap();
        db.execute("insert into crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at)
            values (1, ?1, 'mentions', 0.7, ?2)", rusqlite::params![id, TS]).unwrap();
    }
}

fn merge_state(db: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "concepts",
        "concept_facets",
        "concept_facet_story_scopes",
        "concept_facet_language_tags",
        "concept_facet_semantic_tags",
        "concept_semantic_tags",
        "crystal_concepts",
        "audit_log",
    ]
    .into_iter()
    .map(|table| {
        let mut stmt = db
            .prepare(&format!("select * from {table} order by rowid"))
            .unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    })
    .collect()
}

fn assert_merge_rollback(trigger: &str, ids: Value) {
    let fx = setup();
    let db = db(&fx);
    seed_merge_evidence(&db);
    // The merge also creates a former-label facet; rollback must remove its
    // FTS entry, not merely restore the re-parented existing facets.
    db.execute(
        "update concepts set canonical_name='Riverlabel' where id=3",
        [],
    )
    .unwrap();
    db.execute("insert into concept_semantic_tags(concept_id, tag, confidence, created_at) values (3, 'river', 0.7, ?1)", [TS]).unwrap();
    let before = merge_state(&db);
    db.execute_batch(trigger).unwrap();
    assert!(
        run(
            &fx,
            "merge_selected",
            json!({"ids":ids,"view":"Concepts","confirmed":true})
        )
        .is_err()
    );
    assert_eq!(merge_state(&db), before);
    assert_eq!(
        count(
            &db,
            "select count(*) from concept_facet_fts where concept_facet_fts match 'Riverlabel'"
        ),
        0
    );
    for term in ["Alto", "Verel", "Kess"] {
        assert_eq!(
            db.query_row(
                "select count(*) from concept_facet_fts where concept_facet_fts match ?1",
                [term],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let concept_term = if term == "Kess" { "Riverlabel" } else { term };
        assert_eq!(
            db.query_row(
                "select count(*) from concepts_fts where concepts_fts match ?1",
                [concept_term],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}

#[test]
fn concept_merge_rolls_back_when_audit_fails() {
    assert_merge_rollback(
        "CREATE TRIGGER reject_audit BEFORE INSERT ON audit_log
        BEGIN SELECT RAISE(ABORT, 'audit failure'); END;",
        json!([1, 2, 3]),
    );
}

#[test]
fn concept_merge_rolls_back_when_late_source_fails() {
    assert_merge_rollback(
        "CREATE TRIGGER reject_last_merge BEFORE UPDATE OF status ON concepts
        WHEN OLD.id = 3 AND NEW.status = 'merged'
        BEGIN SELECT RAISE(ABORT, 'late source failure'); END;",
        json!([1, 2, 3]),
    );
}

#[test]
fn concept_merge_rejects_repeated_sources_before_writes() {
    assert_merge_rollback("", json!([1, 2, 2]));
}

#[test]
fn concept_merge_keeps_target_canonical_and_moves_scoped_source_evidence() {
    let fx = setup();
    let db = db(&fx);
    seed_merge_evidence(&db);
    run(
        &fx,
        "merge_selected",
        json!({"ids":[1,2,3],"view":"Concepts","confirmed":true}),
    )
    .unwrap();
    assert_eq!(
        count(
            &db,
            "select count(*) from concept_facets where concept_id=1"
        ),
        3
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from concept_facets where concept_id=1 and is_canonical=1 and value='Alto'"
        ),
        1
    );
    assert_eq!(
        count(&db, "select count(*) from concept_facet_story_scopes"),
        3
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from crystal_concepts where concept_id=1"
        ),
        1
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from concepts where status='merged' and merged_into_concept_id=1"
        ),
        2
    );
}

#[test]
fn dream_proposal_materialization_preserves_variant_evidence() {
    let fx = setup();
    let db = db(&fx);
    db.execute(
        "update strict_concept_proposals set approved_variants_json='[\"Verell\"]',
        forbidden_variants_json='[\"Verele\"]' where id=1",
        [],
    )
    .unwrap();
    let out = run(&fx, "approve_proposal", json!({"id":1})).unwrap();
    let id = out["result"]["concept_id"].as_i64().unwrap();
    let store = hieronymus::concepts::ConceptStore::open(&fx.config).unwrap();
    assert_eq!(store.get(id).unwrap().description, "City name proposal");
    let facets = store.list_facets(id).unwrap();
    let view = hiero::application::admin::snapshot(
        &fx.config,
        "Concepts",
        &json!({"series":"s1", "selected_id":id.to_string()}),
    )
    .unwrap();
    assert!(
        view["detail"]["body"]
            .as_str()
            .unwrap()
            .contains("note [forbidden-variant]: Verele")
    );
    assert!(
        facets
            .iter()
            .find(|f| f.value == "Verell")
            .unwrap()
            .semantic_tags
            .contains(&"approved-variant".into())
    );
    assert!(
        facets
            .iter()
            .find(|f| f.value == "Verele")
            .unwrap()
            .semantic_tags
            .contains(&"forbidden-variant".into())
    );
    assert!(
        !facets
            .iter()
            .any(|f| f.value == "Verele" && (f.is_canonical || f.facet_type == "rendering"))
    );

    assert!(facets.iter().any(|f| f.value == "Verel" && f.is_canonical));
    assert!(
        facets
            .iter()
            .any(|f| f.value == "Verell" && f.facet_type == "rendering" && !f.is_canonical)
    );
    assert!(
        facets
            .iter()
            .any(|f| f.value == "Verele" && f.facet_type == "note" && !f.is_canonical)
    );
    assert_eq!(
        count(
            &db,
            "select count(*) from concept_facet_semantic_tags where semantic_tag='forbidden-variant'"
        ),
        1
    );
}
