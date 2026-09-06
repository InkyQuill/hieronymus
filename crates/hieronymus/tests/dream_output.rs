//! Task D2: the complete normalized dream-output contract (ADR 0003, ADR
//! 0011). Every provider output section — crystals with concept names,
//! concept proposals, concepts, facets, supersede actions, and reinforce
//! actions — must normalize, validate, and persist through the staged dream
//! transaction, with malformed entries rejected individually into durable
//! audit channels. Dream has no approval authority: model output may propose
//! rules but can never activate, replace, or archive an active rule, and
//! mutation targets are checked against the selected context before any
//! store call.

use std::collections::BTreeSet;

use hieronymus::concepts::ConceptStore;
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use hieronymus::dream_output::validate_action_targets;
use hieronymus::dream_workflows::WorkflowResolver;
use hieronymus::dreaming::{DreamError, DreamProvider, DreamService};
use hieronymus::memory_models::{ShortTermMemoryRecord, TranslationContext};
use hieronymus::registry::Registry;
use hieronymus::workspace::{ShortTermMemoryInput, WorkspaceStore};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// The brief's regression: no dream action may touch an active rule
// ---------------------------------------------------------------------------

#[test]
fn dream_cannot_supersede_an_active_rule() {
    let allowed = BTreeSet::from([11, 12]);
    let protected = BTreeSet::from([11]);
    let output = json!({"supersede_actions": [
        {"old_crystal_id": 11, "new_crystal_id": 12, "reason": "model suggestion"}
    ]});
    assert!(validate_action_targets(&output, &allowed, &protected).is_err());
}

#[test]
fn dream_cannot_supersede_outside_the_selected_context() {
    let allowed = BTreeSet::from([11, 12]);
    let protected = BTreeSet::new();
    let output = json!({"supersede_actions": [
        {"old_crystal_id": 11, "new_crystal_id": 99, "reason": "other series"}
    ]});
    assert!(validate_action_targets(&output, &allowed, &protected).is_err());
}

#[test]
fn dream_may_supersede_in_context_unprotected_crystals() {
    let allowed = BTreeSet::from([11, 12]);
    let protected = BTreeSet::new();
    let output = json!({"supersede_actions": [
        {"old_crystal_id": 11, "new_crystal_id": 12, "reason": "refined"}
    ]});
    assert!(validate_action_targets(&output, &allowed, &protected).is_ok());
    // A malformed id is a rejection, never a guess.
    let malformed = json!({"supersede_actions": [{"old_crystal_id": "11"}]});
    assert!(validate_action_targets(&malformed, &allowed, &protected).is_err());
    let not_an_array = json!({"supersede_actions": {"old_crystal_id": 11}});
    assert!(validate_action_targets(&not_an_array, &allowed, &protected).is_err());
}

// ---------------------------------------------------------------------------
// Test rig: one dream cycle driven by a scripted provider payload
// ---------------------------------------------------------------------------

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

fn completed_session(config: &HieronymusConfig, slug: &str, texts: &[&str]) -> Vec<i64> {
    let workspace = WorkspaceStore::open(config).unwrap();
    let session = workspace.start_session(&context(slug)).unwrap();
    let ids = texts
        .iter()
        .map(|text| {
            workspace
                .add_short_term_memory(session.id, &ShortTermMemoryInput::new("note", *text))
                .unwrap()
                .id
        })
        .collect::<Vec<_>>();
    workspace.complete_session(session.id).unwrap();
    ids
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

/// A provider that answers `knowledge_crystals` with a fixed payload,
/// accounts for the selection on `coverage_audit`, and returns empty output
/// for every other enabled pass.
struct GraphProvider {
    knowledge_crystals: Value,
}

impl DreamProvider for GraphProvider {
    fn name(&self) -> &str {
        "graph-test"
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
            return Ok(self.knowledge_crystals.clone());
        }
        Ok(json!({}))
    }
}

fn run_one_cycle(
    config: &HieronymusConfig,
    payload: Value,
) -> hieronymus::dreaming::DreamRunRecord {
    let service = DreamService::open(
        config,
        WorkflowResolver::serving(move || {
            Box::new(GraphProvider {
                knowledge_crystals: payload.clone(),
            })
        }),
    )
    .unwrap();
    service.run_cycle("manual", false).unwrap()
}

