//! Admin memory projections (plan W2): the read-only domain view behind the
//! web console's ten "Memory" views.
//!
//! [`snapshot`] is a pure projection over the data-root stores. It ports the
//! Python `AdminStore.snapshot` / `_rows_for_view` / `_detail_for_view`
//! result shapes (`src/hieronymus/admin.py`, `admin_models.py`): every row is
//! the frozen `AdminRow` shape
//! `{id, kind, label, status, scope, language_pair, quality_label, tags}` and
//! every detail is `{title, subtitle, body, fields}`. The envelope is
//! `{view, rows, selected, detail, filters}` — identical to Python's
//! `AdminSnapshot`; no `total` key is added so the frozen
//! `http.route.get.api.admin.snapshot` contract stays byte-identical.
//!
//! The `Crystals` and `Lessons` views are ported verbatim from the previous
//! REST handler (`daemon/rest/admin.rs::snapshot_value`) so their frozen
//! fixture stays green; the other eight views are ported from Python.
//!
//! Reads are bounded (`limit`/`offset` from the query, clamped) and share the
//! `Application` series-scope filter: a `series` filter in the query keeps
//! only rows in that series (plus global concepts), so a foreign-context
//! record never leaks. Views with no series column (`Dream Runs`,
//! `Dream Audits`, `Audit Log`) are global and ignore the filter.
//!
//! No SQL failure text or file path is surfaced: store errors collapse to a
//! generic [`AppError::Domain`] message.

use rusqlite::Connection;
use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::db::open_migrated;

use crate::application::AppError;

/// The ten advertised admin views, in navigation order (Python `ADMIN_VIEWS`).
pub const VIEW_NAMES: [&str; 10] = [
    "Concepts",
    "Renderings",
    "Crystals",
    "Lessons",
    "Short-Term Memory",
    "Short-Term Sessions",
    "Dream Runs",
    "Proposals",
    "Dream Audits",
    "Audit Log",
];

/// The view keys parallel to [`VIEW_NAMES`] (Python `ADMIN_VIEW_KEYS`); the
/// REST layer accepts either the label or the key.
const VIEW_KEYS: [&str; 10] = [
    "concepts",
    "renderings",
    "crystals",
    "lessons",
    "short_term_memory",
    "short_term_sessions",
    "dream_runs",
    "proposals",
    "dream_audits",
    "audit_log",
];

/// Default row cap (Python `limit 200`); Short-Term Memory uses 500.
const DEFAULT_LIMIT: i64 = 200;
const SHORT_TERM_LIMIT: i64 = 500;
const MAX_LIMIT: i64 = 500;

/// Normalize a view label or key to its canonical label.
fn canonical_view(view: &str) -> Option<&'static str> {
    if let Some(label) = VIEW_NAMES.iter().find(|label| **label == view) {
        return Some(label);
    }
    VIEW_KEYS
        .iter()
        .position(|key| *key == view)
        .map(|position| VIEW_NAMES[position])
}

/// The parsed query: selection, bounded paging, and the shared series scope.
struct AdminQuery {
    selected_id: Option<String>,
    limit: i64,
    offset: i64,
    series: Option<String>,
}

