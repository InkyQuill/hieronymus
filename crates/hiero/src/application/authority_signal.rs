//! Durable unresolved ingress: no invented domain operation or separate queue.
use super::{AppError, authority::UserCorrectionV1, domain};
use crate::trusted_ingress::{Principal, hash, new_id};
use hieronymus::{
    authority_models::{DecisionErrorV1, TentativeReason},
    consolidation::UnresolvedSignalV1,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

fn identity(principal: &Principal) -> Result<(&str, &str), AppError> {
    match principal {
        Principal::Console(id) => Ok(("console_user", id)),
        Principal::HostEvent => Ok(("host_user_event", "local-host-event")),
        Principal::Agent => Err(AppError::Authority(DecisionErrorV1::UnverifiedOrigin)),
    }
}
/// Check immutable signal identity before re-resolution or revision checks.
pub(super) fn replay(
    db: &Connection,
    principal: &Principal,
    input: &UserCorrectionV1,
) -> Result<Option<Value>, AppError> {
    let (kind, identity) = identity(principal)?;
    let existing:Option<(String,String,String,String)>=db.query_row("select d.canonical_request,d.result_json,o.kind,o.principal from decision_records d join origin_receipts o on o.id=d.origin_id where d.decision_id=?",[&input.decision_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(domain)?;
    if let Some((canonical, result, stored_kind, stored_principal)) = existing {
        let value: Value = serde_json::from_str(&canonical).map_err(domain)?;
        if value["kind"] == "unresolved_signal" {
            let signal: UnresolvedSignalV1 = serde_json::from_value(value).map_err(domain)?;
            if stored_kind != kind
                || stored_principal != identity
                || signal.context != serde_json::to_value(input).map_err(domain)?
            {
                return Err(AppError::Authority(DecisionErrorV1::IdempotencyConflict));
            }
            return Ok(Some(serde_json::from_str(&result).map_err(domain)?));
        }
        // Resolved decisions retain their existing exact receipt replay path.
    }
    Ok(None)
}
pub(super) fn persist(
    db: &mut Connection,
    principal: &Principal,
    input: &UserCorrectionV1,
    text: &str,
    reason: TentativeReason,
    detail: &str,
) -> Result<Value, AppError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(domain)?;
    if let Some(result) = replay(&tx, principal, input)? {
        tx.commit().map_err(domain)?;
        return Ok(result);
    }
    let (kind, identity) = identity(principal)?;
    let occupied:bool=tx.query_row("select exists(select 1 from decision_records where decision_id=?1) or exists(select 1 from origin_receipts where kind=?2 and principal=?3 and event_id=?4)",params![input.decision_id,kind,identity,input.event_id],|r|r.get(0)).map_err(domain)?;
    if occupied {
        return Err(AppError::Authority(DecisionErrorV1::IdempotencyConflict));
    }
    let revision: i64 = tx
        .query_row(
            "select revision from authority_state where series_id=?",
            [input.series_id],
            |r| r.get(0),
        )
        .map_err(domain)?;
    if revision as u64 != input.expected_revision {
        return Err(AppError::Authority(DecisionErrorV1::RevisionConflict {
            current_revision: revision as u64,
        }));
    }
    let context = serde_json::to_value(input).map_err(domain)?;
    let context_text = serde_json::to_string(&context).map_err(domain)?;
    let id = new_id()?;
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute("insert into origin_receipts(id,kind,principal,session_id,event_id,text,context_json,content_hash,created_at) values(?,?,?,?,?,?,?,?,?)",params![id,kind,identity,input.session_id,input.event_id,text,context_text,hash(&format!("{text}\n{context_text}")),now]).map_err(domain)?;
    let signal = UnresolvedSignalV1 {
        version: 1,
        kind: hieronymus::consolidation::UnresolvedSignalKind::UnresolvedSignal,
        decision_id: input.decision_id.clone(),
        origin: hieronymus::authority_models::OriginReceiptId(id.clone()),
        text: text.into(),
        context,
        reasons: vec![reason],
        detail: detail.into(),
    };
    let canonical = serde_json::to_string(&signal).map_err(domain)?;
    if canonical.len() > 256 * 1024 {
        return Err(AppError::Authority(DecisionErrorV1::InvalidRequest));
    }
    let result = json!({"status":"tentative","decision_id":input.decision_id,"origin_receipt":id,"reasons":signal.reasons,"detail":detail,"authority_changed":false,"resulting_revision":revision+1,"consolidation_job_id":input.decision_id});
    tx.execute("insert into decision_records(decision_id,series_id,origin_id,actor_kind,expected_revision,resulting_revision,canonical_request,result_json,status,created_at) values(?,?,?,'explicit_user',?,?,?,?,'tentative',?)",params![input.decision_id,input.series_id,id,revision,revision+1,canonical,result.to_string(),now]).map_err(domain)?;
    tx.execute(
        "update authority_state set revision=revision+1 where series_id=?",
        [input.series_id],
    )
    .map_err(domain)?;
    tx.execute("insert or ignore into provider_recovery_state(provider_slot_id,config_fingerprint,next_recovery_at,updated_at) values('default','unconfigured',?1,?1)",[&now]).map_err(domain)?;
    tx.execute("insert into consolidation_jobs(decision_id,state,attempts,provider_slot_id,created_at,updated_at) values(?1,'pending',0,'default',?2,?2)",params![input.decision_id,now]).map_err(domain)?;
    tx.commit().map_err(domain)?;
    Ok(result)
}
