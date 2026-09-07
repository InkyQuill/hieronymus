//! Admin actions (plan W3).
//!
//! [`run_action`] is the typed, audited write surface behind the web console's
//! per-view action buttons. Every action:
//!
//!   * validates its ids/context and, for destructive actions, requires
//!     `confirmed == true` (the confirmation is a request field; the *actor* is
//!     always the transport-authenticated identity the REST layer passes in,
//!     never a JSON field);
//!   * writes each local mutation and its `audit_log` row in one transaction;
//!     concept batches use the domain merge primitive on that transaction;
//!   * routes rule-crystal lifecycle to plan M3 — a `rule` crystal (any status)
//!     rejects `delete`/`merge`/`split` with a message pointing at the explicit
//!     rule action, so a W3 action can never mint or retire a rule projection
//!     outside M3 (ADR 0011); and
//!   * returns `{result, view, selected_id?}` (plus an inspection payload for
//!     the read-only actions), which the REST layer wraps with the refreshed
//!     `stats`/`snapshot`/status projections.
//!
//! Result shapes port Python `admin.py` / `admin_models.py`
//! (`ActionResult{entity_type, entity_id, action, message}`,
//! `ProvenanceDetail{title, sources}`, `DreamReview{...}`).

use rusqlite::{Connection, Transaction};
use serde_json::{Value, json};

use hieronymus::concepts::ConceptStore;
use hieronymus::data_root::HieronymusConfig;

use crate::application::{AppError, Application, domain};

use super::audit::{
    CrystalRow, crystal_row_json, insert_crystal_fts, insert_derived_crystal, load_crystal_row,
    reject_rule_crystal, replace_crystal_fts, row_json, write_audit,
};
use super::views::excerpt;
use super::{clamp_score, now_rfc3339, open_db, store_unavailable};

const DELTA_CONFIRMED: (f64, f64) = (0.15, 0.20);
const DELTA_CONTRADICTED: (f64, f64) = (-0.20, -0.25);
const DELTA_DELETED: (f64, f64) = (-0.50, -0.35);
/// Python `_ARCHIVE_STRENGTH_THRESHOLD`: a `deleted_by_user` event that drops
/// strength below this archives the crystal outright.
const ARCHIVE_STRENGTH_THRESHOLD: f64 = 0.05;

// --------------------------------------------------------------------------
// Request decoding helpers (typed per-action DTOs over the raw JSON body)
// --------------------------------------------------------------------------

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    match args.get(key) {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn require_i64(args: &Value, key: &str) -> Result<i64, AppError> {
    arg_i64(args, key).ok_or_else(|| AppError::Invalid(format!("{key} must be an integer")))
}

fn require_str(args: &Value, key: &str) -> Result<String, AppError> {
    arg_str(args, key)
        .map(str::to_string)
        .ok_or_else(|| AppError::Invalid(format!("{key} must not be empty")))
}

/// Destructive actions require an explicit `confirmed=true` in the request
/// body; the browser dialog binds this to its confirm control.
fn require_confirmed(args: &Value) -> Result<(), AppError> {
    if args.get("confirmed").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(AppError::Invalid(
            "this action requires confirmed=true".to_string(),
        ))
    }
}

/// The selected ids: either an `ids` array or a single `id`. Any entry that
/// is not an integer (or integer-valued string) is an [`AppError::Invalid`] —
/// a malformed id is never silently dropped, since a destructive action must
/// not proceed on a partial selection.
fn id_list(args: &Value) -> Result<Vec<i64>, AppError> {
    if let Some(array) = args.get("ids").and_then(Value::as_array) {
        return array
            .iter()
            .map(|value| match value {
                Value::Number(number) => number.as_i64(),
                Value::String(text) => text.trim().parse().ok(),
                _ => None,
            })
            .collect::<Option<Vec<i64>>>()
            .ok_or_else(|| AppError::Invalid("ids must all be integers".to_string()));
    }
    Ok(arg_i64(args, "id").into_iter().collect())
}

/// The view the action ran against (drives which store a shared action like
/// `delete_selected`/`merge_selected` touches, and the refreshed snapshot).
fn arg_view<'a>(args: &'a Value, default: &'a str) -> &'a str {
    args.get("view").and_then(Value::as_str).unwrap_or(default)
}

fn part_has_text(part: &Value) -> bool {
    match part {
        Value::String(text) => !text.trim().is_empty(),
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty()),
        _ => false,
    }
}

fn part_title_text(part: &Value) -> (String, String) {
    match part {
        Value::String(text) => (String::new(), text.trim().to_string()),
        Value::Object(map) => (
            map.get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            map.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        _ => (String::new(), String::new()),
    }
}

// --------------------------------------------------------------------------
// Public interface
// --------------------------------------------------------------------------

/// Shape-validate one action request without touching the store. Callable
/// before [`run_action`] (the REST layer and the frontend both lean on it).
pub fn validate_action_request(action: &str, args: &Value) -> Result<(), AppError> {
    if action == "split_crystal" {
        let parts = args
            .get("parts")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| AppError::Invalid("parts must be an array".into()))?;
        if parts.len() < 2 || parts.iter().any(|part| !part_has_text(part)) {
            return Err(AppError::Invalid(
                "split requires at least two nonempty parts".into(),
            ));
        }
    }
    Ok(())
}

