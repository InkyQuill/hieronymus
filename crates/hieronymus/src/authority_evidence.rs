//! Immutable snapshots and their typed identity/alignment bindings.
use crate::{authority_models::*, story_applicability::ApplicabilityV1};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBindingV1 {
    pub concept_id: i64,
    pub source_language: String,
    pub target_language: String,
    pub applicability: ApplicabilityV1,
    pub position_id: i64,
    pub paragraph_start: usize,
    pub paragraph_end: usize,
    pub identity_anchor: bool,
    pub aligned_source_id: Option<i64>,
    pub rendering: Option<String>,
    pub contradicts_rule: Option<i64>,
    pub conflict_kind: Option<String>,
}
use crate::authority_applicability as applicability;
use crate::story_applicability::{
    Eligibility, QueryMode, StoryApplicability, StoryQueryV1, Viewpoint,
};
use DecisionErrorV1 as Error;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
pub(crate) fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
pub(crate) struct ResolvedEvidence {
    pub id: i64,
    pub kind: EvidenceKind,
    pub identity: String,
    pub hash: String,
    pub start: usize,
    pub end: usize,
    pub binding: EvidenceBindingV1,
}
fn resolve(
    db: &Connection,
    e: &EvidenceRef,
    r: &DecisionRequestV1,
) -> Result<ResolvedEvidence, Error> {
    type StoredEvidence = (i64, String, String, String, i64, i64, String, String);
    let row:Option<StoredEvidence>=db.query_row("select series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json from evidence_records where id=?",[e.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
    let (series, kind, identity, h, start, end, content, binding) =
        row.ok_or(Error::EvidenceMismatch)?;
    if series != r.series_id
        || kind != e.kind.as_str()
        || h != e.content_hash
        || hash(&content) != h
        || usize::try_from(start).ok() != Some(e.span_start)
        || usize::try_from(end).ok() != Some(e.span_end)
        || content.get(e.span_start..e.span_end).is_none()
        || e.span_start >= e.span_end
        || identity.is_empty()
    {
        return Err(Error::EvidenceMismatch);
    }
    let binding: EvidenceBindingV1 =
        serde_json::from_str(&binding).map_err(|_| Error::EvidenceMismatch)?;
    if r.concept_id.is_some_and(|id| id != binding.concept_id)
        || binding.source_language != r.source_language
        || Some(&binding.target_language) != r.target_language.as_ref()
        || binding.applicability.series_id != series
        || !applicability::overlaps(db, &binding.applicability, &r.applicability)?
    {
        return Err(Error::EvidenceMismatch);
    }
    applicability::validate(db, &binding.applicability).map_err(|_| Error::EvidenceMismatch)?;
    let keys: Option<(i64, String, String)> = db
        .query_row(
            "select timeline_id,volume_key,chapter_key from story_positions where id=?",
            [binding.position_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (timeline, volume, chapter) = keys.ok_or(Error::EvidenceMismatch)?;
    let mut scopes = binding.applicability.scope_predicates.clone();
    scopes.extend([format!("volume:{volume}"), format!("chapter:{chapter}")]);
    let q = StoryQueryV1 {
        series_id: series,
        timeline_id: Some(timeline),
        position_id: Some(binding.position_id),
        viewpoint: Viewpoint::Narrator,
        scope_predicates: scopes,
        mode: QueryMode::Current,
    };
    if !matches!(
        StoryApplicability::evaluate(db, &binding.applicability, &q)
            .map_err(|_| Error::EvidenceMismatch)?,
        Eligibility::Current | Eligibility::FutureOrOutsideViewpoint
    ) {
        return Err(Error::EvidenceMismatch);
    }
    if e.kind == EvidenceKind::SourcePassage
        && (binding.paragraph_start > e.span_start
            || binding.paragraph_end < e.span_end
            || content
                .get(binding.paragraph_start..binding.paragraph_end)
                .is_none())
    {
        return Err(Error::EvidenceMismatch);
    }
    if e.kind == EvidenceKind::SourcePassage {
        let prefix = &content[..binding.paragraph_start];
        let suffix = &content[binding.paragraph_end..];
        if !(prefix.is_empty() || prefix.ends_with("\n\n") || prefix.ends_with("\r\n\r\n"))
            || !(suffix.is_empty() || suffix.starts_with("\n\n") || suffix.starts_with("\r\n\r\n"))
        {
            return Err(Error::EvidenceMismatch);
        }
    }
    if binding.conflict_kind.as_deref().is_some_and(|kind| {
        !matches!(
            kind,
            "erroneous_mapping" | "scope_mismatch" | "story_evolution"
        )
    }) || binding.contradicts_rule.is_some() != binding.conflict_kind.is_some()
    {
        return Err(Error::EvidenceMismatch);
    }
    if e.kind == EvidenceKind::AlignedRendering
        && binding.rendering.as_deref() != content.get(e.span_start..e.span_end)
    {
        return Err(Error::EvidenceMismatch);
    }
    if e.kind == EvidenceKind::AlignedRendering {
        let id = binding.aligned_source_id.ok_or(Error::EvidenceMismatch)?;
        let reference:Option<(String,i64,i64)>=db.query_row("select source_hash,span_start,span_end from evidence_records where id=? and kind='source_passage'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (hash, start, end) = reference.ok_or(Error::EvidenceMismatch)?;
        let source = resolve(
            db,
            &EvidenceRef {
                kind: EvidenceKind::SourcePassage,
                id,
                content_hash: hash,
                span_start: start as usize,
                span_end: end as usize,
            },
            r,
        )?;
        if source.binding.concept_id != binding.concept_id
            || source.binding.position_id != binding.position_id
            || source.binding.applicability != binding.applicability
            || source.binding.paragraph_start != binding.paragraph_start
            || source.binding.paragraph_end != binding.paragraph_end
        {
            return Err(Error::EvidenceMismatch);
        }
    }
    Ok(ResolvedEvidence {
        id: e.id,
        kind: e.kind,
        identity,
        hash: h,
        start: e.span_start,
        end: e.span_end,
        binding,
    })
}
pub(crate) fn resolve_all(
    db: &Connection,
    r: &DecisionRequestV1,
) -> Result<Vec<ResolvedEvidence>, Error> {
    r.evidence_refs.iter().map(|e| resolve(db, e, r)).collect()
}
pub(crate) fn learned_policy(
    db: &Connection,
    r: &DecisionRequestV1,
    e: &[ResolvedEvidence],
    rendering: &str,
    old: Option<i64>,
) -> Result<Vec<TentativeReason>, Error> {
    if r.applicability.volume_key.is_none()
        && (r.applicability.valid_from.is_none() || r.applicability.valid_until.is_none())
    {
        return Ok(vec![TentativeReason::InsufficientEvidence]);
    }
    let mut anchors: Vec<&ResolvedEvidence> = vec![];
    let mut contradiction = false;
    for source in e.iter().filter(|e| e.kind == EvidenceKind::SourcePassage) {
        let matches = e
            .iter()
            .filter(|a| {
                a.kind == EvidenceKind::AlignedRendering
                    && a.binding.aligned_source_id == Some(source.id)
            })
            .collect::<Vec<_>>();
        for a in &matches {
            if a.binding.concept_id != source.binding.concept_id
                || a.binding.applicability != source.binding.applicability
                || a.binding.position_id != source.binding.position_id
                || a.binding.paragraph_start != source.binding.paragraph_start
                || a.binding.paragraph_end != source.binding.paragraph_end
            {
                return Err(Error::EvidenceMismatch);
            }
        }
        if source.binding.contradicts_rule == old
            && old.is_some()
            && matches!(
                source.binding.conflict_kind.as_deref(),
                Some("erroneous_mapping" | "scope_mismatch" | "story_evolution")
            )
        {
            contradiction = true;
        }
        if !matches
            .iter()
            .any(|a| a.binding.rendering.as_deref() == Some(rendering))
        {
            continue;
        }
        // Require scope demonstrated by both anchors; recurrence cannot widen it.
        if !applicability::contains(db, &source.binding.applicability, &r.applicability)? {
            continue;
        }
        if anchors.iter().any(|a| {
            (a.identity == source.identity || a.hash == source.hash)
                && (a.hash != source.hash
                    || (a.start < source.end && source.start < a.end)
                    || (a.binding.paragraph_start < source.binding.paragraph_end
                        && source.binding.paragraph_start < a.binding.paragraph_end))
        }) {
            continue;
        }
        anchors.push(source);
    }
    // Evaluate the entire immutable selected-concept evidence set, including
    // contradictions omitted from the request. Unrelated concepts never count.
    let mut stmt=db.prepare("select id,source_hash,span_start,span_end,binding_json from evidence_records where series_id=?1 and kind='aligned_rendering'")?;
    let all = stmt
        .query_map([r.series_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, h, start, end, binding) in all {
        let Ok(b): Result<EvidenceBindingV1, _> = serde_json::from_str(&binding) else {
            continue;
        };
        if Some(b.concept_id) != r.concept_id
            || b.source_language != r.source_language
            || Some(&b.target_language) != r.target_language.as_ref()
            || !applicability::overlaps(db, &b.applicability, &r.applicability)?
        {
            continue;
        }
        let resolved = resolve(
            db,
            &EvidenceRef {
                kind: EvidenceKind::AlignedRendering,
                id,
                content_hash: h,
                span_start: start as usize,
                span_end: end as usize,
            },
            r,
        )?;
        if resolved.binding.rendering.as_deref() != Some(rendering) {
            // Prior supporting evidence may be superseded only by the required
            // structurally explained source contradiction of that prior rule.
            let is_old = if let Some(old) = old {
                db.query_row(
                    "select canonical_translation=?2 from term_rules where id=?1",
                    rusqlite::params![old, resolved.binding.rendering],
                    |r| r.get::<_, bool>(0),
                )?
            } else {
                false
            };
            if !(contradiction && is_old) {
                return Ok(vec![TentativeReason::ConflictingEvidence]);
            }
        }
    }
    if anchors.len() < 2
        || !anchors.iter().any(|a| a.binding.identity_anchor)
        || old.is_some() && !contradiction
    {
        return Ok(vec![TentativeReason::InsufficientEvidence]);
    }
    // Every demonstrated structural chapter requires its own two anchors.
    let mut positions=db.prepare("select id,volume_key,chapter_key from story_positions where timeline_id=? order by ordinal")?;
    let positions = positions
        .query_map([r.applicability.timeline_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut segments = std::collections::BTreeSet::new();
    for (id, volume, chapter) in &positions {
        let mut scopes = r.applicability.scope_predicates.clone();
        scopes.extend([format!("volume:{volume}"), format!("chapter:{chapter}")]);
        let mut q = StoryQueryV1 {
            series_id: r.series_id,
            timeline_id: r.applicability.timeline_id,
            position_id: Some(*id),
            viewpoint: Viewpoint::Narrator,
            scope_predicates: scopes,
            mode: QueryMode::Current,
        };
        for gate in &r.applicability.knowledge_gates {
            q.viewpoint = match gate.viewpoint {
                crate::story_applicability::KnowledgeViewpoint::Character(id) => {
                    Viewpoint::Character(id)
                }
                crate::story_applicability::KnowledgeViewpoint::Narrator => Viewpoint::Narrator,
                crate::story_applicability::KnowledgeViewpoint::All => Viewpoint::Unspecified,
            };
            if StoryApplicability::evaluate(db, &r.applicability, &q)
                .map_err(|_| Error::ApplicabilityConflict)?
                == Eligibility::Current
            {
                segments.insert((volume.clone(), chapter.clone()));
            }
        }
    }
    if segments.is_empty() {
        return Ok(vec![TentativeReason::UnknownOrder]);
    }
    for segment in segments {
        let count = anchors
            .iter()
            .filter(|a| {
                positions.iter().any(|(id, v, c)| {
                    *id == a.binding.position_id && (v, c) == (&segment.0, &segment.1)
                })
            })
            .count();
        if count < 2 {
            return Ok(vec![TentativeReason::InsufficientEvidence]);
        }
    }
    Ok(vec![])
}
