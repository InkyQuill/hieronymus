//! Settings routes (`/api/settings/{dream,ingest,release}`): typed round
//! trips over the slice-1 config files. Success envelopes carry the full
//! saved payload; invalid drafts are `400 {"error": ...}`.

use serde_json::{Value, json};

use hieronymus::data_root::HieronymusConfig;
use hieronymus::dream_config::{
    DREAMING_FIELDS, DreamConfig, default_dream_config, load_dream_config,
    redacted_dream_config_payload, save_dream_config, validate_dream_config,
};
use hieronymus::ingest_config::{
    IngestConfig, load_ingest_config, save_ingest_config, validate_ingest_config,
};
use hieronymus::provider_config::load_provider_catalog;
use hieronymus::release_config::{
    ReleaseConfig, load_release_config, save_release_config, validate_release_config,
};

use super::super::DaemonRuntime;
use super::super::http::{Request, Response};
use super::request_body;

/// Which settings file a route addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Dream,
    Ingest,
    Release,
}

/// `GET /api/settings/{kind}`.
pub(super) fn get(_request: &Request, runtime: &DaemonRuntime, kind: Kind) -> Response {
    let config = &runtime.config;
    match kind {
        Kind::Dream => {
            let (dream_config, dream_error) = load_dream(config);
            let (catalog, provider_error) = catalog(config);
            Response::json(
                200,
                &json!({
                    "dream": table_value(&redacted_dream_config_payload(&dream_config)),
                    "providers": providers_payload(&catalog),
                    "model_cache": model_cache_payload(config),
                    "error": first_error(&[dream_error, provider_error]),
                }),
            )
        }
        Kind::Ingest => {
            let (ingest_config, error) = load_ingest(config);
            Response::json(
                200,
                &json!({"ingest": ingest_payload(&ingest_config), "error": error}),
            )
        }
        Kind::Release => {
            let (release_config, error) = load_release(config);
            Response::json(
                200,
                &json!({"release": release_payload(&release_config), "error": error}),
            )
        }
    }
}

/// `POST /api/settings/{kind}`.
pub(super) fn save(request: &Request, runtime: &DaemonRuntime, kind: Kind) -> Response {
    let Some(body) = request_body(request) else {
        return envelope_400(match kind {
            Kind::Dream => "dream must be an object",
            Kind::Ingest => "ingest must be an object",
            Kind::Release => "release must be an object",
        });
    };
    match kind {
        Kind::Dream => save_dream(request, runtime, &body),
        Kind::Ingest => save_ingest(request, runtime, &body),
        Kind::Release => save_release(request, runtime, &body),
    }
}

fn save_dream(_request: &Request, runtime: &DaemonRuntime, body: &Value) -> Response {
    let Some(draft) = body.get("dream").cloned().filter(Value::is_object) else {
        return envelope_400("dream must be an object");
    };
    let (dream_config, load_error) = load_dream(&runtime.config);
    if !load_error.is_empty() {
        return envelope_400(&load_error);
    }
    let dream_config = apply_dream_draft(dream_config, &draft);
    let dream_config = match validate_dream_config(&dream_config) {
        Ok(dream_config) => dream_config,
        Err(error) => return envelope_400(&error.to_string()),
    };
    if let Err(error) = save_dream_config(&runtime.config, &dream_config) {
        return envelope_400(&error.to_string());
    }
    Response::json(
        200,
        &json!({
            "dream": table_value(&redacted_dream_config_payload(&dream_config)),
            "error": "",
        }),
    )
}

fn save_ingest(_request: &Request, runtime: &DaemonRuntime, body: &Value) -> Response {
    let Some(draft) = body.get("ingest").cloned().filter(Value::is_object) else {
        return envelope_400("ingest must be an object");
    };
    let (ingest_config, load_error) = load_ingest(&runtime.config);
    if !load_error.is_empty() {
        return envelope_400(&load_error);
    }
    let ingest_config = apply_ingest_draft(ingest_config, &draft);
    let ingest_config = match validate_ingest_config(&ingest_config) {
        Ok(ingest_config) => ingest_config,
        Err(error) => return envelope_400(&error.to_string()),
    };
    if let Err(error) = save_ingest_config(&runtime.config, &ingest_config) {
        return envelope_400(&error.to_string());
    }
    Response::json(
        200,
        &json!({"ingest": ingest_payload(&ingest_config), "error": ""}),
    )
}