/// The action catalog: the 13 canonical ids the dashboard advertises
/// (`ADMIN_COMMANDS` in `daemon/rest/admin.rs`).
pub const ACTION_NAMES: [&str; 13] = [
    "add_memory",
    "edit_memory",
    "delete_selected",
    "merge_selected",
    "split_crystal",
    "reinforce_crystal",
    "decay_crystal",
    "approve_proposal",
    "reject_proposal",
    "inspect_provenance",
    "inspect_recall_reasons",
    "run_manual_dreaming",
    "review_dream_output",
];

/// Run one typed admin action. `actor` is the transport-authenticated
/// identity (never a request field). Unknown actions are
/// [`AppError::NotImplemented`] so the REST layer can keep the frozen
/// `unknown_admin_action` envelope.
pub fn run_action(
    app: &Application,
    actor: &str,
    action: &str,
    args: &Value,
) -> Result<Value, AppError> {
    validate_action_request(action, args)?;
    let config = app.config();
    match action {
        "add_memory" => add_memory(config, actor, args),
        "edit_memory" => edit_memory(config, actor, args),
        "delete_selected" => delete_selected(config, actor, args),
        "merge_selected" => merge_selected(config, actor, args),
        "split_crystal" => split_crystal(config, actor, args),
        "reinforce_crystal" => feedback_action(config, actor, args, Feedback::Reinforce),
        "decay_crystal" => feedback_action(config, actor, args, Feedback::Decay),
        "approve_proposal" => approve_proposal(config, actor, args),
        "reject_proposal" => reject_proposal(config, actor, args),
        "inspect_provenance" => inspect_provenance(config, args),
        "inspect_recall_reasons" => inspect_recall_reasons(config, args),
        "run_manual_dreaming" => run_manual_dreaming_action(app, actor, args),
        "review_dream_output" => review_dream_output(config, args),
        other => Err(AppError::NotImplemented(other.to_string())),
    }
}

// --------------------------------------------------------------------------
// Shared building blocks
// --------------------------------------------------------------------------

fn action_result(entity_type: &str, entity_id: i64, action: &str, message: &str) -> Value {
    json!({
        "entity_type": entity_type,
        "entity_id": entity_id,
        "action": action,
        "message": message,
    })
}

/// Wrap an [`action_result`] with the view the REST layer should re-project
/// and the row it should keep selected.
fn responded(result: Value, view: &str, selected_id: Option<i64>) -> Value {
    let mut out = json!({ "result": result, "view": view });
    if let Some(id) = selected_id {
        out["selected_id"] = json!(id.to_string());
    }
    out
}

// --------------------------------------------------------------------------
// add_memory
// --------------------------------------------------------------------------

/// `add_memory({series, text, title?, crystal_type?|view?, source_language?,
/// target_language?})` — a new crystal (or lesson) in the current memory
/// view. The crystal row, its FTS row, and the audit row are inserted in ONE
/// transaction (a minimal port of `CrystalStore::add_crystal` for the bare
/// admin shape — no tags/scopes/links). The store's defaults are matched:
/// strength/confidence 0.5, `source_credibility` `observation`, status
/// `active`, `tags_json` `[]`.
fn add_memory(config: &HieronymusConfig, actor: &str, args: &Value) -> Result<Value, AppError> {
    let series = require_str(args, "series")?;
    let text = require_str(args, "text")?;
    let title = arg_str(args, "title").unwrap_or_default().to_string();
    let view = arg_view(args, "Crystals");
    // The Lessons view adds a `lesson`; the Crystals view adds a plain
    // `observation` crystal unless the caller names another allowed type.
    let crystal_type = match arg_str(args, "crystal_type") {
        Some(value) => value.to_string(),
        None if view == "Lessons" => "lesson".to_string(),
        None => "observation".to_string(),
    };
    if !hieronymus::crystals::ALLOWED_CRYSTAL_TYPES.contains(&crystal_type.as_str()) {
        return Err(AppError::Invalid(format!(
            "crystal_type must be one of {:?}",
            hieronymus::crystals::ALLOWED_CRYSTAL_TYPES
        )));
    }
    let now = now_rfc3339();

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    let (default_source, default_target): (String, String) = transaction
        .query_row(
            "select default_source_language, default_target_language from series where slug = ?1",
            [&series],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::Domain(format!("unknown series: {series}"))
            }
            _ => AppError::Domain("admin store is unavailable".to_string()),
        })?;
    let source_language = arg_str(args, "source_language").unwrap_or(&default_source);
    let target_language = arg_str(args, "target_language").unwrap_or(&default_target);
    transaction
        .execute(
            "insert into crystals(
               crystal_type, text, title, scope_type, scope_key, series_slug,
               source_language, target_language, tags_json, strength, confidence,
               source_credibility, rule_intent, soft_origin, is_inferred, malformed_penalty,
               status, created_at, updated_at
             )
             values (?1, ?2, ?3, 'series', ?4, ?5, ?6, ?7, '[]', 0.5, 0.5,
                     'observation', '', '', 0, 0.0, 'active', ?8, ?8)",
            rusqlite::params![
                crystal_type,
                text,
                title,
                format!("series:{series}"),
                series,
                source_language,
                target_language,
                now,
            ],
        )
        .map_err(store_unavailable)?;
    let crystal_id = transaction.last_insert_rowid();
    insert_crystal_fts(&transaction, crystal_id, &title, &text).map_err(store_unavailable)?;
    write_audit(
        &transaction,
        actor,
        "add",
        "crystal",
        &crystal_id.to_string(),
        "Added from admin contract",
        "{}",
        "{}",
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    Ok(responded(
        action_result("crystal", crystal_id, "add", "Crystal added"),
        crystal_view(&crystal_type),
        Some(crystal_id),
    ))
}

// --------------------------------------------------------------------------
// edit_memory
// --------------------------------------------------------------------------

