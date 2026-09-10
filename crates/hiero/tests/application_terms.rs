//! M3 regression: the explicit structured-rule lifecycle (ADR 0011). A
//! proposed term stays advisory until an explicit, authenticated approval;
//! archive and replacement atomically update the structured authority AND
//! its rule-crystal projection; passive processes (dreaming, scoring,
//! feedback) can never deactivate or activate the authority.
//!
//! Coverage: the step-1 propose/approve/contract regression, the versioned
//! `compatibility/rust/rule-lifecycle-v2.json` expectation, revision
//! conflicts, idempotency replay and conflicts, transactional rollback of
//! projection-coupled archive (the crash-before-commit proxy), replacement
//! semantics, active rules surviving feedback/decay, dream-generated
//! candidates that cannot activate, deterministic validation and ambiguity
//! before advisory findings, and the M2-deferred recall-contract check (an
//! approved rule's canonical rendering survives recall limit=1 with a
//! conflicting RAG hit).

use hiero::application::Application;
use hieronymus::crystals::{CrystalStore, NewCrystal};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::memory_models::TranslationContext;
use hieronymus::terminology::{ProposeFields, RuleAction, RuleActionRequest, Termbase};
use serde_json::{Value, json};

const ACTOR: &str = "local-user";
const LIFECYCLE_V2_FIXTURE: &str =
    include_str!("../../../compatibility/rust/rule-lifecycle-v2.json");

fn test_application() -> (tempfile::TempDir, Application) {
    let root = tempfile::tempdir().unwrap();
    let application = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    (root, application)
}

fn config_of(root: &tempfile::TempDir) -> HieronymusConfig {
    HieronymusConfig::new(root.path())
}

fn create_series(app: &Application, slug: &str, source: &str, target: &str) {
    app.call(
        "hieronymus_series_create",
        &json!({"slug": slug, "title": "Book", "source_language": source, "target_language": target}),
        ACTOR,
    )
    .unwrap();
}

fn termbase(root: &tempfile::TempDir) -> Termbase {
    let context = TranslationContext::new("book", "ja", "ru", "translation");
    Termbase::open(&config_of(root), &context).unwrap()
}

fn plain_fields() -> ProposeFields {
    ProposeFields::default()
}

fn connection(root: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(config_of(root).database_path()).unwrap()
}

fn expect_domain(error: hiero::application::AppError, needle: &str) {
    match error {
        hiero::application::AppError::Domain(message) => assert!(
            message.contains(needle),
            "expected {needle:?} in domain diagnostic: {message}"
        ),
        other => panic!("expected a domain error, got: {other}"),
    }
}

/// One active rule crystal linked to a term rule (the projection link the
/// upgrade converter builds for legacy rules; tests write it directly
/// because the link belongs to the authority row).
fn seed_linked_rule_crystal(root: &tempfile::TempDir, sentence: &str, rule_id: i64) -> i64 {
    let context = TranslationContext::new("book", "ja", "ru", "translation");
    let crystal = NewCrystal::new("rule", sentence)
        .strength(1.0)
        .confidence(1.0);
    let crystal_id = CrystalStore::open(&config_of(root))
        .unwrap()
        .add_crystal(&context, "rule", &crystal)
        .unwrap();
    connection(root)
        .execute(
            "update term_rules set rule_crystal_id = ?1 where id = ?2",
            rusqlite::params![crystal_id, rule_id],
        )
        .unwrap();
    crystal_id
}

// ------------------------------------------------------------ step 1 regression

#[test]
fn proposed_term_does_not_enforce_until_explicit_approval() {
    let root = tempfile::tempdir().unwrap();
    let app = Application::open(&HieronymusConfig::new(root.path())).unwrap();
    app.call(
        "hieronymus_series_create",
        &json!({"slug":"book","title":"Book",
        "source_language":"ja","target_language":"ru"}),
        "local-user",
    )
    .unwrap();
    let draft = app
        .call(
            "hieronymus_termbase_propose",
            &json!({"series_slug":"book",
        "category":"name","source_text":"猫","canonical_translation":"Кот"}),
            "local-user",
        )
        .unwrap();
    let args = json!({"series_slug":"book","raw_text":"猫"});
    assert_eq!(
        app.call("hieronymus_termbase_contract", &args, "local-user")
            .unwrap()["results"],
        json!([])
    );
    app.call(
        "hieronymus_termbase_approve",
        &json!({"series_slug":"book","term_id":draft["id"]}),
        "local-user",
    )
    .unwrap_err();
    termbase(&root)
        .approve(
            draft["id"].as_i64().unwrap(),
            ACTOR,
            "trusted local domain fixture",
        )
        .unwrap();
    assert_eq!(
        app.call("hieronymus_termbase_contract", &args, "local-user")
            .unwrap()["results"][0]["canonical_translation"],
        "Кот"
    );
}

