//! Reconsolidation and recall feedback (task 5 of the Rust port), ported from
//! the §Testing Considerations of
//! `docs/superpowers/specs/2026-07-18-memory-reconsolidation-design.md` under
//! the ADR 0011 feedback contract: feedback addresses one `recall_id`,
//! dreaming consumes each feedback event at most once, and active
//! deterministic rule authority is never touched by passive evidence.

use hieronymus::concepts::{ConceptStore, NewConcept};
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_config::{default_dream_config, load_dream_config, save_dream_config};
use hieronymus::dreaming::{DeterministicDreamProvider, DreamRunRecord, DreamService};
use hieronymus::feedback::{
    FeedbackError, FeedbackStore, RECALLED_AGAIN_DELTAS, RECALLED_MISS_DELTAS,
    RECALLED_USEFUL_DELTAS, RecallFeedback,
};
use hieronymus::memory_models::TranslationContext;
use hieronymus::recall::{
    RecallHit, RecallResponse, RecallService, SPREADING_ACTIVATION_THRESHOLD,
};
use hieronymus::registry::Registry;
use hieronymus::terminology::Termbase;
use hieronymus::workspace::WorkspaceStore;
use serde_json::{Value, json};

// ----------------------------------------------------------------------
// Fixtures
// ----------------------------------------------------------------------

fn config(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path().join("hieronymus"))
}

fn context(series_slug: &str) -> TranslationContext {
    TranslationContext::new(series_slug, "ja", "en", "translation")
        .volume("1")
        .chapter("2")
}

/// An active session in a registered series, ready for recall.
fn active_session(root: &tempfile::TempDir, slug: &str) -> (HieronymusConfig, i64) {
    let config = config(root);
    Registry::open(&config)
        .unwrap()
        .create_series(slug, slug, "ja", "en", None)
        .unwrap();
    let session = WorkspaceStore::open(&config)
        .unwrap()
        .start_session(&context(slug))
        .unwrap();
    (config, session.id)
}

fn add_crystal(config: &HieronymusConfig, crystal_context: &TranslationContext, text: &str) -> i64 {
    add_crystal_with(config, crystal_context, "lesson", text, |new| new)
}

fn add_crystal_with(
    config: &HieronymusConfig,
    crystal_context: &TranslationContext,
    crystal_type: &str,
    text: &str,
    build: impl FnOnce(NewCrystal) -> NewCrystal,
) -> i64 {
    CrystalStore::open(config)
        .unwrap()
        .add_crystal(
            crystal_context,
            crystal_type,
            &build(NewCrystal::new("lesson", text)),
        )
        .unwrap()
}

