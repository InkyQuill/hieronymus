use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use chrono::Utc;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::{
    config::HieronymusConfig,
    db::{CrystalActivationRecord, DreamRunRecord, MemoryEventRecord},
    domain::{
        AddCrystalInput, ConceptProposalStore, ConceptStore, CreateConceptInput,
        CreateProposalInput, CrystalStore, ScoreDelta, TranslationContext, WorkspaceStore,
        apply_score_delta,
    },
    provider::PassName,
};

use super::{
    DecayManager, DecayScope, DreamAuditError, DreamAuditStore, DreamConfig,
    DreamCycleAlreadyRunning, DreamCycleGuard, DreamPhase, DreamPhaseError, DreamProviderResolver,
    DreamRunCompletion, LinkOutcome, LinkReinforcer, PhaseOutput, PhaseProfile, PhaseRunStart,
    Reconsolidator, ReinforcementManager, WorkflowProfile, acquire_dream_cycle_lock,
    execute_provider_passes,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleOptions {
    pub owner: String,
    pub wait: bool,
    pub skip_when_locked: bool,
    pub trigger_type: String,
    pub ignore_minimum: bool,
}

impl Default for CycleOptions {
    fn default() -> Self {
        Self {
            owner: "manual".into(),
            wait: false,
            skip_when_locked: false,
            trigger_type: "manual".into(),
            ignore_minimum: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaintenancePayload {
    pub reinforce: Vec<i64>,
    pub decay: Vec<i64>,
    pub combine: Vec<(i64, i64)>,
    pub supersede: Vec<(i64, i64)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaintenanceResult {
    pub reinforced: usize,
    pub decayed: usize,
    pub combined: usize,
    pub superseded: usize,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DreamServiceError {
    #[error(transparent)]
    Lock(#[from] DreamCycleAlreadyRunning),
    #[error(transparent)]
    Audit(#[from] DreamAuditError),
    #[error(transparent)]
    Phase(#[from] DreamPhaseError),
    #[error("dream service database operation failed")]
    Database(#[source] sqlx::Error),
    #[error("dream service input is invalid: {0}")]
    InvalidInput(&'static str),
    #[error("dream service domain operation failed: {0}")]
    Domain(String),
}

impl From<sqlx::Error> for DreamServiceError {
    fn from(source: sqlx::Error) -> Self {
        Self::Database(source)
    }
}

pub type Result<T> = std::result::Result<T, DreamServiceError>;

#[derive(Clone)]
pub struct DreamService<'a> {
    pool: &'a SqlitePool,
    config: &'a HieronymusConfig,
    dream_config: &'a DreamConfig,
    resolver: Arc<dyn DreamProviderResolver>,
}

impl<'a> DreamService<'a> {
    #[must_use]
    pub fn new(
        pool: &'a SqlitePool,
        config: &'a HieronymusConfig,
        dream_config: &'a DreamConfig,
        resolver: Arc<dyn DreamProviderResolver>,
    ) -> Self {
        Self {
            pool,
            config,
            dream_config,
            resolver,
        }
    }

    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        self.pool
    }

    #[must_use]
    pub fn config(&self) -> &HieronymusConfig {
        self.config
    }

    pub async fn run_cycle(&self, opts: CycleOptions) -> Result<DreamRunRecord> {
        let guard = match acquire_dream_cycle_lock(self.config, &opts.owner, opts.wait) {
            Ok(guard) => guard,
            Err(error) if opts.skip_when_locked && error.is_already_running() => {
                return self.record_skipped_run("dream cycle already running").await;
            }
            Err(error) => return Err(error.into()),
        };
        let mut lifecycle = LockAuditGuard::new(self.pool.clone(), guard);
        let result = self
            .run_unlocked(
                self.dream_config.max_short_term_memories_per_cycle,
                opts.ignore_minimum,
                &opts.trigger_type,
                Some(lifecycle.run_id()),
            )
            .await;
        lifecycle.disarm_if_terminal();
        result
    }

    pub async fn run_all(&self, opts: CycleOptions) -> Result<DreamRunRecord> {
        let guard = match acquire_dream_cycle_lock(self.config, &opts.owner, opts.wait) {
            Ok(guard) => guard,
            Err(error) if opts.skip_when_locked && error.is_already_running() => {
                return self.record_skipped_run("dream cycle already running").await;
            }
            Err(error) => return Err(error.into()),
        };
        let mut lifecycle = LockAuditGuard::new(self.pool.clone(), guard);
        let result = async {
            let mut last = self
                .run_unlocked(
                    self.dream_config.max_short_term_memories_per_run,
                    opts.ignore_minimum,
                    &opts.trigger_type,
                    Some(lifecycle.run_id()),
                )
                .await?;
            while self.pending_count().await? > 0 {
                last = self
                    .run_unlocked(
                        self.dream_config.max_short_term_memories_per_run,
                        true,
                        &opts.trigger_type,
                        Some(lifecycle.run_id()),
                    )
                    .await?;
            }
            Ok(last)
        }
        .await;
        lifecycle.disarm_if_terminal();
        result
    }

    /// Runs one due cycle while the caller holds the cross-process cycle lock.
    pub async fn run_due(&self) -> Result<Option<DreamRunRecord>> {
        if !self.dream_config.enabled
            || self.pending_count().await?
                < self.dream_config.min_pending_short_term_memories as i64
        {
            return Ok(None);
        }
        self.run_unlocked(
            self.dream_config.max_short_term_memories_per_cycle,
            false,
            "autostart",
            None,
        )
        .await
        .map(Some)
    }

    pub(crate) async fn run_due_with_lock(
        &self,
        guard: DreamCycleGuard,
    ) -> Result<Option<DreamRunRecord>> {
        let mut lifecycle = LockAuditGuard::new(self.pool.clone(), guard);
        let result = if !self.dream_config.enabled
            || self.pending_count().await?
                < self.dream_config.min_pending_short_term_memories as i64
        {
            Ok(None)
        } else {
            self.run_unlocked(
                self.dream_config.max_short_term_memories_per_cycle,
                false,
                "autostart",
                Some(lifecycle.run_id()),
            )
            .await
            .map(Some)
        };
        lifecycle.disarm_if_terminal();
        result
    }

    pub async fn decay_candidates(
        &self,
        ids: &[i64],
        reason: &str,
        deltas: ScoreDelta,
    ) -> Result<Vec<i64>> {
        if reason.trim().is_empty() {
            return Err(DreamServiceError::InvalidInput(
                "maintenance reason must not be empty",
            ));
        }
        let mut applied = Vec::new();
        for id in canonical_ids(ids, usize::MAX) {
            let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
            let crystal: crate::db::CrystalRecord =
                sqlx::query_as("SELECT * FROM crystals WHERE id=?")
                    .bind(id)
                    .fetch_one(&mut *transaction)
                    .await?;
            let (strength, confidence, status) = apply_score_delta(&crystal, deltas);
            if strength != crystal.strength || confidence != crystal.confidence {
                sqlx::query(
                    "UPDATE crystals SET strength=?,confidence=?,status=?,updated_at=? WHERE id=?",
                )
                .bind(strength)
                .bind(confidence)
                .bind(status)
                .bind(Utc::now())
                .bind(id)
                .execute(&mut *transaction)
                .await?;
                sqlx::query("INSERT INTO memory_events(crystal_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,created_at) VALUES (?,'maintenance_decay','system',?,?,?,1,?)")
                    .bind(id)
                    .bind(reason)
                    .bind(strength - crystal.strength)
                    .bind(confidence - crystal.confidence)
                    .bind(Utc::now())
                    .execute(&mut *transaction)
                    .await?;
                applied.push(id);
            }
            transaction.commit().await?;
        }
        Ok(applied)
    }

    pub async fn apply_maintenance(
        &self,
        payload: &MaintenancePayload,
        cycle_id: i64,
    ) -> Result<MaintenanceResult> {
        let limit = self.dream_config.max_changed_crystals_per_cycle;
        let reinforced_ids = canonical_ids(&payload.reinforce, limit);
        let mut reinforced = 0;
        for id in reinforced_ids {
            let event: MemoryEventRecord = sqlx::query_as(
                "INSERT INTO memory_events(crystal_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,cycle_id,created_at)
                 VALUES (?,'recalled_again','system','maintenance reinforcement',0,0,0,?,?) RETURNING *",
            )
            .bind(id)
            .bind(cycle_id)
            .bind(Utc::now())
            .fetch_one(self.pool)
            .await?;
            reinforced += ReinforcementManager::new(cycle_id)
                .run(self.pool, vec![event])
                .await?
                .len();
        }

        let remaining = limit.saturating_sub(reinforced);
        let decayed = self
            .decay_candidates(
                &canonical_ids(&payload.decay, remaining),
                "maintenance decay",
                ScoreDelta {
                    strength: -super::STRENGTH_DECAY_PER_CYCLE,
                    confidence: -super::CONFIDENCE_DECAY_PER_CYCLE,
                },
            )
            .await?
            .len();

        let mut combined = 0;
        let mut remaining = limit.saturating_sub(reinforced + decayed);
        for (left, right) in canonical_pairs(&payload.combine, remaining) {
            let session_id = synthetic_maintenance_session(self.pool, cycle_id).await?;
            let activations =
                synthetic_activations(self.pool, session_id, cycle_id, left, right).await?;
            combined += LinkReinforcer::new(cycle_id)
                .run(self.pool, activations)
                .await?
                .into_iter()
                .filter(|outcome| matches!(outcome, LinkOutcome::Combined { .. }))
                .count();
            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                break;
            }
        }

        let mut superseded = 0;
        for (old_id, new_id) in canonical_pairs(&payload.supersede, remaining) {
            let updated = sqlx::query(
                "UPDATE crystals SET status='superseded',updated_at=? WHERE id=? AND id<>? AND status IN ('active','candidate')",
            )
            .bind(Utc::now())
            .bind(old_id)
            .bind(new_id)
            .execute(self.pool)
            .await?;
            superseded += usize::try_from(updated.rows_affected()).unwrap_or(usize::MAX);
        }
        Ok(MaintenanceResult {
            reinforced,
            decayed,
            combined,
            superseded,
        })
    }

    async fn run_unlocked(
        &self,
        limit: usize,
        ignore_minimum: bool,
        trigger_type: &str,
        shared_run_id: Option<&Arc<AtomicI64>>,
    ) -> Result<DreamRunRecord> {
        let pending = self.pending_count().await?;
        let cycle_id = self.next_cycle_id().await?;
        let provider = self.provider_label();
        let run = DreamAuditStore::new(self.pool)
            .start_run(cycle_id, &provider)
            .await?;
        if let Some(shared_run_id) = shared_run_id {
            shared_run_id.store(run.id, Ordering::Release);
        }
        let mut cancellation = CancellationAudit::new(self.pool.clone(), run.id);
        DreamAuditStore::new(self.pool)
            .record(
                run.id,
                None,
                "run_trigger",
                "dream run trigger recorded",
                &serde_json::json!({"trigger_type": trigger_type}),
            )
            .await?;
        if !ignore_minimum && pending < self.dream_config.min_pending_short_term_memories as i64 {
            DreamAuditStore::new(self.pool)
                .complete_run(run.id, DreamRunCompletion::new(0, 0, 0))
                .await?;
            cancellation.disarm();
            clear_run_id(shared_run_id);
            return self.read_run(run.id).await;
        }

        let outcome = self.execute_run(run.id, cycle_id, limit).await;
        match outcome {
            Ok(counts) => {
                DreamAuditStore::new(self.pool)
                    .complete_run(run.id, counts)
                    .await?;
                cancellation.disarm();
                clear_run_id(shared_run_id);
                self.read_run(run.id).await
            }
            Err(error) => {
                let close_result = DreamAuditStore::new(self.pool)
                    .fail_run(run.id, DreamRunCompletion::new(0, 0, 0), &error.to_string())
                    .await;
                close_result?;
                cancellation.disarm();
                clear_run_id(shared_run_id);
                Err(error)
            }
        }
    }

    async fn execute_run(
        &self,
        run_id: i64,
        cycle_id: i64,
        limit: usize,
    ) -> Result<DreamRunCompletion> {
        let batches = self.pending_batches(limit).await?;
        if batches.is_empty() {
            return Ok(DreamRunCompletion::new(0, 0, 0));
        }
        let mut memories = Vec::new();
        let mut context = None;
        let mut session_ids = Vec::new();
        for batch in batches {
            context.get_or_insert(batch.context);
            session_ids.push(batch.session_id);
            memories.extend(batch.memories);
        }
        let context = context.ok_or(DreamServiceError::InvalidInput(
            "selected dream input has no translation context",
        ))?;
        let workflows = self.workflows();
        let mut outputs = Vec::with_capacity(workflows.len());
        for workflow in workflows {
            let phase = DreamAuditStore::new(self.pool)
                .start_phase(PhaseRunStart {
                    dream_run_id: run_id,
                    phase: workflow.phase.as_str(),
                    provider_profile: &workflow.provider,
                    provider_type: &workflow.provider,
                    model: &workflow.model,
                    input_count: memories.len() as i64,
                    prompt_hash: "",
                })
                .await?;
            let output = self
                .execute_phase(&workflow, context.clone(), memories.clone())
                .await?;
            let count = phase_output_count(&output);
            DreamAuditStore::new(self.pool)
                .complete_phase(phase.id, count as i64)
                .await?;
            outputs.push(output);
        }
        require_complete_coverage(&outputs, &memories)?;
        let (created, proposals) = self
            .apply_outputs(run_id, cycle_id, &context, &memories, &outputs)
            .await?;
        self.run_algorithmic_phases(cycle_id, &memories, &session_ids)
            .await?;
        for memory in &memories {
            let archived: bool = sqlx::query_scalar(
                "SELECT archived_at IS NOT NULL FROM short_term_memories WHERE id=?",
            )
            .bind(memory.id)
            .fetch_one(self.pool)
            .await?;
            if !archived {
                WorkspaceStore::new(self.pool)
                    .archive(memory.id)
                    .await
                    .map_err(|error| DreamServiceError::Domain(error.to_string()))?;
            }
        }
        sqlx::query(
            "UPDATE task_sessions SET cycle_id=? WHERE id IN (SELECT session_id FROM short_term_memories GROUP BY session_id HAVING max(archived_at IS NULL)=0)",
        )
        .bind(cycle_id)
        .execute(self.pool)
        .await?;
        Ok(DreamRunCompletion::new(
            memories.len() as i64,
            created as i64,
            proposals as i64,
        ))
    }

    async fn execute_phase(
        &self,
        workflow: &WorkflowProfile,
        context: TranslationContext,
        memories: Vec<crate::domain::ShortTermMemory>,
    ) -> Result<PhaseOutput> {
        execute_provider_passes(
            self.pool,
            self.resolver.as_ref(),
            std::slice::from_ref(workflow),
            context,
            memories,
        )
        .await?
        .pop()
        .ok_or(DreamServiceError::InvalidInput(
            "configured phase produced no output",
        ))
    }

    async fn apply_outputs(
        &self,
        run_id: i64,
        cycle_id: i64,
        context: &TranslationContext,
        memories: &[crate::domain::ShortTermMemory],
        outputs: &[PhaseOutput],
    ) -> Result<(usize, usize)> {
        let allowed: BTreeSet<i64> = memories.iter().map(|memory| memory.id).collect();
        let mut created = 0;
        let mut proposals = 0;
        let mut changed = BTreeSet::new();
        for output in outputs {
            match output {
                PhaseOutput::Concepts(value) => {
                    for candidate in &value.concepts {
                        let concept = ConceptStore::new(self.pool)
                            .create(CreateConceptInput {
                                canonical_name: candidate.concept.canonical_name.clone(),
                                scope_type: "series".into(),
                                scope_key: context.scope_key.clone(),
                            })
                            .await
                            .map_err(|_| {
                                DreamServiceError::InvalidInput(
                                    "concept output could not be persisted",
                                )
                            })?;
                        for (language, facet_type, facet_value) in &candidate.concept.facets {
                            ConceptStore::new(self.pool)
                                .add_facet(concept.id, language, facet_type, facet_value)
                                .await
                                .map_err(|_| {
                                    DreamServiceError::InvalidInput(
                                        "concept facet output could not be persisted",
                                    )
                                })?;
                        }
                    }
                }
                PhaseOutput::TerminologyCandidates(value) => {
                    for candidate in &value.concept_proposals {
                        let id = ConceptProposalStore::new(self.pool)
                            .create(CreateProposalInput {
                                series_slug: context.series_slug.clone(),
                                source_language: context.source_language.clone(),
                                target_language: context.target_language.clone(),
                                concept_text: candidate.concept_text.clone(),
                                source_form: candidate.source_form.clone(),
                                canonical_rendering: candidate.canonical_rendering.clone(),
                            })
                            .await
                            .map_err(|_| {
                                DreamServiceError::InvalidInput(
                                    "proposal output could not be persisted",
                                )
                            })?;
                        sqlx::query("UPDATE concept_proposals SET dream_run_id=? WHERE id=?")
                            .bind(run_id)
                            .bind(id)
                            .execute(self.pool)
                            .await?;
                        proposals += 1;
                    }
                }
                PhaseOutput::RuleCrystals(value) | PhaseOutput::KnowledgeCrystals(value) => {
                    for candidate in &value.crystals {
                        if changed.len() >= self.dream_config.max_changed_crystals_per_cycle {
                            break;
                        }
                        if candidate
                            .source_memory_ids
                            .iter()
                            .any(|id| !allowed.contains(id))
                        {
                            return Err(DreamServiceError::InvalidInput(
                                "crystal output references unknown memory",
                            ));
                        }
                        let id = CrystalStore::new(self.pool)
                            .add(AddCrystalInput {
                                crystal_type: candidate.crystal.crystal_type.clone(),
                                title: candidate.crystal.title.clone(),
                                text: candidate.crystal.text.clone(),
                                scope_type: "series".into(),
                                scope_key: context.scope_key.clone(),
                                series_slug: context.series_slug.clone(),
                                source_language: context.source_language.clone(),
                                target_language: context.target_language.clone(),
                                source_credibility: candidate.crystal.source_credibility.clone(),
                                rule_intent: candidate.crystal.rule_intent.clone(),
                                confidence: candidate.crystal.confidence,
                                malformed_penalty: candidate.crystal.malformed_penalty,
                                status: "active".into(),
                                ..AddCrystalInput::default()
                            })
                            .await
                            .map_err(|_| {
                                DreamServiceError::InvalidInput(
                                    "crystal output could not be persisted",
                                )
                            })?;
                        sqlx::query("UPDATE crystals SET created_cycle=? WHERE id=?")
                            .bind(cycle_id)
                            .bind(id)
                            .execute(self.pool)
                            .await?;
                        for source_id in &candidate.source_memory_ids {
                            sqlx::query("INSERT OR IGNORE INTO crystal_sources(crystal_id,short_term_memory_id) VALUES (?,?)")
                                .bind(id)
                                .bind(source_id)
                                .execute(self.pool)
                                .await?;
                        }
                        changed.insert(id);
                        created += 1;
                    }
                }
                PhaseOutput::Relations(value) => {
                    for relation in value
                        .relations
                        .iter()
                        .take(self.dream_config.max_relation_records_per_pass)
                    {
                        CrystalStore::new(self.pool)
                            .link(relation.source_id, relation.target_id, &relation.relation)
                            .await
                            .map_err(|_| {
                                DreamServiceError::InvalidInput(
                                    "relation output could not be persisted",
                                )
                            })?;
                    }
                }
                PhaseOutput::Reinforcement(value) => {
                    let mut candidates = value.reinforce.clone();
                    candidates.sort_by_key(|candidate| candidate.crystal_id);
                    candidates.dedup_by_key(|candidate| candidate.crystal_id);
                    for candidate in candidates
                        .into_iter()
                        .take(self.dream_config.max_changed_crystals_per_cycle)
                    {
                        let (strength_delta, confidence_delta) =
                            (candidate.strength_delta, candidate.confidence_delta);
                        let event: MemoryEventRecord = sqlx::query_as(
                            "INSERT INTO memory_events(crystal_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,cycle_id,created_at)
                             VALUES (?,'recalled_again','dream','provider reinforcement',?,?,0,?,?) RETURNING *",
                        )
                        .bind(candidate.crystal_id)
                        .bind(strength_delta)
                        .bind(confidence_delta)
                        .bind(cycle_id)
                        .bind(Utc::now())
                        .fetch_one(self.pool)
                        .await?;
                        ReinforcementManager::new(cycle_id)
                            .run(self.pool, vec![event])
                            .await?;
                    }
                }
                PhaseOutput::CoverageAudit(_) => {}
            }
        }
        Ok((created, proposals))
    }

    async fn run_algorithmic_phases(
        &self,
        cycle_id: i64,
        memories: &[crate::domain::ShortTermMemory],
        session_ids: &[i64],
    ) -> Result<()> {
        let limit = self.dream_config.max_changed_crystals_per_cycle;
        let mut working = Vec::new();
        for memory in memories
            .iter()
            .filter(|memory| memory.source_crystal_id.is_some())
            .take(limit)
        {
            let Some(source_crystal_id) = memory.source_crystal_id else {
                continue;
            };
            let source = CrystalStore::new(self.pool)
                .get(source_crystal_id)
                .await
                .map_err(|_| {
                    DreamServiceError::InvalidInput("working-copy source is unavailable")
                })?;
            working.push((memory.clone(), source));
        }
        let reconsolidated =
            Reconsolidator::new(self.dream_config.reconsolidation_diff_threshold, cycle_id)
                .run(self.pool, working)
                .await?;
        let mut changed = reconsolidated.len();

        let activations = self
            .bounded_activations(session_ids, cycle_id, limit.saturating_sub(changed))
            .await?;
        let outcomes = LinkReinforcer::new(cycle_id)
            .run(self.pool, activations.clone())
            .await?;
        changed += outcomes.len();
        let recalled_ids = activations
            .iter()
            .map(|activation| activation.crystal_id)
            .collect::<Vec<_>>();
        let linked_ids = outcomes
            .iter()
            .flat_map(|outcome| match outcome {
                LinkOutcome::Strengthened {
                    source_id,
                    target_id,
                } => [*source_id, *target_id],
                LinkOutcome::Combined {
                    survivor_id,
                    absorbed_id,
                } => [*survivor_id, *absorbed_id],
            })
            .collect();
        DecayManager
            .run(
                self.pool,
                DecayScope {
                    after_id: 0,
                    current_cycle: cycle_id,
                    stale_before_cycle: cycle_id,
                    recalled_ids,
                    linked_ids,
                    limit: limit.saturating_sub(changed),
                },
            )
            .await?;
        Ok(())
    }

    async fn bounded_activations(
        &self,
        session_ids: &[i64],
        cycle_id: i64,
        limit: usize,
    ) -> Result<Vec<CrystalActivationRecord>> {
        if session_ids.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut ids = session_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let mut update = QueryBuilder::<Sqlite>::new("UPDATE crystal_activations SET cycle_id=");
        update.push_bind(cycle_id).push(" WHERE session_id IN (");
        let mut separated = update.separated(",");
        for id in &ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") AND outcome='useful' AND cycle_id IS NULL");
        update.build().execute(self.pool).await?;

        let mut select =
            QueryBuilder::<Sqlite>::new("SELECT * FROM crystal_activations WHERE session_id IN (");
        let mut separated = select.separated(",");
        for id in &ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") AND outcome='useful' AND cycle_id=");
        select
            .push_bind(cycle_id)
            .push(" ORDER BY crystal_id,id LIMIT ")
            .push_bind(limit as i64);
        Ok(select
            .build_query_as::<CrystalActivationRecord>()
            .fetch_all(self.pool)
            .await?)
    }

    async fn pending_batches(&self, limit: usize) -> Result<Vec<PendingBatch>> {
        let session_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT task_sessions.id
             FROM task_sessions
             JOIN short_term_memories ON short_term_memories.session_id=task_sessions.id
             WHERE task_sessions.status='completed' AND short_term_memories.archived_at IS NULL
             GROUP BY task_sessions.id
             ORDER BY task_sessions.id",
        )
        .fetch_all(self.pool)
        .await?;
        let workspace = WorkspaceStore::new(self.pool);
        let mut selected = Vec::new();
        let mut remaining = limit;
        for session_id in session_ids {
            if remaining == 0 {
                break;
            }
            let session = workspace
                .get_session(session_id)
                .await
                .map_err(|_| DreamServiceError::InvalidInput("selected session is unavailable"))?;
            let mut memories = workspace.list_short_term(session_id).await.map_err(|_| {
                DreamServiceError::InvalidInput("selected memories are unavailable")
            })?;
            memories.truncate(remaining);
            if memories.is_empty() {
                continue;
            }
            remaining -= memories.len();
            let context = TranslationContext::new(
                &session.series_slug,
                &session.source_language,
                &session.target_language,
            )
            .with_metadata(
                &session.language_tags,
                &session.story_scopes,
                &session.semantic_tags,
                &[],
            );
            selected.push(PendingBatch {
                session_id,
                context,
                memories,
            });
        }
        Ok(selected)
    }

    async fn pending_count(&self) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM short_term_memories JOIN task_sessions ON task_sessions.id=short_term_memories.session_id WHERE task_sessions.status='completed' AND short_term_memories.archived_at IS NULL",
        )
        .fetch_one(self.pool)
        .await?)
    }

    async fn next_cycle_id(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar::<_, Option<i64>>("SELECT max(cycle_id) FROM dream_runs")
                .fetch_one(self.pool)
                .await?
                .unwrap_or(0)
                + 1,
        )
    }

    async fn read_run(&self, id: i64) -> Result<DreamRunRecord> {
        Ok(sqlx::query_as("SELECT * FROM dream_runs WHERE id=?")
            .bind(id)
            .fetch_one(self.pool)
            .await?)
    }

    async fn record_skipped_run(&self, reason: &str) -> Result<DreamRunRecord> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let cycle_id: i64 =
            sqlx::query_scalar::<_, Option<i64>>("SELECT max(cycle_id) FROM dream_runs")
                .fetch_one(&mut *transaction)
                .await?
                .unwrap_or(0)
                + 1;
        let now = Utc::now();
        let run = sqlx::query_as(
            "INSERT INTO dream_runs(cycle_id,status,provider,error,created_at,completed_at)
             VALUES (?,'skipped',?,?,?,?) RETURNING *",
        )
        .bind(cycle_id)
        .bind(self.provider_label())
        .bind(reason)
        .bind(now)
        .bind(now)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(run)
    }

    fn workflows(&self) -> Vec<WorkflowProfile> {
        PassName::ALL
            .into_iter()
            .filter_map(|phase| {
                let profile = &self.dream_config.workflows[&phase];
                profile.enabled.then(|| workflow(phase, profile))
            })
            .collect()
    }

    fn provider_label(&self) -> String {
        self.workflows()
            .first()
            .map_or_else(|| "none".into(), |workflow| workflow.provider.clone())
    }
}