/// `edit_memory({id, text, title?})` — replace the selected crystal's text
/// (Python `edit_crystal`), with a before/after audit row.
fn edit_memory(config: &HieronymusConfig, actor: &str, args: &Value) -> Result<Value, AppError> {
    let crystal_id = require_i64(args, "id")?;
    let text = require_str(args, "text")?;

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    let before = load_crystal_row(&transaction, crystal_id)?;
    let title = match arg_str(args, "title") {
        Some(value) => value.to_string(),
        None => before.title.clone(),
    };
    let before_json = serde_json::to_string(&before).unwrap_or_else(|_| "{}".to_string());
    let now = now_rfc3339();
    transaction
        .execute(
            "update crystals set title = ?1, text = ?2, updated_at = ?3 where id = ?4",
            rusqlite::params![title, text, now, crystal_id],
        )
        .map_err(store_unavailable)?;
    replace_crystal_fts(
        &transaction,
        crystal_id,
        &before.title,
        &before.text,
        &title,
        &text,
    )
    .map_err(store_unavailable)?;
    let after_json = crystal_row_json(&transaction, crystal_id)?;
    write_audit(
        &transaction,
        actor,
        "edit",
        "crystal",
        &crystal_id.to_string(),
        "",
        &before_json,
        &after_json,
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    Ok(responded(
        action_result("crystal", crystal_id, "edit", "Crystal edited"),
        crystal_view(&before.crystal_type),
        Some(crystal_id),
    ))
}

fn crystal_view(crystal_type: &str) -> &'static str {
    if crystal_type == "lesson" {
        "Lessons"
    } else {
        "Crystals"
    }
}

// --------------------------------------------------------------------------
// delete_selected
// --------------------------------------------------------------------------

/// `delete_selected({ids|id, view, confirmed})` — archive the selected rows,
/// all-or-nothing. Both branches pre-flight every id inside the transaction
/// (missing row, inactive concept, rule crystal) before any mutation, then
/// mutate + audit in that one transaction.
///
/// * Concepts: an inlined port of `ConceptStore::archive_concept` (an active
///   concept's status → `archived`); the projection carries no other state.
/// * Crystals/lessons: the Python `delete_crystal` path — a `deleted_by_user`
///   feedback event, the row zeroed and archived, a before/after audit row.
///   A rule crystal (any status) is rejected → the M3 rule action.
fn delete_selected(
    config: &HieronymusConfig,
    actor: &str,
    args: &Value,
) -> Result<Value, AppError> {
    require_confirmed(args)?;
    let ids = id_list(args)?;
    if ids.is_empty() {
        return Err(AppError::Invalid("id or ids is required".to_string()));
    }
    let view = arg_view(args, "Crystals").to_string();
    let now = now_rfc3339();

    if view == "Concepts" {
        let mut connection = open_db(config)?;
        let transaction = connection.transaction().map_err(store_unavailable)?;
        // Pre-flight: every concept must exist and be active (inactive
        // concepts cannot be re-archived — a partial batch would be
        // unrecoverable).
        for concept_id in &ids {
            let status: String = transaction
                .query_row(
                    "select status from concepts where id = ?1",
                    [concept_id],
                    |row| row.get(0),
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        AppError::Domain(format!("unknown concept: {concept_id}"))
                    }
                    _ => AppError::Domain("admin store is unavailable".to_string()),
                })?;
            if matches!(status.as_str(), "archived" | "merged") {
                return Err(AppError::Domain(format!(
                    "concept {concept_id} is already {status}"
                )));
            }
        }
        for concept_id in &ids {
            transaction
                .execute(
                    "update concepts set status = 'archived', updated_at = ?1 where id = ?2",
                    rusqlite::params![now, concept_id],
                )
                .map_err(store_unavailable)?;
            write_audit(
                &transaction,
                actor,
                "archive",
                "concept",
                &concept_id.to_string(),
                "archived from admin contract",
                "{}",
                "{}",
            )
            .map_err(store_unavailable)?;
        }
        transaction.commit().map_err(store_unavailable)?;
        let last = ids.last().copied();
        return Ok(responded(
            action_result(
                "concept",
                last.unwrap_or_default(),
                "delete",
                "Concept archived",
            ),
            "Concepts",
            last,
        ));
    }

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    // Pre-flight every id so nothing is half-deleted (rule-crystal rejection,
    // missing rows).
    let mut rows = Vec::with_capacity(ids.len());
    for crystal_id in &ids {
        let row = load_crystal_row(&transaction, *crystal_id)?;
        reject_rule_crystal(&row, "delete")?;
        rows.push((*crystal_id, row));
    }
    for (crystal_id, row) in &rows {
        let before_json = row_json(row);
        apply_immediate_feedback(
            &transaction,
            *crystal_id,
            row,
            DELTA_DELETED,
            "deleted_by_user",
            "",
        )?;
        transaction
            .execute(
                "update crystals set status = 'archived', strength = 0, confidence = 0,
                     updated_at = ?1 where id = ?2",
                rusqlite::params![now, crystal_id],
            )
            .map_err(store_unavailable)?;
        let after_json = crystal_row_json(&transaction, *crystal_id)?;
        write_audit(
            &transaction,
            actor,
            "delete",
            "crystal",
            &crystal_id.to_string(),
            "",
            &before_json,
            &after_json,
        )
        .map_err(store_unavailable)?;
    }
    transaction.commit().map_err(store_unavailable)?;

    let (last_id, last_row) = rows.last().expect("ids is non-empty");
    Ok(responded(
        action_result("crystal", *last_id, "delete", "Crystal deleted"),
        crystal_view(&last_row.crystal_type),
        Some(*last_id),
    ))
}