impl AdminQuery {
    fn parse(query: &Value, default_limit: i64) -> Self {
        let string = |key: &str| {
            query
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let integer = |key: &str| match query.get(key) {
            Some(Value::Number(number)) => number.as_i64(),
            Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
            _ => None,
        };
        let limit = integer("limit")
            .unwrap_or(default_limit)
            .clamp(1, MAX_LIMIT);
        let offset = integer("offset").unwrap_or(0).max(0);
        AdminQuery {
            selected_id: string("selected_id").or_else(|| string("id")),
            limit,
            offset,
            series: string("series").or_else(|| string("context")),
        }
    }
}

/// Project one admin view over the data root. `view` is a label or a key;
/// `query` may carry `selected_id`/`id`, `limit`, `offset`, and `series`
/// (aliased `context`). Unknown views are [`AppError::Invalid`].
pub fn snapshot(config: &HieronymusConfig, view: &str, query: &Value) -> Result<Value, AppError> {
    let view = canonical_view(view)
        .ok_or_else(|| AppError::Invalid(format!("unknown admin view: {view}")))?;
    let default_limit = if view == "Short-Term Memory" {
        SHORT_TERM_LIMIT
    } else {
        DEFAULT_LIMIT
    };
    let params = AdminQuery::parse(query, default_limit);

    let connection = open_migrated(&config.database_path())
        .map_err(|_| AppError::Domain("admin store is unavailable".to_string()))?;

    let rows = rows_for_view(&connection, view, &params)
        .map_err(|_| AppError::Domain(format!("failed to load the {view} view")))?;
    let selected = select_row(&rows, params.selected_id.as_deref());
    let detail = detail_for_view(&connection, view, selected.as_ref())
        .map_err(|_| AppError::Domain(format!("failed to load the {view} detail")))?;

    Ok(json!({
        "view": view,
        "rows": rows,
        "selected": selected.unwrap_or(Value::Null),
        "detail": detail,
        "filters": [],
    }))
}

/// Pick the selected row: the matching id, else the first row, else `None`
/// (Python `_select_row`).
fn select_row(rows: &[Value], selected_id: Option<&str>) -> Option<Value> {
    let first = rows.first()?.clone();
    let Some(wanted) = selected_id else {
        return Some(first);
    };
    rows.iter()
        .find(|row| row_id_string(row).as_deref() == Some(wanted))
        .cloned()
        .or(Some(first))
}

fn row_id_string(row: &Value) -> Option<String> {
    match row.get("id") {
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

fn rows_for_view(
    connection: &Connection,
    view: &str,
    params: &AdminQuery,
) -> rusqlite::Result<Vec<Value>> {
    match view {
        "Concepts" => concept_rows(connection, params),
        "Renderings" => rendering_rows(connection, params),
        "Crystals" => crystal_rows(connection, params, None),
        "Lessons" => crystal_rows(connection, params, Some("lesson")),
        "Short-Term Memory" => short_term_rows(connection, params),
        "Short-Term Sessions" => session_rows(connection, params),
        "Dream Runs" => dream_run_rows(connection, params),
        "Proposals" => proposal_rows(connection, params),
        "Dream Audits" => dream_audit_rows(connection, params),
        "Audit Log" => audit_log_rows(connection, params),
        _ => unreachable!("canonical_view already validated {view}"),
    }
}

/// Build one row in the frozen `AdminRow` shape. The eight parameters are
/// exactly that dataclass's eight fields, so a positional constructor is the
/// clearest mapping to the Python oracle.
#[allow(clippy::too_many_arguments)]
fn admin_row(
    id: Value,
    kind: &str,
    label: String,
    status: &str,
    scope: String,
    language_pair: String,
    quality_label: String,
    tags: Vec<String>,
) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "label": label,
        "status": status,
        "scope": scope,
        "language_pair": language_pair,
        "quality_label": quality_label,
        "tags": tags,
    })
}

