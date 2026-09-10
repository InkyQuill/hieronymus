//! Durable per-batch pair progress for the dream link-reinforcement phase
//! (task D4; memory-reconsolidation design, astra finding 3.2).
//!
//! The phase consumes `useful` crystal activations as unique unordered
//! crystal pairs. Before this module, a cycle budget smaller than the pair
//! count still stamped every selected activation consumed
//! (`crystal_activations.cycle_id`), permanently dropping the unprocessed
//! pairs. Here the eligible activations of one session are snapshotted into
//! ONE durable batch with activation membership and deletion-safe crystal
//! identities. Cursor offsets enumerate unique pairs lazily; only terminal
//! pairs occupy `dream_link_pairs`. Each effect, terminal row, cursor advance,
//! and audit commits atomically, so budget exhaustion or a crash resumes
//! without replay. Legacy materialized batches retain their queued rows and
//! drain first. Activations are consumed only after the final pair completes.

use rusqlite::Connection;
use serde_json::{Value, json};

use crate::crystals::is_active_rule;
use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::dream_audit::{DreamAuditStore, commit_audited};
use crate::dream_config::{DreamConfig, load_dream_config};
use crate::dreaming::{
    COMBINATION_SIMILARITY_THRESHOLD, DreamError, now, token_similarity, tx_error,
};

/// Weight of a `crystal_links` row created by hebbian co-activation.
const LINK_INITIAL_WEIGHT: f64 = 0.5;

/// Additive weight gain per co-activation cycle, capped at [`LINK_WEIGHT_MAX`].
const HEBBIAN_STRENGTH_DELTA: f64 = 0.1;

const LINK_WEIGHT_MAX: f64 = 1.0;

/// `crystal_links.link_type` for hebbian co-activation links.
const CO_ACTIVATION_LINK_TYPE: &str = "co_activation";

const BATCH_PAIR_TOTALS_SQL: &str =
    "select applied_pair_count, skipped_pair_count from dream_link_batches where id=?1";

/// Unique unordered pairs of the given ids in deterministic order: ids are
/// deduplicated and sorted, then paired as (smaller, larger) — the
/// `dream_link_pairs` snapshot contract (`left_id < right_id`).
pub fn canonical_pairs(ids: &[i64]) -> Vec<(i64, i64)> {
    let unique: Vec<_> = ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut pairs = Vec::new();
    for (index, left) in unique.iter().enumerate() {
        for right in unique.iter().skip(index + 1) {
            pairs.push((*left, *right));
        }
    }
    pairs
}

/// What one pair's terminalizing commit did; the source of both the pair
/// row's `result_json` and the phase summary's action entry.
#[derive(Debug)]
enum PairEffect {
    Combined { survivor: i64, absorbed: i64 },
    Linked,
    Skipped { missing: Vec<i64> },
}

impl PairEffect {
    fn status(&self) -> &'static str {
        match self {
            Self::Skipped { .. } => "skipped",
            _ => "applied",
        }
    }

    fn result_json(&self, left: i64, right: i64) -> Value {
        match self {
            Self::Combined { survivor, absorbed } => json!({
                "action": "combined",
                "survivor_crystal_id": survivor,
                "absorbed_crystal_id": absorbed,
            }),
            Self::Linked => json!({
                "action": "co_activation_link",
                "crystal_ids": [left, right],
            }),
            Self::Skipped { missing } => json!({
                "reason": "crystal_missing",
                "missing_crystal_ids": missing,
            }),
        }
    }

    /// The phase summary's action entry (same shapes as the pre-D4 single
    /// transaction wrote, plus the new audited skip tombstones).
    fn summary_action(&self, left: i64, right: i64) -> Value {
        match self {
            Self::Combined { survivor, absorbed } => json!({
                "survivor_crystal_id": survivor,
                "absorbed_crystal_id": absorbed,
                "action": "combined",
            }),
            Self::Linked => json!({
                "crystal_ids": [left, right],
                "action": "co_activation_link",
            }),
            Self::Skipped { missing } => json!({
                "crystal_ids": [left, right],
                "action": "skipped",
                "reason": "crystal_missing",
                "missing_crystal_ids": missing,
            }),
        }
    }

    fn changed_crystal_ids(&self) -> Vec<i64> {
        match self {
            Self::Combined { survivor, absorbed } => vec![*survivor, *absorbed],
            _ => Vec::new(),
        }
    }
}

