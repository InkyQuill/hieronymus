//! Authoritative SQLite claim eligibility shared by every retrieval lane.
use crate::{
    authority_applicability as applicability,
    authority_models::DecisionErrorV1,
    story_applicability::{Eligibility, StoryApplicability, StoryQueryV1},
};
use rusqlite::{Connection, OptionalExtension, params};

/// A bound item is one indivisible assertion group. Any unknown or invalid
/// member prevents plain current-truth presentation of the whole item.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", content = "qualifications", rename_all = "snake_case")]
pub enum ClaimDisposition {
    Current,
    Qualified(Vec<String>),
    Invalid,
    Unknown,
    OutsideContext,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "source", content = "id", rename_all = "snake_case")]
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
    let eligible = evaluate_candidate(db, &app, query)?;
    match eligible {
        Eligibility::Unknown => return Ok(ClaimDisposition::Unknown),
        Eligibility::Excluded | Eligibility::FutureOrOutsideViewpoint => {
            return Ok(ClaimDisposition::OutsideContext);
        }
        Eligibility::Current => {}
    }
    for effect in effect_annotations(db, claim_id)? {
        match effect_eligibility(db, &effect, query)? {
            Eligibility::Current => return Ok(effect.disposition),
            Eligibility::Unknown => return Ok(ClaimDisposition::Unknown),
            _ => {}
        }
    }
    Ok(status_disposition(&status, qualification))
}
/// A valid other-timeline candidate is outside this query, whereas malformed
/// timeline/position ownership must remain an error. Validate both timelines
/// and the query before taking the incompatibility shortcut.
fn evaluate_candidate(
    db: &Connection,
    app: &crate::story_applicability::ApplicabilityV1,
    query: &StoryQueryV1,
) -> Result<Eligibility, DecisionErrorV1> {
    if app.series_id != query.series_id {
        return Err(DecisionErrorV1::ApplicabilityConflict);
    }
    if let (Some(candidate), Some(requested)) = (app.timeline_id, query.timeline_id)
        && candidate != requested
    {
        for (timeline, series) in [(candidate, app.series_id), (requested, query.series_id)] {
            if !db.query_row(
                "select exists(select 1 from story_timelines where id=?1 and series_id=?2)",
                params![timeline, series],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(DecisionErrorV1::ApplicabilityConflict);
            }
        }
        let mut neutral = app.clone();
        neutral.series_id = query.series_id;
        neutral.timeline_id = query.timeline_id;
        neutral.volume_key = None;
        neutral.chapter_key = None;
        neutral.scope_predicates.clear();
        neutral.valid_from = None;
        neutral.valid_until = None;
        neutral.knowledge_gates.clear();
        StoryApplicability::evaluate(db, &neutral, query)
            .map_err(|_| DecisionErrorV1::ApplicabilityConflict)?;
        return Ok(Eligibility::Excluded);
    }
    StoryApplicability::evaluate(db, app, query).map_err(|_| DecisionErrorV1::ApplicabilityConflict)
}

fn effect_eligibility(
    db: &Connection,
    effect: &ClaimEffectAnnotation,
    query: &StoryQueryV1,
) -> Result<Eligibility, DecisionErrorV1> {
    let eligibility = evaluate_candidate(db, &effect.applicability, query)?;
    if eligibility != Eligibility::Current {
        return Ok(eligibility);
    }
    for mask in &effect.exclusions {
        match evaluate_candidate(db, mask, query)? {
            Eligibility::Current => return Ok(Eligibility::Excluded),
            Eligibility::Unknown => return Ok(Eligibility::Unknown),
            _ => {}
        }
    }
    Ok(Eligibility::Current)
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
    let mut unresolved = None;
    for id in ids {
        match claim_disposition(db, id, query)? {
            ClaimDisposition::Current => {}
            ClaimDisposition::Qualified(values) => qualifications.extend(values),
            ClaimDisposition::Invalid => return Ok(ClaimDisposition::Invalid),
            ClaimDisposition::Unknown => unresolved = Some(ClaimDisposition::Unknown),
            ClaimDisposition::OutsideContext => {
                unresolved.get_or_insert(ClaimDisposition::OutsideContext);
            }
        }
    }
    if let Some(disposition) = unresolved {
        return Ok(disposition);
    }
    if qualifications.is_empty() {
        Ok(ClaimDisposition::Current)
    } else {
        Ok(ClaimDisposition::Qualified(qualifications))
    }
}

/// Hard per-lane candidate budget. SQL ranks candidates before this bounded
/// hydration pass; final ranking/limits only consume eligible assertions.
pub(crate) const CANDIDATE_BUDGET: usize = 512;

pub(crate) fn select_candidates<T>(
    db: &Connection,
    rows: Vec<T>,
    target: impl Fn(&T) -> ClaimTarget,
    query: Option<&StoryQueryV1>,
    limit: usize,
) -> Result<Vec<T>, DecisionErrorV1> {
    let mut eligible = Vec::new();
    let mut metadata = Vec::new();
    for row in rows {
        let current = match query {
            None => true,
            Some(query) => matches!(
                rehydrate_claims(db, target(&row), query)?,
                ClaimDisposition::Current | ClaimDisposition::Qualified(_)
            ),
        };
        if current {
            if eligible.len() < limit {
                eligible.push(row);
            }
        } else if metadata.len() < limit {
            metadata.push(row);
        }
    }
    eligible.extend(metadata);
    Ok(eligible)
}