fn query(config: &HieronymusConfig, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Vec<Vec<Value>> {
    let connection = hieronymus::db::open_migrated(&config.database_path()).unwrap();
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

fn crystal_score(config: &HieronymusConfig, column: &str, crystal_id: i64) -> f64 {
    scalar_params(
        config,
        &format!("select {column} from crystals where id = ?1"),
        &[&crystal_id],
    )
    .as_f64()
    .unwrap()
}

fn working_copy_id(config: &HieronymusConfig, crystal_id: i64) -> i64 {
    scalar_params(
        config,
        "select id from short_term_memories where source_crystal_id = ?1",
        &[&crystal_id],
    )
    .as_i64()
    .unwrap()
}

fn set_working_copy_text(config: &HieronymusConfig, memory_id: i64, text: &str) {
    hieronymus::db::open_migrated(&config.database_path())
        .unwrap()
        .execute(
            "update short_term_memories set text = ?1 where id = ?2",
            rusqlite::params![text, memory_id],
        )
        .unwrap();
}

fn insert_link(config: &HieronymusConfig, source: i64, target: i64, weight: f64, link_type: &str) {
    hieronymus::db::open_migrated(&config.database_path())
        .unwrap()
        .execute(
            "insert into crystal_links(source_crystal_id, target_crystal_id, link_type, weight)
             values (?1, ?2, ?3, ?4)",
            rusqlite::params![source, target, link_type, weight],
        )
        .unwrap();
}

fn link_crystal_to_concept(config: &HieronymusConfig, crystal_id: i64, concept_id: i64) {
    hieronymus::db::open_migrated(&config.database_path())
        .unwrap()
        .execute(
            "insert into crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at)
             values (?1, ?2, 'mentions', 0.5, datetime('now'))",
            rusqlite::params![crystal_id, concept_id],
        )
        .unwrap();
}

fn dream(config: &HieronymusConfig) -> DreamRunRecord {
    DreamService::open(config, DeterministicDreamProvider)
        .unwrap()
        .run_all("admin", true, false)
        .unwrap()
}

fn feedback(recall_id: &str, useful: &[i64], miss: &[i64], key: &str) -> RecallFeedback {
    RecallFeedback {
        recall_id: recall_id.to_string(),
        useful_activation_ids: useful.to_vec(),
        missed_activation_ids: miss.to_vec(),
        idempotency_key: key.to_string(),
    }
}

fn recall(
    config: &HieronymusConfig,
    session_id: i64,
    series: &str,
    query_text: &str,
) -> RecallResponse {
    RecallService::open(config)
        .unwrap()
        .recall(session_id, &context(series), query_text, 10)
        .unwrap()
}

/// Long-term hits of a recall response as (crystal_id, activation_id, score).
fn long_term_hits(response: &RecallResponse) -> Vec<(i64, i64, f64)> {
    let mut hits: Vec<(i64, i64, f64)> = response
        .hits
        .iter()
        .filter_map(|hit| match hit {
            RecallHit::LongTerm {
                crystal,
                score,
                activation_id,
                ..
            } => Some((crystal.id, *activation_id, *score)),
            _ => None,
        })
        .collect();
    hits.sort_by(|left, right| right.2.partial_cmp(&left.2).unwrap());
    hits
}

fn activation_of(hits: &[(i64, i64, f64)], crystal_id: i64) -> i64 {
    hits.iter()
        .find(|hit| hit.0 == crystal_id)
        .unwrap_or_else(|| panic!("crystal {crystal_id} missing from recall hits"))
        .1
}

// ----------------------------------------------------------------------
// Recall-time behavior
// ----------------------------------------------------------------------

#[test]
fn two_recalls_create_one_working_copy_two_activations_one_recalled_again() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let crystal_id = add_crystal(&config, &crystal_context, "The binding ritual needs chalk.");

    let first = recall(&config, session_id, "demo", "binding ritual chalk");
    let second = recall(&config, session_id, "demo", "binding ritual chalk");

    // Each invocation has its own recall_id, and every returned activation
    // exposes its activation id (ADR 0011 feedback contract).
    assert!(!first.recall_id.is_empty());
    assert!(!second.recall_id.is_empty());
    assert_ne!(first.recall_id, second.recall_id);
    let first_hits = long_term_hits(&first);
    let second_hits = long_term_hits(&second);
    assert_eq!(first_hits.len(), 1);
    assert_eq!(second_hits.len(), 1);
    assert_eq!(first_hits[0].0, crystal_id);
    assert_eq!(second_hits[0].0, crystal_id);
    assert_ne!(first_hits[0].1, second_hits[0].1);

    // Exactly one session-scoped working copy, seeded from the crystal.
    let working_copies = query(
        &config,
        "select source_crystal_id, text, session_id, archived_at
         from short_term_memories
         where source_crystal_id is not null",
        &[],
    );
    assert_eq!(working_copies.len(), 1);
    assert_eq!(working_copies[0][0], json!(crystal_id));
    assert_eq!(
        working_copies[0][1],
        json!("The binding ritual needs chalk.")
    );
    assert_eq!(working_copies[0][2], json!(session_id));
    assert_eq!(working_copies[0][3], Value::Null);

    // Two activation rows, one per invocation, each stamped with its
    // invocation's recall_id, and the exposed ids match the rows.
    let activations = query(
        &config,
        "select id, recall_id, outcome from crystal_activations
         where crystal_id = ?1 order by id",
        &[&crystal_id],
    );
    assert_eq!(activations.len(), 2);
    assert_eq!(activations[0][0], json!(first_hits[0].1));
    assert_eq!(activations[1][0], json!(second_hits[0].1));
    assert_eq!(activations[0][1], json!(first.recall_id));
    assert_eq!(activations[1][1], json!(second.recall_id));
    assert_eq!(activations[0][2], Value::Null);
    assert_eq!(activations[1][2], Value::Null);

    // The second recall's dedup hit is captured as one recalled_again event
    // for the next reinforcement pass, not applied immediately.
    let events = query(
        &config,
        "select crystal_id, strength_delta, confidence_delta, applied
         from memory_events where event_type = 'recalled_again'",
        &[],
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0][0], json!(crystal_id));
    assert_eq!(events[0][1], json!(RECALLED_AGAIN_DELTAS.0));
    assert_eq!(events[0][2], json!(RECALLED_AGAIN_DELTAS.1));
    assert_eq!(events[0][3], json!(0));
}

#[test]
fn credibility_and_rule_intent_boosts_order_recall_results() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let text = "credibility ranking drives score order";
    let expert = add_crystal_with(&config, &crystal_context, "lesson", text, |new| {
        new.source_credibility("expert")
    });
    let rumor = add_crystal_with(&config, &crystal_context, "lesson", text, |new| {
        new.source_credibility("rumor")
    });
    let with_intent = add_crystal_with(&config, &crystal_context, "lesson", text, |new| {
        new.rule_intent("correction")
    });
    let without_intent = add_crystal_with(&config, &crystal_context, "lesson", text, |new| new);

    let response = recall(&config, session_id, "demo", "credibility ranking score");
    let hits = long_term_hits(&response);
    let score = |crystal_id: i64| {
        hits.iter()
            .find(|hit| hit.0 == crystal_id)
            .unwrap_or_else(|| panic!("crystal {crystal_id} missing from hits"))
            .2
    };

    assert_eq!(hits.len(), 4);
    // Additive credibility confidence: expert (0.85) outranks rumor (0.15).
    assert!(score(expert) > score(rumor));
    assert!((score(expert) - score(rumor) - 0.7).abs() < 1e-6);
    // The flat RULE_INTENT_BOOST applies only when rule_intent is non-empty.
    assert!(score(with_intent) > score(without_intent));
    assert!((score(with_intent) - score(without_intent) - 0.2).abs() < 1e-6);
}