/// Effect summary of the pairs this instance processed, folded into the
/// link phase's `phase_completed` audit payload (same keys as before).
#[derive(Debug, Default)]
pub struct LinkProgressSummary {
    pub changed_crystal_ids: Vec<i64>,
    pub actions: Vec<Value>,
}

/// The durable link-reinforcement progress store: batches the phase's
/// eligible activations, applies at most `pair_budget` pairs per
/// [`LinkProgress::process`] call, and resumes open batches on restart.
pub struct LinkProgress {
    config: HieronymusConfig,
    dream_config: DreamConfig,
    /// Audit context (dream run id, link phase run id) the per-pair and
    /// batch-completion audit entries are written under. The dreaming phase
    /// always sets it before processing; audit entries require a run row, so
    /// without a context progress still commits domain work but writes no
    /// audit entries.
    run: Option<(i64, Option<i64>)>,
    /// Remaining pairwise-combination budget of the current call
    /// (`max_changed_crystals_per_cycle`, mirroring the pre-D4 in-pass cap).
    combination_budget: usize,
    /// Pairs terminalized in the current call; on a failed call this is the
    /// committed prefix (each earlier pair's commit is durable).
    terminalized_in_call: usize,
    summary: LinkProgressSummary,
}

impl LinkProgress {
    pub fn open(config: &HieronymusConfig) -> Result<Self, DreamError> {
        open_migrated(&config.database_path())?;
        let dream_config = load_dream_config(config)?;
        Ok(Self {
            config: config.clone(),
            dream_config,
            run: None,
            combination_budget: 0,
            terminalized_in_call: 0,
            summary: LinkProgressSummary::default(),
        })
    }

    /// Attach the audit context (dream run and link phase run) whose records
    /// the per-pair and batch-completion audit entries are written under.
    pub fn set_run_context(&mut self, dream_run_id: i64, phase_run_id: Option<i64>) {
        self.run = Some((dream_run_id, phase_run_id));
    }

    /// Pairs terminalized so far in the current call. Meaningful on a failed
    /// call too: the committed prefix that stays durable after the rollback
    /// of the failing pair.
    pub fn terminalized_in_call(&self) -> usize {
        self.terminalized_in_call
    }

    /// Drain the effect summary accumulated by [`LinkProgress::process`].
    pub fn take_summary(&mut self) -> LinkProgressSummary {
        std::mem::take(&mut self.summary)
    }

    /// Process at most `pair_budget` queued pairs for `cycle`, resuming open
    /// batches first and then snapshotting fresh ones (one per session). The
    /// returned count is pairs terminalized (applied or skipped) in this
    /// call — not selected activations. A zero budget consumes nothing: no
    /// batch is created, no pair is touched, no activation is stamped.
    pub fn process(&mut self, cycle: i64, pair_budget: usize) -> Result<usize, DreamError> {
        self.terminalized_in_call = 0;
        self.summary = LinkProgressSummary::default();
        if pair_budget == 0 {
            return Ok(0);
        }
        self.combination_budget = self.dream_config.max_changed_crystals_per_cycle.max(0) as usize;
        self.terminalized_in_call = 0;
        let mut connection = open_migrated(&self.config.database_path())?;
        let mut budget = pair_budget;

        // Resume open batches (oldest first): a batch with queued pairs is
        // continued where the previous call stopped, never re-snapshotted.
        for batch_id in open_batch_ids(&connection, pair_budget)? {
            if budget == 0 {
                break;
            }
            self.drive_batch(&mut connection, batch_id, cycle, &mut budget)?;
        }

        // Fresh snapshots: one durable batch per session that still has
        // unconsumed useful activations and no open batch (session
        // isolation: activations from different sessions never share one).
        if budget > 0 {
            for session_id in sessions_without_open_batch(&connection, budget)? {
                if budget == 0 {
                    break;
                }
                let Some(batch_id) = self.snapshot_batch(&mut connection, cycle, session_id)?
                else {
                    continue;
                };
                self.drive_batch(&mut connection, batch_id, cycle, &mut budget)?;
            }
        }
        Ok(self.terminalized_in_call)
    }