fn save_release(_request: &Request, runtime: &DaemonRuntime, body: &Value) -> Response {
    let Some(draft) = body.get("release").cloned().filter(Value::is_object) else {
        return envelope_400("release must be an object");
    };
    let (release_config, load_error) = load_release(&runtime.config);
    if !load_error.is_empty() {
        return envelope_400(&load_error);
    }
    let release_config = release_config.with_update_channel(
        draft
            .get("update_channel")
            .and_then(Value::as_str)
            .unwrap_or(release_config.update_channel()),
    );
    let release_config = match validate_release_config(&release_config) {
        Ok(release_config) => release_config,
        Err(error) => return envelope_400(&error.to_string()),
    };
    if let Err(error) = save_release_config(&runtime.config, &release_config) {
        return envelope_400(&error.to_string());
    }
    Response::json(
        200,
        &json!({"release": release_payload(&release_config), "error": ""}),
    )
}

/// Apply a `{"dreaming": {...}, "workflows": {...}}` draft over the loaded
/// config (port of `_dream_config_from_draft`).
fn apply_dream_draft(base: DreamConfig, draft: &Value) -> DreamConfig {
    let mut next = base;
    if let Some(dreaming) = draft.get("dreaming").filter(|value| value.is_object()) {
        for field in DREAMING_FIELDS {
            let Some(value) = dreaming.get(field) else {
                continue;
            };
            match field {
                "enabled" => {
                    if let Some(enabled) = value.as_bool() {
                        next.enabled = enabled;
                    }
                }
                "general_prompt" => {
                    if let Some(prompt) = value.as_str() {
                        next.general_prompt = prompt.to_string();
                    }
                }
                other => {
                    if let Some(number) = value.as_i64() {
                        set_numeric_field(&mut next, other, number);
                    }
                }
            }
        }
    }
    if let Some(workflows) = draft.get("workflows").filter(|value| value.is_object()) {
        let Some(workflow_drafts) = workflows.as_object() else {
            return next;
        };
        for (name, raw_workflow) in workflow_drafts {
            let Some(raw_workflow) = raw_workflow.as_object() else {
                continue;
            };
            let mut workflow = next.workflows.get(name).cloned().unwrap_or_default();
            if let Some(provider) = raw_workflow.get("provider").and_then(Value::as_str) {
                workflow.provider = provider.to_string();
            }
            if let Some(model) = raw_workflow.get("model").and_then(Value::as_str) {
                workflow.model = model.to_string();
            }
            if let Some(enabled) = raw_workflow.get("enabled").and_then(Value::as_bool) {
                workflow.enabled = enabled;
            }
            if let Some(max_records) = raw_workflow
                .get("max_records_per_pass")
                .and_then(Value::as_i64)
            {
                workflow.max_records_per_pass = max_records;
            }
            next = next.with_workflow(name, workflow);
        }
    }
    next
}

fn set_numeric_field(config: &mut DreamConfig, field: &str, value: i64) {
    match field {
        "schedule_interval_minutes" => config.schedule_interval_minutes = value,
        "min_pending_short_term_memories" => config.min_pending_short_term_memories = value,
        "max_pending_short_term_memories" => config.max_pending_short_term_memories = value,
        "max_short_term_memories_per_cycle" => config.max_short_term_memories_per_cycle = value,
        "not_enough_memories_cycle_threshold" => {
            config.not_enough_memories_cycle_threshold = value;
        }
        "max_changed_crystals_per_cycle" => config.max_changed_crystals_per_cycle = value,
        "max_related_concepts_per_cycle" => config.max_related_concepts_per_cycle = value,
        "max_related_crystals_per_concept" => config.max_related_crystals_per_concept = value,
        "max_total_affected_crystals" => config.max_total_affected_crystals = value,
        "max_short_term_memories_per_run" => config.max_short_term_memories_per_run = value,
        "max_long_term_records_affected_per_run" => {
            config.max_long_term_records_affected_per_run = value;
        }
        "max_relation_records_per_pass" => config.max_relation_records_per_pass = value,
        _ => {}
    }
}

