//! Browser-only correction discovery. Lists immutable source occurrences and
//! actual authority rules; legacy strict-term projection IDs never enter here.
use super::{AppError, Application, decode, domain};
use hieronymus::{
    authority::EvidenceBindingV1, authority_models::DecisionErrorV1, claim_reads::ClaimTarget,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    target: Option<ClaimTarget>,
    rule_id: Option<i64>,
}
pub(crate) fn options(app: &Application, input: &Value) -> Result<Value, AppError> {
    let input: Options = decode(input)?;
    if input.target.is_some() && input.rule_id.is_some() {
        return Err(AppError::Authority(DecisionErrorV1::InvalidRequest));
    }
    let mut db = hieronymus::db::open_migrated(&app.config().database_path()).map_err(domain)?;
    let tx = db.transaction().map_err(domain)?;
    let allowed: Option<Vec<i64>> = if let Some(target) = input.target {
        let (column, id) = match target {
            ClaimTarget::ShortTerm(id) => ("short_term_id", id),
            ClaimTarget::Crystal(id) => ("crystal_id", id),
            ClaimTarget::Facet(id) => ("facet_id", id),
            ClaimTarget::RagChunk(id) => ("rag_chunk_id", id),
        };
        if id <= 0 {
            return Err(AppError::Authority(DecisionErrorV1::InvalidRequest));
        }
        let mut s=tx.prepare(&format!("select distinct c.series_id from memory_claims c join claim_bindings b on b.claim_id=c.id where b.{column}=?")).map_err(domain)?;
        Some(
            s.query_map([id], |r| r.get(0))
                .map_err(domain)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(domain)?,
        )
    } else if let Some(id) = input.rule_id {
        let mut s=tx.prepare("select s.id from series s join concepts c on c.scope_type='global' or c.scope_key='series:'||s.slug join term_rules r on r.concept_id=c.id where r.id=?").map_err(domain)?;
        let ids = s
            .query_map([id], |r| r.get(0))
            .map_err(domain)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(domain)?;
        if ids.is_empty() {
            return Err(AppError::Authority(DecisionErrorV1::UnknownTarget));
        }
        Some(ids)
    } else {
        None
    };
    let mut series = vec![];
    let mut sources = vec![];
    let mut s = tx
        .prepare("select id,title from series order by title,id limit 200")
        .map_err(domain)?;
    let rows = s
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(domain)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(domain)?;
    for (id, title) in rows {
        if allowed.as_ref().is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        series.push(json!({"id":id,"title":title}));
        let mut q=tx.prepare("select id,source_identity,span_start,span_end,content,binding_json from evidence_records where series_id=? and kind='source_passage' order by id desc limit 200").map_err(domain)?;
        let rows = q
            .query_map([id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(domain)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(domain)?;
        for (evidence_id, identity, start, end, content, binding) in rows {
            let binding: EvidenceBindingV1 = serde_json::from_str(&binding).map_err(domain)?;
            let selected = content
                .get(start as usize..end as usize)
                .ok_or(DecisionErrorV1::EvidenceMismatch)?;
            let mut rules=tx.prepare("select id,revision,canonical_translation from term_rules where concept_id=?1 and source_language=?2 and target_language=?3 and status='active' order by id limit 100").map_err(domain)?;
            let rules=rules.query_map(params![binding.concept_id,binding.source_language,binding.target_language],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"revision":r.get::<_,i64>(1)?,"canonical":r.get::<_,String>(2)?}))).map_err(domain)?.collect::<Result<Vec<_>,_>>().map_err(domain)?;
            let mut context_unresolved = false;
            let mut current_rules = vec![];
            for rule in rules {
                match hieronymus::authority::rule_eligibility_at_source(
                    &tx,
                    rule["id"].as_i64().expect("typed rule"),
                    &binding,
                )? {
                    hieronymus::story_applicability::Eligibility::Current => {
                        current_rules.push(rule)
                    }
                    hieronymus::story_applicability::Eligibility::Unknown => {
                        context_unresolved = true
                    }
                    _ => {}
                }
            }
            let rules = current_rules;
            if input
                .rule_id
                .is_some_and(|id| !rules.iter().any(|r| r["id"] == id))
            {
                continue;
            }
            sources.push(json!({"id":evidence_id,"series_id":id,"selected_text":selected,"context":content.get(binding.paragraph_start..binding.paragraph_end).unwrap_or(selected),"source_identity":identity,"start":start,"chapter":binding.applicability.chapter_key,"rules":rules,"context_unresolved":context_unresolved}));
        }
    }
    drop(s);
    tx.commit().map_err(domain)?;
    Ok(json!({"series":series,"sources":sources,"source_inspection":true}))
}