fn persistence_audit(config: &HieronymusConfig, run_id: i64) -> Value {
    audit_payloads(config, run_id, "phase_completed")
        .into_iter()
        .next()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Concepts, facets, and crystal concept names apply to the public stores
// ---------------------------------------------------------------------------

#[test]
fn graph_sections_apply_concepts_facets_and_crystal_links() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let memory_ids = completed_session(&config, "book", &["Fiorire means blossoming."]);

    let payload = json!({
        "concepts": [{
            "canonical_name": "Fiorire",
            "description": "Italian verb for blossoming.",
            "tags": ["flora"],
            "confidence": 0.5,
        }],
        "facets": [
            {
                "concept_name": "fiorire",
                "value": "расцвести",
                "kind": "rendering",
                "language_tags": ["ru"],
                "story_scopes": ["volume:1"],
                "semantic_tags": ["flora"],
                "confidence": 0.7,
                "is_canonical": true,
            },
            {
                "concept_name": "Fiorire",
                "value": "цвести",
                "kind": "name",
                "is_canonical": true,
            },
        ],
        "crystals": [{
            "crystal_type": "observation",
            "title": "Fiorire",
            "text": "Fiorire means blossoming.",
            "confidence": 0.8,
            "source_memory_ids": memory_ids,
            "concept_names": ["Fiorire"],
        }],
    });
    let run = run_one_cycle(&config, payload);

    assert_eq!(run.status, "completed");
    assert_eq!(run.created_crystal_count, 1);

    // Public concept projection: a global candidate with its tags.
    let concepts = ConceptStore::open(&config).unwrap();
    let stored = concepts.list_concepts(None, None).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].canonical_name, "Fiorire");
    assert_eq!(stored[0].status, "candidate");
    assert_eq!(stored[0].scope_type, "global");
    assert_eq!(stored[0].tags, vec!["flora".to_string()]);

    // Facets attach to the concept; canonical uniqueness holds even though
    // the provider flagged both facets as canonical: exactly one wins.
    let facets = concepts.list_facets(stored[0].id).unwrap();
    assert_eq!(facets.len(), 2);
    assert_eq!(facets.iter().filter(|facet| facet.is_canonical).count(), 1);
    let rendering = facets
        .iter()
        .find(|facet| facet.value == "расцвести")
        .unwrap();
    assert_eq!(rendering.kind(), "rendering");
    assert_eq!(rendering.language_tags, vec!["ru".to_string()]);
    assert_eq!(rendering.story_scopes, vec!["volume:1".to_string()]);
    assert_eq!(rendering.semantic_tags, vec!["flora".to_string()]);

    // The crystal resolved its concept name into a real concept link, and
    // the public crystal projection carries the same link.
    let crystals = CrystalStore::open(&config).unwrap();
    let crystal_ids: Vec<i64> = query(&config, "select id from crystals", &[])
        .remove(0)
        .into_iter()
        .map(|value| value.as_i64().unwrap())
        .collect();
    assert_eq!(crystal_ids.len(), 1);
    let crystal = crystals.get(crystal_ids[0]).unwrap();
    assert_eq!(crystal.concept_ids, vec![stored[0].id]);
    assert_eq!(crystal.title, "Fiorire");

    let links = query(
        &config,
        "select crystal_id, concept_id, link_type from crystal_concepts",
        &[],
    );
    assert_eq!(links.len(), 1);
    assert_eq!(links[0][1], json!(stored[0].id));
    assert_eq!(links[0][2], json!("mentions"));
}

#[test]
fn concept_proposals_persist_as_pending_candidates_never_active_rules() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    completed_session(&config, "book", &["Terminology candidate."]);

    let payload = json!({
        "concept_proposals": [{
            "series_slug": "book",
            "source_language": "ja",
            "target_language": "ru",
            "concept_text": "Fiorire",
            "source_form": "fiorire",
            "canonical_rendering": "расцвести",
            "approved_variants": ["расцветёт"],
            "forbidden_variants": ["цветень"],
            "rationale": "Consistent rendering across chapters.",
        }],
    });
    let run = run_one_cycle(&config, payload);

    assert_eq!(run.status, "completed");
    assert_eq!(run.proposal_count, 1);

    // The proposal lands pending with its run attribution; the structured
    // rule authority is untouched (dream has no approval authority).
    let proposals = ConceptStore::open(&config)
        .unwrap()
        .list_proposals()
        .unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0]["concept_text"], json!("Fiorire"));
    assert_eq!(proposals[0]["canonical_rendering"], json!("расцвести"));
    assert_eq!(proposals[0]["status"], json!("pending"));
    assert_eq!(scalar(&config, "select count(*) from term_rules"), json!(0));

    let proposal_row = query(
        &config,
        "select dream_run_id, approved_variants_json, forbidden_variants_json
         from strict_concept_proposals",
        &[],
    )
    .remove(0);
    assert_eq!(proposal_row[0], json!(run.id));
    assert_eq!(proposal_row[1], json!("[\"расцветёт\"]"));
    assert_eq!(proposal_row[2], json!("[\"цветень\"]"));
}