/// `_list_concept_rows`: one row per concept, scoped by `series` when set
/// (exact scope-key match plus global concepts, the `Application` rule).
fn concept_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let scope_key = params
        .series
        .as_ref()
        .map(|series| format!("series:{series}"));
    let mut statement = connection.prepare(
        "select id, canonical_name, status, confidence, scope_type, scope_key
         from concepts
         where (?1 is null or scope_key = ?1 or scope_type = 'global')
         order by id
         limit ?2 offset ?3",
    )?;
    let rows = statement
        .query_map(
            rusqlite::params![scope_key, params.limit, params.offset],
            |row| {
                Ok((
                    row.get::<_, i64>("id")?,
                    row.get::<_, String>("canonical_name")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, f64>("confidence")?,
                    row.get::<_, String>("scope_type")?,
                    row.get::<_, String>("scope_key")?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Tags need their own borrow of the connection: resolve after the row
    // statement's borrow is released.
    let mut out = Vec::with_capacity(rows.len());
    for (id, canonical_name, status, confidence, scope_type, scope_key) in rows {
        let scope = if scope_key.is_empty() {
            scope_type.clone()
        } else {
            scope_key
        };
        out.push(admin_row(
            json!(id),
            &scope_type,
            canonical_name,
            &status,
            scope,
            String::new(),
            format!("{} conf", percent(confidence)),
            load_concept_tags(connection, id)?,
        ));
    }
    Ok(out)
}

fn load_concept_tags(connection: &Connection, concept_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection
        .prepare("select tag from concept_semantic_tags where concept_id = ?1 order by tag")?;
    statement
        .query_map([concept_id], |row| row.get::<_, String>(0))?
        .collect()
}

/// `_list_strict_terms(label_column="canonical_translation")` — the Renderings
/// view is the strict-term table.
fn rendering_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select id, category, canonical_translation, status, series_slug,
                source_language, target_language
         from strict_terms
         where (?1 is null or series_slug = ?1)
         order by id
         limit ?2 offset ?3",
    )?;
    let rows = statement
        .query_map(
            rusqlite::params![params.series, params.limit, params.offset],
            |row| {
                let id: i64 = row.get("id")?;
                let category: String = row.get("category")?;
                let label: String = row.get("canonical_translation")?;
                let status: String = row.get("status")?;
                let series_slug: String = row.get("series_slug")?;
                let language_pair = language_pair(
                    &row.get::<_, String>("source_language")?,
                    &row.get::<_, String>("target_language")?,
                );
                Ok((id, category, label, status, series_slug, language_pair))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, category, label, status, series_slug, language_pair) in rows {
        out.push(admin_row(
            json!(id),
            &category,
            label,
            &status,
            series_slug,
            language_pair,
            String::new(),
            load_strict_term_tags(connection, id)?,
        ));
    }
    Ok(out)
}

fn load_strict_term_tags(connection: &Connection, term_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut statement =
        connection.prepare("select tag from strict_term_tags where term_id = ?1 order by tag")?;
    statement
        .query_map([term_id], |row| row.get::<_, String>(0))?
        .collect()
}

/// The Crystals/Lessons rows — ported verbatim from the previous REST handler
/// so the frozen snapshot fixture stays byte-identical.
fn crystal_rows(
    connection: &Connection,
    params: &AdminQuery,
    kind: Option<&'static str>,
) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select id, crystal_type, title, text, status, series_slug, scope_key, scope_type,
                source_language, target_language, tags_json, confidence, strength
         from crystals
         where (?1 is null or crystal_type = ?1)
           and (?2 is null or series_slug = ?2)
         order by id
         limit ?3 offset ?4",
    )?;
    statement
        .query_map(
            rusqlite::params![kind, params.series, params.limit, params.offset],
            |row| {
                let id: i64 = row.get("id")?;
                let kind: String = row.get("crystal_type")?;
                let title: String = row.get("title")?;
                let text: String = row.get("text")?;
                let status: String = row.get("status")?;
                let series_slug: String = row.get("series_slug")?;
                let scope_key: String = row.get("scope_key")?;
                let scope_type: String = row.get("scope_type")?;
                let source_language: String = row.get("source_language")?;
                let target_language: String = row.get("target_language")?;
                let tags_json: String = row.get("tags_json")?;
                let confidence: f64 = row.get("confidence")?;
                let strength: f64 = row.get("strength")?;
                let label = if title.is_empty() {
                    excerpt(&text)
                } else {
                    title
                };
                let scope = if !series_slug.is_empty() {
                    series_slug
                } else if !scope_key.is_empty() {
                    scope_key
                } else {
                    scope_type
                };
                Ok(admin_row(
                    json!(id),
                    &kind,
                    label,
                    &status,
                    scope,
                    format!("{source_language} -> {target_language}"),
                    quality_label(confidence, strength),
                    serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default(),
                ))
            },
        )?
        .collect()
}