// --------------------------------------------------------------------------
// merge_selected
// --------------------------------------------------------------------------

/// `merge_selected({ids, text, title?, view, confirmed})` — merge distinct
/// selected rows into a new memory.
///
/// * Concepts: every id is pre-flighted inside an immediate transaction;
///   every source merges into `ids[0]`, then the audit commits with the batch.
/// * Crystals: the Python `merge_crystals` path — distinct active/candidate
///   rows in a matching context, a new active crystal, the sources linked
///   `merged_from` and archived, one audit row, all in one transaction. A
///   rule crystal (any status) is rejected → the M3 rule action.
fn merge_selected(config: &HieronymusConfig, actor: &str, args: &Value) -> Result<Value, AppError> {
    require_confirmed(args)?;
    let ids = id_list(args)?;
    let view = arg_view(args, "Crystals").to_string();

    if view == "Concepts" {
        if ids.len() < 2 {
            return Err(AppError::Invalid(
                "merge needs a source and a target concept".to_string(),
            ));
        }
        let distinct: std::collections::BTreeSet<i64> = ids.iter().copied().collect();
        if distinct.len() != ids.len() {
            return Err(AppError::Invalid(
                "concept ids must identify distinct concepts".into(),
            ));
        }
        let target = ids[0];
        let sources = &ids[1..];
        let mut connection = open_db(config)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(store_unavailable)?;
        for id in &ids {
            let status: String = transaction
                .query_row("select status from concepts where id = ?1", [id], |row| {
                    row.get(0)
                })
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        AppError::Domain(format!("unknown concept: {id}"))
                    }
                    _ => store_unavailable(error),
                })?;
            if matches!(status.as_str(), "archived" | "merged") {
                return Err(AppError::Domain(format!(
                    "concept {id} is already inactive"
                )));
            }
        }
        for source in sources {
            ConceptStore::merge_concepts_in_transaction(
                &transaction,
                *source,
                target,
                "merged from admin contract",
            )
            .map_err(domain)?;
        }
        write_audit(
            &transaction,
            actor,
            "merge",
            "concept",
            &target.to_string(),
            &format!("merged {sources:?}"),
            "{}",
            "{}",
        )
        .map_err(store_unavailable)?;
        transaction.commit().map_err(store_unavailable)?;
        return Ok(responded(
            action_result("concept", target, "merge", "Concepts merged"),
            "Concepts",
            Some(target),
        ));
    }

    let distinct: std::collections::BTreeSet<i64> = ids.iter().copied().collect();
    if distinct.len() != ids.len() {
        return Err(AppError::Invalid(
            "crystal ids must identify distinct crystals".to_string(),
        ));
    }
    if ids.len() < 2 {
        return Err(AppError::Invalid(
            "at least two distinct crystals are required".to_string(),
        ));
    }
    let text = require_str(args, "text")?;
    let title = arg_str(args, "title").unwrap_or_default().to_string();

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    let mut rows = Vec::with_capacity(ids.len());
    for crystal_id in &ids {
        let row = load_crystal_row(&transaction, *crystal_id)?;
        reject_rule_crystal(&row, "merge")?;
        if row.status != "active" && row.status != "candidate" {
            return Err(AppError::Domain(
                "crystals must be active or candidate".to_string(),
            ));
        }
        rows.push(row);
    }
    let first = &rows[0];
    for row in &rows[1..] {
        for (label, a, b) in [
            ("series_slug", &first.series_slug, &row.series_slug),
            (
                "source_language",
                &first.source_language,
                &row.source_language,
            ),
            (
                "target_language",
                &first.target_language,
                &row.target_language,
            ),
            ("crystal_type", &first.crystal_type, &row.crystal_type),
            ("scope_type", &first.scope_type, &row.scope_type),
            ("scope_key", &first.scope_key, &row.scope_key),
        ] {
            if a != b {
                return Err(AppError::Domain(format!("crystal {label} does not match")));
            }
        }
    }
    let strength = rows.iter().map(|row| row.strength).fold(0.0_f64, f64::max);
    let confidence = rows
        .iter()
        .map(|row| row.confidence)
        .fold(0.0_f64, f64::max);
    let now = now_rfc3339();
    let merged_id = insert_derived_crystal(
        &transaction,
        first,
        &title,
        &text,
        strength,
        confidence,
        &now,
    )
    .map_err(store_unavailable)?;
    for crystal_id in &ids {
        transaction
            .execute(
                "insert or ignore into crystal_links(source_crystal_id, target_crystal_id, link_type)
                 values (?1, ?2, 'merged_from')",
                rusqlite::params![merged_id, crystal_id],
            )
            .map_err(store_unavailable)?;
        transaction
            .execute(
                "update crystals set status = 'archived', updated_at = ?1 where id = ?2",
                rusqlite::params![now, crystal_id],
            )
            .map_err(store_unavailable)?;
    }
    let after_json = crystal_row_json(&transaction, merged_id)?;
    write_audit(
        &transaction,
        actor,
        "merge",
        "crystal",
        &merged_id.to_string(),
        "",
        &serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string()),
        &after_json,
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    Ok(responded(
        action_result("crystal", merged_id, "merge", "Crystals merged"),
        crystal_view(&first.crystal_type),
        Some(merged_id),
    ))
}

// --------------------------------------------------------------------------
// split_crystal
// --------------------------------------------------------------------------