#[test]
fn spreading_activation_pulls_one_hop_neighbor_at_attenuated_score() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let anchor = add_crystal(
        &config,
        &crystal_context,
        "The chalk binding ritual anchors the spreading test.",
    );
    let neighbor = add_crystal(
        &config,
        &crystal_context,
        "Unrelated neighbor crystal about candle wax storage.",
    );
    let two_hop = add_crystal(
        &config,
        &crystal_context,
        "Second hop crystal that must never be pulled by recall.",
    );
    // A metadata-only candidate enters the pool below the spreading
    // threshold and must not spread.
    let cold = add_crystal_with(
        &config,
        &crystal_context,
        "lesson",
        "Cold crystal about bucket polish.",
        |new| NewCrystal {
            story_scopes: vec!["volume:1".to_string()],
            ..new
        },
    );
    let cold_neighbor = add_crystal(
        &config,
        &crystal_context,
        "Cold neighbor that stays outside the recall response.",
    );

    insert_link(&config, anchor, neighbor, 0.8, "related");
    insert_link(&config, neighbor, two_hop, 0.9, "related");
    insert_link(&config, cold, cold_neighbor, 0.9, "related");

    let response = recall(
        &config,
        session_id,
        "demo",
        "chalk binding ritual spreading",
    );
    let hits = long_term_hits(&response);
    let anchor_score = hits
        .iter()
        .find(|hit| hit.0 == anchor)
        .expect("anchor must be recalled")
        .2;
    assert!(anchor_score > SPREADING_ACTIVATION_THRESHOLD);

    // One hop only, at the attenuated score, never recursing further.
    let neighbor_score = hits
        .iter()
        .find(|hit| hit.0 == neighbor)
        .expect("one-hop neighbor must be pulled")
        .2;
    assert!((neighbor_score - anchor_score * 0.8 * 0.5).abs() < 1e-9);
    assert!(!hits.iter().any(|hit| hit.0 == two_hop));
    // Below-threshold candidates do not spread.
    assert!(!hits.iter().any(|hit| hit.0 == cold_neighbor));

    // A returned spreading-activation hit is a first-class candidate with
    // its own activation row marked by reason.
    let neighbor_reason = scalar_params(
        &config,
        "select reason from crystal_activations where crystal_id = ?1",
        &[&neighbor],
    );
    assert_eq!(neighbor_reason, json!("spreading_activation"));
}

// ----------------------------------------------------------------------
// Feedback signal
// ----------------------------------------------------------------------

#[test]
fn feedback_applies_deltas_outcomes_once_and_replays_noop() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let useful_crystal = add_crystal(&config, &crystal_context, "Useful crystal about chalk.");
    let miss_crystal = add_crystal(&config, &crystal_context, "Missed crystal about candles.");

    let first = recall(&config, session_id, "demo", "useful chalk");
    let second = recall(&config, session_id, "demo", "missed candles");
    let useful_activation = activation_of(&long_term_hits(&first), useful_crystal);
    let miss_activation = activation_of(&long_term_hits(&second), miss_crystal);

    let store = FeedbackStore::open(&config).unwrap();
    let outcome = store
        .record_recall_outcome(&feedback(&first.recall_id, &[useful_activation], &[], "k1"))
        .unwrap();
    assert!(outcome.applied);

    // Immediate deltas via the event-sourced scoring pattern, applied once.
    assert!(
        (crystal_score(&config, "strength", useful_crystal) - (0.5 + RECALLED_USEFUL_DELTAS.0))
            .abs()
            < 1e-9
    );
    assert!(
        (crystal_score(&config, "confidence", useful_crystal) - (0.5 + RECALLED_USEFUL_DELTAS.1))
            .abs()
            < 1e-9
    );
    let events = query(
        &config,
        "select event_type, applied, evidence, strength_delta, confidence_delta
         from memory_events where event_type in ('recalled_useful', 'recalled_miss')",
        &[],
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0][0], json!("recalled_useful"));
    assert_eq!(events[0][1], json!(1));
    assert_eq!(events[0][2], json!("k1"));
    assert_eq!(events[0][3], json!(RECALLED_USEFUL_DELTAS.0));
    assert_eq!(events[0][4], json!(RECALLED_USEFUL_DELTAS.1));
    assert_eq!(
        scalar_params(
            &config,
            "select outcome from crystal_activations where id = ?1",
            &[&useful_activation]
        ),
        json!("useful")
    );

    // Mixed useful/miss feedback in one request addresses each activation.
    let outcome = store
        .record_recall_outcome(&feedback(&second.recall_id, &[], &[miss_activation], "k2"))
        .unwrap();
    assert!(outcome.applied);
    assert!(
        (crystal_score(&config, "strength", miss_crystal) - (0.5 + RECALLED_MISS_DELTAS.0)).abs()
            < 1e-9
    );
    let outcomes: Vec<String> = query(
        &config,
        "select outcome from crystal_activations order by id",
        &[],
    )
    .into_iter()
    .map(|row| row[0].as_str().unwrap().to_string())
    .collect();
    assert_eq!(outcomes, vec!["useful".to_string(), "miss".to_string()]);

    // Replay with the same key is an explicit no-op: no new events, no
    // further score change.
    let replay = store
        .record_recall_outcome(&feedback(&first.recall_id, &[useful_activation], &[], "k1"))
        .unwrap();
    assert!(!replay.applied);
    assert!(
        (crystal_score(&config, "strength", useful_crystal) - (0.5 + RECALLED_USEFUL_DELTAS.0))
            .abs()
            < 1e-9
    );

    // A different key over already-scored activations is the same replay:
    // the activation outcomes are the other half of the at-most-once ledger.
    let replay = store
        .record_recall_outcome(&feedback(&first.recall_id, &[useful_activation], &[], "k9"))
        .unwrap();
    assert!(!replay.applied);
    assert_eq!(
        scalar(
            &config,
            "select count(*) from memory_events
             where event_type in ('recalled_useful', 'recalled_miss')"
        ),
        json!(2)
    );
}