// ------------------------------------------------------------ versioned fixture

#[test]
fn rule_lifecycle_v2_fixture_pins_the_tool_lifecycle() {
    let fixture: Value = serde_json::from_str(LIFECYCLE_V2_FIXTURE).unwrap();
    assert_eq!(fixture["adr"], json!("0011"));
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "ru");

    for expectation in fixture["expectations"].as_array().unwrap() {
        if expectation["seed"] == json!("active rule crystal linked to term 1") {
            let crystal_id = seed_linked_rule_crystal(&root, "猫 is translated as Кот.", 1);
            assert_eq!(crystal_id, 1, "fixture arguments pin crystal 1");
        }
        let Some(tool) = expectation["tool"].as_str() else {
            continue;
        };
        // ADR0016 active Rust delta: ordinary tools cannot approve/archive. Exercise
        // the retained domain lifecycle explicitly for this historical fixture only.
        if tool == "hieronymus_termbase_approve" {
            expect_domain(
                app.call(tool, &expectation["arguments"], ACTOR)
                    .unwrap_err(),
                "unverified origin",
            );
            let result = termbase(&root).approve(
                expectation["arguments"]["term_id"].as_i64().unwrap(),
                ACTOR,
                "historical domain fixture",
            );
            if expectation["expected"]["error_contains"].is_string() {
                assert!(result.is_err());
            } else {
                result.unwrap();
            }
            continue;
        }
        if tool == "hieronymus_rule_crystal_archive" {
            expect_domain(
                app.call(tool, &expectation["arguments"], ACTOR)
                    .unwrap_err(),
                "unverified origin",
            );
            let store = termbase(&root);
            let rule = store.get_rule(1).unwrap();
            if rule.status == "active" {
                store
                    .apply_action(&RuleActionRequest {
                        rule_id: 1,
                        action: RuleAction::Archive,
                        actor: ACTOR.into(),
                        reason: "historical domain fixture".into(),
                        expected_revision: rule.revision,
                        idempotency_key: "historical-archive".into(),
                    })
                    .unwrap();
            }
            continue;
        }
        let result = app.call(tool, &expectation["arguments"], ACTOR);
        let expected = &expectation["expected"];

        if let Some(needle) = expected["error_contains"].as_str() {
            let error = match result {
                Ok(payload) => panic!("expected {tool} to fail with {needle:?}, got: {payload}"),
                Err(error) => error,
            };
            expect_domain(error, needle);
            continue;
        }
        let envelope = result.unwrap_or_else(|error| panic!("expected {tool} to succeed: {error}"));
        let payload = if matches!(
            tool,
            "hieronymus_termbase_contract" | "hieronymus_termbase_validate"
        ) {
            assert!(envelope["resulting_revision"].is_u64());
            &envelope["results"]
        } else {
            &envelope
        };

        if let Some(subset) = expected["subset"].as_object() {
            for (key, value) in subset {
                assert_eq!(payload[key], *value, "{tool} field {key}: {payload}");
            }
        }
        if let Some(rows) = expected["contract_rows"].as_array() {
            let actual = payload.as_array().unwrap();
            assert_eq!(actual.len(), rows.len(), "{tool}: {payload}");
            for row in rows {
                let source_text = &row["source_text"];
                let actual_row = actual
                    .iter()
                    .find(|term| &term["source_text"] == source_text)
                    .unwrap_or_else(|| panic!("contract row {source_text} missing: {payload}"));
                for key in [
                    "canonical_translation",
                    "forbidden_variants",
                    "tags",
                    "notes",
                ] {
                    if let Some(expected_value) = row.get(key) {
                        assert_eq!(
                            actual_row[key], *expected_value,
                            "contract field {key} mismatch: {payload}"
                        );
                    }
                }
            }
        }
        if let Some(wanted) = expected["finding_kinds"].as_array() {
            let mut actual_kinds: Vec<String> = payload
                .as_array()
                .unwrap()
                .iter()
                .map(|finding| finding["kind"].as_str().unwrap().to_string())
                .collect();
            let mut expected_kinds: Vec<String> = wanted
                .iter()
                .map(|kind| kind.as_str().unwrap().to_string())
                .collect();
            actual_kinds.sort();
            expected_kinds.sort();
            assert_eq!(actual_kinds, expected_kinds, "{tool}: {payload}");
        }
        if let Some(length) = expected["list_length"].as_u64() {
            assert_eq!(
                payload.as_array().unwrap().len() as u64,
                length,
                "{payload}"
            );
        }
        if tool == "hieronymus_rule_crystals_list" {
            assert_eq!(payload[0]["claim_annotation"]["source_inspection"], true);
        }
        if let Some(row) = expected["first_row"].as_object() {
            for (key, value) in row {
                assert_eq!(
                    payload[0][key], *value,
                    "{tool} first row field {key}: {payload}"
                );
            }
        }
    }
}