/// `split_crystal({id, parts, confirmed})` — split the selected crystal into
/// two or more new memories (Python `split_crystal`). `parts` is validated by
/// [`validate_action_request`]. Rule crystals are rejected.
fn split_crystal(config: &HieronymusConfig, actor: &str, args: &Value) -> Result<Value, AppError> {
    require_confirmed(args)?;
    let crystal_id = require_i64(args, "id")?;
    let parts: Vec<(String, String)> = args
        .get("parts")
        .and_then(Value::as_array)
        .map(|array| array.iter().map(part_title_text).collect())
        .unwrap_or_default();

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    let source = load_crystal_row(&transaction, crystal_id)?;
    reject_rule_crystal(&source, "split")?;
    if source.status != "active" && source.status != "candidate" {
        return Err(AppError::Domain(
            "crystals must be active or candidate".to_string(),
        ));
    }
    let now = now_rfc3339();
    let mut new_ids = Vec::with_capacity(parts.len());
    for (title, text) in &parts {
        if text.trim().is_empty() {
            return Err(AppError::Invalid("part text must not be empty".to_string()));
        }
        let new_id = insert_derived_crystal(
            &transaction,
            &source,
            title,
            text,
            source.strength,
            source.confidence,
            &now,
        )
        .map_err(store_unavailable)?;
        transaction
            .execute(
                "insert or ignore into crystal_links(source_crystal_id, target_crystal_id, link_type)
                 values (?1, ?2, 'split_from')",
                rusqlite::params![new_id, crystal_id],
            )
            .map_err(store_unavailable)?;
        new_ids.push(new_id);
    }
    let before_json = row_json(&source);
    transaction
        .execute(
            "update crystals set status = 'archived', updated_at = ?1 where id = ?2",
            rusqlite::params![now, crystal_id],
        )
        .map_err(store_unavailable)?;
    write_audit(
        &transaction,
        actor,
        "split",
        "crystal",
        &crystal_id.to_string(),
        "",
        &before_json,
        &serde_json::to_string(&new_ids).unwrap_or_else(|_| "[]".to_string()),
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    let first_new = new_ids.first().copied();
    Ok(responded(
        json!({
            "entity_type": "crystal",
            "entity_id": crystal_id,
            "action": "split",
            "message": "Crystal split",
            "new_crystal_ids": new_ids,
        }),
        crystal_view(&source.crystal_type),
        first_new,
    ))
}

// --------------------------------------------------------------------------
// reinforce_crystal / decay_crystal
// --------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Feedback {
    Reinforce,
    Decay,
}

/// The immediate-feedback transaction (Python `reinforce_crystal` /
/// `decay_crystal`): a `memory_events` row, the clamped score update, and an
/// audit row — all together.
fn feedback_action(
    config: &HieronymusConfig,
    actor: &str,
    args: &Value,
    feedback: Feedback,
) -> Result<Value, AppError> {
    let crystal_id = require_i64(args, "id")?;
    let (event_type, deltas, audit_action, message, default_evidence) = match feedback {
        Feedback::Reinforce => (
            "confirmed_by_user",
            DELTA_CONFIRMED,
            "reinforce",
            "Crystal reinforced",
            "Reinforced from admin bridge",
        ),
        Feedback::Decay => (
            "contradicted_by_user",
            DELTA_CONTRADICTED,
            "decay",
            "Crystal decayed",
            "Decayed from admin bridge",
        ),
    };
    let evidence = arg_str(args, "evidence")
        .unwrap_or(default_evidence)
        .to_string();

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    let row = load_crystal_row(&transaction, crystal_id)?;
    apply_immediate_feedback(
        &transaction,
        crystal_id,
        &row,
        deltas,
        event_type,
        &evidence,
    )?;
    write_audit(
        &transaction,
        actor,
        audit_action,
        "crystal",
        &crystal_id.to_string(),
        &evidence,
        "{}",
        "{}",
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    Ok(responded(
        action_result("crystal", crystal_id, audit_action, message),
        crystal_view(&row.crystal_type),
        Some(crystal_id),
    ))
}

/// Python `_record_immediate_feedback`: the `memory_events` row plus the
/// clamped score update (and the `deleted_by_user` archive rule).
fn apply_immediate_feedback(
    transaction: &Transaction<'_>,
    crystal_id: i64,
    row: &CrystalRow,
    deltas: (f64, f64),
    event_type: &str,
    evidence: &str,
) -> Result<(), AppError> {
    let (strength_delta, confidence_delta) = deltas;
    let now = now_rfc3339();
    transaction
        .execute(
            "insert into memory_events(
               crystal_id, session_id, event_type, source_role, evidence,
               strength_delta, confidence_delta, applied, created_at
             )
             values (?1, null, ?2, 'user', ?3, ?4, ?5, 1, ?6)",
            rusqlite::params![
                crystal_id,
                event_type,
                evidence,
                strength_delta,
                confidence_delta,
                now,
            ],
        )
        .map_err(store_unavailable)?;
    let strength = clamp_score(row.strength + strength_delta);
    let confidence = clamp_score(row.confidence + confidence_delta);
    let mut status = row.status.clone();
    if event_type == "deleted_by_user" && strength < ARCHIVE_STRENGTH_THRESHOLD {
        status = "archived".to_string();
    }
    transaction
        .execute(
            "update crystals set strength = ?1, confidence = ?2, status = ?3, updated_at = ?4
             where id = ?5",
            rusqlite::params![strength, confidence, status, now, crystal_id],
        )
        .map_err(store_unavailable)?;
    Ok(())
}

// --------------------------------------------------------------------------
// approve_proposal / reject_proposal
// --------------------------------------------------------------------------

