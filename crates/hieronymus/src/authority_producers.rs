//! Narrow, server-owned import of immutable order and full-document evidence.
use crate::authority_evidence::{hash, validate_capture};
use crate::{authority::EvidenceBindingV1, authority_models::*, story_applicability::*};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
#[derive(Debug, Clone)]
pub enum SnapshotInput {
    File {
        path: PathBuf,
        expected_hash: String,
    },
    Retained {
        evidence_id: i64,
        expected_hash: String,
    },
}
/// A bounded assertion about an absolute selection, never document content.
#[derive(Debug, Clone)]
pub struct DocumentSelection {
    pub start: usize,
    pub end: usize,
    pub expected_text: String,
}
#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    #[error("invalid snapshot or binding")]
    Invalid,
    #[error("immutable evidence binding conflict")]
    BindingConflict,
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Applicability(#[from] ApplicabilityError),
    #[error(transparent)]
    Evidence(#[from] DecisionErrorV1),
}
#[derive(Debug, Serialize)]
pub struct RegisteredPosition {
    pub id: i64,
    pub position: ManifestPosition,
}
#[derive(Debug, Serialize)]
pub struct RegisteredManifest {
    pub timeline_id: i64,
    pub revision: u64,
    pub evidence_id: i64,
    pub positions: Vec<RegisteredPosition>,
}
#[derive(Debug, Serialize)]
pub struct CapturedEvidence {
    pub reference: EvidenceRef,
    pub source_identity: String,
    pub selected_text: String,
    pub paragraph_start: usize,
    pub paragraph_end: usize,
    pub paragraph_text: Option<String>,
}
pub struct EvidenceProducer<'a> {
    db: &'a mut Connection,
}
impl<'a> EvidenceProducer<'a> {
    pub fn new(db: &'a mut Connection) -> Self {
        Self { db }
    }
    /// Read a full manifest exactly once; its parsed ordered list is the sole order authority.
    pub fn register_manifest(
        &mut self,
        series: i64,
        input: &SnapshotInput,
        timeline: Option<i64>,
        expected: u64,
    ) -> Result<RegisteredManifest, ProducerError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let snapshot = read_snapshot(&tx, series, input, true)?;
        let manifest: ManifestDocument = serde_json::from_str(&snapshot.content)?;
        if manifest.version != 1
            || manifest.series_id != series
            || manifest.timeline_name.trim().is_empty()
            || manifest.positions.is_empty()
            || manifest
                .positions
                .iter()
                .any(|p| p.volume_key.trim().is_empty() || p.chapter_key.trim().is_empty())
        {
            return Err(ProducerError::Invalid);
        }
        let id = if let Some(id) = timeline {
            let valid:bool=tx.query_row("select exists(select 1 from story_timelines where id=?1 and series_id=?2 and name=?3)",params![id,series,manifest.timeline_name],|r|r.get(0))?;
            if !valid {
                return Err(ProducerError::Invalid);
            }
            id
        } else {
            let existing: Option<i64> = tx
                .query_row(
                    "select id from story_timelines where series_id=?1 and name=?2",
                    params![series, manifest.timeline_name],
                    |r| r.get(0),
                )
                .optional()?;
            match existing {
                Some(id) => id,
                None => {
                    if expected != 0 {
                        return Err(ApplicabilityError::RevisionConflict.into());
                    }
                    tx.execute(
                        "insert into story_timelines(series_id,name) values(?1,?2)",
                        params![series, manifest.timeline_name],
                    )?;
                    tx.last_insert_rowid()
                }
            }
        };
        let binding =
            serde_json::json!({"type":"order_manifest_v1","series_id":series,"timeline_id":id});
        let evidence = insert_evidence(
            &tx,
            series,
            &snapshot,
            EvidenceKind::Observation,
            0,
            snapshot.content.len(),
            &binding,
        )?;
        let order = OrderManifest {
            series_id: series,
            timeline_id: id,
            positions: manifest.positions,
        };
        let revision = StoryApplicability::register_order_tx(&tx, &order, evidence.id, expected)?;
        let positions = {
            let mut stmt=tx.prepare("select id,volume_key,chapter_key,scene_key from story_positions where timeline_id=? order by ordinal")?;
            stmt.query_map([id], |r| {
                Ok(RegisteredPosition {
                    id: r.get(0)?,
                    position: ManifestPosition {
                        volume_key: r.get(1)?,
                        chapter_key: r.get(2)?,
                        scene_key: r.get(3)?,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        tx.commit()?;
        Ok(RegisteredManifest {
            timeline_id: id,
            revision,
            evidence_id: evidence.id,
            positions,
        })
    }
    /// Capture a bounded absolute UTF-8 selection from a complete file or retained snapshot.
    /// File paths are canonicalized for stable identity, and bytes are never normalized.
    /// The application adapter must treat File as an explicit import, never as proof of
    /// a previous RAG ingestion. Retained IDs never reread their historical mutable path.
    /// The producer derives paragraph bounds (ignoring draft paragraph fields).
    /// An aligned rendering inherits its linked source paragraph bounds; its own
    /// selection still indexes the complete target snapshot. Captures are atomic;
    /// the source and a later alignment are two independently validated captures.
    pub fn capture(
        &mut self,
        series: i64,
        input: &SnapshotInput,
        kind: EvidenceKind,
        selection: &DocumentSelection,
        binding: &EvidenceBindingV1,
    ) -> Result<CapturedEvidence, ProducerError> {
        let (start, end) = (selection.start, selection.end);
        if !matches!(
            kind,
            EvidenceKind::SourcePassage | EvidenceKind::AlignedRendering
        ) {
            return Err(ProducerError::Invalid);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let snapshot = read_snapshot(&tx, series, input, false)?;
        let selected = snapshot
            .content
            .get(start..end)
            .filter(|s| !s.is_empty() && s.len() <= MAX_PROJECTION)
            .ok_or(ProducerError::Invalid)?;
        if selected != selection.expected_text {
            return Err(ProducerError::Invalid);
        }
        let mut binding = binding.clone();
        let paragraph_text = if kind == EvidenceKind::SourcePassage {
            if binding.aligned_source_id.is_some() || binding.rendering.is_some() {
                return Err(ProducerError::Invalid);
            }
            let (ps, pe) =
                paragraph(&snapshot.content, start, end).ok_or(ProducerError::Invalid)?;
            binding.paragraph_start = ps;
            binding.paragraph_end = pe;
            (pe - ps <= MAX_PROJECTION).then(|| snapshot.content[ps..pe].to_owned())
        } else {
            let source_id = binding.aligned_source_id.ok_or(ProducerError::Invalid)?;
            let source:Option<(String,String)>=tx.query_row("select binding_json,content from evidence_records where id=?1 and series_id=?2 and kind='source_passage'",params![source_id,series],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let (source_binding, source_content) = source.ok_or(ProducerError::Invalid)?;
            let source: EvidenceBindingV1 = serde_json::from_str(&source_binding)?;
            binding.paragraph_start = source.paragraph_start;
            binding.paragraph_end = source.paragraph_end;
            if binding.rendering.as_ref().is_some_and(|r| r != selected) {
                return Err(ProducerError::Invalid);
            }
            binding.rendering = Some(selected.to_owned());
            source_content
                .get(source.paragraph_start..source.paragraph_end)
                .filter(|p| p.len() <= MAX_PROJECTION)
                .map(str::to_owned)
        };
        let reference = insert_evidence(
            &tx,
            series,
            &snapshot,
            kind,
            start,
            end,
            &serde_json::to_value(&binding)?,
        )?;
        validate_capture(&tx, &reference, &binding, series)?;
        let result = CapturedEvidence {
            reference,
            source_identity: snapshot.identity,
            selected_text: selected.to_owned(),
            paragraph_start: binding.paragraph_start,
            paragraph_end: binding.paragraph_end,
            paragraph_text,
        };
        tx.commit()?;
        Ok(result)
    }
}
const MAX_DOCUMENT: u64 = 64 * 1024 * 1024;
const MAX_PROJECTION: usize = 8192;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    version: u8,
    series_id: i64,
    timeline_name: String,
    positions: Vec<ManifestPosition>,
}
struct Snapshot {
    identity: String,
    hash: String,
    content: String,
}
fn read_snapshot(
    db: &Connection,
    series: i64,
    input: &SnapshotInput,
    manifest: bool,
) -> Result<Snapshot, ProducerError> {
    let (snapshot, expected) = match input {
        SnapshotInput::File {
            path,
            expected_hash,
        } => {
            let path = path.canonicalize()?;
            if !std::fs::metadata(&path)?.is_file() {
                return Err(ProducerError::Invalid);
            }
            let file = std::fs::File::open(&path)?;
            if !file.metadata()?.is_file() {
                return Err(ProducerError::Invalid);
            }
            let mut bytes = Vec::new();
            file.take(MAX_DOCUMENT + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_DOCUMENT {
                return Err(ProducerError::Invalid);
            }
            let content = String::from_utf8(bytes).map_err(|_| ProducerError::Invalid)?;
            let identity = format!("file:{}", path.to_str().ok_or(ProducerError::Invalid)?);
            (
                Snapshot {
                    identity,
                    hash: hash(&content),
                    content,
                },
                expected_hash,
            )
        }
        SnapshotInput::Retained {
            evidence_id,
            expected_hash,
        } => {
            let row:Option<(String,String,String,String)>=db.query_row("select source_identity,source_hash,content,kind from evidence_records where id=?1 and series_id=?2",params![evidence_id,series],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
            let (identity, hash, content, kind) = row.ok_or(ProducerError::Invalid)?;
            if !(matches!(kind.as_str(), "source_passage" | "aligned_rendering")
                || manifest && kind == "observation")
            {
                return Err(ProducerError::Invalid);
            }
            (
                Snapshot {
                    identity,
                    hash,
                    content,
                },
                expected_hash,
            )
        }
    };
    if snapshot.identity.is_empty()
        || snapshot.content.is_empty()
        || snapshot.hash != *expected
        || hash(&snapshot.content) != snapshot.hash
    {
        return Err(ProducerError::Invalid);
    }
    Ok(snapshot)
}
fn insert_evidence(
    db: &Connection,
    series: i64,
    snapshot: &Snapshot,
    kind: EvidenceKind,
    start: usize,
    end: usize,
    binding: &serde_json::Value,
) -> Result<EvidenceRef, ProducerError> {
    let old:Option<(i64,i64,String,String)>=db.query_row("select id,series_id,binding_json,content from evidence_records where kind=?1 and source_identity=?2 and source_hash=?3 and span_start=?4 and span_end=?5",params![kind.as_str(),snapshot.identity,snapshot.hash,start as i64,end as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let id = if let Some((id, owner, old_binding, content)) = old {
        if owner != series
            || serde_json::from_str::<serde_json::Value>(&old_binding)? != *binding
            || content != snapshot.content
        {
            return Err(ProducerError::BindingConflict);
        }
        id
    } else {
        db.execute("insert into evidence_records(series_id,kind,source_identity,source_hash,span_start,span_end,content,binding_json,created_at) values(?1,?2,?3,?4,?5,?6,?7,?8,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![series,kind.as_str(),snapshot.identity,snapshot.hash,start as i64,end as i64,snapshot.content,binding.to_string()])?;
        db.last_insert_rowid()
    };
    Ok(EvidenceRef {
        kind,
        id,
        content_hash: snapshot.hash.clone(),
        span_start: start,
        span_end: end,
    })
}
fn paragraph(content: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let mut beginning = 0;
    for (index, _) in content
        .match_indices("\n\n")
        .chain(content.match_indices("\r\n\r\n"))
    {
        let length = if content[index..].starts_with("\r") {
            4
        } else {
            2
        };
        if index + length <= start {
            beginning = beginning.max(index + length);
        }
    }
    let ending = content[beginning..]
        .match_indices("\n\n")
        .chain(content[beginning..].match_indices("\r\n\r\n"))
        .map(|(i, _)| beginning + i)
        .min()
        .unwrap_or(content.len());
    (beginning <= start && start < end && end <= ending).then_some((beginning, ending))
}
