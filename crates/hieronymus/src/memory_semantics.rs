//! Durable, bounded embedding maintenance for memory records. The source rows
//! themselves are the work queue: missing/mismatched derived rows survive a crash.
use crate::{
    data_root::HieronymusConfig,
    db::open_migrated,
    memory_models::TranslationContext,
    semantic_embeddings::EmbeddingProvider,
    semantic_jobs::{AuthoritativeChunk, ChunkTokenizer},
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
};

const SOURCES: &str = "select 'crystal' kind,c.id,c.text,c.series_slug,c.source_language,c.target_language from crystals c where c.status='active' union all select 'short_term',m.id,m.text,s.series_slug,s.source_language,s.target_language from short_term_memories m join task_sessions s on s.id=m.session_id where m.archived_at is null and s.status in ('active','completed')";
pub const BATCH: usize = 32;

fn identity(provider: &dyn EmbeddingProvider) -> String {
    format!("{:x}", Sha256::digest(format!("{:?}", provider.identity())))
}
fn encode(
    provider: &mut dyn EmbeddingProvider,
    tokenizer: &mut dyn ChunkTokenizer,
    series: &str,
    text: &str,
    query: bool,
) -> Result<Vec<f32>, String> {
    let ids = tokenizer
        .tokenize(&AuthoritativeChunk {
            chunk_id: 0,
            series_slug: series.into(),
            text: text.into(),
        })
        .map_err(|e| e.to_string())?;
    let vector = if query {
        provider.embed_query_text(text, &ids)
    } else {
        provider.embed_document_text(text, &ids)
    }
    .map_err(|e| e.to_string())?;
    if vector.len() != provider.identity().dimensions()
        || vector.iter().any(|v| !v.is_finite())
        || vector.iter().all(|v| *v == 0.0)
    {
        return Err("invalid memory embedding".into());
    }
    Ok(vector)
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct IndexProgress {
    pub total: u64,
    pub indexed: u64,
    pub pending: u64,
}

fn ensure_vectors(db: &Connection) -> Result<(), String> {
    db.execute_batch("create table if not exists memory_vectors(kind text not null,id integer not null,identity text not null,source_text text not null,vector_json text not null,primary key(kind,id))").map_err(|e| e.to_string())
}

pub fn progress(
    config: &HieronymusConfig,
    provider: &dyn EmbeddingProvider,
) -> Result<IndexProgress, String> {
    let db = open_migrated(&config.database_path()).map_err(|e| e.to_string())?;
    ensure_vectors(&db)?;
    let (total, indexed): (i64,i64) = db.query_row(&format!("select count(*),coalesce(sum(v.id is not null and v.identity=?1 and v.source_text=s.text),0) from ({SOURCES}) s left join memory_vectors v on v.kind=s.kind and v.id=s.id"), [identity(provider)], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
    Ok(IndexProgress {
        total: total as u64,
        indexed: indexed as u64,
        pending: (total - indexed) as u64,
    })
}

/// One continuing background batch. Source rows are the durable queue.
/// No recall request performs inference or writes derived memory vectors.
pub fn maintain(
    config: &HieronymusConfig,
    provider: &mut dyn EmbeddingProvider,
    tokenizer: &mut dyn ChunkTokenizer,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    provider.verify_identity().map_err(|e| e.to_string())?;
    let db = open_migrated(&config.database_path()).map_err(|e| e.to_string())?;
    ensure_vectors(&db)?;
    let model = identity(provider);
    let sql = format!(
        "select s.kind,s.id,s.text,s.series_slug from ({SOURCES}) s left join memory_vectors v on v.kind=s.kind and v.id=s.id where v.id is null or v.identity!=?1 or v.source_text!=s.text order by s.kind,s.id limit ?2"
    );
    let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![model, BATCH as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    // Check corrupt derived vectors once the ordinary backlog is drained.
    if rows.is_empty() {
        let mut stmt = db.prepare(&format!("select s.kind,s.id,s.text,s.series_slug from ({SOURCES}) s join memory_vectors v on v.kind=s.kind and v.id=s.id where case when json_valid(v.vector_json) then json_array_length(v.vector_json)!=?1 or exists(select 1 from json_each(v.vector_json) where type not in ('real','integer')) or not exists(select 1 from json_each(v.vector_json) where value!=0) else 1 end limit ?2")).map_err(|e| e.to_string())?;
        rows = stmt
            .query_map(
                params![provider.identity().dimensions() as i64, BATCH as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
    }
    for (kind, id, text, series) in rows {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let vector = encode(provider, tokenizer, &series, &text, false)?;
        // A concurrent source edit/deletion cannot publish a stale vector.
        db.execute(&format!("insert into memory_vectors(kind,id,identity,source_text,vector_json) select ?1,?2,?3,?4,?5 where exists(select 1 from ({SOURCES}) where kind=?1 and id=?2 and text=?4) on conflict(kind,id) do update set identity=excluded.identity,source_text=excluded.source_text,vector_json=excluded.vector_json"),params![kind,id,model,text,serde_json::to_string(&vector).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    }
    // Bounded cleanup; queries independently exclude all archived/deleted rows.
    db.execute(&format!("delete from memory_vectors where rowid in(select v.rowid from memory_vectors v where not exists(select 1 from ({SOURCES}) s where s.kind=v.kind and s.id=v.id) limit 32)"),[]).map_err(|e|e.to_string())?;
    Ok(())
}

pub struct Candidates {
    pub ids: Vec<(String, i64)>,
    pub pending: bool,
    pub scan_incomplete: bool,
}

/// Rank authoritative current-text vectors. Claim/context checks run again in
/// the common coherent-read pipeline before any result or activation is exposed.
pub fn search(
    db: &Connection,
    context: &TranslationContext,
    query: &str,
    provider: &mut dyn EmbeddingProvider,
    tokenizer: &mut dyn ChunkTokenizer,
    limit: usize,
) -> Result<Candidates, String> {
    provider.verify_identity().map_err(|e| e.to_string())?;
    let model = identity(provider);
    let query = encode(provider, tokenizer, &context.series_slug, query, true)?;
    let filter = "s.series_slug=?1 and (s.source_language='' or s.source_language=?2) and (s.target_language='' or s.target_language=?3)";
    let mut stmt=db.prepare(&format!("select s.kind,s.id,v.vector_json from ({SOURCES}) s join memory_vectors v on v.kind=s.kind and v.id=s.id and v.source_text=s.text and v.identity=?4 where {filter} order by s.kind,s.id")).map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map(
            params![
                context.series_slug,
                context.source_language,
                context.target_language,
                model
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let pending:bool=db.query_row(&format!("select exists(select 1 from ({SOURCES}) s left join memory_vectors v on v.kind=s.kind and v.id=s.id and v.source_text=s.text and v.identity=?4 where {filter} and v.id is null)"),params![context.series_slug,context.source_language,context.target_language,model],|r|r.get(0)).map_err(|e|e.to_string())?;

    let story = crate::story_applicability::StoryApplicability::resolve_context(db, context)
        .map_err(|e| e.to_string())?;
    let mut ranked = BinaryHeap::new();
    let mut scored = 0;
    // Score the complete index without an ID cutoff, but perform expensive
    // claim hydration only for the strongest bounded candidate pool.
    for row in rows {
        let (kind, id, encoded) = row.map_err(|e| e.to_string())?;
        let vector: Vec<f32> =
            serde_json::from_str(&encoded).map_err(|_| "invalid persisted memory vector")?;
        if vector.len() != query.len() || vector.iter().any(|v| !v.is_finite()) {
            return Err("invalid persisted memory vector".into());
        }
        let dot: f32 = query.iter().zip(&vector).map(|(a, b)| a * b).sum();
        let norm = (query.iter().map(|v| v * v).sum::<f32>()
            * vector.iter().map(|v| v * v).sum::<f32>())
        .sqrt();
        if norm == 0.0 {
            return Err("invalid persisted memory vector".into());
        }
        let score = dot / norm;
        if !score.is_finite() {
            return Err("invalid persisted memory vector norm".into());
        }
        scored += 1;
        retain_candidate(
            &mut ranked,
            Ranked { score, kind, id },
            crate::claim_reads::CANDIDATE_BUDGET,
        );
    }
    let mut candidates = ranked.into_iter().map(|Reverse(v)| v).collect::<Vec<_>>();
    candidates.sort_unstable_by(|a, b| b.cmp(a));
    let mut ids = Vec::new();
    for candidate in candidates {
        if ids.len() >= limit {
            break;
        }
        let target = if candidate.kind == "crystal" {
            crate::claim_reads::ClaimTarget::Crystal(candidate.id)
        } else {
            crate::claim_reads::ClaimTarget::ShortTerm(candidate.id)
        };
        let annotation =
            crate::claim_reads::read_annotation(db, target, &story).map_err(|e| e.to_string())?;
        if matches!(
            annotation.disposition,
            crate::claim_reads::ClaimDisposition::Current
                | crate::claim_reads::ClaimDisposition::Qualified(_)
        ) {
            ids.push((candidate.kind, candidate.id));
        }
    }
    let scan_incomplete = ids.len() < limit && scored > crate::claim_reads::CANDIDATE_BUDGET;
    Ok(Candidates {
        ids,
        pending,
        scan_incomplete,
    })
}

/// Higher score, then lexicographically smaller identity, wins a tie.
struct Ranked {
    score: f32,
    kind: String,
    id: i64,
}
impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Ranked {}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.kind.cmp(&self.kind))
            .then_with(|| other.id.cmp(&self.id))
    }
}

/// Retain at most the strongest `budget` candidates, with the weakest at the root.
fn retain_candidate(heap: &mut BinaryHeap<Reverse<Ranked>>, candidate: Ranked, budget: usize) {
    if budget == 0 {
        return;
    }
    if heap.len() < budget {
        heap.push(Reverse(candidate));
    } else if heap.peek().is_some_and(|weakest| candidate > weakest.0) {
        heap.pop();
        heap.push(Reverse(candidate));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heap_keeps_late_best_candidates_with_stable_ties() {
        let mut heap = BinaryHeap::new();
        for (id, score) in [(9, 0.1), (3, 0.5), (2, 0.5), (100, 0.9), (1, 0.5)] {
            retain_candidate(
                &mut heap,
                Ranked {
                    id,
                    score,
                    kind: "crystal".into(),
                },
                3,
            );
            assert!(heap.len() <= 3);
        }
        let mut values = heap.into_iter().map(|Reverse(v)| v).collect::<Vec<_>>();
        values.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(
            values.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![100, 1, 2]
        );
    }
}