/// `_list_short_term_memory_rows`: unarchived short-term memories joined to
/// their session for the series/language and session status.
fn short_term_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select m.id, m.kind, m.text, m.session_id, m.source_role,
                s.status as session_status, s.source_language, s.target_language
         from short_term_memories as m
         join task_sessions as s on s.id = m.session_id
         where m.archived_at is null
           and (?1 is null or s.series_slug = ?1)
         order by m.id desc
         limit ?2 offset ?3",
    )?;
    statement
        .query_map(
            rusqlite::params![params.series, params.limit, params.offset],
            |row| {
                let id: i64 = row.get("id")?;
                let kind: String = row.get("kind")?;
                let text: String = row.get("text")?;
                let session_id: i64 = row.get("session_id")?;
                let source_role: String = row.get("source_role")?;
                let session_status: String = row.get("session_status")?;
                let source_language: String = row.get("source_language")?;
                let target_language: String = row.get("target_language")?;
                Ok(admin_row(
                    json!(id),
                    &kind,
                    excerpt(&text),
                    &session_status,
                    format!("session:{session_id}"),
                    language_pair(&source_language, &target_language),
                    source_role,
                    Vec::new(),
                ))
            },
        )?
        .collect()
}

/// `_list_sessions`: the Short-Term Sessions view.
fn session_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select id, task_type, series_slug, volume, chapter, status,
                source_language, target_language
         from task_sessions
         where (?1 is null or series_slug = ?1)
         order by id desc
         limit ?2 offset ?3",
    )?;
    statement
        .query_map(
            rusqlite::params![params.series, params.limit, params.offset],
            |row| {
                let id: i64 = row.get("id")?;
                let task_type: String = row.get("task_type")?;
                let series_slug: String = row.get("series_slug")?;
                let volume: String = row.get("volume")?;
                let chapter: String = row.get("chapter")?;
                let status: String = row.get("status")?;
                let source_language: String = row.get("source_language")?;
                let target_language: String = row.get("target_language")?;
                Ok(admin_row(
                    json!(id),
                    &task_type,
                    session_label(&series_slug, &volume, &chapter),
                    &status,
                    series_slug,
                    language_pair(&source_language, &target_language),
                    String::new(),
                    Vec::new(),
                ))
            },
        )?
        .collect()
}

/// `_list_dream_runs`: global scope.
fn dream_run_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select id, provider, cycle_id, status, created_crystal_count, proposal_count
         from dream_runs
         order by id desc
         limit ?1 offset ?2",
    )?;
    statement
        .query_map(rusqlite::params![params.limit, params.offset], |row| {
            let id: i64 = row.get("id")?;
            let provider: String = row.get("provider")?;
            let cycle_id: i64 = row.get("cycle_id")?;
            let status: String = row.get("status")?;
            let crystals: i64 = row.get("created_crystal_count")?;
            let proposals: i64 = row.get("proposal_count")?;
            Ok(admin_row(
                json!(id),
                &provider,
                format!("Cycle {cycle_id}"),
                &status,
                "global".to_string(),
                String::new(),
                format!("{crystals} crystals / {proposals} proposals"),
                Vec::new(),
            ))
        })?
        .collect()
}

/// `_list_dream_audits`: global scope.
fn dream_audit_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select id, event_type, summary, severity, dream_run_id, created_at
         from dream_audit_entries
         order by id desc
         limit ?1 offset ?2",
    )?;
    statement
        .query_map(rusqlite::params![params.limit, params.offset], |row| {
            let id: i64 = row.get("id")?;
            let event_type: String = row.get("event_type")?;
            let summary: String = row.get("summary")?;
            let severity: String = row.get("severity")?;
            let dream_run_id: i64 = row.get("dream_run_id")?;
            let created_at: String = row.get("created_at")?;
            Ok(admin_row(
                json!(id),
                "dream audit",
                format!("{event_type}: {summary}"),
                &severity,
                format!("dream:{dream_run_id}"),
                String::new(),
                created_at,
                Vec::new(),
            ))
        })?
        .collect()
}