#[test]
fn feedback_rejects_mismatched_activation_ids() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let crystal_a = add_crystal(&config, &crystal_context, "Crystal alpha mismatch zone.");
    let crystal_b = add_crystal(&config, &crystal_context, "Crystal beta mismatch region.");

    let first = recall(&config, session_id, "demo", "alpha zone");
    let second = recall(&config, session_id, "demo", "beta region");
    let activation_a = activation_of(&long_term_hits(&first), crystal_a);
    let activation_b = activation_of(&long_term_hits(&second), crystal_b);

    let store = FeedbackStore::open(&config).unwrap();
    // An activation from another recall invocation does not belong here.
    let error = store
        .record_recall_outcome(&feedback(&first.recall_id, &[activation_b], &[], "k1"))
        .unwrap_err();
    assert!(matches!(
        error,
        FeedbackError::ActivationMismatch { mismatched, .. }
            if mismatched == [activation_b]
    ));
    // Unknown activation ids fail with the same clear error.
    let error = store
        .record_recall_outcome(&feedback(&first.recall_id, &[99999], &[], "k2"))
        .unwrap_err();
    assert!(matches!(
        error,
        FeedbackError::ActivationMismatch { mismatched, .. } if mismatched == [99999]
    ));
    // Unknown recall ids fail closed.
    let error = store
        .record_recall_outcome(&feedback("rc-unknown", &[activation_a], &[], "k3"))
        .unwrap_err();
    assert!(matches!(error, FeedbackError::UnknownRecall(_)));
    // An empty idempotency key is rejected.
    let error = store
        .record_recall_outcome(&feedback(&first.recall_id, &[activation_a], &[], ""))
        .unwrap_err();
    assert!(matches!(error, FeedbackError::EmptyIdempotencyKey));

    // None of the rejected requests mutated anything.
    assert_eq!(
        scalar(
            &config,
            "select count(*) from memory_events where event_type like 'recalled_%'"
        ),
        json!(0)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from crystal_activations where outcome is not null"
        ),
        json!(0)
    );
    assert_eq!(crystal_score(&config, "strength", crystal_a), 0.5);
    assert_eq!(crystal_score(&config, "strength", crystal_b), 0.5);
}

#[test]
fn negative_feedback_dampens_rule_intent_but_never_touches_term_rules() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let plain = add_crystal(&config, &crystal_context, "Plain crystal for decay rates.");
    let advisory = add_crystal_with(
        &config,
        &crystal_context,
        "lesson",
        "Advisory rule-intent crystal for decay rates.",
        |new| new.rule_intent("correction"),
    );

    // An approved active rule exists; passive evidence must never alter it.
    let termbase = Termbase::open(&config, &crystal_context).unwrap();
    let rule = termbase
        .propose("Sense", "Сенс", &Default::default())
        .unwrap();
    termbase
        .approve(rule.id, "tester", "approved for the test")
        .unwrap();
    let rule_before = query(
        &config,
        "select id, status, source_text, canonical_translation, revision from term_rules",
        &[],
    );

    let response = recall(&config, session_id, "demo", "decay rates");
    let hits = long_term_hits(&response);
    let plain_activation = activation_of(&hits, plain);
    let advisory_activation = activation_of(&hits, advisory);

    FeedbackStore::open(&config)
        .unwrap()
        .record_recall_outcome(&feedback(
            &response.recall_id,
            &[],
            &[plain_activation, advisory_activation],
            "miss-k1",
        ))
        .unwrap();

    // Identical histories, different rates: the advisory rule-intent crystal
    // decays at dampened rate, and neither is immune.
    let plain_after = crystal_score(&config, "strength", plain);
    let advisory_after = crystal_score(&config, "strength", advisory);
    assert!((plain_after - (0.5 + RECALLED_MISS_DELTAS.0)).abs() < 1e-9);
    assert!(
        (advisory_after - (0.5 + RECALLED_MISS_DELTAS.0 * 0.5)).abs() < 1e-9,
        "advisory rule-intent crystals dampen negative deltas by 0.5"
    );
    assert!(advisory_after < 0.5, "no immunity for advisory rule-intent");

    // The deterministic authority is untouched by negative feedback.
    assert_eq!(
        query(
            &config,
            "select id, status, source_text, canonical_translation, revision from term_rules",
            &[],
        ),
        rule_before
    );
}

