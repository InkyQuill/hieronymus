//! Deterministic narrative order, world validity and positive viewpoint knowledge.
use crate::memory_models::{TranslationContext, normalize_story_scopes};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Viewpoint {
    Narrator,
    Character(i64),
    #[default]
    Unspecified,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryMode {
    #[default]
    Current,
    OmniscientResearch,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryQueryV1 {
    pub series_id: i64,
    pub timeline_id: Option<i64>,
    pub position_id: Option<i64>,
    pub viewpoint: Viewpoint,
    pub scope_predicates: Vec<String>,
    pub mode: QueryMode,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetadataState {
    Resolved,
    Unspecified,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeViewpoint {
    Narrator,
    Character(i64),
    All,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeGateV1 {
    pub viewpoint: KnowledgeViewpoint,
    pub known_from: Option<i64>,
    pub known_until: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicabilityV1 {
    pub series_id: i64,
    pub timeline_id: Option<i64>,
    pub volume_key: Option<String>,
    pub chapter_key: Option<String>,
    pub scope_predicates: Vec<String>,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub metadata_state: MetadataState,
    pub knowledge_gates: Vec<KnowledgeGateV1>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Current,
    FutureOrOutsideViewpoint,
    Unknown,
    Excluded,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryOrdering {
    Before,
    Equal,
    After,
    Unknown,
}
#[derive(Debug, thiserror::Error)]
pub enum ApplicabilityError {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("wrong or missing series")]
    WrongSeries,
    #[error("wrong timeline")]
    WrongTimeline,
    #[error("invalid half-open interval")]
    InvalidInterval,
    #[error("ambiguous story position")]
    AmbiguousPosition,
    #[error("timeline revision conflict")]
    RevisionConflict,
    #[error("missing or wrong-series immutable evidence")]
    InvalidEvidence,
    #[error("duplicate manifest position")]
    DuplicatePosition,
    #[error("manifest omits an existing position; explicit retirement is required")]
    OmittedPosition,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestPosition {
    pub volume_key: String,
    pub chapter_key: String,
    pub scene_key: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderManifest {
    pub series_id: i64,
    pub timeline_id: i64,
    pub positions: Vec<ManifestPosition>,
}
pub struct StoryApplicability;
fn timeline(db: &Connection, id: i64, series: i64) -> Result<(), ApplicabilityError> {
    let owner: Option<i64> = db
        .query_row(
            "select series_id from story_timelines where id=?",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if owner != Some(series) {
        return Err(ApplicabilityError::WrongTimeline);
    }
    Ok(())
}
fn position(db: &Connection, id: i64) -> Result<Option<(i64, i64)>, ApplicabilityError> {
    Ok(db
        .query_row(
            "select timeline_id,ordinal from story_positions where id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}
fn endpoint(
    db: &Connection,
    id: Option<i64>,
    tl: Option<i64>,
) -> Result<Option<i64>, ApplicabilityError> {
    match id {
        None => Ok(None),
        Some(id) => match position(db, id)? {
            None => Ok(None),
            Some((t, o)) => {
                if Some(t) != tl {
                    return Err(ApplicabilityError::WrongTimeline);
                }
                Ok(Some(o))
            }
        },
    }
}
fn interval(
    db: &Connection,
    from: Option<i64>,
    until: Option<i64>,
    tl: Option<i64>,
) -> Result<(Option<i64>, Option<i64>, bool), ApplicabilityError> {
    let a = endpoint(db, from, tl)?;
    let b = endpoint(db, until, tl)?;
    if matches!((a,b),(Some(a),Some(b)) if a>=b) {
        return Err(ApplicabilityError::InvalidInterval);
    }
    Ok((
        a,
        b,
        (from.is_some() && a.is_none()) || (until.is_some() && b.is_none()),
    ))
}
impl StoryApplicability {
    /// Compare stable IDs in one narrative timeline; absent IDs remain unordered.
    pub fn compare(
        db: &Connection,
        a: Option<i64>,
        b: Option<i64>,
    ) -> Result<StoryOrdering, ApplicabilityError> {
        let (Some(a), Some(b)) = (a, b) else {
            return Ok(StoryOrdering::Unknown);
        };
        let (Some((ta, a)), Some((tb, b))) = (position(db, a)?, position(db, b)?) else {
            return Ok(StoryOrdering::Unknown);
        };
        if ta != tb {
            return Err(ApplicabilityError::WrongTimeline);
        }
        Ok(match a.cmp(&b) {
            std::cmp::Ordering::Less => StoryOrdering::Before,
            std::cmp::Ordering::Equal => StoryOrdering::Equal,
            std::cmp::Ordering::Greater => StoryOrdering::After,
        })
    }
    /// Resolve exact identity keys. Multiple timelines or unplaced scenes remain unknown.
    pub fn resolve_context(
        db: &Connection,
        c: &TranslationContext,
    ) -> Result<StoryQueryV1, ApplicabilityError> {
        let series_id = db
            .query_row(
                "select id from series where slug=?",
                [&c.series_slug],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(ApplicabilityError::WrongSeries)?;
        let timeline_id = if let Some(t) = c.story_timeline_id {
            timeline(db, t, series_id)?;
            Some(t)
        } else {
            let mut s =
                db.prepare("select id from story_timelines where series_id=? order by id")?;
            let ids = s
                .query_map([series_id], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            if ids.len() == 1 { Some(ids[0]) } else { None }
        };
        let legacy_scopes = normalize_story_scopes(c.story_scopes.iter().map(String::as_str));
        let key = |prefix: &str, explicit: &str| -> Result<Option<String>, ApplicabilityError> {
            if !explicit.is_empty() {
                return Ok(Some(explicit.to_owned()));
            }
            let keys: std::collections::BTreeSet<_> = legacy_scopes
                .iter()
                .filter_map(|s| s.strip_prefix(prefix))
                .collect();
            if keys.len() > 1 {
                return Err(ApplicabilityError::AmbiguousPosition);
            }
            Ok(keys.first().map(|s| s.to_string()))
        };
        let volume = key("volume:", &c.volume)?;
        let chapter = key("chapter:", &c.chapter)?;
        let mut scopes: Vec<String> = legacy_scopes
            .into_iter()
            .filter(|s| !s.starts_with("volume:") && !s.starts_with("chapter:"))
            .collect();
        for (prefix, value) in [("volume:", &volume), ("chapter:", &chapter)] {
            if let Some(value) = value {
                scopes.push(format!("{prefix}{value}"));
            }
        }
        let scene = c.story_scene_key.as_deref();
        let position_id = if let (Some(t), Some(ch)) = (timeline_id, chapter) {
            let mut s=db.prepare("select id from story_positions where timeline_id=?1 and chapter_key=?2 and (?3 is null or volume_key=?3) and (?4 is null or scene_key=?4)")?;
            let ids = s
                .query_map(params![t, ch, volume, scene], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            // Multiple scenes are unresolved within-chapter order; repeated volumes are ambiguous.
            if ids.len() > 1 {
                let volumes:i64=db.query_row("select count(distinct volume_key) from story_positions where timeline_id=?1 and chapter_key=?2 and (?3 is null or volume_key=?3)",params![t,ch,volume],|r|r.get(0))?;
                if volumes > 1 {
                    return Err(ApplicabilityError::AmbiguousPosition);
                }
                None
            } else {
                ids.first().copied()
            }
        } else {
            None
        };
        Ok(StoryQueryV1 {
            series_id,
            timeline_id,
            position_id,
            viewpoint: c.story_viewpoint,
            scope_predicates: scopes,
            mode: c.story_query_mode,
        })
    }
    /// Research mode retains the same classification: callers may label future hits separately.
    pub fn evaluate(
        db: &Connection,
        a: &ApplicabilityV1,
        q: &StoryQueryV1,
    ) -> Result<Eligibility, ApplicabilityError> {
        if a.series_id != q.series_id {
            return Err(ApplicabilityError::WrongSeries);
        }
        if let Some(t) = a.timeline_id {
            timeline(db, t, a.series_id)?;
        }
        if let Some(t) = q.timeline_id {
            timeline(db, t, q.series_id)?;
        }
        if a.timeline_id.is_some() && q.timeline_id.is_some() && a.timeline_id != q.timeline_id {
            return Err(ApplicabilityError::WrongTimeline);
        }
        let mut required = normalize_story_scopes(a.scope_predicates.iter().map(String::as_str));
        if let Some(v) = &a.volume_key {
            required.push(format!("volume:{v}"));
        }
        if let Some(c) = &a.chapter_key {
            required.push(format!("chapter:{c}"));
        }
        if required.iter().any(|s| !q.scope_predicates.contains(s)) {
            return Ok(Eligibility::Excluded);
        }
        if let Some(id) = q.position_id {
            let keys: Option<(String, String)> = db
                .query_row(
                    "select volume_key,chapter_key from story_positions where id=?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((volume, chapter)) = keys
                && required.iter().any(|s| {
                    s.strip_prefix("volume:").is_some_and(|v| v != volume)
                        || s.strip_prefix("chapter:").is_some_and(|c| c != chapter)
                })
            {
                return Ok(Eligibility::Excluded);
            }
        }
        let series_slug: String = db
            .query_row("select slug from series where id=?", [a.series_id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or(ApplicabilityError::WrongSeries)?;
        for id in a
            .knowledge_gates
            .iter()
            .filter_map(|g| match g.viewpoint {
                KnowledgeViewpoint::Character(id) => Some(id),
                _ => None,
            })
            .chain(match q.viewpoint {
                Viewpoint::Character(id) => Some(id),
                _ => None,
            })
        {
            let scope: Option<(String, String)> = db
                .query_row(
                    "select scope_type,scope_key from concepts where id=?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if !scope.is_some_and(|(kind, key)| {
                kind == "global" || (kind == "series" && key == format!("series:{series_slug}"))
            }) {
                return Err(ApplicabilityError::WrongSeries);
            }
        }
        let (from, until, missing) = interval(db, a.valid_from, a.valid_until, a.timeline_id)?;
        let gates = a
            .knowledge_gates
            .iter()
            .map(|g| interval(db, g.known_from, g.known_until, a.timeline_id))
            .collect::<Result<Vec<_>, _>>()?;
        let p = endpoint(db, q.position_id, q.timeline_id)?;
        if a.metadata_state == MetadataState::Unspecified
            || a.timeline_id.is_none()
            || q.timeline_id.is_none()
            || missing
            || p.is_none()
        {
            return Ok(Eligibility::Unknown);
        }
        let p = p.expect("checked position");
        if until.is_some_and(|u| p >= u) {
            return Ok(Eligibility::Excluded);
        }
        if from.is_some_and(|f| p < f) {
            return Ok(Eligibility::FutureOrOutsideViewpoint);
        }
        let mut unknown = false;
        for (g, (from, until, missing)) in a.knowledge_gates.iter().zip(gates) {
            let matches = match (q.viewpoint, g.viewpoint) {
                (Viewpoint::Character(a), KnowledgeViewpoint::Character(b)) => a == b,
                (Viewpoint::Narrator, KnowledgeViewpoint::Narrator) => true,
                (Viewpoint::Unspecified | Viewpoint::Narrator, KnowledgeViewpoint::All) => true,
                _ => false,
            };
            if !matches {
                continue;
            }
            if missing {
                unknown = true;
                continue;
            }
            if from.is_none_or(|f| p >= f) && until.is_none_or(|u| p < u) {
                return Ok(Eligibility::Current);
            }
        }
        Ok(if unknown {
            Eligibility::Unknown
        } else {
            Eligibility::FutureOrOutsideViewpoint
        })
    }
    /// Replace order within the caller's transaction. Existing positions must all remain.
    /// A savepoint makes validation/write failure atomic without committing the caller.
    pub fn register_order_tx(
        tx: &Transaction<'_>,
        manifest: &OrderManifest,
        evidence_id: i64,
        expected_timeline_revision: u64,
    ) -> Result<u64, ApplicabilityError> {
        tx.execute_batch("SAVEPOINT story_order_registration")?;
        let result = register_order(tx, manifest, evidence_id, expected_timeline_revision);
        if result.is_err() {
            tx.execute_batch("ROLLBACK TO story_order_registration")?;
        }
        tx.execute_batch("RELEASE story_order_registration")?;
        result
    }
}
fn register_order(
    tx: &Transaction<'_>,
    m: &OrderManifest,
    evidence: i64,
    expected: u64,
) -> Result<u64, ApplicabilityError> {
    timeline(tx, m.timeline_id, m.series_id)?;
    let evidence_series = tx
        .query_row(
            "select series_id from evidence_records where id=?",
            [evidence],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if evidence_series != Some(m.series_id) {
        return Err(ApplicabilityError::InvalidEvidence);
    }
    let mut seen = std::collections::HashSet::new();
    if m.positions.iter().any(|p| !seen.insert(p)) {
        return Err(ApplicabilityError::DuplicatePosition);
    }
    let existing = {
        let mut s = tx.prepare(
            "select volume_key,chapter_key,scene_key from story_positions where timeline_id=?",
        )?;
        s.query_map([m.timeline_id], |r| {
            Ok(ManifestPosition {
                volume_key: r.get(0)?,
                chapter_key: r.get(1)?,
                scene_key: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    if existing.iter().any(|p| !seen.contains(p)) {
        return Err(ApplicabilityError::OmittedPosition);
    }
    if tx.execute(
        "update story_timelines set revision=revision+1 where id=?1 and revision=?2",
        params![
            m.timeline_id,
            i64::try_from(expected)
                .ok()
                .filter(|v| *v < i64::MAX)
                .ok_or(ApplicabilityError::RevisionConflict)?
        ],
    )? != 1
    {
        return Err(ApplicabilityError::RevisionConflict);
    }
    let offset: i64 = tx.query_row(
        "select coalesce(max(ordinal),0)+?2+1 from story_positions where timeline_id=?1",
        params![m.timeline_id, m.positions.len() as i64],
        |r| r.get(0),
    )?;
    tx.execute(
        "update story_positions set ordinal=ordinal+?2 where timeline_id=?1",
        params![m.timeline_id, offset],
    )?;
    for (i, p) in m.positions.iter().enumerate() {
        tx.execute("insert into story_positions(timeline_id,volume_key,chapter_key,scene_key,ordinal,evidence_id) values(?1,?2,?3,?4,?5,?6) on conflict(timeline_id,volume_key,chapter_key,scene_key) do update set ordinal=excluded.ordinal,evidence_id=excluded.evidence_id",params![m.timeline_id,p.volume_key,p.chapter_key,p.scene_key,i as i64,evidence])?;
    }
    let invalid:bool=tx.query_row("select exists(select 1 from applicabilities a join story_positions f on f.id=a.valid_from join story_positions u on u.id=a.valid_until where a.timeline_id=?1 and f.ordinal>=u.ordinal) or exists(select 1 from knowledge_gates g join applicabilities a on a.id=g.applicability_id join story_positions f on f.id=g.known_from join story_positions u on u.id=g.known_until where a.timeline_id=?1 and f.ordinal>=u.ordinal)",[m.timeline_id],|r|r.get(0))?;
    if invalid {
        return Err(ApplicabilityError::InvalidInterval);
    }
    tx.execute("insert into authority_state(series_id,revision) values(?1,1) on conflict(series_id) do update set revision=revision+1",[m.series_id])?;
    Ok(expected + 1)
}
