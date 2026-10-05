//! Passive salience decay gives one opportunity per fully drained project/language
//! scope in a cycle, regardless of session count. Its opportunity ledger,
//! strength deltas and audit share the caller's immediate transaction.

use std::collections::HashSet;

use rusqlite::{Transaction, params};
use serde_json::{Value, json};

use crate::{
    claim_reads::{ClaimDisposition, ClaimTarget},
    data_root::HieronymusConfig,
    dream_audit::DreamAuditStore,
    dream_config::DreamConfig,
    dreaming::{DreamError, now},
    story_applicability::{ApplicabilityError, QueryMode, StoryApplicability},
};

const STRENGTH_STEP: f64 = 0.02;
const STRENGTH_FLOOR: f64 = 0.20;
const OPPORTUNITY: &str = "dream_decay_opportunity";

pub(crate) fn complete_in_transaction(
    tx: &Transaction<'_>,
    config: &HieronymusConfig,
    settings: &DreamConfig,
    run_id: i64,
    cycle_id: i64,
) -> Result<(), DreamError> {
    // Reserve only the remaining crystal budget after the preceding phases.
    let affected: i64 = tx.query_row(
        "select count(distinct item.value)
         from dream_audit_entries a, json_each(a.payload_json) section,
              json_each(section.value) item
         where a.dream_run_id=?1 and a.event_type='phase_completed'
           and section.key in ('created_crystals','changed_crystals',
                               'reinforced_crystals','superseded_crystals')
           and item.type='integer'",
        [run_id],
        |row| row.get(0),
    )?;
    let cap = settings
        .max_changed_crystals_per_cycle
        .min(settings.max_long_term_records_affected_per_run)
        .min(settings.max_total_affected_crystals)
        .max(0);
    let mut budget = cap.saturating_sub(affected).max(0) as usize;
    let mut statement = tx.prepare(
        "select s.id, s.created_at from task_sessions s
         where exists(select 1 from dream_audit_entries a,
             json_each(a.payload_json,'$.input_dispositions') disposition
             join short_term_memories input on input.id=json_extract(disposition.value,'$.memory_id')
             where a.dream_run_id=?4 and a.event_type='phase_completed'
               and json_extract(disposition.value,'$.disposition')='represented'
               and input.session_id=s.id)
           and not exists (select 1 from memory_events e
                           where e.cycle_id=?1 and e.event_type=?2
                             and e.session_id in(select id from task_sessions where series_slug=s.series_slug and source_language=s.source_language and target_language=s.target_language))
           and exists (
             select 1 from short_term_memories m
             where m.session_id=s.id and m.archived_at is not null
               and m.source_crystal_id is null
               and (exists (select 1 from crystal_sources cs
                            where cs.short_term_memory_id=m.id)
                    or exists (select 1 from claim_bindings source
                               join claim_bindings facet on facet.claim_id=source.claim_id
                               where source.short_term_id=m.id and facet.facet_id is not null))
           )
         and not exists (
             select 1 from short_term_memories pending
             join task_sessions owner on owner.id=pending.session_id
             where pending.archived_at is null and pending.source_crystal_id is null
               and owner.series_slug=s.series_slug and owner.source_language=s.source_language
               and owner.target_language=s.target_language
           )
         order by s.id limit ?3",
    )?;
    let sessions = statement
        .query_map(
            params![
                cycle_id,
                OPPORTUNITY,
                settings.max_short_term_memories_per_run,
                run_id
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let workspace = crate::workspace::WorkspaceStore::for_read(config);
    let mut decayed = HashSet::new();
    let mut opportunities = HashSet::new();
    for (session_id, started_at) in sessions {
        let session = match workspace.get_session_with_connection(tx, session_id) {
            Ok(session) => session,
            Err(crate::workspace::WorkspaceError::StoryContext(ApplicabilityError::Database(
                error,
            ))) => return Err(error.into()),
            Err(crate::workspace::WorkspaceError::StoryContext(_)) => continue,
            Err(error) => return Err(error.into()),
        };
        let query = match StoryApplicability::resolve_context(tx, &session.context) {
            Ok(query) => query,
            Err(ApplicabilityError::Database(error)) => return Err(error.into()),
            Err(_) => continue,
        };
        // Unknown narrative order and unresolved viewpoint context
        // cannot establish an opportunity to have used an applicable memory.
        if query.mode != QueryMode::Current
            || query.timeline_id.is_none()
            || query.position_id.is_none()
        {
            continue;
        }
        let timestamp = now();
        let phase_id = tx.execute(
            "insert into dream_phase_runs(
               dream_run_id,phase,provider_profile,provider_type,model,status,input_count,created_at
             ) values(?1,'salience_decay','deterministic','deterministic','deterministic','running',1,?2)",
            params![run_id,timestamp],
        ).map(|_|tx.last_insert_rowid())?;
        let mut statement = tx.prepare(
            "select c.id,c.strength from crystals c
             where c.status='active' and c.crystal_type!='rule'
               and c.scope_type='series' and c.scope_key=?1 and c.series_slug=?2
               and c.source_language=?3 and c.target_language=?4
               and c.rule_intent in ('','none')
               and c.source_credibility not in ('explicit_user','user_rule','user_suggestion')
               and c.strength>?5 and c.created_cycle!=?6
               and julianday(c.created_at)<=julianday(?7)
               and julianday(c.updated_at)<julianday(?7)
               and not exists (select 1 from term_rules r
                               where r.rule_crystal_id=c.id and r.status='active')
               and not exists (select 1 from crystal_activations a where a.crystal_id=c.id
                               and (a.session_id=?8 or julianday(a.created_at)>=julianday(?7)))
               and not exists (select 1 from short_term_memories m
                               where m.source_crystal_id=c.id and m.session_id=?8)
               and not exists (select 1 from memory_events e where e.crystal_id=c.id
                               and e.event_type!=?9
                               and (e.session_id=?8 or julianday(e.created_at)>=julianday(?7)))
               and not exists (select 1 from memory_events e where e.crystal_id=c.id
                               and (e.source_role='user' or e.event_type='confirmed_by_user'))
               and not exists (select 1 from claim_bindings b
                               join claim_effects effect on effect.claim_id=b.claim_id
                               join decision_records d on d.decision_id=effect.decision_id
                               where b.crystal_id=c.id and d.actor_kind='explicit_user')
             order by c.updated_at,c.id limit ?10",
        )?;
        let candidates = statement
            .query_map(
                params![
                    session.context.scope_key(),
                    session.context.series_slug,
                    session.context.source_language,
                    session.context.target_language,
                    STRENGTH_FLOOR,
                    cycle_id,
                    started_at,
                    session_id,
                    "cycle_decay",
                    settings.max_total_affected_crystals,
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let scanned = candidates.len();
        let mut changes: Vec<Value> = Vec::new();
        for (id, before) in candidates {
            if decayed.contains(&id) {
                continue;
            }
            if budget == 0 {
                break;
            }
            if !matches!(
                crate::claim_reads::rehydrate_claims(tx, ClaimTarget::Crystal(id), &query)?,
                ClaimDisposition::Current
            ) {
                continue;
            }
            let after = (before - STRENGTH_STEP).max(STRENGTH_FLOOR);
            // Passive non-use cannot write confidence, status, text or authority.
            tx.execute(
                "update crystals set strength=?1 where id=?2",
                params![after, id],
            )?;
            tx.execute(
                "insert into memory_events(crystal_id,session_id,event_type,source_role,evidence,
                   strength_delta,confidence_delta,applied,cycle_id,created_at)
                 values(?1,?2,'cycle_decay','system',?3,?4,0,1,?5,?6)",
                params![
                    id,
                    session_id,
                    format!("unused during project consolidation (context {session_id})"),
                    after - before,
                    cycle_id,
                    timestamp
                ],
            )?;
            changes.push(json!({"crystal_id":id,"strength_before":before,"strength_after":after,"strength_delta":after-before,"confidence_delta":0.0}));
            decayed.insert(id);
            budget -= 1;
        }
        // One ledger entry per project/language scope; evaluate all represented
        // story contexts but never decay the same crystal twice in this cycle.
        let scope = (
            session.context.series_slug.clone(),
            session.context.source_language.clone(),
            session.context.target_language.clone(),
        );
        if opportunities.insert(scope) {
            tx.execute(
                "insert into memory_events(session_id,event_type,source_role,evidence,applied,cycle_id,created_at)
                 values(?1,?2,'system','project consolidation salience opportunity v1',1,?3,?4)",
                params![session_id,OPPORTUNITY,cycle_id,timestamp],
            )?;
        }
        tx.execute("update dream_phase_runs set status='completed',output_count=?1,completed_at=?2 where id=?3", params![changes.len() as i64,timestamp,phase_id])?;
        DreamAuditStore::append_in_transaction(
            tx,
            run_id,
            Some(phase_id),
            "phase_completed",
            "info",
            "completed salience_decay phase",
            &json!({
                "phase_name":"salience_decay","session_id":session_id,"series_slug":session.context.series_slug,
                "policy":"consolidated_project_v1","step":STRENGTH_STEP,"floor":STRENGTH_FLOOR,
                "candidate_scan_limit":settings.max_total_affected_crystals,"scanned_candidates":scanned,
                "crystal_budget":cap,"remaining_budget":budget,"decayed_crystals":changes,
            }),
        )?;
    }
    Ok(())
}