/// `_list_proposals`: the strict concept proposals (Rust keeps only this
/// strict half, per `compatibility/rust/recall-v2.json`).
fn proposal_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select id, concept_text, status, series_slug, canonical_rendering,
                source_language, target_language
         from strict_concept_proposals
         where (?1 is null or series_slug = ?1)
         order by id desc
         limit ?2 offset ?3",
    )?;
    statement
        .query_map(
            rusqlite::params![params.series, params.limit, params.offset],
            |row| {
                let id: i64 = row.get("id")?;
                let concept_text: String = row.get("concept_text")?;
                let status: String = row.get("status")?;
                let series_slug: String = row.get("series_slug")?;
                let canonical_rendering: String = row.get("canonical_rendering")?;
                let source_language: String = row.get("source_language")?;
                let target_language: String = row.get("target_language")?;
                Ok(admin_row(
                    json!(id),
                    "strict concept",
                    concept_text,
                    &status,
                    series_slug,
                    language_pair(&source_language, &target_language),
                    canonical_rendering,
                    Vec::new(),
                ))
            },
        )?
        .collect()
}

/// `_list_audit_log`: the Rust data root always has the `audit_log` table, so
/// this is the `audit_table != "memory_events"` branch.
fn audit_log_rows(connection: &Connection, params: &AdminQuery) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare(
        "select id, action, note, entity_type, entity_id, created_at
         from audit_log
         order by id desc
         limit ?1 offset ?2",
    )?;
    statement
        .query_map(rusqlite::params![params.limit, params.offset], |row| {
            let id: i64 = row.get("id")?;
            let action: String = row.get("action")?;
            let note: String = row.get("note")?;
            let entity_type: String = row.get("entity_type")?;
            let entity_id: String = row.get("entity_id")?;
            let created_at: String = row.get("created_at")?;
            Ok(admin_row(
                json!(id),
                &action,
                note,
                &entity_type,
                entity_id,
                String::new(),
                created_at,
                Vec::new(),
            ))
        })?
        .collect()
}

// ---------------------------------------------------------------------------
// Details
// ---------------------------------------------------------------------------

fn detail(title: String, subtitle: String, body: String, fields: Vec<(&str, String)>) -> Value {
    json!({
        "title": title,
        "subtitle": subtitle,
        "body": body,
        "fields": fields
            .into_iter()
            .map(|(name, value)| json!([name, value]))
            .collect::<Vec<Value>>(),
    })
}

fn missing_detail(what: &str) -> Value {
    detail(
        format!("Missing {what}"),
        String::new(),
        String::new(),
        Vec::new(),
    )
}

fn detail_for_view(
    connection: &Connection,
    view: &str,
    selected: Option<&Value>,
) -> rusqlite::Result<Value> {
    let Some(selected) = selected else {
        return Ok(detail(
            view.to_string(),
            "No rows".to_string(),
            String::new(),
            Vec::new(),
        ));
    };
    let id = selected.get("id").and_then(Value::as_i64);
    match view {
        "Crystals" | "Lessons" => match id {
            Some(id) => crystal_detail(connection, id),
            None => Ok(missing_detail("crystal")),
        },
        "Concepts" => match id {
            Some(id) => concept_detail(connection, id),
            None => Ok(missing_detail("concept")),
        },
        "Renderings" => match id {
            Some(id) => strict_term_detail(connection, id),
            None => Ok(missing_detail("term")),
        },
        "Short-Term Memory" => match id {
            Some(id) => short_term_detail(connection, id),
            None => Ok(missing_detail("short-term memory")),
        },
        "Short-Term Sessions" => match id {
            Some(id) => session_detail(connection, id),
            None => Ok(missing_detail("session")),
        },
        "Dream Runs" => match id {
            Some(id) => dream_run_detail(connection, id),
            None => Ok(missing_detail("dream run")),
        },
        "Dream Audits" => match id {
            Some(id) => dream_audit_detail(connection, id),
            None => Ok(missing_detail("dream audit")),
        },
        "Proposals" => match id {
            Some(id) => proposal_detail(connection, id),
            None => Ok(missing_detail("proposal")),
        },
        "Audit Log" => Ok(row_detail(selected)),
        _ => unreachable!("canonical_view already validated {view}"),
    }
}

