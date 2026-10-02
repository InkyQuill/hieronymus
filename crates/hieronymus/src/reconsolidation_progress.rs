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
    let tx = db.unchecked_transaction()?;
    tx.execute_batch(
        "create table if not exists reconsolidation_checked(
           memory_id integer primary key references short_term_memories(id) on delete cascade,
           fingerprint text not null, dream_run_id integer not null,
           retry_after text, routing text not null default ''
         );
         create index if not exists reconsolidation_checked_run on reconsolidation_checked(dream_run_id);"
    )?;
    let has_routing: bool = tx.query_row(
        "select exists(select 1 from pragma_table_info('reconsolidation_checked') where name='routing')",
        [], |row| row.get(0),
    )?;
    if !has_routing {
        tx.execute_batch("alter table reconsolidation_checked add column routing text not null default ''; delete from reconsolidation_checked;")?;
    }
    install_invalidation_triggers(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Invalidate affected markers on writes instead of hydrating every parked copy
/// on scheduler reads. BEFORE triggers retain old bindings during deletion;
/// UPDATE checks both identities so moving a binding invalidates both sides.
fn install_invalidation_triggers(db: &Connection) -> Result<(), DreamError> {
    for (table, predicate) in [
        ("short_term_memories", "m.id=ROW.id"),
        ("crystals", "m.source_crystal_id=ROW.id"),
        ("task_sessions", "m.session_id=ROW.id"),
        (
            "claim_bindings",
            "m.id=ROW.short_term_id or m.source_crystal_id=ROW.crystal_id",
        ),
        (
            "memory_claims",
            "exists(select 1 from claim_bindings b where (b.short_term_id=m.id or b.crystal_id=m.source_crystal_id) and b.claim_id=ROW.id)",
        ),
        (
            "claim_effects",
            "exists(select 1 from claim_bindings b where (b.short_term_id=m.id or b.crystal_id=m.source_crystal_id) and b.claim_id=ROW.claim_id)",
        ),
        (
            "evidence_records",
            "exists(select 1 from claim_bindings b where (b.short_term_id=m.id or b.crystal_id=m.source_crystal_id) and 'claim:' || b.claim_id=ROW.source_identity)",
        ),
        (
            "applicabilities",
            "exists(select 1 from claim_bindings b join memory_claims c on c.id=b.claim_id where (b.short_term_id=m.id or b.crystal_id=m.source_crystal_id) and c.applicability_id=ROW.id)",
        ),
        (
            "knowledge_gates",
            "exists(select 1 from claim_bindings b join memory_claims c on c.id=b.claim_id where (b.short_term_id=m.id or b.crystal_id=m.source_crystal_id) and c.applicability_id=ROW.applicability_id)",
        ),
    ] {
        for (event, timing, rows) in [
            ("insert", "after", &["new"][..]),
            ("update", "before", &["old", "new"][..]),
            ("delete", "before", &["old"][..]),
        ] {
            let predicate = rows
                .iter()
                .map(|row| format!("({})", predicate.replace("ROW", row)))
                .collect::<Vec<_>>()
                .join(" or ");
            // Strength, access counts and timestamps do not change snapshots.
            let event_clause = if event == "update" {
                match table {
                    "short_term_memories" => {
                        "update of id,text,session_id,source_crystal_id,source_credibility,rule_intent,archived_at"
                    }
                    "crystals" => {
                        "update of id,text,series_slug,source_language,target_language,source_credibility,status,crystal_type,rule_intent"
                    }
                    "task_sessions" => "update of id,series_slug,source_language,target_language",
                    _ => event,
                }
            } else {
                event
            };
            db.execute_batch(&format!(
                "create trigger if not exists reconsolidation_invalidate_{table}_{event}
                 {timing} {event_clause} on {table} begin
                   delete from reconsolidation_checked where memory_id in
                     (select m.id from reconsolidation_checked parked
                      cross join short_term_memories m on m.id=parked.memory_id
                      where {predicate});
                 end;"
            ))?;
        }
    }
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
/// SQL excludes parked rows before snapshot hydration and applies the processing cap.
pub(crate) fn select(
    db: &Connection,
    routing: &str,
    selected: Option<&HashSet<i64>>,
    limit: usize,
) -> Result<Vec<Copy>, DreamError> {
    let selected = serde_json::to_string(&selected.map(|s| s.iter().copied().collect::<Vec<_>>()))
        .map_err(|e| DreamError::Json(e.to_string()))?;
    let mut stmt = db.prepare(
        "select m.id,m.session_id,m.source_crystal_id,m.text
         from short_term_memories m left join reconsolidation_checked c on c.memory_id=m.id
         where m.archived_at is null and m.source_crystal_id is not null
           and (?1='null' or m.id in(select value from json_each(?1)))
           and (c.memory_id is null or c.routing!=?2 or c.retry_after<=datetime('now'))
         order by m.id limit ?3",
    )?;
    let rows = stmt.query_map(params![selected, routing, limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (memory_id, session_id, crystal_id, text) = row?;
        let fingerprint = fingerprint(db, memory_id, crystal_id, routing)?;
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
        "insert into reconsolidation_checked(memory_id,fingerprint,dream_run_id,retry_after,routing)
         values(?1,?2,?3,case when ?4 then datetime('now','+5 minutes') end,?5)
         on conflict(memory_id) do update set fingerprint=excluded.fingerprint,
           dream_run_id=excluded.dream_run_id,retry_after=excluded.retry_after,routing=excluded.routing",
        params![copy.memory_id, copy.fingerprint, run, retry, routing],
    )?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parked_rows_are_filtered_before_any_snapshot_hydration() {
        let db = Connection::open_in_memory().unwrap();
        // Deliberately omit snapshot/claim tables: even one attempted hydration
        // fails. A large parked queue must need only the selection query.
        db.execute_batch("create table short_term_memories(id integer primary key, session_id integer, source_crystal_id integer, text text, archived_at text);
            create table reconsolidation_checked(memory_id integer primary key, routing text, retry_after text);
            with recursive ids(id) as (select 1 union all select id+1 from ids where id<2000)
            insert into short_term_memories select id,1,id,'retained',null from ids;
            insert into reconsolidation_checked select id,'routing',null from short_term_memories;").unwrap();
        assert!(select(&db, "routing", None, 1).unwrap().is_empty());
        assert!(select(&db, "new routing", None, 0).unwrap().is_empty());
        // A changed route is eligible and therefore reaches snapshot hydration.
        assert!(select(&db, "new routing", None, 1).is_err());
    }
}
