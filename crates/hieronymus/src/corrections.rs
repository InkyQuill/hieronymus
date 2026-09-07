//! Immediate scoped claim effects. Ingestion owns receipts, jobs and commits.
use crate::{
    authority, authority_applicability as applicability,
    authority_models::*,
    feedback::{self, RecallFeedback},
    story_applicability::{ApplicabilityV1, MetadataState},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Exact immediate effect; the enclosing ingestion records the committed receipt.
#[derive(Debug, Clone)]
pub struct CorrectionEffect {
    pub affected_claims: Vec<(i64, u64)>,
    pub affected_rules: Vec<(i64, u64)>,
    pub effective_applicability: ApplicabilityV1,
    pub effective_exclusions: Vec<ApplicabilityV1>,
    pub effect: String,
}

pub(crate) fn validate_correction(
    db: &Connection,
    request: &DecisionRequestV1,
    intent: &CorrectionIntentV1,
) -> Result<(CorrectionEffect, Vec<TentativeReason>), DecisionErrorV1> {
    use DecisionErrorV1 as Error;
    authority::validate_request(db, request)?;
    if request.operation
        != (OperationV1::Correct {
            intent: intent.clone(),
        })
    {
        return Err(Error::OriginMismatch);
    }
    // Even unused supplied evidence is verified, never accepted as inline authority.
    let resolved = crate::authority_evidence::resolve_all(db, request)?;
    let mut reasons = vec![];
    let mut effect = CorrectionEffect {
        affected_claims: vec![],
        affected_rules: vec![],
        effective_applicability: request.applicability.clone(),
        effective_exclusions: vec![],
        effect: String::new(),
    };
    match intent {
        CorrectionIntentV1::Fact {
            claim_id,
            claim_revision,
            effect: fact,
        } => {
            if *claim_id <= 0 || *claim_revision >= i64::MAX as u64 {
                return Err(Error::InvalidRequest);
            }
            if let FactEffect::Qualify { qualification } = fact
                && (qualification.trim().is_empty() || qualification.chars().any(char::is_control))
            {
                return Err(Error::InvalidRequest);
            }
            let claim:Option<(i64,Option<i64>,i64,i64)>=db.query_row("select series_id,concept_id,revision,applicability_id from memory_claims where id=?",[claim_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
            let (series, concept, revision, app) = claim.ok_or(Error::UnknownTarget)?;
            if series != request.series_id || concept != request.concept_id {
                return Err(Error::EvidenceMismatch);
            }
            if revision as u64 != *claim_revision {
                return Err(Error::RevisionConflict {
                    current_revision: revision as u64,
                });
            }
            let base = applicability::load(db, app)?.ok_or(Error::ApplicabilityConflict)?;
            effect.effective_applicability =
                applicability::intersection(db, &base, &request.applicability)?
                    .ok_or(Error::ApplicabilityConflict)?;
            if concept.is_none() {
                reasons.push(TentativeReason::AmbiguousIdentity);
            }
            if effect.effective_applicability.metadata_state != MetadataState::Resolved
                || effect.effective_applicability.timeline_id.is_none()
                || effect.effective_applicability.knowledge_gates.is_empty()
            {
                reasons.push(TentativeReason::UnknownOrder);
            }
            if request.actor_kind != ActorKind::ExplicitUser {
                // Learned corrections cannot reverse any overlapping explicit effect.
                let mut masks=db.prepare("select e.applicability_id from claim_effects e join decision_records d on d.decision_id=e.decision_id where e.claim_id=? and d.actor_kind='explicit_user'")?;
                let masks = masks
                    .query_map([claim_id], |r| r.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                for mask in masks {
                    let mask =
                        applicability::load(db, mask)?.ok_or(Error::ApplicabilityConflict)?;
                    if applicability::overlaps(db, &mask, &effect.effective_applicability)? {
                        return Err(Error::AuthorityConflict);
                    }
                }
                let mut effective_request = request.clone();
                effective_request.applicability = effect.effective_applicability.clone();
                reasons.extend(crate::authority_evidence::learned_fact_policy(
                    db,
                    &effective_request,
                    &resolved,
                    *claim_id,
                    fact,
                )?);
            }
            effect
                .affected_claims
                .push((*claim_id, *claim_revision + 1));
            effect.effect = match fact {
                FactEffect::Invalidate => "invalid",
                FactEffect::Qualify { .. } => "qualified",
            }
            .into();
        }
        CorrectionIntentV1::Relevance {
            recall_id,
            useful,
            missed,
        } => {
            if recall_id.trim().is_empty()
                || useful.is_empty() && missed.is_empty()
                || useful.iter().chain(missed).any(|id| *id <= 0)
            {
                return Err(Error::InvalidRequest);
            }
            let mut rows=db.prepare("select distinct series.id from crystal_activations a join task_sessions s on s.id=a.session_id join series on series.slug=s.series_slug where a.recall_id=?")?;
            let series = rows
                .query_map([recall_id], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            if series.is_empty() {
                return Err(Error::UnknownTarget);
            }
            if series.iter().any(|id| *id != request.series_id) {
                return Err(Error::EvidenceMismatch);
            }
            effect.effect = "relevance".into();
        }
        CorrectionIntentV1::Rendering { .. } => return Err(Error::InvalidRequest),
    }
    Ok((effect, reasons))
}

/// Apply an already admissible correction inside the caller's ingestion transaction.
/// The decision record must already exist for FK audit ownership. No provider call,
/// vector write, receipt, work enqueue or independent commit occurs here.
pub fn apply_correction_tx(
    tx: &Transaction<'_>,
    request: &DecisionRequestV1,
    intent: &CorrectionIntentV1,
) -> Result<CorrectionEffect, DecisionErrorV1> {
    let (effect, reasons) = validate_correction(tx, request, intent)?;
    if !reasons.is_empty() {
        return Err(DecisionErrorV1::InvalidRequest);
    }
    match intent {
        CorrectionIntentV1::Fact {
            claim_id,
            claim_revision,
            effect: fact,
        } => {
            let app = applicability::store(tx, &effect.effective_applicability)?;
            let qualification = match fact {
                FactEffect::Invalidate => None,
                FactEffect::Qualify { qualification } => Some(qualification.as_str()),
            };
            tx.execute("insert into claim_effects(claim_id,decision_id,applicability_id,effect,qualification) values(?1,?2,?3,?4,?5)",params![claim_id,request.decision_id,app,effect.effect,qualification])?;
            tx.execute(
                "update memory_claims set revision=?2,updated_at=?3 where id=?1",
                params![
                    claim_id,
                    *claim_revision as i64 + 1,
                    chrono::Utc::now().to_rfc3339()
                ],
            )?;
        }
        CorrectionIntentV1::Relevance {
            recall_id,
            useful,
            missed,
        } => {
            feedback::record_recall_outcome_tx(
                tx,
                &RecallFeedback {
                    recall_id: recall_id.clone(),
                    useful_activation_ids: useful.clone(),
                    missed_activation_ids: missed.clone(),
                    idempotency_key: format!("decision:{}", request.decision_id),
                },
            )
            .map_err(|e| match e {
                feedback::FeedbackError::Database(_) | feedback::FeedbackError::Open(_) => {
                    DecisionErrorV1::StorageUnavailable
                }
                _ => DecisionErrorV1::InvalidRequest,
            })?;
        }
        CorrectionIntentV1::Rendering { .. } => return Err(DecisionErrorV1::InvalidRequest),
    }
    Ok(effect)
}
