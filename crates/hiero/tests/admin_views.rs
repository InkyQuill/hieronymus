//! Plan W2: the ten admin views are real read-only domain projections.
//!
//! Every advertised view resolves for an empty database, projects seeded
//! records with stable ids and Python-parity row/detail shapes, and applies
//! the shared series scope so a foreign-context record never leaks.

use hiero::application::admin::{VIEW_NAMES, snapshot};
use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;
use rusqlite::Connection;
use serde_json::{Value, json};

const TS: &str = "2026-01-02T03:04:05Z";

#[test]
fn every_advertised_view_accepts_an_empty_database() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let _app = hiero::application::Application::open(&config).unwrap();
    assert_eq!(VIEW_NAMES.len(), 10);
    for name in VIEW_NAMES {
        let result = snapshot(&config, name, &json!({}));
        assert!(result.is_ok(), "{name}: {result:?}");
        let value = result.unwrap();
        assert_eq!(value["view"], json!(name));
        assert_eq!(value["rows"], json!([]), "{name} empty rows");
        assert_eq!(value["selected"], Value::Null, "{name} empty selection");
        assert_eq!(value["detail"]["subtitle"], json!("No rows"), "{name}");
        assert_eq!(value["filters"], json!([]));
    }
}

#[test]
fn an_unknown_view_is_an_invalid_request() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let error = snapshot(&config, "Nonsense", &json!({})).unwrap_err();
    assert!(matches!(error, hiero::application::AppError::Invalid(_)));
}

#[test]
fn a_view_key_resolves_to_its_label() {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let value = snapshot(&config, "short_term_sessions", &json!({})).unwrap();
    assert_eq!(value["view"], json!("Short-Term Sessions"));
}

