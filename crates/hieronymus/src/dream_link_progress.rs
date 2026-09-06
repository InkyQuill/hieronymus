//! Durable per-batch pair progress for the dream link-reinforcement phase
//! (task D4; memory-reconsolidation design, astra finding 3.2).
//!
//! The phase consumes `useful` crystal activations as unique unordered
//! crystal pairs. Before this module, a cycle budget smaller than the pair
//! count still stamped every selected activation consumed
//! (`crystal_activations.cycle_id`), permanently dropping the unprocessed
//! pairs. Here the eligible activations of one session are snapshotted into
//! ONE durable batch ([`crate::db`] schema v2 `dream_link_batches` /
//! `dream_link_members`) whose unique unordered pairs are materialized as
//! `queued` rows (`dream_link_pairs`). Every budgeted pair commits its
//! reinforcement, its pair-row terminalization (status / applied_cycle /
//! result_json), and its audit entry in ONE immediate transaction (the task
//! D3 primitives), so an exhausted budget or a crash leaves the remaining
//! pairs queued and the next cycle resumes exactly where the previous one
//! stopped. A batch's activations are stamped consumed only when its last
//! pair is applied or explicitly skipped (deleted crystals produce audited
//! skip tombstones, never silent drops), in the same transaction as the
//! batch completion. An open batch is resumed, never re-snapshotted: work
//! already committed is never recreated from still-unconsumed activations.

use std::collections::BTreeSet;

use rusqlite::Connection;
use serde_json::{Value, json};

use crate::data_root::HieronymusConfig;
use crate::db::open_migrated;
use crate::dream_audit::{DreamAuditStore, commit_audited};
use crate::dream_config::{DreamConfig, load_dream_config};
use crate::dreaming::{
    COMBINATION_SIMILARITY_THRESHOLD, DreamError, is_active_rule, now, token_similarity, tx_error,
};

/// Weight of a `crystal_links` row created by hebbian co-activation.
const LINK_INITIAL_WEIGHT: f64 = 0.5;

/// Additive weight gain per co-activation cycle, capped at [`LINK_WEIGHT_MAX`].
const HEBBIAN_STRENGTH_DELTA: f64 = 0.1;

const LINK_WEIGHT_MAX: f64 = 1.0;