// ---------------------------------------------------------------------------
// Reinforce and supersede actions apply with graded-memory bookkeeping
// ---------------------------------------------------------------------------

#[test]
fn reinforce_and_supersede_actions_apply_through_the_run_transaction() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let memory_ids = completed_session(&config, "book", &["Conclusion."]);

    let store = CrystalStore::open(&config).unwrap();
    let old_id = store
        .add_crystal(
            &context("book"),
            "observation",
            &NewCrystal::new("observation", "Old conclusion."),
        )
        .unwrap();
    let new_id = store
        .add_crystal(
            &context("book"),
            "observation",
            &NewCrystal::new("observation", "Refined conclusion."),
        )
        .unwrap();

    let payload = json!({
        "reinforce": [{
            "crystal_id": new_id,
            "strength_delta": 0.1,
            "confidence_delta": 0.05,
            "source_memory_ids": memory_ids,
        }],
        "supersede": [{
            "old_crystal_id": old_id,
            "new_crystal_id": new_id,
            "reason": "Refined by the next chapter.",
        }],
    });
    let run = run_one_cycle(&config, payload);

    assert_eq!(run.status, "completed");

    // Supersede projection: old superseded, new points back.
    let old = store.get(old_id).unwrap();
    assert_eq!(old.status, "superseded");
    let new = store.get(new_id).unwrap();
    assert_eq!(new.supersedes_crystal_id, Some(old_id));

    // Reinforce projection: the clamped delta landed and the event records
    // the actual deltas, consumed exactly once.
    assert!((new.strength - 0.6).abs() < 1e-9);
    assert!((new.confidence - 0.55).abs() < 1e-9);
    let events = query(
        &config,
        "select event_type, source_role, evidence, strength_delta, confidence_delta, applied, cycle_id
         from memory_events where crystal_id = ?1 order by event_type",
        &[&new_id],
    );
    let reinforce = events
        .iter()
        .find(|row| row[0] == json!("dream_reinforce"))
        .unwrap();
    assert_eq!(reinforce[1], json!("system"));
    assert_eq!(reinforce[2], json!("dream reinforcement"));
    assert!((reinforce[3].as_f64().unwrap() - 0.1).abs() < 1e-9);
    assert!((reinforce[4].as_f64().unwrap() - 0.05).abs() < 1e-9);
    assert_eq!(reinforce[5], json!(1));
    assert_eq!(reinforce[6], json!(run.cycle_id));
    assert!(
        query(
            &config,
            "select count(*) from memory_events
             where crystal_id = ?1 and event_type = 'supersede'
               and evidence = 'Refined by the next chapter.'",
            &[&old_id],
        )
        .remove(0)
        .remove(0)
            == json!(1),
        "the supersede event is recorded on the old crystal"
    );

    // The persistence audit carries the real mutation sets.
    let audit = persistence_audit(&config, run.id);
    assert_eq!(audit["superseded_crystals"], json!([old_id]));
    assert_eq!(audit["reinforced_crystals"], json!([new_id]));
    assert_eq!(audit["accepted_entries"]["supersede_actions"], json!(1));
}

// ---------------------------------------------------------------------------
// Context isolation: per-context resolution, no cross-series leakage
// ---------------------------------------------------------------------------

