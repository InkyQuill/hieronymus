//! Authoritative SQLite claim eligibility shared by every retrieval lane.
use crate::{
    authority_applicability as applicability,
    authority_models::DecisionErrorV1,
    story_applicability::{Eligibility, StoryApplicability, StoryQueryV1},
};
use rusqlite::{Connection, params};

/// A bound item is one indivisible assertion group. Any unknown or invalid
/// member prevents plain current-truth presentation of the whole item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDisposition {
    Current,
    Qualified(Vec<String>),
    Invalid,
    Unknown,
    OutsideContext,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimTarget {
    ShortTerm(i64),
    Crystal(i64),
    Facet(i64),
    RagChunk(i64),
}
impl ClaimTarget {
    pub(crate) fn column(self) -> &'static str {
        match self {
            Self::ShortTerm(_) => "short_term_id",
            Self::Crystal(_) => "crystal_id",
            Self::Facet(_) => "facet_id",
            Self::RagChunk(_) => "rag_chunk_id",
        }
    }
    pub(crate) fn id(self) -> i64 {
        match self {
            Self::ShortTerm(id) | Self::Crystal(id) | Self::Facet(id) | Self::RagChunk(id) => id,
        }
    }
}

pub fn claim_disposition(
    db: &Connection,
    claim_id: i64,
    query: &StoryQueryV1,
) -> Result<ClaimDisposition, DecisionErrorV1> {
    let (series, status, qualification, app): (i64, String, Option<String>, i64) = db.query_row(
        "select series_id,status,qualification,applicability_id from memory_claims where id=?",
        [claim_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    if series != query.series_id {
        return Ok(ClaimDisposition::OutsideContext);
    }
    let Some(app) = applicability::load(db, app)? else {
        return Ok(ClaimDisposition::Unknown);
    };
    let eligible = StoryApplicability::evaluate(db, &app, query)
        .map_err(|_| DecisionErrorV1::ApplicabilityConflict)?;
    match eligible {
        Eligibility::Unknown => return Ok(ClaimDisposition::Unknown),
        Eligibility::Excluded | Eligibility::FutureOrOutsideViewpoint => {
            return Ok(ClaimDisposition::OutsideContext);
        }
        Eligibility::Current => {}
    }
    let mut effects=db.prepare("select e.applicability_id,e.effect,e.qualification from claim_effects e join decision_records d on d.decision_id=e.decision_id where e.claim_id=? order by d.resulting_revision desc,e.id desc")?;
    let effects = effects
        .query_map([claim_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (app, effect, qualification) in effects {
        let Some(app) = applicability::load(db, app)? else {
            return Ok(ClaimDisposition::Unknown);
        };
        match StoryApplicability::evaluate(db, &app, query)
            .map_err(|_| DecisionErrorV1::ApplicabilityConflict)?
        {
            Eligibility::Current => return Ok(status_disposition(&effect, qualification)),
            Eligibility::Unknown => return Ok(ClaimDisposition::Unknown),
            _ => {}
        }
    }
    Ok(status_disposition(&status, qualification))
}
fn status_disposition(status: &str, qualification: Option<String>) -> ClaimDisposition {
    match status {
        "current" => ClaimDisposition::Current,
        "invalid" => ClaimDisposition::Invalid,
        "qualified" => qualification
            .filter(|q| !q.trim().is_empty())
            .map(|q| ClaimDisposition::Qualified(vec![q]))
            .unwrap_or(ClaimDisposition::Unknown),
        _ => ClaimDisposition::Unknown,
    }
}
pub fn rehydrate_claims(
    db: &Connection,
    target: ClaimTarget,
    query: &StoryQueryV1,
) -> Result<ClaimDisposition, DecisionErrorV1> {
    let mut stmt = db.prepare(&format!(
        "select claim_id from claim_bindings where {}=? order by claim_id",
        target.column()
    ))?;
    let ids = stmt
        .query_map(params![target.id()], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty() {
        return Ok(ClaimDisposition::Unknown);
    }
    let mut qualifications = vec![];
    for id in ids {
        match claim_disposition(db, id, query)? {
            ClaimDisposition::Current => {}
            ClaimDisposition::Qualified(values) => qualifications.extend(values),
            other => return Ok(other),
        }
    }
    if qualifications.is_empty() {
        Ok(ClaimDisposition::Current)
    } else {
        Ok(ClaimDisposition::Qualified(qualifications))
    }
}

/// IDs eligible for current retrieval, calculated from the authoritative snapshot
/// before the store's SQL ranking/limit. Unknown rows remain available through
/// explicit source inspection, never through this current-truth filter.
pub(crate) fn eligible_ids(
    db: &Connection,
    kind: ClaimTarget,
    query: &StoryQueryV1,
) -> Result<String, DecisionErrorV1> {
    if query.mode == crate::story_applicability::QueryMode::OmniscientResearch {
        let table = match kind {
            ClaimTarget::ShortTerm(_) => "short_term_memories",
            ClaimTarget::Crystal(_) => "crystals",
            ClaimTarget::Facet(_) => "concept_facets",
            ClaimTarget::RagChunk(_) => "rag_chunks",
        };
        let mut stmt = db.prepare(&format!("select id from {table} order by id"))?;
        let ids = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(serde_json::to_string(&ids)?);
    }
    let mut stmt = db.prepare(&format!(
        "select distinct {} from claim_bindings where {} is not null",
        kind.column(),
        kind.column()
    ))?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut eligible = vec![];
    for id in ids {
        let target = match kind {
            ClaimTarget::ShortTerm(_) => ClaimTarget::ShortTerm(id),
            ClaimTarget::Crystal(_) => ClaimTarget::Crystal(id),
            ClaimTarget::Facet(_) => ClaimTarget::Facet(id),
            ClaimTarget::RagChunk(_) => ClaimTarget::RagChunk(id),
        };
        if query.mode == crate::story_applicability::QueryMode::OmniscientResearch
            || matches!(
                rehydrate_claims(db, target, query)?,
                ClaimDisposition::Current | ClaimDisposition::Qualified(_)
            )
        {
            eligible.push(id);
        }
    }
    Ok(serde_json::to_string(&eligible)?)
}
