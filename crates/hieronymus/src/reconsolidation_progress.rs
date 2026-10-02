//! Durable parking of unchanged unresolved working-copy snapshots.
//! Markers commit with the phase audit; source, authority or routing changes
//! reopen a copy, and transient failures have a bounded cooldown.
use crate::{claim_reads::ClaimTarget, dreaming::DreamError, memory_comparison::snapshot};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// A bounded eligible copy and the exact snapshot selected for inspection.
pub(crate) struct Copy {
    pub memory_id: i64,
    pub session_id: i64,
    pub crystal_id: i64,
    pub text: String,
    fingerprint: String,
}

/// Initialize rebuildable progress without modifying authoritative source rows.
pub(crate) fn initialize(db: &Connection) -> Result<(), DreamError> {
    db.execute_batch(
        "create table if not exists reconsolidation_checked(
           memory_id integer primary key references short_term_memories(id) on delete cascade,
           fingerprint text not null, dream_run_id integer not null,
           retry_after text
         );
         create index if not exists reconsolidation_checked_run on reconsolidation_checked(dream_run_id);"
    )?;
    Ok(())
}

/// Include exact scope, claim/evidence revisions, text, status and provider routing.
fn fingerprint(
    db: &Connection,
    memory: i64,
    crystal: i64,
    routing: &str,
) -> Result<String, DreamError> {
    let pair = serde_json::json!({
        "copy": snapshot(db, ClaimTarget::ShortTerm(memory))?,
        "source": snapshot(db, ClaimTarget::Crystal(crystal))?,
        "routing": routing
    });
    Ok(format!("{:x}", Sha256::digest(pair.to_string())))
}

/// Skip previously inspected snapshots before applying the processing cap.
/// Enumeration reads metadata only; model calls and retained inputs stay bounded.
pub(crate) fn select(
    db: &Connection,
    routing: &str,
    selected: Option<&HashSet<i64>>,
    limit: usize,
) -> Result<Vec<Copy>, DreamError> {
    let selected = serde_json::to_string(&selected.map(|s| s.iter().copied().collect::<Vec<_>>()))
        .map_err(|e| DreamError::Json(e.to_string()))?;
    let mut stmt = db.prepare(
        "select m.id,m.session_id,m.source_crystal_id,m.text,
                case when c.retry_after is null or c.retry_after>datetime('now') then c.fingerprint end
         from short_term_memories m left join reconsolidation_checked c on c.memory_id=m.id
         where m.archived_at is null and m.source_crystal_id is not null
           and (?1='null' or m.id in(select value from json_each(?1)))
         order by m.id"
    )?;
    let rows = stmt.query_map([selected], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        if result.len() >= limit {
            break;
        }
        let (memory_id, session_id, crystal_id, text, checked) = row?;
        let fingerprint = fingerprint(db, memory_id, crystal_id, routing)?;
        if checked.as_ref() == Some(&fingerprint) {
            continue;
        }
        result.push(Copy {
            memory_id,
            session_id,
            crystal_id,
            text,
            fingerprint,
        });
    }
    Ok(result)
}

/// Park only the inspected snapshot, atomically with its pending audit action.
/// Exhausted run budgets do not authorize parking an unassessed pair.
pub(crate) fn park(
    db: &Connection,
    copy: &Copy,
    routing: &str,
    run: i64,
    reason: &str,
) -> Result<bool, DreamError> {
    if reason == "comparison run budget exhausted"
        || fingerprint(db, copy.memory_id, copy.crystal_id, routing)? != copy.fingerprint
    {
        return Ok(false);
    }
    let retry = reason == "assigned comparison providers unavailable";
    db.execute(
        "insert into reconsolidation_checked(memory_id,fingerprint,dream_run_id,retry_after)
         values(?1,?2,?3,case when ?4 then datetime('now','+5 minutes') end)
         on conflict(memory_id) do update set fingerprint=excluded.fingerprint,
           dream_run_id=excluded.dream_run_id,retry_after=excluded.retry_after",
        params![copy.memory_id, copy.fingerprint, run, retry],
    )?;
    Ok(true)
}