/// `crystal_links.link_type` for hebbian co-activation links.
const CO_ACTIVATION_LINK_TYPE: &str = "co_activation";

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
        if pair_budget == 0 {
            return Ok(0);
        }
        self.combination_budget = self.dream_config.max_changed_crystals_per_cycle.max(0) as usize;
        self.terminalized_in_call = 0;
        let mut connection = open_migrated(&self.config.database_path())?;
        let mut budget = pair_budget;

        // Resume open batches (oldest first): a batch with queued pairs is
        // continued where the previous call stopped, never re-snapshotted.
        for batch_id in open_batch_ids(&connection)? {
            if budget == 0 {
                break;
            }
            self.drive_batch(&mut connection, batch_id, cycle, &mut budget)?;
        }

        // Fresh snapshots: one durable batch per session that still has
        // unconsumed useful activations and no open batch (session
        // isolation: activations from different sessions never share one).
        if budget > 0 {
            for session_id in sessions_without_open_batch(&connection)? {
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
        // An open batch whose pairs are all terminal (e.g. a crash between
        // the last pair commit and batch completion) completes without new
        // pair work.
        if queued_pair_count(connection, batch_id)? == 0 {
            self.complete_batch(connection, batch_id, cycle)?;
            return Ok(());
        }
        let queued = queued_pairs(connection, batch_id)?;
        for (left, right) in queued {
            if *budget == 0 {
                return Ok(());
            }
            self.terminalize_pair(connection, batch_id, cycle, left, right)?;
            *budget -= 1;
            if queued_pair_count(connection, batch_id)? == 0 {
                self.complete_batch(connection, batch_id, cycle)?;
            }
        }
        Ok(())
    }

    /// Snapshot one session's unconsumed useful activations into a durable
    /// batch: UNIQUE activation membership, unique unordered crystal pairs
    /// materialized as queued rows. One immediate transaction, so a crash
    /// leaves either no batch or a complete one. `None` when the session has
    /// no unconsumed activations (no batch is created).
    fn snapshot_batch(
        &mut self,
        connection: &mut Connection,
        cycle: i64,
        session_id: i64,
    ) -> Result<Option<i64>, DreamError> {
        Ok(commit_audited(connection, |transaction| {
            let activations: Vec<(i64, i64)> = {
                let mut statement = transaction.prepare(
                    "select id, crystal_id from crystal_activations
                     where outcome = 'useful' and cycle_id is null and session_id = ?1
                     order by id",
                )?;
                let rows =
                    statement.query_map([session_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            if activations.is_empty() {
                return Ok(None);
            }
            let mut member_ids = BTreeSet::new();
            let mut crystal_ids = BTreeSet::new();
            for (activation_id, crystal_id) in activations {
                member_ids.insert(activation_id);
                crystal_ids.insert(crystal_id);
            }
            transaction.execute(
                "insert into dream_link_batches(session_id, created_cycle) values (?1, ?2)",
                rusqlite::params![session_id, cycle],
            )?;
            let batch_id = transaction.last_insert_rowid();
            for activation_id in &member_ids {
                transaction.execute(
                    "insert into dream_link_members(batch_id, activation_id) values (?1, ?2)",
                    rusqlite::params![batch_id, activation_id],
                )?;
            }
            let crystals: Vec<i64> = crystal_ids.into_iter().collect();
            for (left, right) in canonical_pairs(&crystals) {
                transaction.execute(
                    "insert into dream_link_pairs(batch_id, left_id, right_id) values (?1, ?2, ?3)",
                    rusqlite::params![batch_id, left, right],
                )?;
            }
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
            // Fail closed: the queued pair row must be the one this commit
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
            let counts: Vec<(String, i64)> = {
                let mut statement = transaction.prepare(
                    "select status, count(*) from dream_link_pairs
                     where batch_id = ?1 group by status order by status",
                )?;
                let rows = statement.query_map([batch_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let applied = counts
                .iter()
                .find(|(status, _)| status == "applied")
                .map_or(0, |(_, count)| *count);
            let skipped = counts
                .iter()
                .find(|(status, _)| status == "skipped")
                .map_or(0, |(_, count)| *count);
            let members: Vec<i64> = {
                let mut statement = transaction.prepare(
                    "select activation_id from dream_link_members
                     where batch_id = ?1 order by activation_id",
                )?;
                let rows = statement.query_map([batch_id], |row| row.get(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
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
                        "activation_ids": members,
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
fn open_batch_ids(connection: &Connection) -> Result<Vec<i64>, DreamError> {
    let mut statement = connection.prepare(
        "select id from dream_link_batches
         where completed_cycle is null order by id",
    )?;
    let rows = statement.query_map([], |row| row.get(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

/// Sessions with unconsumed useful activations but no open batch, in
/// deterministic (session id) order.
fn sessions_without_open_batch(connection: &Connection) -> Result<Vec<i64>, DreamError> {
    let mut statement = connection.prepare(
        "select distinct ca.session_id
         from crystal_activations ca
         where ca.outcome = 'useful' and ca.cycle_id is null
           and not exists (
             select 1 from dream_link_batches b
             where b.session_id = ca.session_id and b.completed_cycle is null
           )
         order by ca.session_id",
    )?;
    let rows = statement.query_map([], |row| row.get(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn queued_pair_count(connection: &Connection, batch_id: i64) -> Result<usize, DreamError> {
    let count: i64 = connection.query_row(
        "select count(*) from dream_link_pairs
         where batch_id = ?1 and status = 'queued'",
        [batch_id],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

fn queued_pairs(connection: &Connection, batch_id: i64) -> Result<Vec<(i64, i64)>, DreamError> {
    let mut statement = connection.prepare(
        "select left_id, right_id from dream_link_pairs
         where batch_id = ?1 and status = 'queued'
         order by left_id, right_id",
    )?;
    let rows = statement.query_map([batch_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut pairs = Vec::new();
    for row in rows {
        pairs.push(row?);
    }
    Ok(pairs)
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
}