    /// Drive one batch forward: terminalize its queued pairs while budget
    /// remains, completing the batch (stamping its activations consumed)
    /// when its last pair is terminal.
    fn drive_batch(
        &mut self,
        connection: &mut Connection,
        batch_id: i64,
        cycle: i64,
        budget: &mut usize,
    ) -> Result<(), DreamError> {
        loop {
            // Legacy materialized work is read one row at a time, before any
            // lazy cursor work. Never load a quadratic queued-pair vector.
            let queued = queued_pair(connection, batch_id)?;
            let lazy = lazy_pair(connection, batch_id)?;
            let pair = queued.or(lazy);
            let Some((left, right)) = pair else {
                self.complete_batch(connection, batch_id, cycle)?;
                return Ok(());
            };
            if *budget == 0 {
                return Ok(());
            }
            self.terminalize_pair(connection, batch_id, cycle, left, right)?;
            *budget -= 1;
        }
    }

    /// Snapshot one session's unconsumed useful activations into a durable
    /// batch: UNIQUE activation membership and sorted crystal identities.
    /// Pair enumeration itself is lazy. One immediate transaction, so a crash
    /// leaves either no batch or a complete one. `None` when the session has
    /// no unconsumed activations (no batch is created).
    fn snapshot_batch(
        &mut self,
        connection: &mut Connection,
        cycle: i64,
        session_id: i64,
    ) -> Result<Option<i64>, DreamError> {
        Ok(commit_audited(connection, |transaction| {
            let exists: bool = transaction.query_row(
                "select exists(select 1 from crystal_activations where outcome='useful' and cycle_id is null and session_id=?1)",
                [session_id], |row| row.get(0))?;
            if !exists {
                return Ok(None);
            }
            transaction.execute(
                "insert into dream_link_batches(session_id, created_cycle, lazy_pairs) values (?1, ?2, 1)",
                rusqlite::params![session_id, cycle],
            )?;
            let batch_id = transaction.last_insert_rowid();
            transaction.execute(
                "insert into dream_link_members(batch_id, activation_id)
                 select ?1, id from crystal_activations where outcome='useful' and cycle_id is null and session_id=?2",
                rusqlite::params![batch_id, session_id])?;
            // Linear snapshot work, entirely in SQLite. No pair cross join,
            // and no Rust vector proportional to the number of activations.
            transaction.execute(
                "insert into dream_link_crystals(batch_id, member_offset, crystal_id)
                 select ?1, row_number() over (order by crystal_id)-1, crystal_id
                 from (select distinct ca.crystal_id from crystal_activations ca
                   join dream_link_members m on m.activation_id=ca.id where m.batch_id=?1)",
                [batch_id],
            )?;
            Ok(Some(batch_id))
        })?)
    }

