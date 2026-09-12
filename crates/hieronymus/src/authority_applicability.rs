//! Authority storage adapters; eligibility remains owned by StoryApplicability.
use crate::authority_models::DecisionErrorV1 as Error;
use crate::story_applicability::*;
use rusqlite::{Connection, Transaction, params};

// Direction is a mutually exclusive task identity. Editions deliberately are
// ordinary positive predicates: primary and reference sources may coexist.
fn direction(predicates: &[String]) -> Result<Option<&str>, Error> {
    let mut selected = None;
    for predicate in predicates {
        if let Some(id) = predicate.trim().strip_prefix("cws:direction:") {
            if predicate != predicate.trim()
                || !crate::cws_project::valid_identity(id)
                || selected.is_some_and(|old| old != id)
            {
                return Err(Error::ApplicabilityConflict);
            }
            selected = Some(id);
        }
    }
    Ok(selected)
}

pub(crate) fn validate(db: &Connection, a: &ApplicabilityV1) -> Result<(), Error> {
    direction(&a.scope_predicates)?;
    if a.series_id <= 0
        || a.chapter_key.is_some() && a.volume_key.is_none()
        || a.scope_predicates.iter().any(|x| x.trim().is_empty())
    {
        return Err(Error::ApplicabilityConflict);
    }
    for id in [a.valid_from, a.valid_until].into_iter().flatten().chain(
        a.knowledge_gates
            .iter()
            .flat_map(|g| [g.known_from, g.known_until].into_iter().flatten()),
    ) {
        let valid: bool = db.query_row(
            "select exists(select 1 from story_positions where id=?1 and timeline_id=?2)",
            params![id, a.timeline_id],
            |r| r.get(0),
        )?;
        if !valid {
            return Err(Error::ApplicabilityConflict);
        }
    }
    let q = StoryQueryV1 {
        series_id: a.series_id,
        timeline_id: a.timeline_id,
        position_id: None,
        viewpoint: Viewpoint::Unspecified,
        scope_predicates: scopes(a),
        mode: QueryMode::Current,
    };
    StoryApplicability::evaluate(db, a, &q).map_err(|_| Error::ApplicabilityConflict)?;
    Ok(())
}
fn scopes(a: &ApplicabilityV1) -> Vec<String> {
    let mut s = a.scope_predicates.clone();
    if let Some(v) = &a.volume_key {
        s.push(format!("volume:{v}"));
    }
    if let Some(c) = &a.chapter_key {
        s.push(format!("chapter:{c}"));
    }
    s
}
pub(crate) fn store(tx: &Transaction<'_>, a: &ApplicabilityV1) -> Result<i64, Error> {
    validate(tx, a)?;
    tx.execute("insert into applicabilities(series_id,timeline_id,volume_key,chapter_key,scope_predicates_json,valid_from,valid_until,metadata_state) values(?1,?2,?3,?4,?5,?6,?7,?8)",params![a.series_id,a.timeline_id,a.volume_key,a.chapter_key,serde_json::to_string(&a.scope_predicates)?,a.valid_from,a.valid_until,if a.metadata_state==MetadataState::Resolved {"resolved"} else {"unspecified"}])?;
    let id = tx.last_insert_rowid();
    for g in &a.knowledge_gates {
        let (kind, concept) = match g.viewpoint {
            KnowledgeViewpoint::All => ("all", None),
            KnowledgeViewpoint::Narrator => ("narrator", None),
            KnowledgeViewpoint::Character(c) => ("character", Some(c)),
        };
        tx.execute("insert into knowledge_gates(applicability_id,viewpoint_kind,viewpoint_concept_id,known_from,known_until) values(?1,?2,?3,?4,?5)",params![id,kind,concept,g.known_from,g.known_until])?;
    }
    Ok(id)
}
pub(crate) fn load(db: &Connection, id: i64) -> Result<Option<ApplicabilityV1>, Error> {
    type StoredApplicability = (
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        String,
        Option<i64>,
        Option<i64>,
        String,
    );
    let (series,timeline,volume,chapter,pred,from,until,state):StoredApplicability=db.query_row("select series_id,timeline_id,volume_key,chapter_key,scope_predicates_json,valid_from,valid_until,metadata_state from applicabilities where id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)))?;
    let Some(series_id) = series else {
        return Ok(None);
    };
    let mut s=db.prepare("select viewpoint_kind,viewpoint_concept_id,known_from,known_until from knowledge_gates where applicability_id=? order by id")?;
    let gates = s
        .query_map([id], |r| {
            let kind: String = r.get(0)?;
            Ok(KnowledgeGateV1 {
                viewpoint: match kind.as_str() {
                    "character" => KnowledgeViewpoint::Character(r.get(1)?),
                    "narrator" => KnowledgeViewpoint::Narrator,
                    _ => KnowledgeViewpoint::All,
                },
                known_from: r.get(2)?,
                known_until: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(ApplicabilityV1 {
        series_id,
        timeline_id: timeline,
        volume_key: volume,
        chapter_key: chapter,
        scope_predicates: serde_json::from_str(&pred)?,
        valid_from: from,
        valid_until: until,
        metadata_state: if state == "resolved" {
            MetadataState::Resolved
        } else {
            MetadataState::Unspecified
        },
        knowledge_gates: gates,
    }))
}
/// Conservative overlap: distinct structural keys/intervals are disjoint;
/// unknown order is never interpreted as permission to overwrite authority.
fn structural_overlap(
    db: &Connection,
    a: &ApplicabilityV1,
    b: &ApplicabilityV1,
) -> Result<bool, Error> {
    if matches!((direction(&a.scope_predicates)?, direction(&b.scope_predicates)?), (Some(left), Some(right)) if left != right)
    {
        return Ok(false);
    }
    if a.series_id != b.series_id
        || matches!((&a.volume_key,&b.volume_key),(Some(a),Some(b)) if a!=b)
        || matches!((&a.chapter_key,&b.chapter_key),(Some(a),Some(b)) if a!=b)
    {
        return Ok(false);
    }
    if a.timeline_id.is_some() && b.timeline_id.is_some() && a.timeline_id != b.timeline_id {
        return Ok(false);
    }
    for (until, from) in [(a.valid_until, b.valid_from), (b.valid_until, a.valid_from)] {
        if matches!(
            StoryApplicability::compare(db, until, from)
                .map_err(|_| Error::ApplicabilityConflict)?,
            StoryOrdering::Before | StoryOrdering::Equal
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(crate) fn contains(
    db: &Connection,
    outer: &ApplicabilityV1,
    inner: &ApplicabilityV1,
) -> Result<bool, Error> {
    if outer.series_id != inner.series_id
        || outer.timeline_id != inner.timeline_id
        || outer
            .volume_key
            .as_ref()
            .is_some_and(|v| Some(v) != inner.volume_key.as_ref())
        || outer
            .chapter_key
            .as_ref()
            .is_some_and(|v| Some(v) != inner.chapter_key.as_ref())
        || !outer
            .scope_predicates
            .iter()
            .all(|v| inner.scope_predicates.contains(v))
        || outer.knowledge_gates != inner.knowledge_gates
    {
        return Ok(false);
    }
    if let Some(from) = outer.valid_from
        && !matches!(
            StoryApplicability::compare(db, Some(from), inner.valid_from)
                .map_err(|_| Error::ApplicabilityConflict)?,
            StoryOrdering::Before | StoryOrdering::Equal
        )
    {
        return Ok(false);
    }
    if let Some(until) = outer.valid_until
        && !matches!(
            StoryApplicability::compare(db, inner.valid_until, Some(until))
                .map_err(|_| Error::ApplicabilityConflict)?,
            StoryOrdering::Before | StoryOrdering::Equal
        )
    {
        return Ok(false);
    }
    Ok(true)
}
/// Rendering eligibility after source occurrence identity has been resolved.
pub(crate) fn rule_is_current(
    db: &Connection,
    id: i64,
    context: &crate::memory_models::TranslationContext,
) -> Result<bool, Error> {
    use rusqlite::OptionalExtension;
    let authority:Option<(i64,String,String)>=db.query_row("select a.applicability_id,r.source_language,r.target_language from rule_authority a join term_rules r on r.id=a.rule_id where r.id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (a, source, target) = match authority {
        Some((id, source, target)) => (load(db, id)?, source, target),
        None => (
            None,
            context.source_language.clone(),
            context.target_language.clone(),
        ),
    };
    if a.is_some() && (source != context.source_language || target != context.target_language) {
        return Ok(false);
    }
    let query = StoryApplicability::resolve_context(db, context);
    let query = match query {
        Ok(q) => q,
        Err(_) => return Ok(a.is_none()),
    };
    if let Some(a) = a {
        if a.series_id != query.series_id {
            return Ok(false);
        }
        if StoryApplicability::evaluate(db, &a, &query).map_err(|_| Error::ApplicabilityConflict)?
            != Eligibility::Current
        {
            return Ok(false);
        }
    }
    let mut s = db.prepare("select applicability_id from rule_exclusions where rule_id=?")?;
    let exclusions = s
        .query_map([id], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for id in exclusions {
        if let Some(a) = load(db, id)? {
            if a.series_id != query.series_id {
                continue;
            }
            if matches!(
                StoryApplicability::evaluate(db, &a, &query)
                    .map_err(|_| Error::ApplicabilityConflict)?,
                Eligibility::Current | Eligibility::Unknown
            ) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn later(db: &Connection, a: Option<i64>, b: Option<i64>) -> Result<Option<i64>, Error> {
    match (a, b) {
        (None, b) => Ok(b),
        (a, None) => Ok(a),
        (Some(a), Some(b)) => match StoryApplicability::compare(db, Some(a), Some(b))
            .map_err(|_| Error::ApplicabilityConflict)?
        {
            StoryOrdering::Before => Ok(Some(b)),
            StoryOrdering::Equal | StoryOrdering::After => Ok(Some(a)),
            StoryOrdering::Unknown => Err(Error::ApplicabilityConflict),
        },
    }
}
fn earlier(db: &Connection, a: Option<i64>, b: Option<i64>) -> Result<Option<i64>, Error> {
    match (a, b) {
        (None, b) => Ok(b),
        (a, None) => Ok(a),
        (Some(a), Some(b)) => match StoryApplicability::compare(db, Some(a), Some(b))
            .map_err(|_| Error::ApplicabilityConflict)?
        {
            StoryOrdering::After => Ok(Some(b)),
            StoryOrdering::Equal | StoryOrdering::Before => Ok(Some(a)),
            StoryOrdering::Unknown => Err(Error::ApplicabilityConflict),
        },
    }
}
/// Structural intersection uses the reviewed order and positive viewpoint
/// semantics; it never substitutes relevance or lexical chapter ordering.
pub(crate) fn intersection(
    db: &Connection,
    a: &ApplicabilityV1,
    b: &ApplicabilityV1,
) -> Result<Option<ApplicabilityV1>, Error> {
    if !structural_overlap(db, a, b)? {
        return Ok(None);
    }
    let mut out = b.clone();
    if a.metadata_state == MetadataState::Unspecified
        || b.metadata_state == MetadataState::Unspecified
        || a.timeline_id.is_none()
        || b.timeline_id.is_none()
    {
        out.metadata_state = MetadataState::Unspecified;
    }
    out.timeline_id = a.timeline_id.or(b.timeline_id);
    out.volume_key = a.volume_key.clone().or(b.volume_key.clone());
    out.chapter_key = a.chapter_key.clone().or(b.chapter_key.clone());
    out.valid_from = later(db, a.valid_from, b.valid_from)?;
    out.valid_until = earlier(db, a.valid_until, b.valid_until)?;
    out.scope_predicates.extend(a.scope_predicates.clone());
    out.scope_predicates.sort();
    out.scope_predicates.dedup();
    out.knowledge_gates.clear();
    for left in &a.knowledge_gates {
        for right in &b.knowledge_gates {
            let viewpoint = if left.viewpoint == right.viewpoint {
                Some(left.viewpoint)
            } else {
                match (left.viewpoint, right.viewpoint) {
                    (KnowledgeViewpoint::All, KnowledgeViewpoint::Narrator)
                    | (KnowledgeViewpoint::Narrator, KnowledgeViewpoint::All) => {
                        Some(KnowledgeViewpoint::Narrator)
                    }
                    _ => None,
                }
            };
            if let Some(viewpoint) = viewpoint {
                let from = later(db, left.known_from, right.known_from)?;
                let until = earlier(db, left.known_until, right.known_until)?;
                if matches!(
                    StoryApplicability::compare(
                        db,
                        later(db, from, out.valid_from)?,
                        earlier(db, until, out.valid_until)?
                    )
                    .map_err(|_| Error::ApplicabilityConflict)?,
                    StoryOrdering::After | StoryOrdering::Equal
                ) {
                    continue;
                }
                let gate = KnowledgeGateV1 {
                    viewpoint,
                    known_from: from,
                    known_until: until,
                };
                if !out.knowledge_gates.contains(&gate) {
                    out.knowledge_gates.push(gate);
                }
            }
        }
    }
    if out.knowledge_gates.is_empty() {
        return Ok(None);
    }
    validate(db, &out)?;
    Ok(Some(out))
}

/// Two resolved authority sets compete only where both world validity and a
/// common positive viewpoint gate permit truth.
pub(crate) fn overlaps(
    db: &Connection,
    a: &ApplicabilityV1,
    b: &ApplicabilityV1,
) -> Result<bool, Error> {
    effective_overlap(db, Some(a), b, &[])
}
pub(crate) fn exclusions(db: &Connection, rule: i64) -> Result<Vec<ApplicabilityV1>, Error> {
    let mut s =
        db.prepare("select applicability_id from rule_exclusions where rule_id=? order by id")?;
    let ids = s
        .query_map([rule], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.into_iter()
        .map(|id| load(db, id)?.ok_or(Error::ApplicabilityConflict))
        .collect()
}
/// Query one effective set using exactly the story predicate. An unknown mask
/// cannot prove exclusion, and unknown base metadata never becomes Current.
pub(crate) fn effective_eligibility(
    db: &Connection,
    base: &ApplicabilityV1,
    masks: &[ApplicabilityV1],
    q: &StoryQueryV1,
) -> Result<Eligibility, Error> {
    let state =
        StoryApplicability::evaluate(db, base, q).map_err(|_| Error::ApplicabilityConflict)?;
    if state != Eligibility::Current {
        return Ok(state);
    }
    let mut unknown = false;
    for mask in masks {
        if mask.series_id != q.series_id
            || (mask.timeline_id.is_some()
                && q.timeline_id.is_some()
                && mask.timeline_id != q.timeline_id)
        {
            continue;
        }
        match StoryApplicability::evaluate(db, mask, q).map_err(|_| Error::ApplicabilityConflict)? {
            Eligibility::Current => return Ok(Eligibility::Excluded),
            Eligibility::Unknown => unknown = true,
            _ => {}
        }
    }
    Ok(if unknown {
        Eligibility::Unknown
    } else {
        Eligibility::Current
    })
}
/// The universe of Current story queries is the registered position manifest.
/// Test each eligible position/viewpoint with the minimum required predicates;
/// an exclusion needing extra predicates cannot cover that whole region.
/// Unknown regions are conservatively retained, never invented as Current.
pub(crate) fn effective_overlap(
    db: &Connection,
    base: Option<&ApplicabilityV1>,
    requested: &ApplicabilityV1,
    masks: &[ApplicabilityV1],
) -> Result<bool, Error> {
    let region = match base {
        Some(base) => match intersection(db, base, requested)? {
            Some(region) => region,
            None => return Ok(false),
        },
        None => requested.clone(),
    };
    if region.metadata_state != MetadataState::Resolved || region.timeline_id.is_none() {
        return Ok(true);
    }
    let mut statement=db.prepare("select id,volume_key,chapter_key from story_positions where timeline_id=? order by ordinal")?;
    let positions = statement
        .query_map([region.timeline_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, volume, chapter) in positions {
        let mut predicates = region.scope_predicates.clone();
        predicates.extend([format!("volume:{volume}"), format!("chapter:{chapter}")]);
        // All authorizes narrator and unspecified, never a character.
        let mut viewpoints = vec![Viewpoint::Narrator, Viewpoint::Unspecified];
        for g in &region.knowledge_gates {
            if let KnowledgeViewpoint::Character(id) = g.viewpoint {
                viewpoints.push(Viewpoint::Character(id));
            }
        }
        for viewpoint in viewpoints {
            let q = StoryQueryV1 {
                series_id: region.series_id,
                timeline_id: region.timeline_id,
                position_id: Some(id),
                viewpoint,
                scope_predicates: predicates.clone(),
                mode: QueryMode::Current,
            };
            if matches!(
                effective_eligibility(db, &region, masks, &q)?,
                Eligibility::Current | Eligibility::Unknown
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
/// Establish which identities belong to the selected series/language universe,
/// without discarding an identity because of chapter, world or viewpoint gates.
pub(crate) fn rule_identity_in_context(
    db: &Connection,
    id: i64,
    context: &crate::memory_models::TranslationContext,
) -> Result<bool, Error> {
    use rusqlite::OptionalExtension;
    let row:Option<(i64,String,String)>=db.query_row("select ra.applicability_id,tr.source_language,tr.target_language from rule_authority ra join term_rules tr on tr.id=ra.rule_id where tr.id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((id, source, target)) = row else {
        return Ok(true);
    };
    let Some(app) = load(db, id)? else {
        return Ok(true);
    };
    let series: Option<i64> = db
        .query_row(
            "select id from series where slug=?",
            [&context.series_slug],
            |r| r.get(0),
        )
        .optional()?;
    Ok(series == Some(app.series_id)
        && source == context.source_language
        && target == context.target_language)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope(predicates: &[&str]) -> ApplicabilityV1 {
        ApplicabilityV1 {
            series_id: 1,
            timeline_id: None,
            volume_key: None,
            chapter_key: None,
            scope_predicates: predicates.iter().map(|s| (*s).into()).collect(),
            valid_from: None,
            valid_until: None,
            metadata_state: MetadataState::Unspecified,
            knowledge_gates: vec![KnowledgeGateV1 {
                viewpoint: KnowledgeViewpoint::All,
                known_from: None,
                known_until: None,
            }],
        }
    }
    #[test]
    fn cws_directions_are_exclusive_but_reference_editions_can_coexist() {
        let db = Connection::open_in_memory().unwrap();
        let left = scope(&["cws:direction:ru-main", "cws:edition:ja"]);
        let right = scope(&["cws:direction:ru-literary", "cws:edition:en"]);
        assert!(!structural_overlap(&db, &left, &right).unwrap());
        let compatible = scope(&["cws:direction:ru-main", "cws:edition:en"]);
        assert!(structural_overlap(&db, &left, &compatible).unwrap());
        let malformed = scope(&["cws:direction:ru-main", "cws:direction:ru-literary"]);
        assert!(direction(&malformed.scope_predicates).is_err());
        let padded = scope(&[" cws:direction:ru-main"]);
        assert!(direction(&padded.scope_predicates).is_err());
        let empty_identity = scope(&["cws:direction:"]);
        assert!(direction(&empty_identity.scope_predicates).is_err());
        assert!(direction(&left.scope_predicates).is_ok());
        assert!(matches!(
            validate(&db, &malformed),
            Err(Error::ApplicabilityConflict)
        ));
    }
}