/// Rows for one view, as `(id, label)` pairs.
fn rows(config: &HieronymusConfig, view: &str, query: &Value) -> Vec<(i64, String)> {
    snapshot(config, view, query).unwrap()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_i64().unwrap(),
                row["label"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn labels(config: &HieronymusConfig, view: &str, query: &Value) -> Vec<String> {
    rows(config, view, query)
        .into_iter()
        .map(|(_, label)| label)
        .collect()
}

/// The full row object with the given label, or a panic naming the view.
fn find_row(config: &HieronymusConfig, view: &str, query: &Value, label: &str) -> Value {
    snapshot(config, view, query).unwrap()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["label"] == json!(label))
        .cloned()
        .unwrap_or_else(|| panic!("{view}: no row labelled {label:?}"))
}

/// The detail field values as a `[[name, value], ...]` vector of string pairs.
fn detail_fields(value: &Value) -> Vec<(String, String)> {
    value["detail"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            (
                pair[0].as_str().unwrap().to_string(),
                pair[1].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn seed_series(connection: &Connection, slug: &str) {
    connection
        .execute(
            "insert into series(slug, title, default_source_language, default_target_language,
                                created_at, updated_at)
             values (?1, ?1, 'ja', 'en', ?2, ?2)",
            rusqlite::params![slug, TS],
        )
        .unwrap();
}

fn seeded_root() -> (tempfile::TempDir, HieronymusConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = HieronymusConfig::new(root.path());
    let _app = hiero::application::Application::open(&config).unwrap();
    let connection = open_migrated(&config.database_path()).unwrap();
    seed_series(&connection, "main");
    seed_series(&connection, "other");

    // Concepts: one global (visible under any series), one scoped to `other`.
    connection
        .execute(
            "insert into concepts(canonical_name, description, scope_type, scope_key, status,
                                  confidence, created_at, updated_at)
             values ('Concept Main', 'desc', 'global', '', 'candidate', 0.5, ?1, ?1)",
            [TS],
        )
        .unwrap();
    let concept_main_id = connection.last_insert_rowid();
    connection
        .execute(
            "insert into concept_facets(concept_id, language, facet_type, value, confidence,
                                        is_canonical, created_at, updated_at)
             values (?1, 'en', 'rendering', 'Canon Name', 0.5, 1, ?2, ?2)",
            rusqlite::params![concept_main_id, TS],
        )
        .unwrap();
    connection
        .execute(
            "insert into concept_semantic_tags(concept_id, tag, confidence, created_at)
             values (?1, 'lore', 0.5, ?2)",
            rusqlite::params![concept_main_id, TS],
        )
        .unwrap();
    connection
        .execute(
            "insert into concepts(canonical_name, description, scope_type, scope_key, status,
                                  confidence, created_at, updated_at)
             values ('Concept Other', '', 'series', 'series:other', 'candidate', 0.5, ?1, ?1)",
            [TS],
        )
        .unwrap();

    // Renderings (strict_terms).
    for (series, rendering) in [("main", "Rendering Main"), ("other", "Rendering Other")] {
        connection
            .execute(
                "insert into strict_terms(series_slug, source_language, target_language, category,
                                          source_text, canonical_translation, status, notes,
                                          created_at, updated_at)
                 values (?1, 'ja', 'en', 'name', 'src', ?2, 'approved', 'note', ?3, ?3)",
                rusqlite::params![series, rendering, TS],
            )
            .unwrap();
    }

    // Crystals + Lessons.
    for (kind, series, title) in [
        ("rule", "main", "Crystal Main"),
        ("rule", "other", "Crystal Other"),
        ("lesson", "main", "Lesson Main"),
        ("lesson", "other", "Lesson Other"),
    ] {
        connection
            .execute(
                "insert into crystals(crystal_type, text, title, scope_type, scope_key, series_slug,
                                      source_language, target_language, tags_json, strength,
                                      confidence, status, created_at, updated_at)
                 values (?1, 'body text', ?2, 'series', 'series:' || ?3, ?3,
                         'ja', 'en', '[]', 0.8, 0.9, 'active', ?4, ?4)",
                rusqlite::params![kind, title, series, TS],
            )
            .unwrap();
    }

    // Short-Term Sessions + Short-Term Memory.
    for (series, note) in [("main", "STM Main"), ("other", "STM Other")] {
        connection
            .execute(
                "insert into task_sessions(series_slug, source_language, target_language, task_type,
                                           volume, chapter, status, cycle_id, created_at,
                                           last_activity_at)
                 values (?1, 'ja', 'en', 'translation', '1', '2', 'active', null, ?2, ?2)",
                rusqlite::params![series, TS],
            )
            .unwrap();
        let session_id = connection.last_insert_rowid();
        connection
            .execute(
                "insert into short_term_memories(session_id, source_role, kind, text, source_ref,
                                                 created_at)
                 values (?1, 'user', 'note', ?2, '', ?3)",
                rusqlite::params![session_id, note, TS],
            )
            .unwrap();
    }

    // Dream Runs + Dream Audits (global scope).
    for cycle in [1_i64, 2] {
        connection
            .execute(
                "insert into dream_runs(cycle_id, status, provider, created_at)
                 values (?1, 'completed', 'det', ?2)",
                rusqlite::params![cycle, TS],
            )
            .unwrap();
        let run_id = connection.last_insert_rowid();
        connection
            .execute(
                "insert into dream_audit_entries(dream_run_id, event_type, severity, summary,
                                                 payload_json, created_at)
                 values (?1, 'phase', 'info', ?2, '{}', ?3)",
                rusqlite::params![run_id, format!("audit for cycle {cycle}"), TS],
            )
            .unwrap();
    }

    // Proposals (strict_concept_proposals).
    for (series, text) in [("main", "Proposal Main"), ("other", "Proposal Other")] {
        connection
            .execute(
                "insert into strict_concept_proposals(series_slug, source_language, target_language,
                                                      concept_text, source_form, canonical_rendering,
                                                      status, created_at, updated_at)
                 values (?1, 'ja', 'en', ?2, 'src', 'rend', 'pending', ?3, ?3)",
                rusqlite::params![series, text, TS],
            )
            .unwrap();
    }

    // Audit Log (global scope).
    for entity in ["11", "22"] {
        connection
            .execute(
                "insert into audit_log(action, entity_type, entity_id, note, created_at)
                 values ('reinforce', 'crystal', ?1, ?2, ?3)",
                rusqlite::params![entity, format!("audit note {entity}"), TS],
            )
            .unwrap();
    }

    (root, config)
}