// ----------------------------------------------------------------------
// Dream-time phases
// ----------------------------------------------------------------------

#[test]
fn reinforcement_consumes_recalled_again_once_and_never_reapplies_feedback() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let twice = add_crystal(
        &config,
        &crystal_context,
        "Repeated recall emphasis crystal.",
    );
    let once = add_crystal(&config, &crystal_context, "Single sighting crystal.");

    let first = recall(&config, session_id, "demo", "repeated recall emphasis");
    let _second = recall(&config, session_id, "demo", "single sighting");
    let third = recall(&config, session_id, "demo", "repeated recall emphasis");
    let twice_activation = activation_of(&long_term_hits(&first), twice);
    FeedbackStore::open(&config)
        .unwrap()
        .record_recall_outcome(&feedback(
            &first.recall_id,
            &[twice_activation],
            &[],
            "useful-k",
        ))
        .unwrap();

    let run = dream(&config);

    // Both crystals are trivially reconsolidated (+0.02 in place); the
    // recalled_again event from the third recall adds its passive
    // strength-only delta to `twice` exactly once; the immediate useful
    // delta is not reapplied by dreaming.
    let expected_once = 0.5 + 0.02;
    let expected_twice = expected_once + RECALLED_USEFUL_DELTAS.0 + RECALLED_AGAIN_DELTAS.0;
    assert!((crystal_score(&config, "strength", once) - expected_once).abs() < 1e-9);
    assert!((crystal_score(&config, "strength", twice) - expected_twice).abs() < 1e-9);
    assert!(
        (crystal_score(&config, "confidence", twice) - (0.5 + RECALLED_USEFUL_DELTAS.1)).abs()
            < 1e-9
    );
    let events = query(
        &config,
        "select crystal_id, applied, cycle_id from memory_events
         where event_type = 'recalled_again'",
        &[],
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0][0], json!(twice));
    assert_eq!(events[0][1], json!(1));
    assert_eq!(events[0][2], json!(run.cycle_id));
    let _ = third;

    // A second cycle must not consume anything again.
    dream(&config);
    assert!((crystal_score(&config, "strength", once) - expected_once).abs() < 1e-9);
    assert!((crystal_score(&config, "strength", twice) - expected_twice).abs() < 1e-9);
    assert_eq!(
        scalar(
            &config,
            "select count(*) from memory_events where event_type = 'recalled_useful'"
        ),
        json!(1)
    );
}

#[test]
fn reconsolidation_supersedes_diverged_working_copy() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let concept = ConceptStore::open(&config)
        .unwrap()
        .create_concept("chalk ritual", &NewConcept::default())
        .unwrap();
    let crystal_id = add_crystal(&config, &crystal_context, "The binding ritual needs chalk.");
    let control = add_crystal(
        &config,
        &crystal_context,
        "Control crystal stays untouched.",
    );
    link_crystal_to_concept(&config, crystal_id, concept.id);

    recall(&config, session_id, "demo", "binding ritual chalk");
    let working_copy = working_copy_id(&config, crystal_id);
    // Diverge far beyond the default threshold (0.20 of token count).
    set_working_copy_text(
        &config,
        working_copy,
        "Completely different content about quantum tea ceremony protocols.",
    );

    let _run = dream(&config);

    // The original is superseded, a new crystal carries the working copy's
    // text with the original's concepts copied, and the working copy is
    // archived. No unrelated crystal is touched (affected-set containment).
    let original = query(
        &config,
        "select status, text from crystals where id = ?1",
        &[&crystal_id],
    )
    .remove(0);
    assert_eq!(original[0], json!("superseded"));
    let successor = query(
        &config,
        "select id, supersedes_crystal_id, text, status from crystals
         where supersedes_crystal_id = ?1",
        &[&crystal_id],
    )
    .remove(0);
    assert_ne!(successor[0].as_i64().unwrap(), crystal_id);
    assert_eq!(successor[1], json!(crystal_id));
    assert_eq!(
        successor[2],
        json!("Completely different content about quantum tea ceremony protocols.")
    );
    assert_eq!(successor[3], json!("active"));
    let successor_id = successor[0].as_i64().unwrap();
    assert_eq!(
        query(
            &config,
            "select crystal_id from crystal_concepts where concept_id = ?1 order by crystal_id",
            &[&concept.id]
        ),
        vec![vec![json!(crystal_id)], vec![json!(successor_id)]]
    );
    assert_eq!(
        scalar(&config, "select count(*) from crystals"),
        json!(3),
        "exactly one new crystal: affected-set containment"
    );
    assert_eq!(
        scalar_params(
            &config,
            "select archived_at is not null from short_term_memories where id = ?1",
            &[&working_copy]
        ),
        json!(1)
    );
    assert_eq!(crystal_score(&config, "strength", control), 0.5);
}