/// The Crystals/Lessons detail — ported verbatim from the previous REST
/// handler (`body` is the crystal text only, matching the frozen fixture).
fn crystal_detail(connection: &Connection, crystal_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select title, text, crystal_type, status, series_slug,
                    source_language, target_language, confidence, strength
             from crystals where id = ?1",
            [crystal_id],
            |row| {
                Ok((
                    row.get::<_, String>("title")?,
                    row.get::<_, String>("text")?,
                    row.get::<_, String>("crystal_type")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, String>("series_slug")?,
                    row.get::<_, String>("source_language")?,
                    row.get::<_, String>("target_language")?,
                    row.get::<_, f64>("confidence")?,
                    row.get::<_, f64>("strength")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((title, text, kind, status, series_slug, source, target, confidence, strength)) = row
    else {
        return Ok(missing_detail("crystal"));
    };
    let label = if title.is_empty() {
        excerpt(&text)
    } else {
        title
    };
    Ok(detail(
        label,
        format!("{kind} / {status}"),
        text,
        vec![
            ("Series", series_slug),
            ("Language", language_pair(&source, &target)),
            ("Quality", quality_label(confidence, strength)),
        ],
    ))
}

fn concept_detail(connection: &Connection, concept_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select canonical_name, description, status, confidence, scope_type, scope_key
             from concepts where id = ?1",
            [concept_id],
            |row| {
                Ok((
                    row.get::<_, String>("canonical_name")?,
                    row.get::<_, String>("description")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, f64>("confidence")?,
                    row.get::<_, String>("scope_type")?,
                    row.get::<_, String>("scope_key")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((name, description, status, confidence, scope_type, scope_key)) = row else {
        return Ok(missing_detail("concept"));
    };
    let mut facet_statement = connection.prepare(
        "select facet_type, value,
                coalesce((select group_concat(semantic_tag, ', ') from
                    (select semantic_tag from concept_facet_semantic_tags
                     where facet_id = f.id order by semantic_tag)), '') as evidence_tags
         from concept_facets f
         where concept_id = ?1 and superseded_at is null order by id",
    )?;
    let facets: Vec<(String, String, String)> = facet_statement
        .query_map([concept_id], |row| {
            Ok((
                row.get::<_, String>("facet_type")?,
                row.get::<_, String>("value")?,
                row.get::<_, String>("evidence_tags")?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    // Python `_concept_detail` prefixes each line with the facet's language
    // tags (`kind [ja,en]: value`); this non-frozen view drops the bracket
    // and shows `facet_type: value`.
    let facet_lines: Vec<String> = facets
        .iter()
        .map(|(facet_type, value, tags)| {
            if tags.is_empty() {
                format!("{facet_type}: {value}")
            } else {
                format!("{facet_type} [{tags}]: {value}")
            }
        })
        .collect();
    let body = if facet_lines.is_empty() {
        description.clone()
    } else {
        facet_lines.join("\n")
    };
    let scope = if scope_key.is_empty() {
        scope_type.clone()
    } else {
        scope_key
    };
    Ok(detail(
        name,
        format!("{scope_type} / {status}"),
        body,
        vec![
            ("Description", description),
            ("Scope", scope),
            ("Confidence", percent(confidence)),
            ("Facets", facets.len().to_string()),
        ],
    ))
}

fn strict_term_detail(connection: &Connection, term_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select source_text, category, status, notes, canonical_translation,
                    series_slug, source_language, target_language
             from strict_terms where id = ?1",
            [term_id],
            |row| {
                Ok((
                    row.get::<_, String>("source_text")?,
                    row.get::<_, String>("category")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, String>("notes")?,
                    row.get::<_, String>("canonical_translation")?,
                    row.get::<_, String>("series_slug")?,
                    row.get::<_, String>("source_language")?,
                    row.get::<_, String>("target_language")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((source_text, category, status, notes, rendering, series_slug, source, target)) = row
    else {
        return Ok(missing_detail("term"));
    };
    Ok(detail(
        source_text,
        format!("{category} / {status}"),
        notes,
        vec![
            ("Rendering", rendering),
            ("Series", series_slug),
            ("Language", language_pair(&source, &target)),
        ],
    ))
}

fn short_term_detail(connection: &Connection, memory_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select m.text, m.kind, s.status as session_status, m.session_id, m.source_role,
                    m.source_ref, s.series_slug, s.source_language, s.target_language, m.created_at
             from short_term_memories as m
             join task_sessions as s on s.id = m.session_id
             where m.id = ?1 and m.archived_at is null",
            [memory_id],
            |row| {
                Ok((
                    row.get::<_, String>("text")?,
                    row.get::<_, String>("kind")?,
                    row.get::<_, String>("session_status")?,
                    row.get::<_, i64>("session_id")?,
                    row.get::<_, String>("source_role")?,
                    row.get::<_, String>("source_ref")?,
                    row.get::<_, String>("series_slug")?,
                    row.get::<_, String>("source_language")?,
                    row.get::<_, String>("target_language")?,
                    row.get::<_, String>("created_at")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((
        text,
        kind,
        session_status,
        session_id,
        source_role,
        source_ref,
        series_slug,
        source,
        target,
        created_at,
    )) = row
    else {
        return Ok(missing_detail("short-term memory"));
    };
    Ok(detail(
        excerpt(&text),
        format!("{kind} / session {session_status}"),
        text,
        vec![
            ("Session", session_id.to_string()),
            ("Source role", source_role),
            ("Source reference", source_ref),
            ("Series", series_slug),
            ("Language", language_pair(&source, &target)),
            ("Created", created_at),
        ],
    ))
}

fn session_detail(connection: &Connection, session_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select series_slug, volume, chapter, task_type, status, cycle_id,
                    source_language, target_language
             from task_sessions where id = ?1",
            [session_id],
            |row| {
                Ok((
                    row.get::<_, String>("series_slug")?,
                    row.get::<_, String>("volume")?,
                    row.get::<_, String>("chapter")?,
                    row.get::<_, String>("task_type")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, Option<i64>>("cycle_id")?,
                    row.get::<_, String>("source_language")?,
                    row.get::<_, String>("target_language")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((series_slug, volume, chapter, task_type, status, cycle_id, source, target)) = row
    else {
        return Ok(missing_detail("session"));
    };
    Ok(detail(
        session_label(&series_slug, &volume, &chapter),
        format!("{task_type} / {status}"),
        String::new(),
        vec![
            ("Series", series_slug),
            ("Language", language_pair(&source, &target)),
            (
                "Cycle",
                cycle_id.map(|id| id.to_string()).unwrap_or_default(),
            ),
        ],
    ))
}

fn dream_run_detail(connection: &Connection, run_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select cycle_id, provider, status, error, input_count,
                    created_crystal_count, proposal_count
             from dream_runs where id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, i64>("cycle_id")?,
                    row.get::<_, String>("provider")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, String>("error")?,
                    row.get::<_, i64>("input_count")?,
                    row.get::<_, i64>("created_crystal_count")?,
                    row.get::<_, i64>("proposal_count")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((cycle_id, provider, status, error, inputs, crystals, proposals)) = row else {
        return Ok(missing_detail("dream run"));
    };
    Ok(detail(
        format!("Cycle {cycle_id}"),
        format!("{provider} / {status}"),
        error,
        vec![
            ("Inputs", inputs.to_string()),
            ("Crystals", crystals.to_string()),
            ("Proposals", proposals.to_string()),
        ],
    ))
}

fn dream_audit_detail(connection: &Connection, audit_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select event_type, summary, severity, payload_json, dream_run_id,
                    phase_run_id, created_at
             from dream_audit_entries where id = ?1",
            [audit_id],
            |row| {
                Ok((
                    row.get::<_, String>("event_type")?,
                    row.get::<_, String>("summary")?,
                    row.get::<_, String>("severity")?,
                    row.get::<_, String>("payload_json")?,
                    row.get::<_, i64>("dream_run_id")?,
                    row.get::<_, Option<i64>>("phase_run_id")?,
                    row.get::<_, String>("created_at")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((event_type, summary, severity, payload_json, dream_run_id, phase_run_id, created_at)) =
        row
    else {
        return Ok(missing_detail("dream audit"));
    };
    // `serde_json` is built without `preserve_order` in this workspace, so its
    // `Map` is a `BTreeMap` and `to_string_pretty` emits keys sorted — the
    // same shape as Python's `json.dumps(payload, indent=2, sort_keys=True)`.
    let body = match serde_json::from_str::<Value>(&payload_json) {
        Ok(payload) => serde_json::to_string_pretty(&payload).unwrap_or(payload_json),
        Err(_) => json!({ "_invalid_json": payload_json }).to_string(),
    };
    Ok(detail(
        format!("{event_type}: {summary}"),
        severity.clone(),
        body,
        vec![
            ("Dream run", dream_run_id.to_string()),
            (
                "Phase run",
                phase_run_id.map(|id| id.to_string()).unwrap_or_default(),
            ),
            ("Severity", severity),
            ("Created", created_at),
        ],
    ))
}

fn proposal_detail(connection: &Connection, proposal_id: i64) -> rusqlite::Result<Value> {
    let row = connection
        .query_row(
            "select concept_text, status, rationale, source_form, canonical_rendering,
                    series_slug, source_language, target_language,
                    approved_variants_json, forbidden_variants_json
             from strict_concept_proposals where id = ?1",
            [proposal_id],
            |row| {
                Ok((
                    row.get::<_, String>("concept_text")?,
                    row.get::<_, String>("status")?,
                    row.get::<_, String>("rationale")?,
                    row.get::<_, String>("source_form")?,
                    row.get::<_, String>("canonical_rendering")?,
                    row.get::<_, String>("series_slug")?,
                    row.get::<_, String>("source_language")?,
                    row.get::<_, String>("target_language")?,
                    row.get::<_, String>("approved_variants_json")?,
                    row.get::<_, String>("forbidden_variants_json")?,
                ))
            },
        )
        .map(Some)
        .or_else(no_rows)?;
    let Some((
        concept_text,
        status,
        rationale,
        source_form,
        rendering,
        series_slug,
        source,
        target,
        approved_variants,
        forbidden_variants,
    )) = row
    else {
        return Ok(missing_detail("proposal"));
    };
    // Keep malformed legacy evidence visible rather than silently dropping it.
    let variants = |raw: String| {
        serde_json::from_str::<Vec<String>>(&raw)
            .map(|items| items.join("\n"))
            .unwrap_or(raw)
    };
    Ok(detail(
        concept_text,
        format!("strict concept / {status}"),
        rationale,
        vec![
            ("Source form", source_form),
            ("Rendering", rendering),
            ("Approved variants", variants(approved_variants)),
            ("Forbidden variants", variants(forbidden_variants)),
            ("Series", series_slug),
            ("Language", language_pair(&source, &target)),
        ],
    ))
}

/// `_detail_for_view`'s Audit Log branch: the row projected into a detail.
fn row_detail(row: &Value) -> Value {
    let string = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let label = string("label");
    let title = if label.is_empty() {
        string("kind")
    } else {
        label
    };
    detail(
        title,
        string("status"),
        string("quality_label"),
        vec![
            ("Kind", string("kind")),
            ("Scope", string("scope")),
            ("Language", string("language_pair")),
            ("Quality", string("quality_label")),
        ],
    )
}

// ---------------------------------------------------------------------------
// Helpers (ported from admin.py)
// ---------------------------------------------------------------------------

fn no_rows<T>(error: rusqlite::Error) -> rusqlite::Result<Option<T>> {
    match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    }
}

fn language_pair(source: &str, target: &str) -> String {
    format!("{source} -> {target}")
}

fn session_label(series_slug: &str, volume: &str, chapter: &str) -> String {
    let mut parts = vec![series_slug.to_string()];
    if !volume.is_empty() {
        parts.push(format!("v{volume}"));
    }
    if !chapter.is_empty() {
        parts.push(format!("ch{chapter}"));
    }
    parts.join(" / ")
}

fn quality_label(confidence: f64, strength: f64) -> String {
    format!("{} conf / {} str", percent(confidence), percent(strength))
}

/// Python `f"{round(float(value) * 100):.0f}%"` (round-half-even, which
/// `{:.0}` reproduces over the binary value).
fn percent(value: f64) -> String {
    format!("{:.0}%", value * 100.0)
}

pub(super) fn excerpt(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const LIMIT: usize = 80;
    if normalized.chars().count() <= LIMIT {
        return normalized;
    }
    let cut: String = normalized.chars().take(LIMIT - 1).collect();
    format!("{cut}...")
}