#[test]
fn scoped_views_project_seeded_rows_and_hide_foreign_context() {
    let (_root, config) = seeded_root();
    let main = json!({"series": "main"});

    // Concepts: the global concept is visible under `main`; the `other`-scoped
    // concept is not.
    let concepts = labels(&config, "Concepts", &main);
    assert!(concepts.contains(&"Concept Main".to_string()));
    assert!(!concepts.contains(&"Concept Other".to_string()));

    for (view, visible, foreign) in [
        ("Renderings", "Rendering Main", "Rendering Other"),
        ("Crystals", "Crystal Main", "Crystal Other"),
        ("Lessons", "Lesson Main", "Lesson Other"),
        ("Short-Term Memory", "STM Main", "STM Other"),
        ("Proposals", "Proposal Main", "Proposal Other"),
    ] {
        let scoped = labels(&config, view, &main);
        assert!(scoped.contains(&visible.to_string()), "{view}: {scoped:?}");
        assert!(
            !scoped.iter().any(|label| label == foreign),
            "{view} leaked foreign row: {scoped:?}"
        );
        // Unfiltered, both rows are present.
        let all = labels(&config, view, &json!({}));
        assert!(all.contains(&visible.to_string()) && all.contains(&foreign.to_string()));
    }

    // Short-Term Sessions labels are derived (`slug / v1 / ch2`); assert by id
    // count instead.
    let sessions_main = rows(&config, "Short-Term Sessions", &main);
    assert_eq!(sessions_main.len(), 1, "{sessions_main:?}");
    assert_eq!(rows(&config, "Short-Term Sessions", &json!({})).len(), 2);
}

#[test]
fn lessons_view_excludes_non_lesson_crystals() {
    let (_root, config) = seeded_root();
    let lessons = labels(&config, "Lessons", &json!({}));
    assert!(lessons.iter().all(|label| label.starts_with("Lesson")));
    assert_eq!(lessons.len(), 2);
}

#[test]
fn global_views_project_all_rows_and_page_within_bounds() {
    let (_root, config) = seeded_root();
    for view in ["Dream Runs", "Dream Audits", "Audit Log"] {
        assert_eq!(rows(&config, view, &json!({})).len(), 2, "{view}");
        // Series scope does not drop global rows.
        assert_eq!(
            rows(&config, view, &json!({"series": "main"})).len(),
            2,
            "{view}"
        );
        // Bounded pagination.
        assert_eq!(rows(&config, view, &json!({"limit": 1})).len(), 1, "{view}");
        assert_eq!(
            rows(&config, view, &json!({"limit": 1, "offset": 1})).len(),
            1,
            "{view}"
        );
    }
}

#[test]
fn selection_and_detail_follow_the_stable_row_id() {
    let (_root, config) = seeded_root();
    let all = rows(&config, "Crystals", &json!({}));
    let (target_id, target_label) = all.last().cloned().unwrap();

    let value = snapshot(
        &config,
        "Crystals",
        &json!({ "selected_id": target_id.to_string() }),
    )
    .unwrap();
    assert_eq!(value["selected"]["id"], json!(target_id));
    assert_eq!(value["detail"]["title"], json!(target_label));
    assert_eq!(value["detail"]["body"], json!("body text"));
    // The detail carries the Python-parity field labels.
    let field_names: Vec<String> = value["detail"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| pair[0].as_str().unwrap().to_string())
        .collect();
    assert_eq!(field_names, ["Series", "Language", "Quality"]);

    // An out-of-range id falls back to the first row (Python `_select_row`).
    let fallback = snapshot(&config, "Crystals", &json!({"selected_id": "999999"})).unwrap();
    assert_eq!(fallback["selected"]["id"], json!(all[0].0));
}

