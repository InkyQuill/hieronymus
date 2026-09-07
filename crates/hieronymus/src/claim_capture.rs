//! Audited binding during normal writes; reads never create claim metadata.
use crate::{
    authority_applicability as applicability, authority_models::DecisionErrorV1,
    claim_reads::ClaimTarget, memory_models::TranslationContext, story_applicability::*,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
    tx.execute_batch("SAVEPOINT claim_capture")?;
    let result = capture_claim_inner(tx, target, input);
    if result.is_err() {
        tx.execute_batch("ROLLBACK TO claim_capture")?;
    }
    tx.execute_batch("RELEASE claim_capture")?;
    result
}
fn capture_claim_inner(
    tx: &Transaction<'_>,
    target: ClaimTarget,
    input: &ClaimInput,
) -> Result<i64, DecisionErrorV1> {
    if target.id() <= 0 || input.text.trim().is_empty() {
        return Err(DecisionErrorV1::InvalidRequest);
    }
    validate_target(
        tx,
        target,
        input.applicability.series_id,
        input.concept_id,
        true,
    )?;
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
    let binding = serde_json::json!({"claim_id":id,"applicability":input.applicability,"target_kind":target.column(),"target_id":target.id(),"target_snapshot":target_snapshot(tx,target)?});
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
        Viewpoint::Character(id) => Some(KnowledgeViewpoint::Character(id)),
        Viewpoint::Narrator => Some(KnowledgeViewpoint::Narrator),
        Viewpoint::Unspecified => None,
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
        knowledge_gates: viewpoint
            .into_iter()
            .map(|viewpoint| KnowledgeGateV1 {
                viewpoint,
                known_from: q.position_id,
                known_until: None,
            })
            .collect(),
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
/// Validate concrete ownership before creating or copying a binding.
fn validate_target(
    db: &Connection,
    target: ClaimTarget,
    series: i64,
    concept: Option<i64>,
    exact_concept: bool,
) -> Result<(), DecisionErrorV1> {
    let query = match target {
        ClaimTarget::ShortTerm(_) => {
            "select exists(select 1 from short_term_memories m join task_sessions t on t.id=m.session_id join series s on s.slug=t.series_slug where m.id=?1 and s.id=?2)"
        }
        ClaimTarget::Crystal(_) => {
            "select exists(select 1 from crystals c join series s on s.slug=c.series_slug where c.id=?1 and s.id=?2)"
        }
        ClaimTarget::RagChunk(_) => {
            "select exists(select 1 from rag_chunks c join series s on s.slug=c.series_slug where c.id=?1 and s.id=?2)"
        }
        ClaimTarget::Facet(_) => {
            "select exists(select 1 from concept_facets f join concepts c on c.id=f.concept_id join series s on s.id=?2 where f.id=?1 and (c.scope_type='global' or c.scope_type='series' and c.scope_key='series:'||s.slug))"
        }
    };
    if !db.query_row(query, params![target.id(), series], |r| r.get::<_, bool>(0))? {
        return Err(DecisionErrorV1::UnknownTarget);
    }
    if let Some(concept) = concept {
        let valid: bool = db.query_row("select exists(select 1 from concepts c join series s on s.id=?2 where c.id=?1 and (c.scope_type='global' or c.scope_type='series' and c.scope_key='series:'||s.slug))",params![concept,series],|r|r.get(0))?;
        if !valid {
            return Err(DecisionErrorV1::UnknownTarget);
        }
    }
    if let ClaimTarget::Facet(id) = target
        && exact_concept
    {
        let owner: i64 = db.query_row(
            "select concept_id from concept_facets where id=?",
            [id],
            |r| r.get(0),
        )?;
        if concept != Some(owner) {
            return Err(DecisionErrorV1::EvidenceMismatch);
        }
    }
    Ok(())
}
/// Immutable lifecycle evidence records the original claim bytes, identity and
/// both endpoints. Object deletion never deletes the claim or its effects.
pub(crate) fn audit_binding(
    db: &Connection,
    claim: i64,
    source: ClaimTarget,
    target: Option<ClaimTarget>,
    reason: &str,
) -> Result<(), DecisionErrorV1> {
    let (series, text, app): (i64, String, i64) = db.query_row(
        "select series_id,text,applicability_id from memory_claims where id=?",
        [claim],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let prior_detach: Option<i64> = if reason == "rag_rebind" {
        db.query_row("select id from evidence_records where json_extract(binding_json,'$.event')='rag_replace_detach' and json_extract(binding_json,'$.claim_id')=?1 and json_extract(binding_json,'$.from.id')=?2 order by id desc limit 1",params![claim,source.id()],|r|r.get(0)).optional()?
    } else {
        None
    };
    let source_snapshot = if reason == "rag_rebind" {
        None
    } else {
        target_snapshot(db, source)?
    };
    let binding = serde_json::json!({"event":reason,"source_snapshot":source_snapshot,"prior_detach_evidence_id":prior_detach,"target_snapshot":target.map(|t|target_snapshot(db,t)).transpose()?,"claim_id":claim,"applicability":applicability::load(db,app)?,"from":{"kind":source.column(),"id":source.id()},"to":target.map(|t|serde_json::json!({"kind":t.column(),"id":t.id()}))});
    db.execute("insert into evidence_records(series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,'observation',?2,?3,0,?4,?5,?6,?7)",params![series,format!("claim:{claim}:lifecycle:{}",db.query_row("select coalesce(max(id),0)+1 from evidence_records",[],|r|r.get::<_,i64>(0))?),crate::authority_evidence::hash(&text),text.len() as i64,text,binding.to_string(),chrono::Utc::now().to_rfc3339()])?;
    Ok(())
}
pub(crate) fn binding_ids(
    db: &Connection,
    target: ClaimTarget,
) -> Result<Vec<i64>, DecisionErrorV1> {
    let mut s = db.prepare(&format!(
        "select claim_id from claim_bindings where {}=? order by claim_id",
        target.column()
    ))?;
    Ok(s.query_map([target.id()], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}
pub(crate) fn detach_bindings(
    db: &Connection,
    target: ClaimTarget,
    reason: &str,
) -> Result<(), DecisionErrorV1> {
    for claim in binding_ids(db, target)? {
        audit_binding(db, claim, target, None, reason)?;
    }
    db.execute(
        &format!("delete from claim_bindings where {}=?", target.column()),
        [target.id()],
    )?;
    Ok(())
}
/// Exact copies retain the same claim identity and all future scoped effects.
/// The declared source must belong to the facet's concept scope even when
/// it has no bindings. Copy alongside supplemental claims, never instead.
pub(crate) fn inherit_facet_source(
    db: &Connection,
    facet: i64,
    source: i64,
) -> Result<(), DecisionErrorV1> {
    let series: i64 = db
        .query_row(
            "select s.id from crystals c join series s on s.slug=c.series_slug where c.id=?",
            [source],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(DecisionErrorV1::UnknownTarget)?;
    validate_target(db, ClaimTarget::Facet(facet), series, None, false)?;
    copy_bindings_tx(db, ClaimTarget::Crystal(source), ClaimTarget::Facet(facet))?;
    Ok(())
}

pub(crate) fn copy_bindings_tx(
    db: &Connection,
    source: ClaimTarget,
    target: ClaimTarget,
) -> Result<usize, DecisionErrorV1> {
    let claims = binding_ids(db, source)?;
    for claim in &claims {
        let (series, concept): (i64, Option<i64>) = db.query_row(
            "select series_id,concept_id from memory_claims where id=?",
            [claim],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        validate_target(db, target, series, concept, false)?;
    }
    let mut count = 0;
    for claim in claims {
        let inserted = db.execute(
            &format!(
                "insert or ignore into claim_bindings(claim_id,{}) values(?1,?2)",
                target.column()
            ),
            params![claim, target.id()],
        )?;
        if inserted > 0 {
            audit_binding(db, claim, source, Some(target), "copy")?;
        }
        count += inserted;
    }
    Ok(count)
}

fn target_snapshot(
    db: &Connection,
    target: ClaimTarget,
) -> Result<Option<serde_json::Value>, DecisionErrorV1> {
    if let ClaimTarget::Facet(id) = target {
        return Ok(db.query_row("select value,concept_id,facet_type,language from concept_facets where id=?",[id],|r|Ok(serde_json::json!({"value":r.get::<_,String>(0)?,"concept_id":r.get::<_,i64>(1)?,"facet_type":r.get::<_,String>(2)?,"language":r.get::<_,String>(3)?}))).optional()?);
    }
    if let ClaimTarget::RagChunk(id) = target {
        let snapshot=db.query_row("select c.text,c.location,s.source_ref,s.checksum,s.source_type,c.chunk_kind from rag_chunks c join rag_sources s on s.id=c.source_id where c.id=?",[id],|r|Ok(serde_json::json!({"text":r.get::<_,String>(0)?,"location":r.get::<_,String>(1)?,"source_ref":r.get::<_,String>(2)?,"checksum":r.get::<_,String>(3)?,"source_type":r.get::<_,String>(4)?,"chunk_kind":r.get::<_,String>(5)?}))).optional()?;
        return Ok(snapshot);
    }
    Ok(None)
}

/// An explicit lineage assertion preserves an existing claim's exact identity,
/// applicability and effects. Callers cannot widen it while rebinding an object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExistingClaimInput {
    pub claim_id: i64,
    pub concept_id: Option<i64>,
    pub applicability: ApplicabilityV1,
}
pub(crate) fn bind_existing_claim_tx(
    tx: &Transaction<'_>,
    target: ClaimTarget,
    input: &ExistingClaimInput,
) -> Result<(), DecisionErrorV1> {
    let (series, concept, app): (i64, Option<i64>, i64) = tx
        .query_row(
            "select series_id,concept_id,applicability_id from memory_claims where id=?",
            [input.claim_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(DecisionErrorV1::UnknownTarget)?;
    if concept != input.concept_id
        || applicability::load(tx, app)?.as_ref() != Some(&input.applicability)
    {
        return Err(DecisionErrorV1::EvidenceMismatch);
    }
    validate_target(tx, target, series, concept, true)?;
    if tx.execute(
        &format!(
            "insert or ignore into claim_bindings(claim_id,{}) values(?1,?2)",
            target.column()
        ),
        params![input.claim_id, target.id()],
    )? > 0
    {
        audit_binding(tx, input.claim_id, target, Some(target), "explicit_lineage")?;
    }
    Ok(())
}

/// Copy original crystal identity and applicability without granting authority.
/// The enclosing transaction owns commit; failed copies cannot leave partial
/// bindings even when its caller handles the error and commits other work.
pub fn copy_crystal_lineage_tx(
    tx: &Transaction<'_>,
    source: i64,
    target: i64,
) -> Result<usize, DecisionErrorV1> {
    let same_series: bool = tx.query_row("select exists(select 1 from crystals a join crystals b on b.series_slug=a.series_slug where a.id=?1 and b.id=?2 and a.series_slug != '')",params![source,target],|r|r.get(0))?;
    if !same_series {
        return Err(DecisionErrorV1::InvalidRequest);
    }
    tx.execute_batch("SAVEPOINT crystal_lineage")?;
    let result = copy_bindings_tx(
        tx,
        ClaimTarget::Crystal(source),
        ClaimTarget::Crystal(target),
    );
    if result.is_err() {
        tx.execute_batch("ROLLBACK TO crystal_lineage")?;
    }
    tx.execute_batch("RELEASE crystal_lineage")?;
    result
}