// ------------------------------------------------- apply_action transaction core

#[test]
fn apply_action_replays_idempotent_retries_and_conflicts_on_payload_changes() {
    let (root, _app) = test_application();
    create_series_stub(&root);
    let termbase = termbase(&root);
    let rule = termbase.propose("猫", "Кот", &plain_fields()).unwrap();

    let request = RuleActionRequest {
        rule_id: rule.id,
        action: RuleAction::Approve,
        actor: ACTOR.to_string(),
        reason: "review".to_string(),
        expected_revision: rule.revision,
        idempotency_key: "approve:1".to_string(),
    };
    let first = termbase.apply_action(&request).unwrap();
    assert_eq!(first.status, "active");
    assert_eq!(first.revision, rule.revision + 1);

    // Same key + same canonical request: the stored result is replayed and
    // nothing re-executes.
    let replay = termbase.apply_action(&request).unwrap();
    assert_eq!(replay, first);
    let actions: i64 = connection(&root)
        .query_row("select count(*) from term_rule_actions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(actions, 1, "retries must not add audit rows");
    let stored = termbase.get_rule(rule.id).unwrap();
    assert_eq!(stored.revision, rule.revision + 1);

    // Same key + a differing payload is a conflict, not a replay.
    let conflicting = RuleActionRequest {
        reason: "a different reason".to_string(),
        ..request.clone()
    };
    let error = termbase.apply_action(&conflicting).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("already used with a different request"),
        "{error}"
    );

    // A fresh key with a CURRENT expected revision still hits the transition
    // guard: the rule is no longer a candidate.
    let error = termbase
        .apply_action(&RuleActionRequest {
            idempotency_key: "approve:1-again".to_string(),
            expected_revision: rule.revision + 1,
            ..request.clone()
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("only candidate rules"),
        "{error}"
    );

    // And a stale caller with a fresh key gets the revision conflict, not a
    // silent success.
    let error = termbase
        .apply_action(&RuleActionRequest {
            idempotency_key: "approve:1-stale".to_string(),
            ..request.clone()
        })
        .unwrap_err();
    assert!(error.to_string().contains("revision conflict"), "{error}");
}

#[test]
fn apply_action_rejects_stale_revisions() {
    let (root, _app) = test_application();
    create_series_stub(&root);
    let termbase = termbase(&root);
    let rule = termbase.propose("猫", "Кот", &plain_fields()).unwrap();
    termbase.approve(rule.id, ACTOR, "review").unwrap();

    // A caller holding the pre-approval revision is stale: conflict, no
    // partial writes, authority untouched.
    let error = termbase
        .apply_action(&RuleActionRequest {
            rule_id: rule.id,
            action: RuleAction::Archive,
            actor: ACTOR.to_string(),
            reason: "stale archive".to_string(),
            expected_revision: rule.revision,
            idempotency_key: "archive:stale".to_string(),
        })
        .unwrap_err();
    match error {
        hieronymus::terminology::TermbaseError::RevisionConflict {
            rule_id: conflicted,
            expected_revision,
            current_revision,
        } => {
            assert_eq!(conflicted, rule.id);
            assert_eq!(expected_revision, rule.revision);
            assert_eq!(current_revision, rule.revision + 1);
        }
        other => panic!("expected a revision conflict, got: {other}"),
    }
    let stored = termbase.get_rule(rule.id).unwrap();
    assert_eq!(stored.status, "active");
    assert_eq!(
        termbase.contract("猫").unwrap().len(),
        1,
        "the stale rejection must not disturb enforcement"
    );
}

#[test]
fn projection_archive_failure_rolls_back_the_whole_action() {
    // The crash-before-commit proxy: the projection write fails AFTER the
    // authority row was mutated, and the single transaction must undo both.
    let (root, _app) = test_application();
    create_series_stub(&root);
    let termbase = termbase(&root);
    let rule = termbase.propose("猫", "Кот", &plain_fields()).unwrap();
    termbase.approve(rule.id, ACTOR, "review").unwrap();

    // A dangling projection link makes the projection write fail loudly.
    connection(&root)
        .execute(
            "update term_rules set rule_crystal_id = 424242 where id = ?1",
            [rule.id],
        )
        .unwrap();
    let request = RuleActionRequest {
        rule_id: rule.id,
        action: RuleAction::Archive,
        actor: ACTOR.to_string(),
        reason: "rollback drill".to_string(),
        expected_revision: rule.revision + 1,
        idempotency_key: "archive:rollback".to_string(),
    };
    let error = termbase.apply_action(&request).unwrap_err();
    assert!(
        error.to_string().contains("rule-crystal projection"),
        "{error}"
    );

    let stored = termbase.get_rule(rule.id).unwrap();
    assert_eq!(stored.status, "active", "the authority must roll back");
    assert_eq!(
        stored.revision,
        rule.revision + 1,
        "no phantom revision bump"
    );
    let audit: i64 = connection(&root)
        .query_row(
            "select count(*) from term_rule_actions where idempotency_key = ?1",
            ["archive:rollback"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(audit, 0, "a rolled-back action must leave no audit row");

    // Healing the link lets the SAME request (same idempotency key and
    // canonical payload) execute: a failed attempt never stored a result.
    let crystal_id = seed_linked_rule_crystal(&root, "猫 is translated as Кот.", rule.id);
    termbase.apply_action(&request).unwrap();
    let stored = termbase.get_rule(rule.id).unwrap();
    assert_eq!(stored.status, "archived");
    let crystal = CrystalStore::open(&config_of(&root))
        .unwrap()
        .get(crystal_id)
        .unwrap();
    assert_eq!(
        crystal.status, "archived",
        "projection archived with authority"
    );
}

#[test]
fn replace_supersedes_the_authority_and_moves_the_projection() {
    let (root, _app) = test_application();
    create_series_stub(&root);
    let termbase = termbase(&root);
    let first = termbase.propose("猫", "Кот", &plain_fields()).unwrap();
    let second = termbase.propose("猫", "Котик", &plain_fields()).unwrap();
    termbase.approve(first.id, ACTOR, "review").unwrap();
    let crystal_id = seed_linked_rule_crystal(&root, "猫 is translated as Кот.", first.id);

    let result = termbase
        .apply_action(&RuleActionRequest {
            rule_id: first.id,
            action: RuleAction::Replace {
                replacement_id: second.id,
            },
            actor: ACTOR.to_string(),
            reason: "better rendering".to_string(),
            expected_revision: 2,
            idempotency_key: "replace:1:2".to_string(),
        })
        .unwrap();

    // The replacement is the resulting authority: active, projected, and
    // enforced.
    assert_eq!(result.id, second.id);
    assert_eq!(result.status, "active");
    assert_eq!(result.rule_crystal_id, Some(crystal_id));

    let replaced = termbase.get_rule(first.id).unwrap();
    assert_eq!(replaced.status, "superseded");
    assert_eq!(replaced.rule_crystal_id, None, "the link moved");
    assert_eq!(
        replaced.revision, 3,
        "supersede bumps the replaced rule's revision"
    );

    let replacement = termbase.get_rule(second.id).unwrap();
    assert_eq!(
        replacement.revision, 2,
        "activation bumps the replacement's revision"
    );
    let crystal = CrystalStore::open(&config_of(&root))
        .unwrap()
        .get(crystal_id)
        .unwrap();
    assert_eq!(
        crystal.text, "猫 is translated as Котик.",
        "derived projection follows the authority"
    );

    let terms = termbase.contract("猫").unwrap();
    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].canonical_translation, "Котик");

    // Self-replacement and non-active targets are deterministic rejections.
    let error = termbase
        .apply_action(&RuleActionRequest {
            rule_id: second.id,
            action: RuleAction::Replace {
                replacement_id: second.id,
            },
            actor: ACTOR.to_string(),
            reason: "self".to_string(),
            expected_revision: 2,
            idempotency_key: "replace:self".to_string(),
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("cannot replace itself"),
        "{error}"
    );
}

// ------------------------------------------------ passive paths never activate

#[test]
fn dream_candidates_stay_advisory_until_explicit_approval() {
    let (root, _app) = test_application();
    create_series_stub(&root);

    // A dream-shaped, perfectly-formed rule crystal (advisory projection,
    // active status, high scores) must NOT satisfy the deterministic
    // contract: only the structured authority enforces.
    let context = TranslationContext::new("book", "ja", "ru", "translation");
    let dream_crystal = NewCrystal {
        soft_origin: "dream".to_string(),
        source_credibility: "inferred".to_string(),
        rule_intent: "dreamed terminology".to_string(),
        ..NewCrystal::new("rule", "猫 is translated as Кошка.")
            .strength(0.95)
            .confidence(0.95)
    };
    let dream_id = CrystalStore::open(&config_of(&root))
        .unwrap()
        .add_crystal(&context, "rule", &dream_crystal)
        .unwrap();
    let termbase = termbase(&root);
    assert!(
        termbase.contract("猫").unwrap().is_empty(),
        "a dreamed rule crystal is advisory, never enforceable"
    );

    // Even a store-level candidate has no activation without the audited
    // API, and the audited API demands an authenticated actor.
    let rule = termbase.propose("猫", "Кот", &plain_fields()).unwrap();
    let error = termbase
        .apply_action(&RuleActionRequest {
            rule_id: rule.id,
            action: RuleAction::Approve,
            actor: "   ".to_string(),
            reason: "no actor".to_string(),
            expected_revision: 1,
            idempotency_key: "dream:approve".to_string(),
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("actor must not be empty"),
        "{error}"
    );
    assert_eq!(termbase.get_rule(rule.id).unwrap().status, "candidate");
    assert!(
        termbase.contract("猫").unwrap().is_empty(),
        "a rejected activation must not enforce"
    );

    // The explicit authenticated approval is the only path to active; the
    // dream crystal stays an unrelated advisory row.
    termbase.approve(rule.id, ACTOR, "explicit review").unwrap();
    assert_eq!(termbase.get_rule(rule.id).unwrap().status, "active");
    let dream = CrystalStore::open(&config_of(&root))
        .unwrap()
        .get(dream_id)
        .unwrap();
    assert_eq!(
        dream.status, "active",
        "approval never writes crystal status"
    );
    let terms = termbase.contract("猫").unwrap();
    assert_eq!(
        terms[0].canonical_translation, "Кот",
        "the dreamed rendering lost"
    );
}

#[test]
fn active_rules_survive_feedback_and_decay() {
    let (root, _app) = test_application();
    create_series_stub(&root);
    let termbase = termbase(&root);
    let rule = termbase
        .propose(
            "猫",
            "Кот",
            &ProposeFields {
                forbidden_variants: vec!["кошка".to_string()],
                ..ProposeFields::default()
            },
        )
        .unwrap();
    termbase.approve(rule.id, ACTOR, "review").unwrap();
    let crystal_id =
        seed_linked_rule_crystal(&root, "猫 is translated as Кот, not кошка.", rule.id);

    // Negative feedback decays the advisory projection's scores...
    let decay_connection = connection(&root);
    hieronymus::feedback::apply_score_delta(
        &decay_connection,
        crystal_id,
        -0.4,
        -0.4,
        "2026-09-04T00:00:00+00:00",
    )
    .unwrap();

    // ...the maintenance ordering skips the active rule crystal...
    let crystals = CrystalStore::open(&config_of(&root)).unwrap();
    assert!(
        crystals
            .low_confidence_first(&[crystal_id], 5)
            .unwrap()
            .is_empty(),
        "active rule crystals are never maintenance victims"
    );
    let advisory = crystals
        .add_crystal(
            &TranslationContext::new("book", "ja", "ru", "translation"),
            "lesson",
            &NewCrystal::new("lesson", "an ordinary note").confidence(0.2),
        )
        .unwrap();
    assert_eq!(
        crystals
            .low_confidence_first(&[crystal_id, advisory], 5)
            .unwrap(),
        vec![advisory],
        "non-rule crystals stay eligible"
    );

    // ...and the authority is untouched: same status, same contract.
    let stored = termbase.get_rule(rule.id).unwrap();
    assert_eq!(stored.status, "active");
    assert_eq!(
        stored.revision, 2,
        "passive processes never bump the revision"
    );
    let terms = termbase.contract("猫").unwrap();
    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].canonical_translation, "Кот");
    assert_eq!(terms[0].forbidden_variants, vec!["кошка"]);
    let crystal = crystals.get(crystal_id).unwrap();
    assert_eq!(
        crystal.status, "active",
        "decay never archives the projection"
    );
}

// --------------------------------------------- deterministic findings over MCP

#[test]
fn termbase_validate_reports_ambiguity_and_deterministic_findings() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "ru");
    let termbase = termbase(&root);
    let first = termbase
        .propose(
            "猫",
            "Кот",
            &ProposeFields {
                forbidden_variants: vec!["кошка".to_string()],
                ..ProposeFields::default()
            },
        )
        .unwrap();
    termbase.approve(first.id, ACTOR, "review").unwrap();
    // A second active rule for the same surface with a different rendering
    // is a deterministic conflict.
    let second = termbase.propose("猫", "Котик", &plain_fields()).unwrap();
    termbase.approve(second.id, ACTOR, "review").unwrap();

    // Ambiguity is evaluated first, and a conflicting surface is NOT
    // enforced: the validator surfaces the conflict instead of guessing a
    // rendering, so no per-term findings ride along.
    let findings = app
        .call(
            "hieronymus_termbase_validate",
            &json!({"series_slug": "book", "raw_text": "猫",
                    "translated_text": "кошка sits here"}),
            ACTOR,
        )
        .unwrap();
    let rows = findings["results"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{findings}");
    assert_eq!(rows[0]["kind"], json!("conflicting_active_rules"));
    assert_eq!(rows[0]["severity"], json!("warning"));
    assert_eq!(rows[0]["expected"], json!("Кот, Котик"));

    // The explicit archive of the conflicting rule resolves the ambiguity,
    // and deterministic enforcement resumes: forbidden variant (high) and
    // missing canonical (medium) come back for the surviving rule.
    termbase
        .apply_action(&RuleActionRequest {
            rule_id: second.id,
            action: RuleAction::Archive,
            actor: ACTOR.to_string(),
            reason: "resolve the conflict".to_string(),
            expected_revision: 2,
            idempotency_key: "archive:2:resolve".to_string(),
        })
        .unwrap();
    let findings = app
        .call(
            "hieronymus_termbase_validate",
            &json!({"series_slug": "book", "raw_text": "猫",
                    "translated_text": "кошка sits here"}),
            ACTOR,
        )
        .unwrap();
    let rows = findings["results"].as_array().unwrap();
    let kinds: Vec<&str> = rows
        .iter()
        .map(|row| row["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"forbidden_variant"), "{findings}");
    assert!(kinds.contains(&"missing_canonical"), "{findings}");
    let forbidden = rows
        .iter()
        .find(|row| row["kind"] == json!("forbidden_variant"))
        .unwrap();
    assert_eq!(forbidden["severity"], json!("high"));
    assert_eq!(forbidden["observed"], json!("кошка"));
    assert_eq!(forbidden["expected"], json!("Кот"));
    assert_eq!(forbidden["term_id"], json!(first.id));
}

// ------------------------------------------------- projection links never guess

#[test]
fn crystal_archive_fails_on_unknown_and_ambiguous_links() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "ru");
    let termbase = termbase(&root);
    let context = TranslationContext::new("book", "ja", "ru", "translation");

    // An unlinked rule crystal is rejected: the projection is never
    // archived alone.
    let unlinked = CrystalStore::open(&config_of(&root))
        .unwrap()
        .add_crystal(
            &context,
            "rule",
            &NewCrystal::new("rule", "犬 is translated as собака."),
        )
        .unwrap();
    let error = app
        .call(
            "hieronymus_rule_crystal_archive",
            &json!({"crystal_id": unlinked}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unverified origin");
    let crystal = CrystalStore::open(&config_of(&root))
        .unwrap()
        .get(unlinked)
        .unwrap();
    assert_eq!(
        crystal.status, "active",
        "the rejected projection is untouched"
    );

    // Two authority rows claiming one crystal is ambiguous: rejected, and
    // nothing is archived.
    let first = termbase.propose("猫", "Кот", &plain_fields()).unwrap();
    let second = termbase.propose("猫", "Котик", &plain_fields()).unwrap();
    let crystal_id = seed_linked_rule_crystal(&root, "猫 is translated as Кот.", first.id);
    connection(&root)
        .execute(
            "update term_rules set rule_crystal_id = ?1 where id = ?2",
            rusqlite::params![crystal_id, second.id],
        )
        .unwrap();
    let error = app
        .call(
            "hieronymus_rule_crystal_archive",
            &json!({"crystal_id": crystal_id}),
            ACTOR,
        )
        .unwrap_err();
    expect_domain(error, "unverified origin");
    let still_active = termbase.get_rule(first.id).unwrap();
    assert_eq!(
        still_active.status, "candidate",
        "ambiguous links change nothing"
    );
}

// ------------------------------------------- M2-deferred: contract over recall

#[test]
fn approved_contract_survives_recall_with_conflicting_rag_hit() {
    let (root, app) = test_application();
    create_series(&app, "book", "ja", "ru");
    let termbase = termbase(&root);
    let rule = termbase
        .propose(
            "猫",
            "Кот",
            &ProposeFields {
                forbidden_variants: vec!["кошка".to_string()],
                ..ProposeFields::default()
            },
        )
        .unwrap();
    // Approval runs through the MCP tool: the M3 lifecycle feeds the M2
    // recall surface.
    assert!(
        app.call(
            "hieronymus_termbase_approve",
            &json!({"series_slug":"book","term_id":rule.id}),
            ACTOR
        )
        .is_err()
    );
    termbase
        .approve(rule.id, ACTOR, "trusted local domain fixture")
        .unwrap();
    current_story::register(app.config(), "book");
    let session_id = app
        .call(
            "hieronymus_session_start",
            &json!({"series_slug":"book","volume":"I","chapter":"Opening"}),
            ACTOR,
        )
        .unwrap()["session_id"]
        .as_i64()
        .unwrap();
    app.call(
        "hieronymus_short_term_add",
        &json!({"session_id": session_id, "kind": "note", "text": "whisker观察 note"}),
        ACTOR,
    )
    .unwrap();

    // The conflicting RAG hit carries the forbidden rendering AND matches
    // the query surface, so it competes for the ranked list. (The FTS
    // tokenizer treats each contiguous CJK run as one token, so 猫 stands
    // alone.)
    let chapter = root.path().join("chapter.txt");
    std::fs::write(&chapter, "猫 кошка wrong rendering.\n").unwrap();
    app.call(
        "hieronymus_rag_import",
        &json!({"series_slug": "book", "path": chapter.to_str().unwrap(),"claims":{"0":[current_story::claim(app.config(),"book","猫 кошка wrong rendering.")]}}),
        ACTOR,
    )
    .unwrap();

    let recall = app
        .call(
            "hieronymus_recall",
            &json!({"session_id": session_id, "series_slug": "book", "query": "猫", "limit": 1}),
            ACTOR,
        )
        .unwrap();

    // The deterministic contract survives limit=1 whole, with the canonical
    // rendering intact — never overridden by the conflicting RAG evidence.
    let contract = recall["deterministic_contract"].as_array().unwrap();
    assert_eq!(contract.len(), 1, "{recall}");
    assert_eq!(contract[0]["canonical_translation"], json!("Кот"));
    assert_eq!(contract[0]["forbidden_variants"], json!(["кошка"]));

    // The advisory hit keeps its evidence but carries the conflict marker.
    let results = recall["results"].as_array().unwrap();
    let rag_row = results
        .iter()
        .find(|row| row["tier"] == json!("rag"))
        .expect("the conflicting RAG hit is present as evidence: {recall}");
    assert_eq!(
        rag_row["conflicts_with_rule_ids"],
        json!([rule.id]),
        "{recall}"
    );
}

/// The terms tools run over a registered series; tests that bypass the MCP
/// surface still need the registry row.
fn create_series_stub(root: &tempfile::TempDir) {
    let registry = hieronymus::registry::Registry::open(&config_of(root)).unwrap();
    registry
        .create_series("book", "Book", "ja", "ru", None)
        .unwrap();
}

#[path = "../../hieronymus/tests/support/current_story.rs"]
mod current_story;