#[test]
fn dream_audit_detail_pretty_prints_the_payload() {
    let (_root, config) = seeded_root();
    let value = snapshot(&config, "Dream Audits", &json!({})).unwrap();
    assert_eq!(value["detail"]["subtitle"], json!("info"));
    assert!(
        value["detail"]["title"]
            .as_str()
            .unwrap()
            .starts_with("phase: audit for cycle")
    );
}

#[test]
fn seeded_rows_carry_every_admin_row_field() {
    let (_root, config) = seeded_root();
    let main = json!({"series": "main"});

    let assert_fields = |view: &str,
                         label: &str,
                         kind: &str,
                         status: &str,
                         scope: &str,
                         lang: &str,
                         quality: &str| {
        let row = find_row(&config, view, &main, label);
        assert_eq!(row["kind"], json!(kind), "{view} kind");
        assert_eq!(row["status"], json!(status), "{view} status");
        assert_eq!(row["scope"], json!(scope), "{view} scope");
        assert_eq!(row["language_pair"], json!(lang), "{view} language_pair");
        assert_eq!(row["quality_label"], json!(quality), "{view} quality_label");
    };

    assert_fields(
        "Renderings",
        "Rendering Main",
        "name",
        "approved",
        "main",
        "ja -> en",
        "",
    );
    assert_fields(
        "Crystals",
        "Crystal Main",
        "rule",
        "active",
        "main",
        "ja -> en",
        "90% conf / 80% str",
    );
    assert_fields(
        "Lessons",
        "Lesson Main",
        "lesson",
        "active",
        "main",
        "ja -> en",
        "90% conf / 80% str",
    );
    assert_fields(
        "Proposals",
        "Proposal Main",
        "strict concept",
        "pending",
        "main",
        "ja -> en",
        "rend",
    );
    assert_fields(
        "Concepts",
        "Concept Main",
        "global",
        "candidate",
        "global",
        "",
        "50% conf",
    );

    // Concepts carry their semantic tags.
    let concept = find_row(&config, "Concepts", &main, "Concept Main");
    assert_eq!(concept["tags"], json!(["lore"]));

    // Short-Term Memory: scope is the session pointer, quality is the role.
    let stm = find_row(&config, "Short-Term Memory", &main, "STM Main");
    assert_eq!(stm["kind"], json!("note"));
    assert_eq!(stm["status"], json!("active"));
    assert!(
        stm["scope"].as_str().unwrap().starts_with("session:"),
        "{stm:?}"
    );
    assert_eq!(stm["language_pair"], json!("ja -> en"));
    assert_eq!(stm["quality_label"], json!("user"));

    // Short-Term Sessions: derived label, series scope.
    let session = &snapshot(&config, "Short-Term Sessions", &main).unwrap()["rows"][0];
    assert_eq!(session["kind"], json!("translation"));
    assert_eq!(session["status"], json!("active"));
    assert_eq!(session["scope"], json!("main"));
    assert_eq!(session["label"], json!("main / v1 / ch2"));

    // Global views ignore the series filter.
    let run = find_row(&config, "Dream Runs", &json!({}), "Cycle 1");
    assert_eq!(run["kind"], json!("det"));
    assert_eq!(run["status"], json!("completed"));
    assert_eq!(run["scope"], json!("global"));
    assert_eq!(run["quality_label"], json!("0 crystals / 0 proposals"));

    let entry = find_row(
        &config,
        "Dream Audits",
        &json!({}),
        "phase: audit for cycle 1",
    );
    assert_eq!(entry["kind"], json!("dream audit"));
    assert_eq!(entry["status"], json!("info"));
    assert!(entry["scope"].as_str().unwrap().starts_with("dream:"));
    assert_eq!(entry["quality_label"], json!(TS));

    let audit = find_row(&config, "Audit Log", &json!({}), "audit note 11");
    assert_eq!(audit["kind"], json!("reinforce"));
    assert_eq!(audit["status"], json!("crystal"));
    assert_eq!(audit["scope"], json!("11"));
    assert_eq!(audit["language_pair"], json!(""));
    assert_eq!(audit["quality_label"], json!(TS));
}

