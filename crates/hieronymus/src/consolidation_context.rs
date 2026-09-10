//! Trusted bounded selection and origin minting for correction provider drafts.
use crate::{
    authority_models::{EvidenceKind, EvidenceRef, OriginReceiptId},
    consolidation::*,
    dream_config::DreamConfig,
    dream_output::DecisionsDraftV1,
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_CONTEXT_BYTES: usize = MAX_CORRECTION_CONTEXT_BYTES;
const MAX_RECORD_BYTES: usize = 16 * 1024;

/// Only this snapshot may provide identifiers, revisions or evidence to a result.
pub struct CorrectionSelection {
    evidence: Vec<EvidenceRef>,
    claims: Vec<SelectedClaimV1>,
    rules: BTreeMap<i64, (u64, i64, String, String)>,
    projection: Value,
}
impl CorrectionSelection {
    pub fn projection(&self) -> &Value {
        &self.projection
    }
}
fn local(message: &str) -> ConsolidationError {
    ConsolidationError::Invariant(message.into())
}

/// Project exact selected UTF-8 byte spans; references retain the immutable full
/// document hash and original byte offsets. Full document text never enters the prompt.
/// The caps also honor the ordinary Dream selected-record configuration.
pub fn select_correction_context(
    db: &Connection,
    lease: &ConsolidationLease,
    config: &DreamConfig,
) -> Result<CorrectionSelection, ConsolidationError> {
    let evidence_limit = config.max_short_term_memories_per_cycle.clamp(1, 512);
    let claim_limit = config.max_total_affected_crystals.clamp(1, 200) as usize;
    let rule_limit = config.max_changed_crystals_per_cycle.clamp(1, 100);
    let request: String = db.query_row(
        "select canonical_request from decision_records where decision_id=?",
        [&lease.decision_id],
        |r| r.get(0),
    )?;
    if request.len() > MAX_UNRESOLVED_SIGNAL_BYTES {
        return Err(local("correction request exceeds context bound"));
    }
    let request: Value =
        serde_json::from_str(&request).map_err(|_| local("corrupt correction request"))?;
    if request["kind"] == "unresolved_signal" {
        let signal: UnresolvedSignalV1 = serde_json::from_value(request.clone())
            .map_err(|_| local("corrupt unresolved signal"))?;
        if signal.version != 1
            || signal.decision_id != lease.decision_id
            || signal.reasons.is_empty()
        {
            return Err(local("invalid unresolved signal identity"));
        }
        let (text, context, digest): (String, String, String) = db.query_row(
            "select text,context_json,content_hash from origin_receipts where id=?",
            [&signal.origin.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if text != signal.text
            || serde_json::from_str::<Value>(&context).ok() != signal.submitted_context()
            || crate::authority_evidence::hash(&format!("{text}\n{context}")) != digest
        {
            return Err(local("unresolved origin mismatch"));
        }
    }
    if request["kind"] != "unresolved_signal" && request.to_string().len() > MAX_RECORD_BYTES {
        return Err(local("correction request exceeds context bound"));
    }
    let mut bytes = request.to_string().len() + 128;
    let mut evidence = vec![];
    let mut claims = vec![];
    let mut records = vec![];
    let mut concepts = std::collections::BTreeSet::new();
    let mut statement = db.prepare("select id,kind,source_identity,source_hash,span_start,span_end,cast(substr(cast(content as blob),span_start+1,span_end-span_start) as text),binding_json from evidence_records where series_id=?1 and json_extract(binding_json,'$.event') is null and span_end-span_start+length(cast(binding_json as blob))<=?2 order by exists(select 1 from decision_evidence d where d.decision_id=?3 and d.source_id=evidence_records.id) desc,id desc limit ?4")?;
    let rows = statement.query_map(
        params![
            lease.series_id,
            MAX_RECORD_BYTES as i64,
            lease.decision_id,
            evidence_limit
        ],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        },
    )?;
    for row in rows {
        let (id, kind, identity, hash, start, end, content, binding) = row?;
        let kind: EvidenceKind =
            serde_json::from_value(json!(kind)).map_err(|_| local("corrupt evidence kind"))?;
        let binding: Value =
            serde_json::from_str(&binding).map_err(|_| local("corrupt evidence binding"))?;
        let mut claim = None;
        if kind == EvidenceKind::Observation && identity.starts_with("claim:") {
            if claims.len() >= claim_limit {
                continue;
            }
            let claim_id: i64 = identity[6..]
                .parse()
                .map_err(|_| local("corrupt claim identity"))?;
            let live: bool = db.query_row(
                "select exists(select 1 from claim_bindings where claim_id=?)",
                [claim_id],
                |r| r.get(0),
            )?;
            if !live {
                if crate::consolidation_completion::audited_detached_claim(db, claim_id)? {
                    continue;
                }
                return Err(local("claim capture has no live target or detach proof"));
            }
            let (revision, status, qualification): (i64,String,Option<String>) = db.query_row("select revision,status,qualification from memory_claims where id=?1 and series_id=?2", params![claim_id,lease.series_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            claims.push(SelectedClaimV1 {
                claim_id,
                revision: revision as u64,
                evidence_id: id,
            });
            claim = Some(
                json!({"claim_id":claim_id,"revision":revision,"status":status,"qualification":qualification}),
            );
        } else {
            // Other audit observations are not learned-rule evidence.
            let Ok(typed) = serde_json::from_value::<crate::authority_evidence::EvidenceBindingV1>(
                binding.clone(),
            ) else {
                continue;
            };
            concepts.insert(typed.concept_id);
        }
        let reference = EvidenceRef {
            kind,
            id,
            content_hash: hash,
            span_start: start as usize,
            span_end: end as usize,
        };
        let record = json!({"reference":reference,"selected_excerpt":content,"binding":binding,"claim":claim});
        bytes += record.to_string().len() + 1;
        if bytes > MAX_CONTEXT_BYTES {
            if claim.is_some() {
                claims.pop();
            }
            break;
        }
        evidence.push(reference);
        records.push(record);
    }
    let mut rules = BTreeMap::new();
    let mut rule_records = vec![];
    let mut statement = db.prepare("select r.id,r.revision,r.concept_id,r.source_language,r.target_language,r.source_text,r.canonical_translation,r.status,a.authority,a.applicability_id from term_rules r left join rule_authority a on a.rule_id=r.id where r.concept_id in (select cast(value as integer) from json_each(?1)) order by r.id limit ?2")?;
    for row in statement.query_map(params![json!(concepts).to_string(), rule_limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<i64>>(9)?,
        ))
    })? {
        let (id, revision, concept, source, target, text, rendering, status, authority, app) = row?;
        let app = app
            .map(|id| crate::authority_applicability::load(db, id))
            .transpose()
            .map_err(ConsolidationError::Policy)?
            .flatten();
        let value = json!({"id":id,"revision":revision,"concept_id":concept,"source_language":source,"target_language":target,"source_text":text,"rendering":rendering,"status":status,"authority":authority,"applicability":app});
        bytes += value.to_string().len() + 1;
        if bytes > MAX_CONTEXT_BYTES {
            break;
        }
        rules.insert(id, (revision as u64, concept, source, target));
        rule_records.push(value);
    }
    let projection = json!({"request":request,"evidence":records,"rules":rule_records});
    if projection.to_string().len() > MAX_CONTEXT_BYTES {
        return Err(local("correction projection exceeds context bound"));
    }
    Ok(CorrectionSelection {
        evidence,
        claims,
        rules,
        projection,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum DraftPreparationError {
    #[error("invalid provider decisions: {0}")]
    Provider(String),
    #[error(transparent)]
    Local(#[from] ConsolidationError),
}
/// Reject provider-expanded selection before minting a trusted result origin.
/// A policy refusal rolls back its staged origin, leaving reserved work for a
/// budgeted fresh provider call. Revision conflicts retain bytes for stale finish.
pub fn prepare_correction_draft(
    db: &mut Connection,
    lease: &ConsolidationLease,
    selection: CorrectionSelection,
    draft: DecisionsDraftV1,
    now: DateTime<Utc>,
) -> Result<(), DraftPreparationError> {
    use DraftPreparationError::{Local, Provider};
    if draft.version != 1 || draft.mutations.len() > 100 {
        return Err(Provider("decisions bounds".into()));
    }
    for mutation in &draft.mutations {
        match mutation {
            DerivedMutationV1::LearnedRule {
                concept_id,
                source_language,
                target_language,
                operation,
                ..
            } => {
                let (id, rev) = match operation.as_ref() {
                    LearnedRuleOperationV1::Activate {
                        candidate_id,
                        candidate_revision,
                    } => (*candidate_id, *candidate_revision),
                    LearnedRuleOperationV1::Replace {
                        rule_id,
                        rule_revision,
                        ..
                    }
                    | LearnedRuleOperationV1::Scope {
                        rule_id,
                        rule_revision,
                        ..
                    }
                    | LearnedRuleOperationV1::Archive {
                        rule_id,
                        rule_revision,
                    } => (*rule_id, *rule_revision),
                };
                if selection.rules.get(&id)
                    != Some(&(
                        rev,
                        *concept_id,
                        source_language.clone(),
                        target_language.clone(),
                    ))
                {
                    return Err(Provider("unselected rule or revision".into()));
                }
            }
            DerivedMutationV1::ClaimLineage {
                input_claim_ids,
                output_claim_ids,
            } => {
                if input_claim_ids.is_empty()
                    || output_claim_ids.is_empty()
                    || input_claim_ids.len() > 100
                    || output_claim_ids.len() > 100
                    || input_claim_ids
                        .iter()
                        .chain(output_claim_ids)
                        .any(|id| !selection.claims.iter().any(|c| c.claim_id == *id))
                {
                    return Err(Provider("unselected lineage or bounds".into()));
                }
            }
        }
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(ConsolidationError::from)?;
    let (job, _) = crate::consolidation::current_lease(&tx, &lease.token, now)?;
    if job != lease.decision_id {
        return Err(Local(local("selection lease mismatch")));
    }
    let origin = OriginReceiptId(crate::consolidation::uuid(&tx)?);
    let output = ConsolidationResultV1 {
        version: 1,
        result_id: lease.result_id.clone(),
        job_decision_id: lease.decision_id.clone(),
        generation: lease.generation,
        expected_revision: lease.expected_revision,
        origin,
        evidence_refs: selection.evidence,
        selected_claims: selection.claims,
        mutations: draft.mutations,
    };
    let canonical = serde_json::to_value(&output)
        .map_err(|_| Local(local("canonical encoding")))?
        .to_string();
    let text = "Trusted correction consolidation selection";
    let hash = crate::authority_evidence::hash(&format!("{text}\n{canonical}"));
    tx.execute("insert into origin_receipts(id,kind,principal,event_id,text,context_json,content_hash,created_at) values(?1,'dream','correction-worker',?1,?2,?3,?4,?5)",params![output.origin.0,text,canonical,hash,crate::consolidation::timestamp(now)]).map_err(ConsolidationError::from)?;
    match crate::consolidation_completion::validate_draft_tx(&tx, &output, lease.series_id) {
        Ok(()) | Err(ConsolidationError::RevisionConflict) => {}
        Err(ConsolidationError::Policy(
            error @ (crate::authority_models::DecisionErrorV1::InvalidRequest
            | crate::authority_models::DecisionErrorV1::AuthorityConflict
            | crate::authority_models::DecisionErrorV1::ApplicabilityConflict
            | crate::authority_models::DecisionErrorV1::LanguageMismatch),
        )) => return Err(Provider(error.to_string())),
        Err(ConsolidationError::Draft(problem)) => return Err(Provider(format!("{problem:?}"))),
        Err(error) => return Err(Local(error)),
    }
    tx.commit().map_err(ConsolidationError::from)?;
    ConsolidationStore::new(db).prepare_result(&lease.token, &lease.result_id, &canonical, now)?;
    Ok(())
}