/// `approve_proposal({id, reason?})` — approve a pending strict-concept
/// proposal: create the advisory concept (canonical name, description, the
/// `concept-proposal` semantic tag, and its canonical rendering facet), mark
/// the proposal `approved`, and audit — ALL in one transaction. On any
/// failure nothing persists: no partial concept/facet/tag, the proposal stays
/// `pending`, and no audit row. (Rule *terms* are unaffected: this is the
/// advisory strict-concept surface, not ADR 0011's rule lifecycle. The
/// concept row is inserted directly rather than through `ConceptStore` so the
/// whole approval is atomic; the `concepts_ai` trigger maintains its FTS.)
fn approve_proposal(
    config: &HieronymusConfig,
    actor: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let proposal_id = require_i64(args, "id")?;
    let reason = arg_str(args, "reason").unwrap_or("approved from admin contract");

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    let proposal = load_proposal(&transaction, proposal_id)?;
    if proposal.status != "pending" {
        return Err(AppError::Domain("proposal must be pending".to_string()));
    }
    let name = proposal.concept_text.trim();
    if name.is_empty() {
        return Err(AppError::Invalid(
            "proposal has no concept text".to_string(),
        ));
    }
    let (scope_type, scope_key) = if proposal.series_slug.is_empty() {
        ("global", String::new())
    } else {
        ("series", format!("series:{}", proposal.series_slug))
    };
    let now = now_rfc3339();
    transaction
        .execute(
            "insert into concepts(
               canonical_name, description, scope_type, scope_key, status, confidence,
               created_at, updated_at
             )
             values (?1, ?2, ?3, ?4, 'candidate', 0.2, ?5, ?5)",
            rusqlite::params![name, proposal.rationale, scope_type, scope_key, now],
        )
        .map_err(store_unavailable)?;
    let concept_id = transaction.last_insert_rowid();
    transaction
        .execute(
            "insert into concept_semantic_tags(concept_id, tag, created_at)
             values (?1, 'concept-proposal', ?2)",
            rusqlite::params![concept_id, now],
        )
        .map_err(store_unavailable)?;
    let rendering = proposal.canonical_rendering.trim();
    if !rendering.is_empty() {
        transaction
            .execute(
                "insert into concept_facets(
                   concept_id, language, facet_type, value, confidence, is_canonical,
                   created_at, updated_at
                 )
                 values (?1, ?2, 'rendering', ?3, 0.2, 1, ?4, ?4)",
                rusqlite::params![concept_id, proposal.target_language, rendering, now],
            )
            .map_err(store_unavailable)?;
    }
    // These facets preserve proposal evidence only; they do not establish
    // deterministic terminology authority. A forbidden spelling is a note,
    // never an approved rendering or canonical facet.
    for (variants, kind, tag) in [
        (&proposal.approved_variants, "rendering", "approved-variant"),
        (&proposal.forbidden_variants, "note", "forbidden-variant"),
    ] {
        for variant in variants {
            transaction
                .execute(
                    "insert into concept_facets(concept_id, language, facet_type, value,
                    confidence, is_canonical, created_at, updated_at)
                 values (?1, ?2, ?3, ?4, 0.2, 0, ?5, ?5)",
                    rusqlite::params![concept_id, proposal.target_language, kind, variant, now],
                )
                .map_err(store_unavailable)?;
            let facet_id = transaction.last_insert_rowid();
            transaction.execute(
                "insert into concept_facet_semantic_tags(facet_id, semantic_tag) values (?1, ?2)",
                rusqlite::params![facet_id, tag],
            ).map_err(store_unavailable)?;
        }
    }
    transaction
        .execute(
            "update strict_concept_proposals set status = 'approved', updated_at = ?1 where id = ?2",
            rusqlite::params![now, proposal_id],
        )
        .map_err(store_unavailable)?;
    write_audit(
        &transaction,
        actor,
        "approve",
        "strict_concept_proposal",
        &proposal_id.to_string(),
        reason,
        "{}",
        &json!({ "concept_id": concept_id }).to_string(),
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    Ok(responded(
        json!({
            "entity_type": "strict_concept_proposal",
            "entity_id": proposal_id,
            "action": "approve",
            "message": "Proposal approved",
            "concept_id": concept_id,
        }),
        "Proposals",
        Some(proposal_id),
    ))
}

