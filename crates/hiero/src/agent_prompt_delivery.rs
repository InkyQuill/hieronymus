//! Local trusted-shell binding and durable UserPromptSubmit delivery. Prompt text
//! is accepted only from the host stdin envelope, never the binding command.
use crate::{
    application::authority::{SelectedRevision, UserCorrectionV1},
    client::ClientError,
    daemon::discovery::LocalCredential,
    lifecycle,
};
use hieronymus::{
    authority::EvidenceBindingV1, authority_models::EvidenceRef, data_root::HieronymusConfig,
    story_applicability::ApplicabilityV1,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("invalid hook input or saved context: {0}")]
    Invalid(String),
    #[error("local hook storage: {0}")]
    Io(#[from] std::io::Error),
    #[error("hook context database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(
        "delivery {delivery_id} was saved but not acknowledged; retry-delivery with this ID: {detail}"
    )]
    Pending { delivery_id: String, detail: String },
    #[error("delivery {delivery_id} was rejected: HTTP 409 {detail}")]
    Rejected { delivery_id: String, detail: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostContext {
    version: u8,
    host: String,
    host_session_id: String,
    series_id: i64,
    session_id: i64,
    expected_revision: u64,
    source_language: Option<String>,
    target_language: Option<String>,
    applicability: Option<ApplicabilityV1>,
    #[serde(default)]
    selected_sources: Vec<EvidenceRef>,
    #[serde(default)]
    selected_claims: Vec<SelectedRevision>,
    selected_rule: Option<SelectedRevision>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Delivery {
    version: u8,
    host: String,
    host_session_id: String,
    request: UserCorrectionV1,
    response: Option<Value>,
}
fn invalid(detail: impl Into<String>) -> DeliveryError {
    DeliveryError::Invalid(detail.into())
}
fn decode<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, DeliveryError> {
    serde_json::from_value(v.clone()).map_err(|e| invalid(e.to_string()))
}
fn host(value: &str) -> Result<(), DeliveryError> {
    if matches!(value, "claude" | "codex" | "zcode") {
        Ok(())
    } else {
        Err(invalid("unsupported host"))
    }
}

fn context_path(config: &HieronymusConfig, h: &str, session: &str) -> PathBuf {
    config.config_root().join("host-contexts").join(format!(
        "{:x}.json",
        Sha256::digest(format!("{h}\n{session}"))
    ))
}
fn delivery_path(config: &HieronymusConfig, id: &str) -> Result<PathBuf, DeliveryError> {
    if !crate::trusted_ingress::valid_uuid(id) {
        return Err(invalid("invalid delivery ID"));
    }
    Ok(config
        .config_root()
        .join("host-deliveries")
        .join(format!("{id}.json")))
}
fn save(path: &std::path::Path, value: &impl Serialize) -> Result<(), DeliveryError> {
    hieronymus::private_file::replace_private(
        path,
        &serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))?,
    )?;
    Ok(())
}
fn load<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T, DeliveryError> {
    let data = read_json(std::fs::File::open(path)?)?;
    decode(&data)
}
/// Bound stdin; failed/oversized decoding never prints original prompt contents.
pub fn read_json(reader: impl Read) -> Result<Value, DeliveryError> {
    let mut bytes = Vec::new();
    reader.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err(invalid("stdin exceeds 1 MiB"));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("expected one JSON object on stdin"))
}
fn uuid() -> Result<String, DeliveryError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| invalid("randomness unavailable"))?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let h = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}
/// Installed binding command. Stores a read-validated snapshot, mints no origin,
/// and never updates stale revisions to make a future correction pass.
pub fn bind_context(config: &HieronymusConfig, input: &Value) -> Result<Value, DeliveryError> {
    let c: HostContext = decode(input)?;
    host(&c.host)?;
    if c.version != 1
        || c.host_session_id.is_empty()
        || c.host_session_id.len() > 512
        || c.series_id <= 0
        || c.session_id <= 0
        || c.expected_revision >= i64::MAX as u64
        || c.selected_sources.len() > 100
        || c.selected_claims.len() > 100
    {
        return Err(invalid("invalid binding identity or bounds"));
    }
    let mut db = rusqlite::Connection::open_with_flags(
        config.database_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let tx = db.transaction()?;
    let valid:bool=tx.query_row("select exists(select 1 from task_sessions t join series s on s.slug=t.series_slug join authority_state a on a.series_id=s.id where t.id=?1 and s.id=?2 and a.revision=?3 and (?4 is null or t.source_language=?4) and (?5 is null or t.target_language=?5))",params![c.session_id,c.series_id,c.expected_revision as i64,c.source_language,c.target_language],|r|r.get(0))?;
    if !valid {
        return Err(invalid(
            "session, language, series or observed revision mismatch",
        ));
    }
    if c.applicability
        .as_ref()
        .is_some_and(|a| a.series_id != c.series_id)
    {
        return Err(invalid("applicability series mismatch"));
    }
    for reference in &c.selected_sources {
        let (content,binding):(String,String)=tx.query_row("select content,binding_json from evidence_records where id=?1 and series_id=?2 and kind='source_passage' and source_hash=?3 and span_start=?4 and span_end=?5",params![reference.id,c.series_id,reference.content_hash,reference.span_start as i64,reference.span_end as i64],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let binding: EvidenceBindingV1 =
            serde_json::from_str(&binding).map_err(|_| invalid("corrupt source binding"))?;
        if reference.kind != hieronymus::authority_models::EvidenceKind::SourcePassage
            || format!("{:x}", Sha256::digest(content.as_bytes())) != reference.content_hash
            || content
                .get(reference.span_start..reference.span_end)
                .is_none()
            || c.applicability.as_ref() != Some(&binding.applicability)
            || c.source_language.as_deref() != Some(&binding.source_language)
            || c.target_language != binding.target_language
        {
            return Err(invalid("source selection binding mismatch"));
        }
    }
    for claim in &c.selected_claims {
        let valid:bool=tx.query_row("select exists(select 1 from memory_claims where id=?1 and series_id=?2 and revision=?3)",params![claim.id,c.series_id,claim.revision as i64],|r|r.get(0))?;
        if !valid {
            return Err(invalid("claim selection mismatch"));
        }
    }
    if let Some(rule) = &c.selected_rule {
        let valid:bool=tx.query_row("select exists(select 1 from term_rules r join concepts c on c.id=r.concept_id join series s on s.id=?3 where r.id=?1 and r.revision=?2 and (c.scope_type='global' or c.scope_key='series:'||s.slug))",params![rule.id,rule.revision as i64,c.series_id],|r|r.get(0))?;
        if !valid {
            return Err(invalid("rule selection mismatch"));
        }
    }
    tx.commit()?;
    save(&context_path(config, &c.host, &c.host_session_id), &c)?;
    Ok(
        json!({"bound":true,"host":c.host,"host_session_id":c.host_session_id,"session_id":c.session_id,"expected_revision":c.expected_revision,"authority_changed":false}),
    )
}
fn prompt_fields<'a>(h: &str, input: &'a Value) -> Result<(&'a str, &'a str), DeliveryError> {
    host(h)?;
    if input["hook_event_name"] != "UserPromptSubmit" {
        return Err(invalid("expected UserPromptSubmit event"));
    }
    let session = input["session_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .ok_or_else(|| invalid("missing host session"))?;
    let text = input["prompt"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65536)
        .ok_or_else(|| invalid("missing or oversized host prompt"))?;
    Ok((session, text))
}
/// First-session discovery is read-only: it exposes actual host identity but
/// cannot retain or apply an event without an explicitly bound domain context.
pub fn handle_prompt(
    config: &HieronymusConfig,
    h: &str,
    input: &Value,
) -> Result<Value, DeliveryError> {
    let (session, _) = prompt_fields(h, input)?;
    if !context_path(config, h, session).try_exists()? {
        return Ok(
            json!({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":format!(
                "Hieronymus bootstrap: {}. This prompt was NOT applied or retained as a correction. Use actual MCP session, immutable evidence/claim selection, bound languages/applicability and observed authority revision to construct the version:1 context for hiero agent-hook bind-context on stdin. Do not guess IDs, replay this prompt, or add prompt/actor fields to binding. A subsequent genuine UserPromptSubmit can apply after binding. No authority was minted.",
                json!({"status":"binding_required","host":h,"host_session_id":session,"authority_changed":false})
            )}}),
        );
    }
    match submit_prompt(config, h, input) {
        Ok(response) => Ok(hook_output(&response)),
        // A definitive conflict must not prevent the model from reading and
        // binding context for a future event. The rejected delivery is never
        // rebased, acknowledged, or turned into an applied dependency here.
        Err(DeliveryError::Rejected {
            delivery_id,
            detail,
        }) => Ok(json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": format!(
                    "Hieronymus delivery recovery: {}. The current user operation was NOT applied. Its saved text, identity, selection and revisions remain unchanged. Read current public context and explicitly bind a genuinely future event. Do not rebase or automatically resubmit this rejected operation, invent a receipt, or treat dependent work as validated. Tell the user that the correction was rejected; unrelated work may continue. Retrying the saved delivery ID uses only its original immutable context.",
                    json!({"status":"delivery_rejected","delivery_id":delivery_id,"http_status":409,"detail":detail.chars().take(512).collect::<String>(),"authority_changed":false})
                )
            }
        })),
        Err(error) => Err(error),
    }
}