#[test]
fn crystals_resolve_per_series_context_and_ambiguous_ones_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    create_series(&config, "zola");
    let book_ids = completed_session(&config, "book", &["Book conclusion."]);
    let zola_ids = completed_session(&config, "zola", &["Zola conclusion."]);

    let payload = json!({
        "concepts": [{"canonical_name": "Shared", "confidence": 0.4}],
        "crystals": [
            {
                "crystal_type": "observation",
                "title": "Book",
                "text": "Book conclusion.",
                "source_memory_ids": [book_ids[0]],
                "concept_names": ["Shared"],
            },
            {
                // No explicit context for its concept name and sources from
                // the other series: it must not land in book's context.
                "crystal_type": "observation",
                "title": "Zola",
                "text": "Zola conclusion.",
                "source_memory_ids": [zola_ids[0]],
                "concept_names": ["Shared"],
            },
            {
                // Cites memories of two series: no honest context exists.
                "crystal_type": "observation",
                "title": "Ambiguous",
                "text": "Spans two series.",
                "source_memory_ids": [book_ids[0], zola_ids[0]],
            },
            {
                // Same rejection, but with a provider-length title: the
                // rejection record may not echo the whole free-text value.
                "crystal_type": "observation",
                "title": "Very ".repeat(40).trim_end().to_string(),
                "text": "Spans two series with a long title.",
                "source_memory_ids": [book_ids[0], zola_ids[0]],
            },
        ],
    });
    let run = run_one_cycle(&config, payload);

    assert_eq!(run.status, "completed");
    assert_eq!(run.created_crystal_count, 2);

    // Context isolation: each crystal carries its own series' scope, never
    // the first group's context.
    let scopes = query(
        &config,
        "select title, series_slug, scope_key from crystals order by title",
        &[],
    );
    assert_eq!(scopes.len(), 2);
    let zola = scopes.iter().find(|row| row[0] == json!("Zola")).unwrap();
    assert_eq!(zola[1], json!("zola"));
    assert_eq!(zola[2], json!("series:zola"));
    let book = scopes.iter().find(|row| row[0] == json!("Book")).unwrap();
    assert_eq!(book[1], json!("book"));
    assert_eq!(book[2], json!("series:book"));

    // The shared concept resolved once (global) and both crystals link to it.
    assert_eq!(scalar(&config, "select count(*) from concepts"), json!(1));
    assert_eq!(
        scalar(&config, "select count(*) from crystal_concepts"),
        json!(2)
    );

    // The ambiguous crystals are rejected with a durable audit reason.
    let audit = persistence_audit(&config, run.id);
    let rejected = audit["rejected_entries"].as_array().unwrap();
    let ambiguous = rejected
        .iter()
        .find(|entry| entry["reason"] == json!("ambiguous_crystal_context"))
        .unwrap();
    assert_eq!(ambiguous["title"], json!("Ambiguous"));

    // A provider-length title is echoed only as a bounded prefix with an
    // explicit ellipsis marker ("Very " repeated 16 times = 80 chars).
    let bounded = format!("{}[...]", "Very ".repeat(16));
    let long = rejected
        .iter()
        .find(|entry| entry["title"] == json!(bounded))
        .unwrap();
    assert_eq!(long["reason"], json!("ambiguous_crystal_context"));
    assert_eq!(long["source_memory_ids"], json!([book_ids[0], zola_ids[0]]));
}

// ---------------------------------------------------------------------------
// Target protection at the run level: out-of-context and active rules
// ---------------------------------------------------------------------------

#[test]
fn a_run_fails_closed_when_supersede_targets_another_series() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    create_series(&config, "zola");
    completed_session(&config, "book", &["Book input."]);

    let store = CrystalStore::open(&config).unwrap();
    let book_crystal = store
        .add_crystal(
            &context("book"),
            "observation",
            &NewCrystal::new("observation", "Book note."),
        )
        .unwrap();
    let zola_crystal = store
        .add_crystal(
            &context("zola"),
            "observation",
            &NewCrystal::new("observation", "Zola note."),
        )
        .unwrap();

    let payload = json!({
        "supersede_actions": [{
            "old_crystal_id": zola_crystal,
            "new_crystal_id": book_crystal,
            "reason": "cross-series suggestion",
        }],
    });
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(move || {
            Box::new(GraphProvider {
                knowledge_crystals: payload.clone(),
            })
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("not authorized for crystal {zola_crystal}")),
        "{error}"
    );

    // Nothing was applied: both crystals keep their status.
    assert_eq!(store.get(book_crystal).unwrap().status, "active");
    assert_eq!(store.get(zola_crystal).unwrap().status, "active");
}

#[test]
fn a_run_fails_closed_when_supersede_targets_an_active_rule() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let memory_ids = completed_session(&config, "book", &["Rule input."]);

    let store = CrystalStore::open(&config).unwrap();
    let rule_id = store
        .add_crystal(
            &context("book"),
            "rule",
            &NewCrystal::new("rule", "The rule crystal."),
        )
        .unwrap();
    let plain_id = store
        .add_crystal(
            &context("book"),
            "observation",
            &NewCrystal::new("observation", "A plain note."),
        )
        .unwrap();

    let payload = json!({
        "supersede_actions": [{
            "old_crystal_id": rule_id,
            "new_crystal_id": plain_id,
            "reason": "model suggestion",
        }],
        "reinforce": [{
            "crystal_id": rule_id,
            "strength_delta": 0.1,
            "source_memory_ids": memory_ids,
        }],
    });
    let service = DreamService::open(
        &config,
        WorkflowResolver::serving(move || {
            Box::new(GraphProvider {
                knowledge_crystals: payload.clone(),
            })
        }),
    )
    .unwrap();
    let error = service.run_cycle("manual", false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("not authorized for crystal {rule_id}")),
        "{error}"
    );

    // The active rule authority projection is untouched.
    assert_eq!(store.get(rule_id).unwrap().status, "active");
    assert_eq!(store.get(plain_id).unwrap().status, "active");
    assert_eq!(scalar(&config, "select count(*) from term_rules"), json!(0));
}