/// `reject_proposal({id, reason})` — mark a pending proposal `rejected` with a
/// before/after audit row (Python `reject_proposal`).
fn reject_proposal(
    config: &HieronymusConfig,
    actor: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let proposal_id = require_i64(args, "id")?;
    let reason = require_str(args, "reason")?;

    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    let before = load_proposal(&transaction, proposal_id)?;
    if before.status != "pending" {
        return Err(AppError::Domain("proposal must be pending".to_string()));
    }
    transaction
        .execute(
            "update strict_concept_proposals set status = 'rejected', updated_at = ?1 where id = ?2",
            rusqlite::params![now_rfc3339(), proposal_id],
        )
        .map_err(store_unavailable)?;
    write_audit(
        &transaction,
        actor,
        "reject",
        "strict_concept_proposal",
        &proposal_id.to_string(),
        &reason,
        &json!({ "status": before.status }).to_string(),
        &json!({ "status": "rejected" }).to_string(),
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    Ok(responded(
        action_result(
            "strict_concept_proposal",
            proposal_id,
            "reject",
            "Proposal rejected",
        ),
        "Proposals",
        Some(proposal_id),
    ))
}

struct ProposalRow {
    approved_variants: Vec<String>,
    forbidden_variants: Vec<String>,
    concept_text: String,
    canonical_rendering: String,
    rationale: String,
    series_slug: String,
    target_language: String,
    status: String,
}

fn load_proposal(connection: &Connection, proposal_id: i64) -> Result<ProposalRow, AppError> {
    connection
        .query_row(
            "select concept_text, canonical_rendering, rationale, series_slug,
                    target_language, status, approved_variants_json, forbidden_variants_json
             from strict_concept_proposals where id = ?1",
            [proposal_id],
            |row| {
                let variants = |column: &str| -> rusqlite::Result<Vec<String>> {
                    let raw: String = row.get(column)?;
                    serde_json::from_str(&raw).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            row.as_ref().column_index(column).unwrap_or(0),
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })
                };
                Ok(ProposalRow {
                    approved_variants: variants("approved_variants_json")?,
                    forbidden_variants: variants("forbidden_variants_json")?,
                    concept_text: row.get("concept_text")?,
                    canonical_rendering: row.get("canonical_rendering")?,
                    rationale: row.get("rationale")?,
                    series_slug: row.get("series_slug")?,
                    target_language: row.get("target_language")?,
                    status: row.get("status")?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::Domain(format!("unknown concept proposal: {proposal_id}"))
            }
            _ => AppError::Domain("admin store is unavailable".to_string()),
        })
}

// --------------------------------------------------------------------------
// inspect_provenance / inspect_recall_reasons (read-only)
// --------------------------------------------------------------------------

/// `inspect_provenance({id})` — the source short-term memories behind a
/// crystal (Python `ProvenanceDetail{title, sources}`). Read-only.
fn inspect_provenance(config: &HieronymusConfig, args: &Value) -> Result<Value, AppError> {
    let crystal_id = require_i64(args, "id")?;
    let connection = open_db(config)?;
    let crystal = load_crystal_row(&connection, crystal_id)?;
    let mut statement = connection
        .prepare(
            "select m.id, m.session_id, m.source_role, m.kind, m.text, m.source_ref
             from crystal_sources as s
             join short_term_memories as m on m.id = s.short_term_memory_id
             where s.crystal_id = ?1
             order by m.id",
        )
        .map_err(store_unavailable)?;
    let sources: Vec<Value> = statement
        .query_map([crystal_id], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?.to_string(),
                "session_id": row.get::<_, i64>(1)?.to_string(),
                "source_role": row.get::<_, String>(2)?,
                "kind": row.get::<_, String>(3)?,
                "text": row.get::<_, String>(4)?,
                "source_ref": row.get::<_, String>(5)?,
            }))
        })
        .map_err(store_unavailable)?
        .collect::<rusqlite::Result<_>>()
        .map_err(store_unavailable)?;

    let title = if crystal.title.is_empty() {
        excerpt(&crystal.text)
    } else {
        crystal.title.clone()
    };
    let mut out = responded(
        action_result(
            "crystal",
            crystal_id,
            "inspect_provenance",
            "Provenance loaded",
        ),
        crystal_view(&crystal.crystal_type),
        Some(crystal_id),
    );
    out["provenance"] = json!({ "title": title, "sources": sources });
    Ok(out)
}

/// `inspect_recall_reasons({id, recall_id?})` — the recorded recall
/// activations for a crystal (Python `recall_reasons_for_crystal`). Read-only.
fn inspect_recall_reasons(config: &HieronymusConfig, args: &Value) -> Result<Value, AppError> {
    let crystal_id = require_i64(args, "id")?;
    let recall_id = arg_str(args, "recall_id").map(str::to_string);
    let connection = open_db(config)?;
    let crystal = load_crystal_row(&connection, crystal_id)?;
    let mut statement = connection
        .prepare(
            "select session_id, recall_query, rank, score, reason, recall_id
             from crystal_activations
             where crystal_id = ?1 and (?2 is null or recall_id = ?2)
             order by id desc",
        )
        .map_err(store_unavailable)?;
    let reasons: Vec<Value> = statement
        .query_map(rusqlite::params![crystal_id, recall_id], |row| {
            let score: f64 = row.get(3)?;
            Ok(json!({
                "session_id": row.get::<_, i64>(0)?.to_string(),
                "query": row.get::<_, String>(1)?,
                "rank": row.get::<_, i64>(2)?.to_string(),
                "score": format!("{score:.3}"),
                "reason": row.get::<_, String>(4)?,
                "recall_id": row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            }))
        })
        .map_err(store_unavailable)?
        .collect::<rusqlite::Result<_>>()
        .map_err(store_unavailable)?;

    let mut out = responded(
        action_result(
            "crystal",
            crystal_id,
            "inspect_recall_reasons",
            "Recall reasons loaded",
        ),
        crystal_view(&crystal.crystal_type),
        Some(crystal_id),
    );
    out["reasons"] = json!(reasons);
    Ok(out)
}

// --------------------------------------------------------------------------
// run_manual_dreaming / review_dream_output
// --------------------------------------------------------------------------

