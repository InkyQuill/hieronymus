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

/// At most one bounded maintenance batch, with inference outside SQLite writes.
pub fn prepare(
    config: &HieronymusConfig,
    series: &str,
    provider: &mut dyn EmbeddingProvider,
    tokenizer: &mut dyn ChunkTokenizer,
) -> Result<(), String> {
    provider.verify_identity().map_err(|e| e.to_string())?;
    let db = open_migrated(&config.database_path()).map_err(|e| e.to_string())?;
    db.execute_batch("create table if not exists memory_vectors(kind text not null,id integer not null,identity text not null,source_text text not null,vector_json text not null,primary key(kind,id))").map_err(|e|e.to_string())?;
    let model = identity(provider);
    let sql = format!(
        "select s.kind,s.id,s.text from ({SOURCES}) s left join memory_vectors v on v.kind=s.kind and v.id=s.id where s.series_slug=?1 and (v.id is null or v.identity!=?2 or v.source_text!=s.text or case when json_valid(v.vector_json) then json_array_length(v.vector_json)!=?4 or exists(select 1 from json_each(v.vector_json) where type not in ('real','integer')) or not exists(select 1 from json_each(v.vector_json) where value!=0) else 1 end) order by s.kind,s.id limit ?3"
    );
    let mut stmt = db.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![
                series,
                model,
                BATCH as i64,
                provider.identity().dimensions() as i64
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    for (kind, id, text) in rows {
        let vector = encode(provider, tokenizer, series, &text, false)?;
        // A concurrent source edit/deletion cannot publish a stale vector.
        db.execute(&format!("insert into memory_vectors(kind,id,identity,source_text,vector_json) select ?1,?2,?3,?4,?5 where exists(select 1 from ({SOURCES}) where kind=?1 and id=?2 and text=?4) on conflict(kind,id) do update set identity=excluded.identity,source_text=excluded.source_text,vector_json=excluded.vector_json"),params![kind,id,model,text,serde_json::to_string(&vector).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    }
    // Bounded cleanup; queries independently exclude all archived/deleted rows.
    db.execute(&format!("delete from memory_vectors where rowid in(select v.rowid from memory_vectors v where not exists(select 1 from ({SOURCES}) s where s.kind=v.kind and s.id=v.id) limit 32)"),[]).map_err(|e|e.to_string())?;
    Ok(())
}

pub struct Candidates {
    pub ids: Vec<(String, i64)>,
    pub incomplete: bool,
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
    let incomplete = pending;
    let story = crate::story_applicability::StoryApplicability::resolve_context(db, context)
        .map_err(|e| e.to_string())?;
    let mut ranked = Vec::new();
    // Stream the complete index, retaining only eligible top-k. No fixed ID
    // prefix can permanently hide later memories.
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
        let target = if kind == "crystal" {
            crate::claim_reads::ClaimTarget::Crystal(id)
        } else {
            crate::claim_reads::ClaimTarget::ShortTerm(id)
        };
        let annotation =
            crate::claim_reads::read_annotation(db, target, &story).map_err(|e| e.to_string())?;
        if matches!(
            annotation.disposition,
            crate::claim_reads::ClaimDisposition::Current
                | crate::claim_reads::ClaimDisposition::Qualified(_)
        ) {
            ranked.push((dot / norm, kind, id));
            ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            ranked.truncate(limit);
        }
    }
    let ids = ranked.into_iter().map(|(_, kind, id)| (kind, id)).collect();
    Ok(Candidates { ids, incomplete })
}