    /// Terminalize one pair in its own atomic commit (task D4 granularity):
    /// the reinforcement (pairwise combination or hebbian link), the pair
    /// row's status / applied_cycle / result_json, and the audit entry commit
    /// together or not at all. Deleted crystals produce an audited
    /// `skipped` tombstone, never an application failure.
    fn terminalize_pair(
        &mut self,
        connection: &mut Connection,
        batch_id: i64,
        cycle: i64,
        left: i64,
        right: i64,
    ) -> Result<(), DreamError> {
        let combination_budget = self.combination_budget;
        let effect: PairEffect = commit_audited(connection, |transaction| {
            // Both insertion and cursor movement belong to the same effect
            // transaction. A failed audit/status write rolls all of them back.
            transaction.execute(
                "insert or ignore into dream_link_pairs(batch_id, left_id, right_id) values(?1, ?2, ?3)",
                rusqlite::params![batch_id, left, right])?;
            let cores = load_crystal_cores(transaction, &[(left, right)]).map_err(tx_error)?;
            let effect = match (cores.get(&left), cores.get(&right)) {
                (Some(left_core), Some(right_core)) => {
                    // Pairwise combination (July design; ADR 0011 guards):
                    // only active advisory crystals combine; active
                    // deterministic rules are never absorbed, never survivors.
                    let combinable = combination_budget > 0
                        && !is_active_rule(&left_core.crystal_type, &left_core.status)
                        && !is_active_rule(&right_core.crystal_type, &right_core.status)
                        && left_core.status == "active"
                        && right_core.status == "active"
                        && token_similarity(&left_core.text, &right_core.text)
                            >= COMBINATION_SIMILARITY_THRESHOLD;
                    if combinable {
                        let survivor =
                            pick_combination_survivor(left, left_core, right, right_core);
                        let absorbed = if survivor == left { right } else { left };
                        combine_crystals(transaction, survivor, absorbed, cycle)
                            .map_err(tx_error)?;
                        PairEffect::Combined { survivor, absorbed }
                    } else {
                        // Hebbian strengthening between co-activated crystals.
                        strengthen_co_activation_link(transaction, left, right)
                            .map_err(tx_error)?;
                        PairEffect::Linked
                    }
                }
                _ => {
                    let missing: Vec<i64> = [left, right]
                        .into_iter()
                        .filter(|id| !cores.contains_key(id))
                        .collect();
                    PairEffect::Skipped { missing }
                }
            };
            // Fail closed: the pair row must be the one this commit
            // terminalizes; a missing row is corruption, not a skip, and the
            // whole pair transaction (reinforcement included) rolls back.
            let changed = transaction.execute(
                "update dream_link_pairs
                 set status = ?1, applied_cycle = ?2, result_json = ?3
                 where batch_id = ?4 and left_id = ?5 and right_id = ?6
                   and status = 'queued'",
                rusqlite::params![
                    effect.status(),
                    cycle,
                    effect.result_json(left, right).to_string(),
                    batch_id,
                    left,
                    right,
                ],
            )?;
            if changed != 1 {
                return Err(tx_error(DreamError::Json(format!(
                    "queued link pair ({left}, {right}) of batch {batch_id} is missing"
                ))));
            }
            transaction.execute(
                "update dream_link_batches set
                 next_left_offset = case when next_right_offset + 1 >= (select max(member_offset)+1 from dream_link_crystals where batch_id=?1) then next_left_offset+1 else next_left_offset end,
                 next_right_offset = case when next_right_offset + 1 >= (select max(member_offset)+1 from dream_link_crystals where batch_id=?1) then next_left_offset+2 else next_right_offset+1 end
                 where id=?1 and lazy_pairs=1",
                [batch_id])?;
            transaction.execute(
                "update dream_link_batches set applied_pair_count = applied_pair_count + (?2 = 'applied'),
                 skipped_pair_count = skipped_pair_count + (?2 = 'skipped') where id = ?1",
                rusqlite::params![batch_id, effect.status()])?;
            self.append_pair_audit(transaction, batch_id, cycle, left, right, &effect)
                .map_err(tx_error)?;
            Ok(effect)
        })?;
        self.terminalized_in_call += 1;
        if let PairEffect::Combined { .. } = effect {
            self.combination_budget -= 1;
        }
        self.summary
            .changed_crystal_ids
            .extend(effect.changed_crystal_ids());
        self.summary
            .actions
            .push(effect.summary_action(left, right));
        Ok(())
    }

    /// The per-pair audit entry, appended inside the pair's transaction. A
    /// no-op without a run context (audit entries require a dream run row).
    fn append_pair_audit(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        batch_id: i64,
        cycle: i64,
        left: i64,
        right: i64,
        effect: &PairEffect,
    ) -> Result<(), DreamError> {
        let Some((run_id, phase_run_id)) = self.run else {
            return Ok(());
        };
        let status = effect.status();
        DreamAuditStore::append_in_transaction(
            transaction,
            run_id,
            phase_run_id,
            "link_pair_completed",
            "info",
            &format!("{status} link pair ({left}, {right})"),
            &json!({
                "batch_id": batch_id,
                "left_id": left,
                "right_id": right,
                "status": status,
                "applied_cycle": cycle,
                "result": effect.result_json(left, right),
            }),
        )?;
        Ok(())
    }