#[test]
fn view_details_match_the_python_shape() {
    let (_root, config) = seeded_root();
    let main = json!({"series": "main"});

    // Renderings — `_strict_term_detail`.
    let rendering_id = find_row(&config, "Renderings", &main, "Rendering Main")["id"]
        .as_i64()
        .unwrap();
    let rendering = snapshot(
        &config,
        "Renderings",
        &json!({"series": "main", "selected_id": rendering_id.to_string()}),
    )
    .unwrap();
    assert_eq!(rendering["detail"]["title"], json!("src"));
    assert_eq!(rendering["detail"]["subtitle"], json!("name / approved"));
    assert_eq!(rendering["detail"]["body"], json!("note"));
    assert_eq!(
        detail_fields(&rendering),
        vec![
            ("Rendering".to_string(), "Rendering Main".to_string()),
            ("Series".to_string(), "main".to_string()),
            ("Language".to_string(), "ja -> en".to_string()),
        ]
    );

    // Dream Runs — `_dream_run_detail`.
    let run_id = find_row(&config, "Dream Runs", &json!({}), "Cycle 1")["id"]
        .as_i64()
        .unwrap();
    let run = snapshot(
        &config,
        "Dream Runs",
        &json!({"selected_id": run_id.to_string()}),
    )
    .unwrap();
    assert_eq!(run["detail"]["title"], json!("Cycle 1"));
    assert_eq!(run["detail"]["subtitle"], json!("det / completed"));
    assert_eq!(run["detail"]["body"], json!(""));
    assert_eq!(
        detail_fields(&run),
        vec![
            ("Inputs".to_string(), "0".to_string()),
            ("Crystals".to_string(), "0".to_string()),
            ("Proposals".to_string(), "0".to_string()),
        ]
    );

    // Proposals — `_proposal_detail`.
    let proposal_id = find_row(&config, "Proposals", &main, "Proposal Main")["id"]
        .as_i64()
        .unwrap();
    let proposal = snapshot(
        &config,
        "Proposals",
        &json!({"series": "main", "selected_id": proposal_id.to_string()}),
    )
    .unwrap();
    assert_eq!(proposal["detail"]["title"], json!("Proposal Main"));
    assert_eq!(
        proposal["detail"]["subtitle"],
        json!("strict concept / pending")
    );
    assert_eq!(
        detail_fields(&proposal),
        vec![
            ("Source form".to_string(), "src".to_string()),
            ("Rendering".to_string(), "rend".to_string()),
            ("Series".to_string(), "main".to_string()),
            ("Language".to_string(), "ja -> en".to_string()),
        ]
    );

    // Audit Log — `_detail_for_view`'s row projection.
    let audit = snapshot(&config, "Audit Log", &json!({})).unwrap();
    assert_eq!(audit["detail"]["title"], json!("audit note 22"));
    assert_eq!(audit["detail"]["subtitle"], json!("crystal"));
    assert_eq!(audit["detail"]["body"], json!(TS));
    assert_eq!(
        detail_fields(&audit),
        vec![
            ("Kind".to_string(), "reinforce".to_string()),
            ("Scope".to_string(), "22".to_string()),
            ("Language".to_string(), String::new()),
            ("Quality".to_string(), TS.to_string()),
        ]
    );

    // Concepts — `_concept_detail`: the body is the facet line list.
    let concept_id = find_row(&config, "Concepts", &main, "Concept Main")["id"]
        .as_i64()
        .unwrap();
    let concept = snapshot(
        &config,
        "Concepts",
        &json!({"series": "main", "selected_id": concept_id.to_string()}),
    )
    .unwrap();
    assert_eq!(concept["detail"]["title"], json!("Concept Main"));
    assert_eq!(concept["detail"]["subtitle"], json!("global / candidate"));
    assert_eq!(concept["detail"]["body"], json!("rendering: Canon Name"));
    assert_eq!(
        detail_fields(&concept),
        vec![
            ("Description".to_string(), "desc".to_string()),
            ("Scope".to_string(), "global".to_string()),
            ("Confidence".to_string(), "50%".to_string()),
            ("Facets".to_string(), "1".to_string()),
        ]
    );
}