/// `run_manual_dreaming({all?})` — run a manual drain through the daemon's
/// supervised dream controller (task D5): the same coalesced worker the
/// dedicated `/api/admin/actions/run_manual_dreaming` route and the MCP
/// `hieronymus_dream` tool serve through, over the configured workflow
/// lanes. Synchronous: this action waits for the drain and reports the
/// aggregate. Without a running daemon (no controller installed) it fails
/// closed instead of constructing a provider itself.
fn run_manual_dreaming_action(
    app: &Application,
    actor: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let drain_all = args.get("all").and_then(Value::as_bool).unwrap_or(true);
    let controller = app.dream_controller().ok_or_else(|| {
        AppError::Domain(
            "manual dreaming runs on the daemon's dream controller; no controller is \
             installed, so the action is only served by a running daemon"
                .to_string(),
        )
    })?;
    let drain = controller
        .request_and_wait(crate::daemon::dream_worker::DreamRequest {
            all: drain_all,
            manual: true,
        })
        .map_err(domain)?;
    let record = &drain.record;

    let config = app.config();
    let mut connection = open_db(config)?;
    let transaction = connection.transaction().map_err(store_unavailable)?;
    write_audit(
        &transaction,
        actor,
        "run",
        "dream",
        &record.id.to_string(),
        &format!(
            "Manual dream drain ({} batches) with provider {}",
            drain.batches, record.provider
        ),
        "{}",
        "{}",
    )
    .map_err(store_unavailable)?;
    transaction.commit().map_err(store_unavailable)?;

    let mut out = responded(
        action_result("dream", record.id, "run", "Manual dream run complete"),
        "Dream Runs",
        Some(record.id),
    );
    out["run"] = json!({
        "id": record.id,
        "cycle_id": record.cycle_id,
        "status": record.status,
        "provider": record.provider,
        "input_count": drain.input_count,
        "created_crystal_count": drain.created_crystal_count,
        "proposal_count": drain.proposal_count,
        "error": record.error,
    });
    Ok(out)
}

/// `review_dream_output({run_id})` — the review payload for a finished dream
/// run (Python `admin.dream_review` → `DreamReview`). Read-only.
fn review_dream_output(config: &HieronymusConfig, args: &Value) -> Result<Value, AppError> {
    let run_id = require_i64(args, "run_id").or_else(|_| require_i64(args, "id"))?;
    let connection = open_db(config)?;
    let (cycle_id, status, error): (i64, String, String) = connection
        .query_row(
            "select cycle_id, status, error from dream_runs where id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::Domain(format!("unknown dream run: {run_id}"))
            }
            _ => AppError::Domain("admin store is unavailable".to_string()),
        })?;

    let string_list = |sql: &str, param: i64| -> Result<Vec<String>, AppError> {
        let mut statement = connection.prepare(sql).map_err(store_unavailable)?;
        let rows = statement
            .query_map([param], |row| row.get::<_, String>(0))
            .map_err(store_unavailable)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(store_unavailable)?;
        Ok(rows)
    };
    let int_list = |sql: &str, param: i64| -> Result<Vec<i64>, AppError> {
        let mut statement = connection.prepare(sql).map_err(store_unavailable)?;
        let rows = statement
            .query_map([param], |row| row.get::<_, i64>(0))
            .map_err(store_unavailable)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(store_unavailable)?;
        Ok(rows)
    };

    let source_sessions = int_list(
        "select id from task_sessions where cycle_id = ?1 order by id",
        cycle_id,
    )?;
    let consumed_memories = string_list(
        "select m.text from short_term_memories as m
         join task_sessions as s on s.id = m.session_id
         where s.cycle_id = ?1 order by m.id",
        cycle_id,
    )?;
    let created_crystals = string_list(
        "select text from crystals where created_cycle = ?1 order by id",
        cycle_id,
    )?;
    let strict_proposals = string_list(
        "select concept_text from strict_concept_proposals where dream_run_id = ?1 order by id",
        run_id,
    )?;
    let decayed_crystals = string_list(
        "select coalesce(nullif(c.title, ''), c.text, cast(m.crystal_id as text), '') as label
         from memory_events as m
         left join crystals as c on c.id = m.crystal_id
         where m.cycle_id = ?1 and m.applied = 1 and m.event_type = 'cycle_decay'
           and (m.strength_delta < 0 or m.confidence_delta < 0)
         order by m.id",
        cycle_id,
    )?;

    let mut passes_statement = connection
        .prepare(
            "select phase, status, input_count, output_count
             from dream_phase_runs where dream_run_id = ?1 order by id",
        )
        .map_err(store_unavailable)?;
    let passes: Vec<Value> = passes_statement
        .query_map([run_id], |row| {
            let phase: String = row.get(0)?;
            let status: String = row.get(1)?;
            let input_count: i64 = row.get(2)?;
            let output_count: i64 = row.get(3)?;
            let covered = if phase == "coverage_audit" {
                output_count
            } else {
                0
            };
            Ok(json!({
                "name": phase,
                "status": status,
                "input_count": input_count,
                "output_count": output_count,
                "covered_count": covered,
            }))
        })
        .map_err(store_unavailable)?
        .collect::<rusqlite::Result<_>>()
        .map_err(store_unavailable)?;

    let failed_outputs = if status == "failed" && !error.is_empty() {
        vec![error]
    } else {
        Vec::new()
    };

    let mut out = responded(
        action_result(
            "dream",
            run_id,
            "review_dream_output",
            "Dream review loaded",
        ),
        "Dream Runs",
        Some(run_id),
    );
    out["review"] = json!({
        "run_id": run_id,
        "source_sessions": source_sessions,
        "consumed_memories": consumed_memories,
        "created_crystals": created_crystals,
        // Python `AdminStore.dream_review` also returns these two empty (the
        // deterministic dream provider records no crystal updates and no
        // validation errors as separate review lists) — this is byte parity
        // with `admin_models.DreamReview`, not a stub.
        "updated_crystals": [],
        "decayed_crystals": decayed_crystals,
        "strict_proposals": strict_proposals,
        "failed_outputs": failed_outputs,
        "validation_errors": [],
        "passes": passes,
    });
    Ok(out)
}