#[test]
fn reconsolidation_reinforces_trivial_edit() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let crystal_id = add_crystal(&config, &crystal_context, "The binding ritual needs chalk.");

    recall(&config, session_id, "demo", "binding ritual chalk");
    let working_copy = working_copy_id(&config, crystal_id);
    // A one-word addition stays far below the 0.20 token diff threshold.
    set_working_copy_text(&config, working_copy, "The binding ritual needs chalk now.");

    let run = dream(&config);

    // Reinforce the original in place: no new crystal, working copy archived.
    assert_eq!(scalar(&config, "select count(*) from crystals"), json!(1));
    assert!((crystal_score(&config, "strength", crystal_id) - 0.52).abs() < 1e-9);
    let original = query(
        &config,
        "select status, last_reinforced_cycle from crystals where id = ?1",
        &[&crystal_id],
    )
    .remove(0);
    assert_eq!(original[0], json!("active"));
    assert_eq!(original[1], json!(run.cycle_id));
    assert_eq!(
        scalar_params(
            &config,
            "select archived_at is not null from short_term_memories where id = ?1",
            &[&working_copy]
        ),
        json!(1)
    );
}

#[test]
fn reconsolidation_diff_threshold_config_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    assert_eq!(
        load_dream_config(&config)
            .unwrap()
            .reconsolidation_diff_threshold,
        0.20
    );

    let mut dream_config = default_dream_config();
    dream_config.reconsolidation_diff_threshold = 0.5;
    save_dream_config(&config, &dream_config).unwrap();
    assert_eq!(
        load_dream_config(&config)
            .unwrap()
            .reconsolidation_diff_threshold,
        0.5
    );

    // Validation rejects out-of-range and non-float values.
    let mut invalid = default_dream_config();
    invalid.reconsolidation_diff_threshold = 0.0;
    assert!(save_dream_config(&config, &invalid).is_err());
    let mut invalid = default_dream_config();
    invalid.reconsolidation_diff_threshold = 1.5;
    assert!(save_dream_config(&config, &invalid).is_err());
    std::fs::write(
        config.dream_config_path(),
        "[dreaming]\nreconsolidation_diff_threshold = \"high\"\n",
    )
    .unwrap();
    assert!(load_dream_config(&config).is_err());
    std::fs::write(
        config.dream_config_path(),
        "[dreaming]\nreconsolidation_diff_threshold = 0.5\nunknown_field = 1\n",
    )
    .unwrap();
    assert!(load_dream_config(&config).is_err());
}

#[test]
fn combination_merges_near_duplicate_useful_pair() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    // Near-duplicates: token sets overlap far above the similarity threshold.
    let absorbed = add_crystal(
        &config,
        &crystal_context,
        "The binding ritual needs chalk and candles tonight.",
    );
    let survivor = add_crystal_with(
        &config,
        &crystal_context,
        "lesson",
        "The binding ritual needs chalk and candles today.",
        |new| new.source_credibility("expert"),
    );
    let bystander = add_crystal(
        &config,
        &crystal_context,
        "Bystander crystal about hat storage.",
    );
    let concept = ConceptStore::open(&config)
        .unwrap()
        .create_concept("candle ritual", &NewConcept::default())
        .unwrap();
    link_crystal_to_concept(&config, absorbed, concept.id);
    insert_link(&config, absorbed, bystander, 0.7, "related");

    let response = recall(&config, session_id, "demo", "binding ritual chalk candles");
    let hits = long_term_hits(&response);
    let absorbed_activation = activation_of(&hits, absorbed);
    let survivor_activation = activation_of(&hits, survivor);
    FeedbackStore::open(&config)
        .unwrap()
        .record_recall_outcome(&feedback(
            &response.recall_id,
            &[absorbed_activation, survivor_activation],
            &[],
            "combine-k",
        ))
        .unwrap();

    let run = dream(&config);

    // Pairwise combination: one event, survivor keeps the union, absorbed is
    // superseded. `supersedes_crystal_id` is not reused for combination.
    let events = query(
        &config,
        "select crystal_id, event_type, evidence, applied, cycle_id from memory_events
         where event_type = 'combined_into'",
        &[],
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0][0], json!(absorbed));
    assert_eq!(events[0][2], json!(survivor.to_string()));
    assert_eq!(events[0][3], json!(1));
    assert_eq!(events[0][4], json!(run.cycle_id));
    let absorbed_row = query(
        &config,
        "select status, supersedes_crystal_id from crystals where id = ?1",
        &[&absorbed],
    )
    .remove(0);
    assert_eq!(absorbed_row[0], json!("superseded"));
    assert_eq!(absorbed_row[1], Value::Null);
    // Higher source_credibility weight wins the survivor role, and the
    // survivor inherits the absorbed crystal's concepts and links.
    let survivor_row = query(
        &config,
        "select status, source_credibility from crystals where id = ?1",
        &[&survivor],
    )
    .remove(0);
    assert_eq!(survivor_row[0], json!("active"));
    assert_eq!(survivor_row[1], json!("expert"));
    assert_eq!(
        scalar_params(
            &config,
            "select count(*) from crystal_concepts
             where crystal_id = ?1 and concept_id = ?2",
            &[&survivor, &concept.id]
        ),
        json!(1)
    );
    assert_eq!(
        scalar_params(
            &config,
            "select count(*) from crystal_links
             where source_crystal_id = ?1 and target_crystal_id = ?2
               and link_type = 'related'",
            &[&survivor, &bystander]
        ),
        json!(1)
    );
    assert_eq!(
        scalar(&config, "select count(*) from crystals"),
        json!(3),
        "pairwise only: no new crystals created by combination"
    );
}