/// One actual host invocation means one new durable ID, even for identical
/// prompt text or a reused Codex turn_id. Retry uses the separate saved-ID path.
pub fn submit_prompt(
    config: &HieronymusConfig,
    h: &str,
    input: &Value,
) -> Result<Value, DeliveryError> {
    let (session, text) = prompt_fields(h, input)?;
    let c: HostContext = load(&context_path(config, h, session))?;
    if c.version != 1 || c.host != h || c.host_session_id != session {
        return Err(invalid("saved host/session mismatch"));
    }
    let id = uuid()?;
    let request = UserCorrectionV1 {
        version: 1,
        decision_id: id.clone(),
        event_id: format!("{h}:{session}:{id}"),
        expected_revision: c.expected_revision,
        series_id: c.series_id,
        session_id: Some(c.session_id),
        source_language: c.source_language,
        target_language: c.target_language,
        applicability: c.applicability,
        selected_sources: c.selected_sources,
        selected_claims: c.selected_claims,
        selected_rule: c.selected_rule,
        text: Some(text.into()),
        structured: None,
    };
    let delivery = Delivery {
        version: 1,
        host: h.into(),
        host_session_id: session.into(),
        request,
        response: None,
    };
    save(&delivery_path(config, &id)?, &delivery)?;
    retry_delivery(config, &id)
}
/// Returns the exact stored response after an acknowledged delivery. An
/// uncertain transport outcome resends only the original request identity.
pub fn retry_delivery(config: &HieronymusConfig, id: &str) -> Result<Value, DeliveryError> {
    let path = delivery_path(config, id)?;
    let mut d: Delivery = load(&path)?;
    if d.version != 1
        || d.request.decision_id != id
        || d.request.event_id != format!("{}:{}:{id}", d.host, d.host_session_id)
    {
        return Err(invalid("corrupt delivery identity"));
    }
    if let Some(response) = d.response {
        return Ok(response);
    }
    let client = lifecycle::connect(config, false)
        .and_then(|c| c.with_local_credential(config, LocalCredential::HostEvent))
        .map_err(|e| DeliveryError::Pending {
            delivery_id: id.into(),
            detail: e.to_string(),
        })?;
    let result = client
        .post(
            "/authority/host-event",
            &serde_json::to_value(&d.request).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| match e {
            ClientError::Status {
                status: 409,
                detail,
            } => DeliveryError::Rejected {
                delivery_id: id.into(),
                detail,
            },
            other => DeliveryError::Pending {
                delivery_id: id.into(),
                detail: other.to_string(),
            },
        })?;
    let receipt = result
        .get("Applied")
        .or_else(|| result.get("Replayed"))
        .and_then(|v| v.get("receipt"));
    let response = json!({"delivery_id":id,"required_decision_id":receipt.map(|r|r["decision_id"].clone()),"result":result});
    d.response = Some(response.clone());
    save(&path, &d)?;
    Ok(response)
}
/// Common Claude/Codex UserPromptSubmit output; native installation is Task7.
pub fn hook_output(response: &Value) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":format!("Hieronymus trusted correction result: {}. If required_decision_id is present, pass it to dependent validation/read operations. Do not redeem this receipt through hieronymus_correct.",response)}})
}

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn saved_prompt_records_remain_private_after_replacement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("deliveries").join("record.json");
        for value in [
            json!({"text": "private prompt"}),
            json!({"response": "acknowledged"}),
        ] {
            save(&path, &value).unwrap();
            let bytes = hieronymus::private_file::read_private(&path).unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), value);
            assert_eq!(load::<Value>(&path).unwrap(), value);
        }
    }

    #[test]
    fn passive_pi_is_not_a_trusted_delivery_host() {
        assert!(host("claude").is_ok());
        assert!(host("codex").is_ok());
        assert!(host("zcode").is_ok());
        assert!(host("pi").is_err());
    }
}