// ---------------------------------------------------------------------------
// Mixed output: valid entries apply, malformed entries are rejected
// individually into durable audit channels without credential leakage
// ---------------------------------------------------------------------------

#[test]
fn mixed_output_applies_valid_entries_and_audits_rejections_individually() {
    let root = tempfile::tempdir().unwrap();
    let config = config(&root);
    create_series(&config, "book");
    let memory_ids = completed_session(&config, "book", &["Mixed input."]);

    // The valid reinforce action needs an in-context target.
    let target_id = CrystalStore::open(&config)
        .unwrap()
        .add_crystal(
            &context("book"),
            "observation",
            &NewCrystal::new("observation", "An existing note."),
        )
        .unwrap();

    let payload = json!({
        "concepts": [
            {"canonical_name": "Valid", "confidence": 0.5},
            "not-an-object",
            {},
            {"canonical_name": "Leaky", "confidence": "sk-leak-secret"},
        ],
        "facets": [
            {"concept_name": "Valid", "value": "значение", "kind": "note"},
            {"concept_name": "Valid"},
            {"concept_name": "Valid", "value": "x", "confidence": 1.5},
            {"concept_name": "Valid", "value": "compat", "facet_type": "alias"},
        ],
        "concept_proposals": [
            {
                "series_slug": "book",
                "source_language": "ja",
                "target_language": "ru",
                "concept_text": "Valid",
                "source_form": "valid",
                "canonical_rendering": "значение",
            },
            {"series_slug": "book", "concept_text": "incomplete"},
        ],
        "reinforce": [
            {
                "crystal_id": target_id,
                "strength_delta": 0.2,
                "source_memory_ids": memory_ids,
            },
            {"crystal_id": target_id, "strength_delta": 9.0, "source_memory_ids": memory_ids},
        ],
        "crystals": [
            {
                "crystal_type": "observation",
                "text": "Valid crystal.",
                "source_memory_ids": memory_ids,
            },
            {"crystal_type": "observation", "text": "Wrong source.", "source_memory_ids": [999_999]},
        ],
    });
    let run = run_one_cycle(&config, payload);

    assert_eq!(run.status, "completed");

    // Valid entries applied; malformed entries never dropped their section.
    let concepts = ConceptStore::open(&config).unwrap();
    let stored = concepts.list_concepts(None, None).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].canonical_name, "Valid");
    assert_eq!(concepts.list_facets(stored[0].id).unwrap().len(), 2);
    assert_eq!(run.proposal_count, 1);
    assert_eq!(run.created_crystal_count, 1);
    assert_eq!(
        scalar(&config, "select count(*) from strict_concept_proposals"),
        json!(1)
    );

    // Reinforce applied once for the valid action (the second is rejected).
    assert_eq!(
        scalar(
            &config,
            "select count(*) from memory_events where event_type = 'dream_reinforce'"
        ),
        json!(1)
    );

    // Every malformed entry is durably audited with its own reason.
    let audit = persistence_audit(&config, run.id);
    let rejected = audit["rejected_entries"].as_array().unwrap();
    let reasons: Vec<&str> = rejected
        .iter()
        .map(|entry| entry["reason"].as_str().unwrap())
        .collect();
    for expected in [
        "malformed_concept_entry",
        "missing_concept_name",
        "malformed_concept_confidence",
        "missing_facet_value",
        "malformed_facet_confidence",
        "invalid_concept_proposal",
        "reinforce_action_delta_out_of_range",
    ] {
        assert!(
            reasons.contains(&expected),
            "missing rejection reason {expected}: {rejected:?}"
        );
    }
    let skipped = audit["skipped_candidates"].as_array().unwrap();
    assert!(
        skipped
            .iter()
            .any(|entry| entry["reason"] == json!("invalid_source_memory_ids")),
        "{skipped:?}"
    );

    // No provider credential material leaked into the durable audit trail.
    let connection = open_migrated(&config.database_path()).unwrap();
    let all: String = {
        let mut statement = connection
            .prepare("select payload_json from dream_audit_entries where dream_run_id = ?1")
            .unwrap();
        let rows = statement
            .query_map([run.id], |row| row.get::<_, String>(0))
            .unwrap();
        rows.map(|row| row.unwrap()).collect::<Vec<_>>().join("\n")
    };
    assert!(!all.contains("sk-leak-secret"), "credential leaked: {all}");
}