#[test]
fn hebbian_links_strengthen_across_cycles() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    // Dissimilar crystals that still share the query tokens: co-activated
    // useful, never combined.
    let left = add_crystal(
        &config,
        &crystal_context,
        "Quantum chalk experiments unfolded today.",
    );
    let right = add_crystal(
        &config,
        &crystal_context,
        "The quantum chalk ledger closed early.",
    );
    let co_activation_link = "select weight from crystal_links
     where link_type = 'co_activation'
       and ((source_crystal_id = ?1 and target_crystal_id = ?2)
         or (source_crystal_id = ?2 and target_crystal_id = ?1))";

    let first = recall(&config, session_id, "demo", "quantum chalk");
    let hits = long_term_hits(&first);
    FeedbackStore::open(&config)
        .unwrap()
        .record_recall_outcome(&feedback(
            &first.recall_id,
            &[activation_of(&hits, left), activation_of(&hits, right)],
            &[],
            "heb-k1",
        ))
        .unwrap();
    dream(&config);
    let first_weight = scalar_params(&config, co_activation_link, &[&left, &right])
        .as_f64()
        .unwrap();
    assert!(
        (first_weight - 0.5).abs() < 1e-9,
        "created link starts at the initial weight"
    );

    // A second co-activation cycle strengthens the existing link.
    let second = recall(&config, session_id, "demo", "quantum chalk");
    let hits = long_term_hits(&second);
    FeedbackStore::open(&config)
        .unwrap()
        .record_recall_outcome(&feedback(
            &second.recall_id,
            &[activation_of(&hits, left), activation_of(&hits, right)],
            &[],
            "heb-k2",
        ))
        .unwrap();
    dream(&config);
    let second_weight = scalar_params(&config, co_activation_link, &[&left, &right])
        .as_f64()
        .unwrap();
    assert!(
        (second_weight - (first_weight + 0.1)).abs() < 1e-9,
        "hebbian strengthening must raise the weight"
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from crystal_links where link_type = 'co_activation'"
        ),
        json!(1)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from memory_events where event_type = 'combined_into'"
        ),
        json!(0),
        "dissimilar co-activated crystals never combine"
    );
}

#[test]
fn dream_protects_active_rule_crystals_from_supersession_and_combination() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let rule_crystal = add_crystal_with(
        &config,
        &crystal_context,
        "rule",
        "The binding ritual needs chalk.",
        |new| NewCrystal {
            source_credibility: "user_rule".to_string(),
            rule_intent: "correction".to_string(),
            status: "active".to_string(),
            ..new
        },
    );

    // Supersession path: the rule crystal's working copy diverges hard.
    recall(&config, session_id, "demo", "binding ritual chalk");
    let working_copy = working_copy_id(&config, rule_crystal);
    set_working_copy_text(
        &config,
        working_copy,
        "Completely divergent working copy about quantum tea protocols.",
    );
    dream(&config);

    let rule_row = query(
        &config,
        "select status, text from crystals where id = ?1",
        &[&rule_crystal],
    )
    .remove(0);
    assert_eq!(
        rule_row[0],
        json!("active"),
        "dreaming never transitions an active rule crystal"
    );
    assert_eq!(rule_row[1], json!("The binding ritual needs chalk."));
    assert_eq!(
        scalar(&config, "select count(*) from crystals"),
        json!(1),
        "no successor crystal for a protected rule crystal"
    );
    assert_eq!(
        scalar_params(
            &config,
            "select archived_at is not null from short_term_memories where id = ?1",
            &[&working_copy]
        ),
        json!(1),
        "the processed working copy is still archived"
    );
}