struct PendingBatch {
    session_id: i64,
    context: TranslationContext,
    memories: Vec<crate::domain::ShortTermMemory>,
}

fn workflow(phase: PassName, profile: &PhaseProfile) -> WorkflowProfile {
    WorkflowProfile {
        phase,
        provider: profile.provider.clone(),
        model: profile.model.clone(),
        max_records_per_pass: profile.max_records_per_pass,
    }
}

fn phase_output_count(output: &PhaseOutput) -> usize {
    match output {
        PhaseOutput::Concepts(value) => value.concepts.len(),
        PhaseOutput::TerminologyCandidates(value) => value.concept_proposals.len(),
        PhaseOutput::RuleCrystals(value) | PhaseOutput::KnowledgeCrystals(value) => {
            value.crystals.len()
        }
        PhaseOutput::Relations(value) => value.relations.len(),
        PhaseOutput::Reinforcement(value) => value.reinforce.len(),
        PhaseOutput::CoverageAudit(value) => value.covered_memory_ids.len(),
    }
}

fn require_complete_coverage(
    outputs: &[PhaseOutput],
    memories: &[crate::domain::ShortTermMemory],
) -> Result<()> {
    let expected: BTreeSet<i64> = memories.iter().map(|memory| memory.id).collect();
    let covered: BTreeSet<i64> = outputs
        .iter()
        .filter_map(|output| match output {
            PhaseOutput::CoverageAudit(value) => Some(value.covered_memory_ids.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect();
    if outputs
        .iter()
        .any(|output| matches!(output, PhaseOutput::CoverageAudit(_)))
        && covered != expected
    {
        return Err(DreamServiceError::InvalidInput(
            "coverage audit did not cover every selected memory",
        ));
    }
    Ok(())
}

fn canonical_ids(ids: &[i64], limit: usize) -> Vec<i64> {
    ids.iter()
        .copied()
        .filter(|id| *id > 0)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(limit)
        .collect()
}

fn canonical_pairs(pairs: &[(i64, i64)], limit: usize) -> Vec<(i64, i64)> {
    pairs
        .iter()
        .filter_map(|&(left, right)| {
            (left > 0 && right > 0 && left != right).then_some((left.min(right), left.max(right)))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(limit)
        .collect()
}

async fn synthetic_maintenance_session(pool: &SqlitePool, cycle_id: i64) -> Result<i64> {
    let slug = format!("maintenance-{cycle_id}");
    sqlx::query("INSERT OR IGNORE INTO series(slug,title,default_source_language,default_target_language,created_at,updated_at) VALUES (?,?,'','',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)")
        .bind(&slug)
        .bind(&slug)
        .execute(pool)
        .await?;
    Ok(sqlx::query_scalar("INSERT INTO task_sessions(series_slug,source_language,target_language,task_type,volume,chapter,status,cycle_id,created_at,last_activity_at,completed_at) VALUES (?,'','','maintenance','','','completed',?,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP) RETURNING id")
        .bind(slug)
        .bind(cycle_id)
        .fetch_one(pool)
        .await?)
}

async fn synthetic_activations(
    pool: &SqlitePool,
    session_id: i64,
    cycle_id: i64,
    left: i64,
    right: i64,
) -> Result<Vec<CrystalActivationRecord>> {
    let mut records = Vec::new();
    for crystal_id in [left, right] {
        records.push(sqlx::query_as("INSERT INTO crystal_activations(crystal_id,session_id,recall_query,rank,score,reason,outcome,cycle_id,created_at) VALUES (?,?,'maintenance',1,1.0,'maintenance','useful',?,CURRENT_TIMESTAMP) RETURNING *")
            .bind(crystal_id)
            .bind(session_id)
            .bind(cycle_id)
            .fetch_one(pool)
            .await?);
    }
    Ok(records)
}

fn clear_run_id(shared_run_id: Option<&Arc<AtomicI64>>) {
    if let Some(shared_run_id) = shared_run_id {
        shared_run_id.store(0, Ordering::Release);
    }
}

struct LockAuditGuard {
    pool: SqlitePool,
    lock: Option<DreamCycleGuard>,
    run_id: Arc<AtomicI64>,
    armed: bool,
}

impl LockAuditGuard {
    fn new(pool: SqlitePool, lock: DreamCycleGuard) -> Self {
        Self {
            pool,
            lock: Some(lock),
            run_id: Arc::new(AtomicI64::new(0)),
            armed: true,
        }
    }

    fn run_id(&self) -> &Arc<AtomicI64> {
        &self.run_id
    }

    fn disarm_if_terminal(&mut self) {
        if self.run_id.load(Ordering::Acquire) == 0 {
            self.armed = false;
            self.lock.take();
        }
    }
}

impl Drop for LockAuditGuard {
    fn drop(&mut self) {
        let Some(lock) = self.lock.take() else {
            return;
        };
        if !self.armed {
            drop(lock);
            return;
        }
        let pool = self.pool.clone();
        let run_id = self.run_id.load(Ordering::Acquire);
        if run_id == 0 {
            drop(lock);
            return;
        }
        tokio::spawn(async move {
            let _ = DreamAuditStore::new(&pool)
                .fail_run(
                    run_id,
                    DreamRunCompletion::new(0, 0, 0),
                    "dream cycle cancelled",
                )
                .await;
            drop(lock);
        });
    }
}

struct CancellationAudit {
    pool: SqlitePool,
    run_id: i64,
    armed: bool,
}

impl CancellationAudit {
    fn new(pool: SqlitePool, run_id: i64) -> Self {
        Self {
            pool,
            run_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancellationAudit {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pool = self.pool.clone();
        let run_id = self.run_id;
        tokio::spawn(async move {
            let _ = DreamAuditStore::new(&pool)
                .fail_run(
                    run_id,
                    DreamRunCompletion::new(0, 0, 0),
                    "dream cycle cancelled",
                )
                .await;
        });
    }
}
