//! Audited binding during normal writes; reads never create claim metadata.
use crate::{
    authority_applicability as applicability, authority_models::DecisionErrorV1,
    claim_reads::ClaimTarget, memory_models::TranslationContext, story_applicability::*,
};
use rusqlite::{Transaction, params};

#[derive(Debug, Clone)]
pub struct ClaimInput {
    pub text: String,
    pub concept_id: Option<i64>,
    pub applicability: ApplicabilityV1,
}
/// Bind an atomic assertion or explicitly selected compound source. The immutable
/// observation audit preserves the original bytes and exact applicability.
pub fn capture_claim_tx(
    tx: &Transaction<'_>,
    target: ClaimTarget,
    input: &ClaimInput,
) -> Result<i64, DecisionErrorV1> {
    if target.id() <= 0 || input.text.trim().is_empty() {
        return Err(DecisionErrorV1::InvalidRequest);
    }
    let app = applicability::store(tx, &input.applicability)?;
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute("insert into memory_claims(series_id,concept_id,text,revision,status,qualification,applicability_id,created_at,updated_at) values(?1,?2,?3,1,'current',null,?4,?5,?5)",params![input.applicability.series_id,input.concept_id,input.text,app,now])?;
    let id = tx.last_insert_rowid();
    tx.execute(
        &format!(
            "insert into claim_bindings(claim_id,{}) values(?1,?2)",
            target.column()
        ),
        params![id, target.id()],
    )?;
    let binding = serde_json::json!({"claim_id":id,"applicability":input.applicability,"target_kind":target.column(),"target_id":target.id()});
    tx.execute("insert into evidence_records(series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,'observation',?2,?3,0,?4,?5,?6,?7)",params![input.applicability.series_id,format!("claim:{id}"),crate::authority_evidence::hash(&input.text),input.text.len() as i64,input.text,binding.to_string(),now])?;
    Ok(id)
}
/// Existing capture context supplies conservative compound applicability. Missing
/// manifest/position remains unspecified; no timeline or knowledge is invented.
pub(crate) fn capture_context_tx(
    tx: &Transaction<'_>,
    target: ClaimTarget,
    text: &str,
    context: &TranslationContext,
    concept_id: Option<i64>,
) -> Result<i64, DecisionErrorV1> {
    let q = StoryApplicability::resolve_context(tx, context)
        .map_err(|_| DecisionErrorV1::ApplicabilityConflict)?;
    let viewpoint = match q.viewpoint {
        Viewpoint::Character(id) => KnowledgeViewpoint::Character(id),
        Viewpoint::Narrator => KnowledgeViewpoint::Narrator,
        Viewpoint::Unspecified => KnowledgeViewpoint::All,
    };
    let mut volume = None;
    let mut chapter = None;
    if let Some(id) = q.position_id {
        let keys: (String, String) = tx.query_row(
            "select volume_key,chapter_key from story_positions where id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        volume = Some(keys.0);
        chapter = Some(keys.1);
    }
    let a = ApplicabilityV1 {
        series_id: q.series_id,
        timeline_id: q.timeline_id,
        volume_key: volume,
        chapter_key: chapter,
        scope_predicates: q
            .scope_predicates
            .into_iter()
            .filter(|s| !s.starts_with("volume:") && !s.starts_with("chapter:"))
            .collect(),
        valid_from: q.position_id,
        valid_until: None,
        metadata_state: if q.position_id.is_some() {
            MetadataState::Resolved
        } else {
            MetadataState::Unspecified
        },
        knowledge_gates: vec![KnowledgeGateV1 {
            viewpoint,
            known_from: q.position_id,
            known_until: None,
        }],
    };
    capture_claim_tx(
        tx,
        target,
        &ClaimInput {
            text: text.into(),
            concept_id,
            applicability: a,
        },
    )
}
/// Exact copies retain the same claim identity and all future scoped effects.
pub(crate) fn copy_bindings_tx(
    tx: &Transaction<'_>,
    source: ClaimTarget,
    target: ClaimTarget,
) -> Result<usize, DecisionErrorV1> {
    Ok(tx.execute(&format!("insert or ignore into claim_bindings(claim_id,{}) select claim_id,?1 from claim_bindings where {}=?2",target.column(),source.column()),params![target.id(),source.id()])?)
}