#[test]
fn reconsolidation_never_acts_on_a_non_active_source_crystal() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let crystal_id = add_crystal(&config, &crystal_context, "The binding ritual needs chalk.");

    recall(&config, session_id, "demo", "binding ritual chalk");
    let working_copy = working_copy_id(&config, crystal_id);
    // Diverge far beyond the default threshold (0.20): without the guard
    // this working copy supersedes its source.
    set_working_copy_text(
        &config,
        working_copy,
        "Completely different content about quantum tea ceremony protocols.",
    );
    // The source crystal was combined away after the working copy was
    // created (status set directly; the combination path produces the same
    // row state).
    hieronymus::db::open_migrated(&config.database_path())
        .unwrap()
        .execute(
            "update crystals set status = 'superseded' where id = ?1",
            rusqlite::params![crystal_id],
        )
        .unwrap();

    let _run = dream(&config);

    // No successor crystal is crystallized, the absorbed row is untouched,
    // and the pending working copy is archived as unprocessable.
    assert_eq!(
        scalar(&config, "select count(*) from crystals"),
        json!(1),
        "a non-active source must never grow a fresh active successor"
    );
    let source = query(
        &config,
        "select status, strength, last_reinforced_cycle, updated_at is not null
         from crystals where id = ?1",
        &[&crystal_id],
    )
    .remove(0);
    assert_eq!(source[0], json!("superseded"));
    assert_eq!(
        source[1],
        json!(0.5),
        "the absorbed row is never reinforced"
    );
    assert_eq!(source[2], Value::Null);
    assert_eq!(
        scalar_params(
            &config,
            "select archived_at is not null from short_term_memories where id = ?1",
            &[&working_copy]
        ),
        json!(1),
        "the pending copy retires instead of acting on an absorbed source"
    );

    // The reconsolidation audit records why the copy was retired.
    let payloads = query(
        &config,
        "select payload_json from dream_audit_entries where event_type = 'phase_completed'",
        &[],
    );
    let actions: Vec<Value> = payloads
        .iter()
        .flat_map(|row| {
            let payload: Value = serde_json::from_str(row[0].as_str().unwrap()).unwrap();
            payload["actions"].as_array().cloned().unwrap_or_default()
        })
        .collect();
    assert!(
        actions.iter().any(|action| {
            action["action"] == json!("source_inactive")
                && action["memory_id"] == json!(working_copy)
                && action["crystal_id"] == json!(crystal_id)
        }),
        "the retirement must be audited with its reason, saw {actions:?}"
    );
}

#[test]
fn reconsolidation_respects_bounded_mutation_caps() {
    let root = tempfile::tempdir().unwrap();
    let (config, session_id) = active_session(&root, "demo");
    let crystal_context = context("demo");
    let first = add_crystal(
        &config,
        &crystal_context,
        "First capped crystal about chalk rituals.",
    );
    let second = add_crystal(
        &config,
        &crystal_context,
        "Second capped crystal about candle storage.",
    );

    recall(&config, session_id, "demo", "chalk rituals");
    recall(&config, session_id, "demo", "candle storage");
    for crystal_id in [first, second] {
        set_working_copy_text(
            &config,
            working_copy_id(&config, crystal_id),
            "Diverged far beyond the threshold for cap testing purposes indeed.",
        );
    }

    let mut dream_config = default_dream_config();
    dream_config.max_short_term_memories_per_run = 1;
    save_dream_config(&config, &dream_config).unwrap();
    dream(&config);

    // The absolute cap bounds reconsolidation to one working copy per run.
    assert_eq!(
        scalar(
            &config,
            "select count(*) from crystals where status = 'superseded'"
        ),
        json!(1)
    );
    assert_eq!(
        scalar_params(
            &config,
            "select count(*) from crystals
             where supersedes_crystal_id in (?1, ?2)",
            &[&first, &second]
        ),
        json!(1)
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories
             where source_crystal_id is not null and archived_at is not null"
        ),
        json!(1),
        "the second working copy stays pending"
    );
    assert_eq!(
        scalar(
            &config,
            "select count(*) from short_term_memories
             where source_crystal_id is not null and archived_at is null"
        ),
        json!(1)
    );
}

// ----------------------------------------------------------------------
// Algorithmic helpers
// ----------------------------------------------------------------------

#[test]
fn token_diff_ratio_and_similarity_are_bounded_and_meaningful() {
    use hieronymus::dreaming::{token_diff_ratio, token_similarity};

    let source = "the binding ritual needs chalk and candles";
    assert_eq!(token_diff_ratio(source, source), 0.0);
    assert!(
        (token_diff_ratio(source, "quantum tea ceremony protocols unfold tonight") - 1.0).abs()
            < 1e-9
    );
    // A one-token addition on a seven-token source is a trivial edit.
    assert!(token_diff_ratio(source, "the binding ritual needs chalk and candles today") < 0.2);

    assert!(
        (token_similarity(source, "THE BINDING RITUAL NEEDS CHALK AND CANDLES") - 1.0).abs() < 1e-9
    );
    assert_eq!(
        token_similarity(source, "completely unrelated words entirely"),
        0.0
    );
    assert!(token_similarity("needs chalk and candles", "needs chalk and candles today") > 0.7);
}