    /// Complete one open batch in its own atomic commit: the member
    /// activations are stamped consumed and `completed_cycle` is set
    /// together (with the batch audit entry). Idempotent: only an open batch
    /// completes.
    fn complete_batch(
        &mut self,
        connection: &mut Connection,
        batch_id: i64,
        cycle: i64,
    ) -> Result<(), DreamError> {
        commit_audited(connection, |transaction| {
            let session_id: Option<i64> = transaction
                .query_row(
                    "select session_id from dream_link_batches
                     where id = ?1 and completed_cycle is null",
                    [batch_id],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            let Some(session_id) = session_id else {
                return Ok(());
            };
            // The batch's activations are marked consumed only now that all
            // its pairs are applied or explicitly skipped; snapshot members
            // whose activation rows are gone (cascade-deleted) simply match
            // nothing.
            transaction.execute(
                "update crystal_activations set cycle_id = ?1
                 where cycle_id is null
                   and id in (select activation_id from dream_link_members where batch_id = ?2)",
                rusqlite::params![cycle, batch_id],
            )?;
            transaction.execute(
                "update dream_link_batches set completed_cycle = ?1
                 where id = ?2 and completed_cycle is null",
                rusqlite::params![cycle, batch_id],
            )?;
            let (applied, skipped): (i64, i64) =
                transaction.query_row(BATCH_PAIR_TOTALS_SQL, [batch_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?;
            let member_count: i64 = transaction.query_row(
                "select count(*) from dream_link_members where batch_id=?1",
                [batch_id],
                |row| row.get(0),
            )?;
            if let Some((run_id, phase_run_id)) = self.run {
                DreamAuditStore::append_in_transaction(
                    transaction,
                    run_id,
                    phase_run_id,
                    "link_batch_completed",
                    "info",
                    &format!("completed link batch {batch_id}"),
                    &json!({
                        "batch_id": batch_id,
                        "session_id": session_id,
                        "completed_cycle": cycle,
                        "activation_count": member_count,
                        "applied_pairs": applied,
                        "skipped_pairs": skipped,
                    }),
                )
                .map_err(tx_error)?;
            }
            Ok(())
        })?;
        Ok(())
    }
}

/// The open batches in resume order (creation order).
fn open_batch_ids(connection: &Connection, limit: usize) -> Result<Vec<i64>, DreamError> {
    let mut statement = connection.prepare(
        "select id from dream_link_batches
         where completed_cycle is null order by lazy_pairs, id limit ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| row.get(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

/// Sessions with unconsumed useful activations but no open batch, in
/// deterministic (session id) order.
fn sessions_without_open_batch(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<i64>, DreamError> {
    let mut statement = connection.prepare(
        "select distinct ca.session_id
         from crystal_activations ca
         where ca.outcome = 'useful' and ca.cycle_id is null
           and not exists (
             select 1 from dream_link_batches b
             where b.session_id = ca.session_id and b.completed_cycle is null
           )
         order by ca.session_id limit ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| row.get(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn queued_pair(connection: &Connection, batch_id: i64) -> Result<Option<(i64, i64)>, DreamError> {
    use rusqlite::OptionalExtension;
    Ok(connection
        .query_row(
            "select left_id, right_id from dream_link_pairs
      where batch_id=?1 and status='queued' order by left_id, right_id limit 1",
            [batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

fn lazy_pair(connection: &Connection, batch_id: i64) -> Result<Option<(i64, i64)>, DreamError> {
    use rusqlite::OptionalExtension;
    Ok(connection
        .query_row(
            "select l.crystal_id, r.crystal_id from dream_link_batches b
      join dream_link_crystals l on l.batch_id=b.id and l.member_offset=b.next_left_offset
      join dream_link_crystals r on r.batch_id=b.id and r.member_offset=b.next_right_offset
      where b.id=?1 and b.lazy_pairs=1",
            [batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

/// Minimal crystal projection for combination decisions.
struct CrystalCore {
    text: String,
    status: String,
    crystal_type: String,
    source_credibility: String,
    strength: f64,
}

fn load_crystal_cores(
    transaction: &rusqlite::Transaction<'_>,
    pairs: &[(i64, i64)],
) -> Result<std::collections::BTreeMap<i64, CrystalCore>, DreamError> {
    let mut ids: Vec<i64> = pairs
        .iter()
        .flat_map(|(left, right)| [*left, *right])
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let ids_json = format!(
        "[{}]",
        ids.iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut statement = transaction.prepare(
        "select id, text, status, crystal_type, source_credibility, strength
         from crystals
         where id in (select value from json_each(?1))",
    )?;
    let rows = statement.query_map([&ids_json], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            CrystalCore {
                text: row.get(1)?,
                status: row.get(2)?,
                crystal_type: row.get(3)?,
                source_credibility: row.get(4)?,
                strength: row.get(5)?,
            },
        ))
    })?;
    let mut cores = std::collections::BTreeMap::new();
    for row in rows {
        let (id, core) = row?;
        cores.insert(id, core);
    }
    Ok(cores)
}

/// Survivor selection (July design): higher `source_credibility` weight,
/// tie-broken by higher `strength`, then by lower id for determinism.
fn pick_combination_survivor(
    left_id: i64,
    left: &CrystalCore,
    right_id: i64,
    right: &CrystalCore,
) -> i64 {
    let left_weight = crate::dreaming::source_credibility_confidence(&left.source_credibility);
    let right_weight = crate::dreaming::source_credibility_confidence(&right.source_credibility);
    match right_weight.partial_cmp(&left_weight) {
        Some(std::cmp::Ordering::Greater) => right_id,
        Some(std::cmp::Ordering::Equal) => {
            if right.strength > left.strength {
                right_id
            } else {
                left_id
            }
        }
        _ => left_id,
    }
}

/// Pairwise combination: union the absorbed crystal's concepts and links onto
/// the survivor, mark it `superseded`, and record one `combined_into` memory
/// event whose evidence names the survivor (event-sourced; the
/// `supersedes_crystal_id` column is not reused for combination).
fn combine_crystals(
    transaction: &rusqlite::Transaction<'_>,
    survivor: i64,
    absorbed: i64,
    cycle_id: i64,
) -> Result<(), DreamError> {
    crate::claim_capture::copy_crystal_lineage_tx(transaction, absorbed, survivor)?;
    transaction.execute(
        "insert or ignore into crystal_concepts(crystal_id, concept_id, link_type, confidence, created_at)
         select ?1, concept_id, link_type, confidence, created_at
         from crystal_concepts where crystal_id = ?2",
        rusqlite::params![survivor, absorbed],
    )?;
    {
        let mut statement = transaction.prepare(
            "select source_crystal_id, target_crystal_id, link_type, weight
             from crystal_links
             where source_crystal_id = ?1 or target_crystal_id = ?1",
        )?;
        let rows = statement.query_map([absorbed], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })?;
        let links: Vec<(i64, i64, String, f64)> = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (source, target, link_type, weight) in links {
            let other = if source == absorbed { target } else { source };
            if other == survivor || other == absorbed {
                continue;
            }
            // Preserve the link's orientation relative to its other endpoint.
            let (new_source, new_target) = if source == absorbed {
                (survivor, other)
            } else {
                (other, survivor)
            };
            transaction.execute(
                "insert or ignore into crystal_links(
                   source_crystal_id, target_crystal_id, link_type, weight
                 )
                 values (?1, ?2, ?3, ?4)",
                rusqlite::params![new_source, new_target, link_type, weight],
            )?;
        }
    }
    transaction.execute(
        "update crystals set status = 'superseded', updated_at = ?1 where id = ?2",
        rusqlite::params![now(), absorbed],
    )?;
    transaction.execute(
        "insert into memory_events(
           crystal_id, session_id, event_type, source_role, evidence,
           strength_delta, confidence_delta, applied, cycle_id, created_at
         )
         values (?1, null, 'combined_into', 'system', ?2, 0, 0, 1, ?3, ?4)",
        rusqlite::params![absorbed, survivor.to_string(), cycle_id, now()],
    )?;
    Ok(())
}

/// Hebbian strengthening: the canonical (lower, higher) id pair's
/// `co_activation` link gains [`HEBBIAN_STRENGTH_DELTA`], or is created at
/// [`LINK_INITIAL_WEIGHT`], capped at [`LINK_WEIGHT_MAX`].
fn strengthen_co_activation_link(
    transaction: &rusqlite::Transaction<'_>,
    left: i64,
    right: i64,
) -> Result<(), DreamError> {
    let (source, target) = (left.min(right), left.max(right));
    let existing: Option<f64> = transaction
        .query_row(
            "select weight from crystal_links
             where link_type = ?1
               and ((source_crystal_id = ?2 and target_crystal_id = ?3)
                 or (source_crystal_id = ?3 and target_crystal_id = ?2))",
            rusqlite::params![CO_ACTIVATION_LINK_TYPE, source, target],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    match existing {
        Some(weight) => {
            transaction.execute(
                "update crystal_links set weight = ?1
                 where link_type = ?2
                   and ((source_crystal_id = ?3 and target_crystal_id = ?4)
                     or (source_crystal_id = ?4 and target_crystal_id = ?3))",
                rusqlite::params![
                    (weight + HEBBIAN_STRENGTH_DELTA).min(LINK_WEIGHT_MAX),
                    CO_ACTIVATION_LINK_TYPE,
                    source,
                    target
                ],
            )?;
        }
        None => {
            transaction.execute(
                "insert into crystal_links(
                   source_crystal_id, target_crystal_id, link_type, weight
                 )
                 values (?1, ?2, ?3, ?4)",
                rusqlite::params![source, target, CO_ACTIVATION_LINK_TYPE, LINK_INITIAL_WEIGHT],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::canonical_pairs;

    #[test]
    fn canonical_pairs_of_singletons_and_duplicates_is_empty() {
        assert!(canonical_pairs(&[]).is_empty());
        assert!(canonical_pairs(&[7]).is_empty());
        assert!(canonical_pairs(&[5, 5, 5]).is_empty());
    }

    #[test]
    fn canonical_pairs_are_sorted_smaller_first() {
        assert_eq!(canonical_pairs(&[9, 4]), vec![(4, 9)]);
    }
    #[test]
    fn completion_total_lookup_has_constant_vm_work_as_history_grows() {
        use rusqlite::{Connection, StatementStatus};
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("create table dream_link_batches(id integer primary key, applied_pair_count integer, skipped_pair_count integer);
          insert into dream_link_batches values(1,100,0);
          create table dream_link_pairs(batch_id integer, left_id integer, right_id integer, status text);
          create index queue on dream_link_pairs(batch_id,status,left_id,right_id);").unwrap();
        let mut steps = Vec::new();
        for count in [100, 10000] {
            connection
                .execute(
                    "with recursive n(x) as (values(1) union all select x+1 from n where x<?1)
              insert into dream_link_pairs select 1, 0, x, 'applied' from n",
                    [count],
                )
                .unwrap();
            connection
                .execute(
                    "update dream_link_batches set applied_pair_count=?1 where id=1",
                    [count],
                )
                .unwrap();
            let mut statement = connection.prepare(super::BATCH_PAIR_TOTALS_SQL).unwrap();
            let total: i64 = statement.query_row([1], |row| row.get(0)).unwrap();
            assert_eq!(total, count);
            steps.push(statement.get_status(StatementStatus::VmStep));
        }
        assert_eq!(
            steps[0], steps[1],
            "terminal history must not increase completion query work"
        );
        assert!(
            steps[1] < 100,
            "single batch lookup must be bounded: {steps:?}"
        );
    }
}