/// Typed disclosure carried even by raw archive/source store reads. Source
/// inspection never asserts current applicability. Resolved reads supply IDs,
/// revisions and original applicability for deterministic lineage resolution.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClaimReadAnnotation {
    pub disposition: ClaimDisposition,
    pub source_inspection: bool,
    pub claims: Vec<ClaimReadIdentity>,
}
impl Default for ClaimReadAnnotation {
    fn default() -> Self {
        Self {
            disposition: ClaimDisposition::Unknown,
            source_inspection: true,
            claims: vec![],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClaimReadIdentity {
    pub claim_id: i64,
    pub concept_id: Option<i64>,
    pub revision: u64,
    pub effects: Vec<ClaimEffectAnnotation>,
    pub applicability: crate::story_applicability::ApplicabilityV1,
    pub disposition: ClaimDisposition,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClaimEffectAnnotation {
    pub decision_id: String,
    pub disposition: ClaimDisposition,
    pub applicability: crate::story_applicability::ApplicabilityV1,
    pub exclusions: Vec<crate::story_applicability::ApplicabilityV1>,
}

fn effect_annotations(
    db: &Connection,
    claim: i64,
) -> Result<Vec<ClaimEffectAnnotation>, DecisionErrorV1> {
    let mut statement = db.prepare("select e.decision_id,e.effect,e.qualification,e.applicability_id,d.result_json from claim_effects e join decision_records d on d.decision_id=e.decision_id where e.claim_id=? order by d.resulting_revision desc,e.id desc")?;
    let rows = statement
        .query_map([claim], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut effects = vec![];
    for (decision_id, status, qualification, app, json) in rows {
        let receipt: crate::authority_models::DecisionResultV1 = serde_json::from_str(&json)?;
        let applicability =
            applicability::load(db, app)?.ok_or(DecisionErrorV1::ApplicabilityConflict)?;
        effects.push(ClaimEffectAnnotation {
            decision_id,
            disposition: status_disposition(&status, qualification),
            applicability,
            exclusions: receipt.receipt().effective_exclusions.clone(),
        });
    }
    Ok(effects)
}

pub fn read_annotation(
    db: &Connection,
    target: ClaimTarget,
    query: &StoryQueryV1,
) -> Result<ClaimReadAnnotation, DecisionErrorV1> {
    let mut statement = db.prepare(&format!("select c.id,c.concept_id,c.revision,c.applicability_id from memory_claims c join claim_bindings b on b.claim_id=c.id where b.{}=? order by c.id", target.column()))?;
    let rows = statement
        .query_map([target.id()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut claims = vec![];
    for (claim_id, concept_id, revision, app) in rows {
        if let Some(applicability) = applicability::load(db, app)? {
            claims.push(ClaimReadIdentity {
                claim_id,
                concept_id,
                revision,
                effects: effect_annotations(db, claim_id)?,
                applicability,
                disposition: claim_disposition(db, claim_id, query)?,
            });
        }
    }
    let disposition = rehydrate_claims(db, target, query)?;
    let source_inspection = query.mode == crate::story_applicability::QueryMode::OmniscientResearch;
    if !source_inspection {
        for claim in &mut claims {
            // Only the newest applicable unmasked effect controls this query.
            // Keep structural history, but never disclose obsolete/outside prose.
            let mut selected = false;
            for effect in &mut claim.effects {
                let eligibility = effect_eligibility(db, effect, query)?;
                let visible = !selected && eligibility == Eligibility::Current;
                if matches!(eligibility, Eligibility::Current | Eligibility::Unknown) {
                    selected = true;
                }
                if !visible && let ClaimDisposition::Qualified(values) = &mut effect.disposition {
                    values.clear();
                }
            }
        }
    }
    if !source_inspection
        && !matches!(
            disposition,
            ClaimDisposition::Current | ClaimDisposition::Qualified(_)
        )
    {
        for claim in &mut claims {
            if let ClaimDisposition::Qualified(values) = &mut claim.disposition {
                values.clear();
            }
            for effect in &mut claim.effects {
                if let ClaimDisposition::Qualified(values) = &mut effect.disposition {
                    values.clear();
                }
            }
        }
    }
    Ok(ClaimReadAnnotation {
        disposition,
        source_inspection,
        claims,
    })
}

/// Archive inspection exposes original assertion bytes, but always carries
/// immutable claim identity and correction metadata, never a Current label.
pub(crate) fn source_annotation(
    db: &Connection,
    target: ClaimTarget,
) -> Result<ClaimReadAnnotation, DecisionErrorV1> {
    let series = db.query_row(&format!("select c.series_id from memory_claims c join claim_bindings b on b.claim_id=c.id where b.{}=? limit 1", target.column()), [target.id()], |r| r.get::<_,i64>(0)).optional()?;
    let Some(series_id) = series else {
        return Ok(ClaimReadAnnotation::default());
    };
    read_annotation(
        db,
        target,
        &StoryQueryV1 {
            series_id,
            timeline_id: None,
            position_id: None,
            viewpoint: crate::story_applicability::Viewpoint::Unspecified,
            scope_predicates: vec![],
            mode: crate::story_applicability::QueryMode::OmniscientResearch,
        },
    )
}