/// Apply an `{"ingest": {"short_memory": {...}, "learn": {...}}}` draft (port
/// of `_ingest_config_from_draft`, nested and flat keys included).
fn apply_ingest_draft(base: IngestConfig, draft: &Value) -> IngestConfig {
    let mut short_memory = base.short_memory;
    let mut learn = base.learn;
    if let Some(section) = draft.get("short_memory").filter(|value| value.is_object()) {
        if let Some(value) = section
            .get("warning_sentence_count")
            .and_then(Value::as_i64)
        {
            short_memory.warning_sentence_count = value;
        }
        if let Some(value) = section
            .get("rejection_sentence_count")
            .and_then(Value::as_i64)
        {
            short_memory.rejection_sentence_count = value;
        }
        if let Some(value) = section.get("warning_symbol_count").and_then(Value::as_i64) {
            short_memory.warning_symbol_count = value;
        }
        if let Some(value) = section
            .get("rejection_symbol_count")
            .and_then(Value::as_i64)
        {
            short_memory.rejection_symbol_count = value;
        }
    }
    if let Some(value) = draft
        .get("max_block_chars")
        .or_else(|| {
            draft
                .get("learn")
                .and_then(|learn| learn.get("max_block_chars"))
        })
        .and_then(Value::as_i64)
    {
        learn.max_block_chars = value;
    }
    IngestConfig::new(short_memory, learn)
}

fn ingest_payload(ingest_config: &IngestConfig) -> Value {
    json!({
        "learn": {"max_block_chars": ingest_config.learn.max_block_chars},
        "short_memory": {
            "rejection_sentence_count": ingest_config.short_memory.rejection_sentence_count,
            "rejection_symbol_count": ingest_config.short_memory.rejection_symbol_count,
            "warning_sentence_count": ingest_config.short_memory.warning_sentence_count,
            "warning_symbol_count": ingest_config.short_memory.warning_symbol_count,
        },
    })
}

fn release_payload(release_config: &ReleaseConfig) -> Value {
    json!({"update_channel": release_config.update_channel()})
}

fn providers_payload(catalog: &hieronymus::provider_config::ProviderCatalog) -> Vec<Value> {
    catalog
        .providers
        .keys()
        .map(|id| super::providers::editor_payload(catalog, id))
        .collect()
}

/// `llmcache.tmp` content as `{"providers": {...}}`; an absent or unreadable
/// cache is the empty cache (nothing in this slice writes it).
pub(super) fn model_cache_payload(config: &HieronymusConfig) -> Value {
    let providers = std::fs::read_to_string(config.llm_cache_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|payload| payload.get("providers").cloned())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    json!({"providers": providers})
}

fn table_value<T: serde::ser::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or_else(|_| json!({}))
}

fn first_error(errors: &[String]) -> String {
    errors
        .iter()
        .find(|error| !error.is_empty())
        .cloned()
        .unwrap_or_default()
}

fn load_dream(config: &HieronymusConfig) -> (DreamConfig, String) {
    match load_dream_config(config) {
        Ok(dream_config) => (dream_config, String::new()),
        Err(error) => (default_dream_config(), error.to_string()),
    }
}

fn load_ingest(config: &HieronymusConfig) -> (IngestConfig, String) {
    match load_ingest_config(config) {
        Ok(ingest_config) => (ingest_config, String::new()),
        Err(error) => (
            hieronymus::ingest_config::default_ingest_config(),
            error.to_string(),
        ),
    }
}

fn load_release(config: &HieronymusConfig) -> (ReleaseConfig, String) {
    match load_release_config(config) {
        Ok(release_config) => (release_config, String::new()),
        Err(error) => (
            hieronymus::release_config::default_release_config(),
            error.to_string(),
        ),
    }
}

fn catalog(config: &HieronymusConfig) -> (hieronymus::provider_config::ProviderCatalog, String) {
    match load_provider_catalog(config) {
        Ok(catalog) => (catalog, String::new()),
        Err(error) => (
            hieronymus::provider_config::default_provider_catalog(),
            error.to_string(),
        ),
    }
}

fn envelope_400(error: &str) -> Response {
    Response::json(400, &json!({"error": error}))
}
