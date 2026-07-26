use std::{
    collections::BTreeSet,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::Value;
use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

use crate::{
    config::HieronymusConfig,
    db::{CrystalActivationRecord, DreamRunRecord, MemoryEventRecord},
    domain::{
        AddCrystalInput, CrystalStore, ScoreDelta, TranslationContext, WorkspaceStore,
        add_crystal_in_transaction, apply_score_delta,
    },
    provider::ProviderCatalog,
};

use super::{
    Consolidator, DecayManager, DecayScope, DreamAuditError, DreamAuditStore, DreamConfig,
    DreamCycleAlreadyRunning, DreamCycleGuard, DreamPhase, DreamPhaseError, DreamProviderResolver,
    DreamRunCompletion, LinkOutcome, LinkReinforcer, PhaseOutput, PhaseRunStart, Reconsolidator,
    ReinforcementManager, WorkflowProfile, acquire_dream_cycle_lock,
    budget::{AffectedCrystalIds, load_affected, record_affected},
    execute_provider_passes_with_config, resolve_workflows,
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

struct OutputApplication<'a> {
    run_id: i64,
    cycle_id: i64,
    maintenance_cycle_id: i64,
    context: &'a TranslationContext,
    memories: &'a [crate::domain::ShortTermMemory],
    outputs: &'a [PhaseOutput],
}

#[derive(Clone)]
pub struct DreamService<'a> {
    pool: SqlitePool,
    config: HieronymusConfig,
    dream_config: DreamConfig,
    resolver: Arc<dyn DreamProviderResolver>,
    provider_catalog: Option<ProviderCatalog>,
    cleanup_deadline: Option<Duration>,
    cleanup_failed: Arc<AtomicBool>,
    lifetime: PhantomData<&'a ()>,
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
            pool: pool.clone(),
            config: config.clone(),
            dream_config: dream_config.clone(),
            resolver,
            provider_catalog: None,
            cleanup_deadline: None,
            cleanup_failed: Arc::new(AtomicBool::new(false)),
            lifetime: PhantomData,
        }
    }

    #[must_use]
    pub fn new_with_catalog(
        pool: &'a SqlitePool,
        config: &'a HieronymusConfig,
        dream_config: &'a DreamConfig,
        resolver: Arc<dyn DreamProviderResolver>,
        provider_catalog: ProviderCatalog,
    ) -> Self {
        Self {
            pool: pool.clone(),
            config: config.clone(),
            dream_config: dream_config.clone(),
            resolver,
            provider_catalog: Some(provider_catalog),
            cleanup_deadline: None,
            cleanup_failed: Arc::new(AtomicBool::new(false)),
            lifetime: PhantomData,
        }
    }

    #[must_use]
    pub fn with_cleanup_deadline(mut self, deadline: Duration) -> Self {
        self.cleanup_deadline = Some(deadline);
        self
    }

    pub(crate) fn take_cleanup_failure(&self) -> bool {
        self.cleanup_failed.swap(false, Ordering::AcqRel)
    }

    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    #[must_use]
    pub fn config(&self) -> &HieronymusConfig {
        &self.config
    }

    pub async fn run_cycle(&self, opts: CycleOptions) -> Result<DreamRunRecord> {
        let guard = match self.acquire_lock(&opts.owner, opts.wait).await {
            Ok(guard) => guard,
            Err(DreamServiceError::Lock(error))
                if opts.skip_when_locked && error.is_already_running() =>
            {
                return self.record_skipped_run("dream cycle already running").await;
            }
            Err(error) => return Err(error),
        };
        match self.supervise(guard, SupervisedRun::Cycle(opts)).await? {
            SupervisedResult::Run(run) => Ok(run),
            SupervisedResult::Due(_) => Err(DreamServiceError::InvalidInput(
                "cycle supervisor returned an invalid result",
            )),
        }
    }

    pub async fn run_all(&self, opts: CycleOptions) -> Result<DreamRunRecord> {
        let guard = match self.acquire_lock(&opts.owner, opts.wait).await {
            Ok(guard) => guard,
            Err(DreamServiceError::Lock(error))
                if opts.skip_when_locked && error.is_already_running() =>
            {
                return self.record_skipped_run("dream cycle already running").await;
            }
            Err(error) => return Err(error),
        };
        match self.supervise(guard, SupervisedRun::All(opts)).await? {
            SupervisedResult::Run(run) => Ok(run),
            SupervisedResult::Due(_) => Err(DreamServiceError::InvalidInput(
                "run-all supervisor returned an invalid result",
            )),
        }
    }

    pub async fn run_due(&self) -> Result<Option<DreamRunRecord>> {
        let guard = self.acquire_lock("autostart", false).await?;
        match self.supervise(guard, SupervisedRun::Due).await? {
            SupervisedResult::Due(run) => Ok(run),
            SupervisedResult::Run(_) => Err(DreamServiceError::InvalidInput(
                "due-cycle supervisor returned an invalid result",
            )),
        }
    }

    pub(crate) async fn acquire_lock(&self, owner: &str, wait: bool) -> Result<DreamCycleGuard> {
        let owner = owner.to_owned();
        loop {
            let config = self.config.clone();
            let owner = owner.clone();
            let attempt = tokio::task::spawn_blocking(move || {
                acquire_dream_cycle_lock(&config, &owner, false)
            })
            .await
            .map_err(|_| DreamServiceError::Domain("dream lock worker failed".into()))?;
            match attempt {
                Ok(guard) => return Ok(guard),
                Err(error) if wait && error.is_already_running() => {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub(crate) async fn run_due_with_lock(
        &self,
        guard: DreamCycleGuard,
    ) -> Result<Option<DreamRunRecord>> {
        match self.supervise(guard, SupervisedRun::Due).await? {
            SupervisedResult::Due(run) => Ok(run),
            SupervisedResult::Run(_) => Err(DreamServiceError::InvalidInput(
                "due-cycle supervisor returned an invalid result",
            )),
        }
    }

    async fn run_due_unlocked_with_run_id(
        &self,
        run_id: Option<&Arc<AtomicI64>>,
    ) -> Result<Option<DreamRunRecord>> {
        let consolidation_due = Consolidator::maintenance_due(&self.pool).await?;
        if !self.dream_config.enabled
            || (self.resumable_algorithm_batch().await?.is_none()
                && !consolidation_due
                && self.pending_count().await?
                    < self.dream_config.min_pending_short_term_memories as i64)
        {
            return Ok(None);
        }
        let mut long_term_budget =
            LongTermBudget::new(self.dream_config.max_long_term_records_affected_per_run);
        self.run_unlocked(
            self.dream_config.max_short_term_memories_per_cycle,
            false,
            "autostart",
            run_id,
            &mut long_term_budget,
        )
        .await
        .map(Some)
    }

    async fn supervise(
        &self,
        guard: DreamCycleGuard,
        request: SupervisedRun,
    ) -> Result<SupervisedResult> {
        let service = self.to_owned_service();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            supervise_blocking(service, guard, request, sender);
        });
        receiver.await.map_err(|_| {
            DreamServiceError::Domain("dream cycle supervisor stopped unexpectedly".into())
        })?
    }

    fn to_owned_service(&self) -> DreamService<'static> {
        DreamService {
            pool: self.pool.clone(),
            config: self.config.clone(),
            dream_config: self.dream_config.clone(),
            resolver: self.resolver.clone(),
            provider_catalog: self.provider_catalog.clone(),
            cleanup_deadline: self.cleanup_deadline,
            cleanup_failed: self.cleanup_failed.clone(),
            lifetime: PhantomData,
        }
    }

    async fn execute_supervised(
        &self,
        request: SupervisedRun,
        run_id: &Arc<AtomicI64>,
    ) -> Result<SupervisedResult> {
        match request {
            SupervisedRun::Cycle(opts) => {
                let mut long_term_budget =
                    LongTermBudget::new(self.dream_config.max_long_term_records_affected_per_run);
                self.run_unlocked(
                    self.dream_config.max_short_term_memories_per_cycle,
                    opts.ignore_minimum,
                    &opts.trigger_type,
                    Some(run_id),
                    &mut long_term_budget,
                )
                .await
                .map(SupervisedResult::Run)
            }
            SupervisedRun::All(opts) => {
                let mut remaining = self.dream_config.max_short_term_memories_per_run;
                let mut long_term_budget =
                    LongTermBudget::new(self.dream_config.max_long_term_records_affected_per_run);
                let mut last = self
                    .run_unlocked(
                        self.dream_config
                            .max_short_term_memories_per_cycle
                            .min(remaining),
                        opts.ignore_minimum,
                        &opts.trigger_type,
                        Some(run_id),
                        &mut long_term_budget,
                    )
                    .await?;
                remaining = remaining.saturating_sub(last.input_count as usize);
                while remaining > 0 && self.pending_count().await? > 0 {
                    last = self
                        .run_unlocked(
                            self.dream_config
                                .max_short_term_memories_per_cycle
                                .min(remaining),
                            true,
                            &opts.trigger_type,
                            Some(run_id),
                            &mut long_term_budget,
                        )
                        .await?;
                    if last.input_count == 0 {
                        break;
                    }
                    remaining = remaining.saturating_sub(last.input_count as usize);
                }
                Ok(SupervisedResult::Run(last))
            }
            SupervisedRun::Due => self
                .run_due_unlocked_with_run_id(Some(run_id))
                .await
                .map(SupervisedResult::Due),
        }
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
        let mut budget = ChangeBudget::new(self.dream_config.max_changed_crystals_per_cycle);
        let mut reinforced = 0;
        for id in canonical_ids(&payload.reinforce, usize::MAX) {
            if !budget.reserve(&[id]) {
                break;
            }
            let event: MemoryEventRecord = sqlx::query_as(
                "INSERT INTO memory_events(crystal_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,cycle_id,created_at)
                 VALUES (?,'recalled_again','system','maintenance reinforcement',0,0,0,?,?) RETURNING *",
            )
            .bind(id)
            .bind(cycle_id)
            .bind(Utc::now())
            .fetch_one(&self.pool)
            .await?;
            reinforced += ReinforcementManager::new(cycle_id)
                .run(&self.pool, vec![event])
                .await?
                .len();
        }

        let mut decayed = 0;
        for id in canonical_ids(&payload.decay, usize::MAX) {
            if !budget.reserve(&[id]) {
                break;
            }
            decayed += self
                .decay_candidates(
                    &[id],
                    "maintenance decay",
                    ScoreDelta {
                        strength: -super::STRENGTH_DECAY_PER_CYCLE,
                        confidence: -super::CONFIDENCE_DECAY_PER_CYCLE,
                    },
                )
                .await?
                .len();
        }

        let mut combined = 0;
        for (left, right) in canonical_pairs(&payload.combine, usize::MAX) {
            if !budget.reserve(&[left, right]) {
                break;
            }
            let session_id = synthetic_maintenance_session(&self.pool, cycle_id).await?;
            let activations =
                synthetic_activations(&self.pool, session_id, cycle_id, left, right).await?;
            combined += LinkReinforcer::new(cycle_id)
                .run(&self.pool, activations)
                .await?
                .into_iter()
                .filter(|outcome| matches!(outcome, LinkOutcome::Combined { .. }))
                .count();
        }

        let mut superseded = 0;
        for (old_id, new_id) in canonical_directed_pairs(&payload.supersede) {
            if !budget.reserve(&[old_id, new_id]) {
                break;
            }
            let updated = sqlx::query(
                "UPDATE crystals SET status='superseded',updated_at=? WHERE id=? AND id<>? AND status IN ('active','candidate')",
            )
            .bind(Utc::now())
            .bind(old_id)
            .bind(new_id)
            .execute(&self.pool)
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
        long_term_budget: &mut LongTermBudget,
    ) -> Result<DreamRunRecord> {
        let pending = self.pending_count().await?;
        let has_resumable_batch = self.resumable_algorithm_batch().await?.is_some();
        let consolidation_due = Consolidator::maintenance_due(&self.pool).await?;
        let cycle_id = self.next_cycle_id().await?;
        let workflows = self.workflows()?;
        let provider = workflows
            .first()
            .map_or_else(|| "none".into(), |workflow| workflow.provider.clone());
        let run = DreamAuditStore::new(&self.pool)
            .start_run(cycle_id, &provider)
            .await?;
        if let Some(shared_run_id) = shared_run_id {
            shared_run_id.store(run.id, Ordering::Release);
        }
        DreamAuditStore::new(&self.pool)
            .record(
                run.id,
                None,
                "run_trigger",
                "dream run trigger recorded",
                &serde_json::json!({"trigger_type": trigger_type}),
            )
            .await?;
        if !ignore_minimum
            && !has_resumable_batch
            && !consolidation_due
            && pending < self.dream_config.min_pending_short_term_memories as i64
        {
            DreamAuditStore::new(&self.pool)
                .complete_run(run.id, DreamRunCompletion::new(0, 0, 0))
                .await?;
            clear_run_id(shared_run_id);
            return self.read_run(run.id).await;
        }

        let outcome = self
            .execute_run(run.id, cycle_id, limit, &workflows, long_term_budget)
            .await;
        match outcome {
            Ok(counts) => {
                DreamAuditStore::new(&self.pool)
                    .complete_run(run.id, counts)
                    .await?;
                clear_run_id(shared_run_id);
                self.read_run(run.id).await
            }
            Err(error) => {
                let close_result = DreamAuditStore::new(&self.pool)
                    .fail_run(run.id, DreamRunCompletion::new(0, 0, 0), &error.to_string())
                    .await;
                close_result?;
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
        workflows: &[WorkflowProfile],
        long_term_budget: &mut LongTermBudget,
    ) -> Result<DreamRunCompletion> {
        let (batches, resumed_stage) = self.pending_or_resumable_batches(limit).await?;
        if batches.is_empty() {
            let proposals =
                Consolidator::new(run_id, self.dream_config.max_related_concepts_per_cycle)
                    .scan(&self.pool)
                    .await?;
            return Ok(DreamRunCompletion::new(0, 0, proposals.len() as i64));
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
        let stage = match resumed_stage {
            Some(stage) => stage,
            None => {
                self.start_algorithm_batch(run_id, cycle_id, &memories)
                    .await?
            }
        };
        let existing_affected = load_affected(&self.pool, stage.maintenance_cycle_id).await?;
        let mut cycle_budget = AffectedCrystalIds::with_ids(
            self.dream_config
                .max_changed_crystals_per_cycle
                .min(self.dream_config.max_total_affected_crystals),
            existing_affected.iter().copied(),
        );
        long_term_budget.include_existing(existing_affected);
        let mut outputs = Vec::with_capacity(workflows.len());
        for workflow in workflows {
            let phase = DreamAuditStore::new(&self.pool)
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
                .execute_phase(workflow, context.clone(), memories.clone())
                .await?;
            let count = phase_output_count(&output);
            DreamAuditStore::new(&self.pool)
                .complete_phase(phase.id, count as i64)
                .await?;
            outputs.push(output);
        }
        require_complete_coverage(&outputs, &memories)?;
        if !stage.algorithms_completed {
            self.run_algorithmic_phases(
                run_id,
                stage.maintenance_cycle_id,
                &memories,
                &session_ids,
                &mut cycle_budget,
                long_term_budget,
            )
            .await?;
            self.complete_algorithm_batch(run_id, &stage.batch_id)
                .await?;
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let (created, proposals) = self
            .apply_outputs(
                &mut transaction,
                OutputApplication {
                    run_id,
                    cycle_id,
                    maintenance_cycle_id: stage.maintenance_cycle_id,
                    context: &context,
                    memories: &memories,
                    outputs: &outputs,
                },
                &mut cycle_budget,
                long_term_budget,
            )
            .await?;
        let now = Utc::now();
        for memory in &memories {
            sqlx::query(
                "UPDATE short_term_memories SET archived_at=? WHERE id=? AND archived_at IS NULL",
            )
            .bind(now)
            .bind(memory.id)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "UPDATE task_sessions SET cycle_id=? WHERE id IN (SELECT session_id FROM short_term_memories GROUP BY session_id HAVING max(archived_at IS NULL)=0)",
        )
        .bind(cycle_id)
        .execute(&mut *transaction)
        .await?;
        DreamAuditStore::new(&self.pool)
            .record_in_transaction(
                &mut transaction,
                run_id,
                None,
                "algorithm_batch_committed",
                "dream algorithm batch committed",
                &serde_json::json!({"batch": stage.batch_id}),
            )
            .await?;
        transaction.commit().await?;
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
        execute_provider_passes_with_config(
            &self.pool,
            self.resolver.as_ref(),
            std::slice::from_ref(workflow),
            &self.dream_config,
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
        transaction: &mut SqliteConnection,
        application: OutputApplication<'_>,
        cycle_budget: &mut AffectedCrystalIds,
        long_term_budget: &mut LongTermBudget,
    ) -> Result<(usize, usize)> {
        let OutputApplication {
            run_id,
            cycle_id,
            maintenance_cycle_id,
            context,
            memories,
            outputs,
        } = application;
        let allowed: BTreeSet<i64> = memories.iter().map(|memory| memory.id).collect();
        let mut created = 0;
        let mut proposals = 0;
        for output in outputs {
            match output {
                PhaseOutput::Concepts(value) => {
                    for candidate in value
                        .concepts
                        .iter()
                        .take(self.dream_config.max_related_concepts_per_cycle)
                    {
                        let now = Utc::now();
                        let concept_id = sqlx::query("INSERT INTO concepts(canonical_name,scope_type,scope_key,status,confidence,created_at,updated_at) VALUES (?,'series',?,'candidate',0.2,?,?)")
                            .bind(candidate.concept.canonical_name.trim())
                            .bind(&context.scope_key)
                            .bind(now)
                            .bind(now)
                            .execute(&mut *transaction)
                            .await?
                            .last_insert_rowid();
                        for (facet_type, facet_value, language) in &candidate.concept.facets {
                            let facet_id = sqlx::query("INSERT INTO concept_facets(concept_id,language,facet_type,value,created_at,updated_at) VALUES (?,?,?,?,?,?)")
                                .bind(concept_id)
                                .bind(language.trim().to_lowercase())
                                .bind(facet_type.trim())
                                .bind(facet_value.trim())
                                .bind(now)
                                .bind(now)
                                .execute(&mut *transaction)
                                .await?
                                .last_insert_rowid();
                            if !language.trim().is_empty() {
                                sqlx::query("INSERT INTO concept_facet_language_tags(facet_id,language_tag) VALUES (?,?)")
                                    .bind(facet_id)
                                    .bind(language.trim().to_lowercase())
                                    .execute(&mut *transaction)
                                    .await?;
                            }
                        }
                    }
                }
                PhaseOutput::TerminologyCandidates(value) => {
                    for candidate in &value.concept_proposals {
                        let now = Utc::now();
                        sqlx::query("INSERT INTO concept_proposals(dream_run_id,series_slug,source_language,target_language,concept_text,source_form,canonical_rendering,approved_variants_json,forbidden_variants_json,status,created_at,updated_at) VALUES (?,?,?,?,?,?,?,'[]','[]','pending',?,?)")
                            .bind(run_id)
                            .bind(&context.series_slug)
                            .bind(&context.source_language)
                            .bind(&context.target_language)
                            .bind(candidate.concept_text.trim())
                            .bind(candidate.source_form.trim())
                            .bind(candidate.canonical_rendering.trim())
                            .bind(now)
                            .bind(now)
                            .execute(&mut *transaction)
                            .await?;
                        proposals += 1;
                    }
                }
                PhaseOutput::RuleCrystals(value) | PhaseOutput::KnowledgeCrystals(value) => {
                    for candidate in &value.crystals {
                        if !cycle_budget.can_reserve_with_new(&[], 1)
                            || !long_term_budget.can_reserve_with_new(&[], 1)
                        {
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
                        let id = add_crystal_in_transaction(
                            transaction,
                            AddCrystalInput {
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
                            },
                        )
                        .await
                        .map_err(|error| DreamServiceError::Domain(error.to_string()))?;
                        sqlx::query("UPDATE crystals SET created_cycle=? WHERE id=?")
                            .bind(cycle_id)
                            .bind(id)
                            .execute(&mut *transaction)
                            .await?;
                        for source_id in &candidate.source_memory_ids {
                            sqlx::query("INSERT OR IGNORE INTO crystal_sources(crystal_id,short_term_memory_id) VALUES (?,?)")
                                .bind(id)
                                .bind(source_id)
                                .execute(&mut *transaction)
                                .await?;
                        }
                        assert!(
                            cycle_budget.reserve(&[id]),
                            "new crystal budget was prechecked"
                        );
                        assert!(
                            long_term_budget.reserve(&[id]),
                            "new crystal run budget was prechecked"
                        );
                        record_affected(transaction, maintenance_cycle_id, &[id]).await?;
                        created += 1;
                    }
                }
                PhaseOutput::Relations(value) => {
                    for relation in value
                        .relations
                        .iter()
                        .take(self.dream_config.max_relation_records_per_pass)
                    {
                        let ids = [relation.source_id, relation.target_id];
                        if !cycle_budget.can_reserve(&ids) || !long_term_budget.can_reserve(&ids) {
                            break;
                        }
                        let result = sqlx::query("INSERT OR IGNORE INTO crystal_links(source_crystal_id,target_crystal_id,link_type) VALUES (?,?,?)")
                            .bind(relation.source_id)
                            .bind(relation.target_id)
                            .bind(relation.relation.trim())
                            .execute(&mut *transaction)
                            .await?;
                        if result.rows_affected() == 1 {
                            record_affected(transaction, maintenance_cycle_id, &ids).await?;
                            assert!(cycle_budget.reserve(&ids), "relation budget was prechecked");
                            assert!(
                                long_term_budget.reserve(&ids),
                                "relation run budget was prechecked"
                            );
                        }
                    }
                }
                PhaseOutput::Reinforcement(value) => {
                    let mut candidates = value.reinforce.clone();
                    candidates.sort_by_key(|candidate| candidate.crystal_id);
                    candidates.dedup_by_key(|candidate| candidate.crystal_id);
                    for candidate in candidates
                        .into_iter()
                        .take(self.dream_config.max_total_affected_crystals)
                    {
                        let ids = [candidate.crystal_id];
                        if !cycle_budget.can_reserve(&ids) || !long_term_budget.can_reserve(&ids) {
                            break;
                        }
                        Self::apply_provider_reinforcement(
                            transaction,
                            candidate.crystal_id,
                            cycle_id,
                            ScoreDelta {
                                strength: candidate.strength_delta,
                                confidence: candidate.confidence_delta,
                            },
                        )
                        .await?;
                        assert!(
                            cycle_budget.reserve(&ids),
                            "reinforcement budget was prechecked"
                        );
                        assert!(
                            long_term_budget.reserve(&ids),
                            "reinforcement run budget was prechecked"
                        );
                        record_affected(transaction, maintenance_cycle_id, &ids).await?;
                    }
                }
                PhaseOutput::CoverageAudit(_) => {}
            }
        }
        Ok((created, proposals))
    }

    async fn apply_provider_reinforcement(
        transaction: &mut SqliteConnection,
        crystal_id: i64,
        cycle_id: i64,
        delta: ScoreDelta,
    ) -> Result<()> {
        let crystal: crate::db::CrystalRecord = sqlx::query_as("SELECT * FROM crystals WHERE id=?")
            .bind(crystal_id)
            .fetch_one(&mut *transaction)
            .await?;
        let (strength, confidence, status) = apply_score_delta(&crystal, delta);
        let last_reinforced_cycle = if delta.strength > 0.0 {
            Some(cycle_id)
        } else {
            crystal.last_reinforced_cycle
        };
        sqlx::query("UPDATE crystals SET strength=?,confidence=?,status=?,last_reinforced_cycle=?,updated_at=? WHERE id=?")
            .bind(strength)
            .bind(confidence)
            .bind(status)
            .bind(last_reinforced_cycle)
            .bind(Utc::now())
            .bind(crystal_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO memory_events(crystal_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,cycle_id,created_at) VALUES (?,'provider_reinforcement','dream','provider reinforcement',?,?,1,?,?)")
            .bind(crystal_id)
            .bind(delta.strength)
            .bind(delta.confidence)
            .bind(cycle_id)
            .bind(Utc::now())
            .execute(&mut *transaction)
            .await?;
        Ok(())
    }

    async fn run_algorithmic_phases(
        &self,
        run_id: i64,
        cycle_id: i64,
        memories: &[crate::domain::ShortTermMemory],
        session_ids: &[i64],
        cycle_budget: &mut AffectedCrystalIds,
        long_term_budget: &mut LongTermBudget,
    ) -> Result<()> {
        let limit = self
            .dream_config
            .max_changed_crystals_per_cycle
            .min(self.dream_config.max_total_affected_crystals);
        let mut working = Vec::new();
        for memory in memories
            .iter()
            .filter(|memory| memory.source_crystal_id.is_some())
            .take(limit)
        {
            let Some(source_crystal_id) = memory.source_crystal_id else {
                continue;
            };
            let source = CrystalStore::new(&self.pool)
                .get(source_crystal_id)
                .await
                .map_err(|_| {
                    DreamServiceError::InvalidInput("working-copy source is unavailable")
                })?;
            working.push((memory.clone(), source));
        }
        let reconsolidated =
            Reconsolidator::new(self.dream_config.reconsolidation_diff_threshold, cycle_id)
                .with_affected_crystals(cycle_budget.limit(), cycle_budget.ids())
                .with_affected_crystals(long_term_budget.limit(), long_term_budget.ids())
                .run(&self.pool, working)
                .await?;
        reserve_reconsolidation_outcomes(cycle_budget, long_term_budget, &reconsolidated);

        Consolidator::new(run_id, self.dream_config.max_related_concepts_per_cycle)
            .scan(&self.pool)
            .await?;

        let passive_limit = limit;
        if passive_limit > 0 {
            let passive_events: Vec<MemoryEventRecord> = sqlx::query_as(
                "SELECT * FROM memory_events
                 WHERE applied=0
                   AND event_type IN (
                     'cited','used_in_translation','passed_review',
                     'caused_correction','superseded','recalled_again'
                   )
                 ORDER BY id
                 LIMIT ?",
            )
            .bind(passive_limit as i64)
            .fetch_all(&self.pool)
            .await?;
            let reinforced = ReinforcementManager::new(cycle_id)
                .with_affected_crystals(cycle_budget.limit(), cycle_budget.ids())
                .with_affected_crystals(long_term_budget.limit(), long_term_budget.ids())
                .run(&self.pool, passive_events)
                .await?;
            for crystal_id in reinforced {
                assert!(cycle_budget.reserve(&[crystal_id]));
                assert!(long_term_budget.reserve(&[crystal_id]));
            }
        }

        let activations = self
            .bounded_activations(session_ids, cycle_id, limit)
            .await?;
        let outcomes = LinkReinforcer::new(cycle_id)
            .with_affected_crystals(cycle_budget.limit(), cycle_budget.ids())
            .with_affected_crystals(long_term_budget.limit(), long_term_budget.ids())
            .run(&self.pool, activations.clone())
            .await?;
        reserve_link_outcomes(cycle_budget, long_term_budget, &outcomes);
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
        if limit > 0 {
            let decayed = DecayManager
                .run_bounded(
                    &self.pool,
                    DecayScope {
                        after_id: 0,
                        current_cycle: cycle_id,
                        stale_before_cycle: cycle_id,
                        recalled_ids,
                        linked_ids,
                        limit,
                    },
                    vec![cycle_budget.clone(), long_term_budget.clone()],
                )
                .await?;
            for crystal_id in decayed {
                assert!(cycle_budget.reserve(&[crystal_id]));
                assert!(long_term_budget.reserve(&[crystal_id]));
            }
        }
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
        let mut select =
            QueryBuilder::<Sqlite>::new("SELECT * FROM crystal_activations WHERE session_id IN (");
        {
            let mut separated = select.separated(",");
            for id in &ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(") AND outcome='useful' AND (cycle_id IS NULL OR cycle_id=");
        }
        select.push_bind(cycle_id).push(")");
        select
            .push(" ORDER BY crystal_id,id LIMIT ")
            .push_bind(limit as i64);
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let candidates = select
            .build_query_as::<CrystalActivationRecord>()
            .fetch_all(&mut *transaction)
            .await?;
        let mut concept_counts = std::collections::BTreeMap::<i64, usize>::new();
        let mut activations = Vec::new();
        for activation in candidates {
            let concept_ids: Vec<i64> = sqlx::query_scalar(
                "SELECT concept_id FROM crystal_concepts WHERE crystal_id=? ORDER BY concept_id",
            )
            .bind(activation.crystal_id)
            .fetch_all(&mut *transaction)
            .await?;
            if concept_ids.iter().any(|concept_id| {
                concept_counts.get(concept_id).copied().unwrap_or(0)
                    >= self.dream_config.max_related_crystals_per_concept
            }) {
                continue;
            }
            for concept_id in concept_ids {
                *concept_counts.entry(concept_id).or_default() += 1;
            }
            activations.push(activation);
            if activations.len() == limit {
                break;
            }
        }
        if !activations.is_empty() {
            let mut update =
                QueryBuilder::<Sqlite>::new("UPDATE crystal_activations SET cycle_id=");
            update.push_bind(cycle_id).push(" WHERE id IN (");
            let mut separated = update.separated(",");
            for activation in &activations {
                separated.push_bind(activation.id);
            }
            separated.push_unseparated(") AND cycle_id IS NULL");
            update.build().execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(activations
            .into_iter()
            .map(|mut activation| {
                activation.cycle_id = Some(cycle_id);
                activation
            })
            .collect())
    }

    async fn pending_or_resumable_batches(
        &self,
        limit: usize,
    ) -> Result<(Vec<PendingBatch>, Option<AlgorithmBatchStage>)> {
        if let Some(stage) = self.resumable_algorithm_batch().await? {
            let workspace = WorkspaceStore::new(&self.pool);
            let mut grouped = Vec::<PendingBatch>::new();
            for memory_id in &stage.memory_ids {
                let memory = workspace.get_memory(*memory_id).await.map_err(|_| {
                    DreamServiceError::InvalidInput("resumable dream memory is unavailable")
                })?;
                if let Some(batch) = grouped
                    .iter_mut()
                    .find(|batch| batch.session_id == memory.session_id)
                {
                    batch.memories.push(memory);
                    continue;
                }
                let session = workspace
                    .get_session(memory.session_id)
                    .await
                    .map_err(|_| {
                        DreamServiceError::InvalidInput("resumable dream session is unavailable")
                    })?;
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
                grouped.push(PendingBatch {
                    session_id: memory.session_id,
                    context,
                    memories: vec![memory],
                });
            }
            return Ok((grouped, Some(stage)));
        }
        Ok((self.pending_batches(limit).await?, None))
    }

    async fn resumable_algorithm_batch(&self) -> Result<Option<AlgorithmBatchStage>> {
        let entries: Vec<(String, String)> = sqlx::query_as(
            "SELECT event_type,payload_json
             FROM dream_audit_entries
             WHERE event_type IN (
                'algorithm_batch_started',
                'algorithm_batch_completed',
                'algorithm_batch_committed'
             )
             ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut stages = Vec::<AlgorithmBatchStage>::new();
        for (event_type, payload_json) in entries {
            let payload: Value = serde_json::from_str(&payload_json)
                .map_err(|_| DreamServiceError::InvalidInput("algorithm batch audit is invalid"))?;
            let batch_id = payload.get("batch").and_then(Value::as_str).ok_or(
                DreamServiceError::InvalidInput("algorithm batch identifier is unavailable"),
            )?;
            match event_type.as_str() {
                "algorithm_batch_started" => {
                    let maintenance_cycle_id = payload
                        .get("maintenance_cycle_id")
                        .and_then(Value::as_i64)
                        .ok_or(DreamServiceError::InvalidInput(
                            "algorithm batch cycle is unavailable",
                        ))?;
                    let memory_ids = payload
                        .get("memory_ids")
                        .and_then(Value::as_array)
                        .ok_or(DreamServiceError::InvalidInput(
                            "algorithm batch inputs are unavailable",
                        ))?
                        .iter()
                        .map(|id| {
                            id.as_i64().ok_or(DreamServiceError::InvalidInput(
                                "algorithm batch input is invalid",
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    stages.push(AlgorithmBatchStage {
                        batch_id: batch_id.into(),
                        maintenance_cycle_id,
                        memory_ids,
                        algorithms_completed: false,
                        committed: false,
                    });
                }
                "algorithm_batch_completed" => {
                    if let Some(stage) = stages
                        .iter_mut()
                        .rev()
                        .find(|stage| stage.batch_id == batch_id)
                    {
                        stage.algorithms_completed = true;
                    }
                }
                "algorithm_batch_committed" => {
                    if let Some(stage) = stages
                        .iter_mut()
                        .rev()
                        .find(|stage| stage.batch_id == batch_id)
                    {
                        stage.committed = true;
                    }
                }
                _ => {}
            }
        }
        Ok(stages.into_iter().find(|stage| !stage.committed))
    }

    async fn start_algorithm_batch(
        &self,
        run_id: i64,
        maintenance_cycle_id: i64,
        memories: &[crate::domain::ShortTermMemory],
    ) -> Result<AlgorithmBatchStage> {
        let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let batch_id = memory_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        DreamAuditStore::new(&self.pool)
            .record(
                run_id,
                None,
                "algorithm_batch_started",
                "dream algorithm batch started",
                &serde_json::json!({
                    "batch": batch_id,
                    "maintenance_cycle_id": maintenance_cycle_id,
                    "memory_ids": memory_ids,
                }),
            )
            .await?;
        Ok(AlgorithmBatchStage {
            batch_id,
            maintenance_cycle_id,
            memory_ids,
            algorithms_completed: false,
            committed: false,
        })
    }

    async fn complete_algorithm_batch(&self, run_id: i64, batch_id: &str) -> Result<()> {
        DreamAuditStore::new(&self.pool)
            .record(
                run_id,
                None,
                "algorithm_batch_completed",
                "dream algorithm batch completed",
                &serde_json::json!({"batch": batch_id}),
            )
            .await?;
        Ok(())
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
        .fetch_all(&self.pool)
        .await?;
        let workspace = WorkspaceStore::new(&self.pool);
        let mut selected = Vec::new();
        let mut remaining = limit;
        let mut selected_context: Option<TranslationContext> = None;
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
            if selected_context
                .as_ref()
                .is_some_and(|selected| selected != &context)
            {
                continue;
            }
            selected_context.get_or_insert_with(|| context.clone());
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
        .fetch_one(&self.pool)
        .await?)
    }

    async fn next_cycle_id(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar::<_, Option<i64>>("SELECT max(cycle_id) FROM dream_runs")
                .fetch_one(&self.pool)
                .await?
                .unwrap_or(0)
                + 1,
        )
    }

    async fn read_run(&self, id: i64) -> Result<DreamRunRecord> {
        Ok(sqlx::query_as("SELECT * FROM dream_runs WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
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

    fn workflows(&self) -> Result<Vec<WorkflowProfile>> {
        let catalog = match &self.provider_catalog {
            Some(catalog) => catalog.clone(),
            None => ProviderCatalog::load(self.config.provider_config_path())
                .map_err(|error| DreamServiceError::Domain(error.to_string()))?,
        };
        resolve_workflows(&self.dream_config, &catalog)
            .map_err(|error| DreamServiceError::Domain(error.to_string()))
    }

    fn provider_label(&self) -> String {
        self.workflows()
            .ok()
            .and_then(|workflows| workflows.first().cloned())
            .map_or_else(|| "none".into(), |workflow| workflow.provider)
    }
}

struct PendingBatch {
    session_id: i64,
    context: TranslationContext,
    memories: Vec<crate::domain::ShortTermMemory>,
}

struct AlgorithmBatchStage {
    batch_id: String,
    maintenance_cycle_id: i64,
    memory_ids: Vec<i64>,
    algorithms_completed: bool,
    committed: bool,
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

fn canonical_directed_pairs(pairs: &[(i64, i64)]) -> Vec<(i64, i64)> {
    pairs
        .iter()
        .copied()
        .filter(|(old_id, new_id)| *old_id > 0 && *new_id > 0 && old_id != new_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

type ChangeBudget = AffectedCrystalIds;
type LongTermBudget = AffectedCrystalIds;

fn reserve_reconsolidation_outcomes(
    cycle_budget: &mut AffectedCrystalIds,
    long_term_budget: &mut AffectedCrystalIds,
    outcomes: &[super::ReconsolidationOutcome],
) {
    for outcome in outcomes {
        let ids = match *outcome {
            super::ReconsolidationOutcome::ReinforcedInPlace { crystal_id } => vec![crystal_id],
            super::ReconsolidationOutcome::Superseded {
                old_crystal_id,
                new_crystal_id,
            } => vec![old_crystal_id, new_crystal_id],
        };
        assert!(cycle_budget.reserve(&ids), "phase prechecked cycle budget");
        assert!(
            long_term_budget.reserve(&ids),
            "phase prechecked run budget"
        );
    }
}

fn reserve_link_outcomes(
    cycle_budget: &mut AffectedCrystalIds,
    long_term_budget: &mut AffectedCrystalIds,
    outcomes: &[LinkOutcome],
) {
    for outcome in outcomes {
        let ids = match *outcome {
            LinkOutcome::Strengthened {
                source_id,
                target_id,
            } => [source_id, target_id],
            LinkOutcome::Combined {
                survivor_id,
                absorbed_id,
            } => [survivor_id, absorbed_id],
        };
        assert!(cycle_budget.reserve(&ids), "phase prechecked cycle budget");
        assert!(
            long_term_budget.reserve(&ids),
            "phase prechecked run budget"
        );
    }
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

#[derive(Debug)]
enum SupervisedRun {
    Cycle(CycleOptions),
    All(CycleOptions),
    Due,
}

#[derive(Debug)]
enum SupervisedResult {
    Run(DreamRunRecord),
    Due(Option<DreamRunRecord>),
}

enum SupervisorOutcome {
    Complete(Result<SupervisedResult>),
    Cancelled,
}

fn supervise_blocking(
    service: DreamService<'static>,
    mut guard: DreamCycleGuard,
    request: SupervisedRun,
    mut sender: tokio::sync::oneshot::Sender<Result<SupervisedResult>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = sender.send(Err(DreamServiceError::Domain(
                "dream cycle supervisor runtime could not start".into(),
            )));
            return;
        }
    };
    let run_id = Arc::new(AtomicI64::new(0));
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        runtime.block_on(async {
            let operation = service.execute_supervised(request, &run_id);
            tokio::pin!(operation);
            tokio::select! {
                biased;
                _ = sender.closed() => SupervisorOutcome::Cancelled,
                result = &mut operation => SupervisorOutcome::Complete(result),
            }
        })
    }));

    let open_run_id = run_id.load(Ordering::Acquire);
    let mut recovery_failure = None;
    if open_run_id != 0 {
        let cleanup_deadline = service
            .cleanup_deadline
            .and_then(|duration| Instant::now().checked_add(duration));
        let reason = match &outcome {
            Ok(SupervisorOutcome::Cancelled) => "dream cycle cancelled",
            Ok(SupervisorOutcome::Complete(_)) => "dream cycle cleanup failed",
            Err(_) => "dream cycle supervisor panicked",
        };
        let audit = DreamAuditStore::new(&service.pool);
        let mut cleanup =
            runtime.block_on(audit.fail_run(open_run_id, DreamRunCompletion::new(0, 0, 0), reason));
        if let Err(cleanup_error) = &cleanup
            && let Err(publication_error) = guard.mark_audit_recovery()
        {
            recovery_failure = Some(format!(
                "audit cleanup failed: {cleanup_error}; audit recovery state publication failed: {publication_error}"
            ));
        }
        while cleanup.is_err() {
            if cleanup_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                let cleanup_error = cleanup
                    .as_ref()
                    .expect_err("cleanup failure was checked")
                    .to_string();
                let deadline_failure =
                    format!("audit cleanup did not complete before its deadline: {cleanup_error}");
                recovery_failure = Some(
                    recovery_failure.map_or(deadline_failure.clone(), |failure| {
                        format!("{failure}; {deadline_failure}")
                    }),
                );
                service.cleanup_failed.store(true, Ordering::Release);
                break;
            }
            runtime.block_on(tokio::time::sleep(std::time::Duration::from_millis(50)));
            cleanup = runtime.block_on(audit.fail_run(
                open_run_id,
                DreamRunCompletion::new(0, 0, 0),
                reason,
            ));
        }
        if cleanup.is_ok() {
            run_id.store(0, Ordering::Release);
        }
    }

    drop(guard);
    match outcome {
        Ok(SupervisorOutcome::Complete(result)) => {
            let result =
                recovery_failure.map_or(result, |failure| Err(DreamServiceError::Domain(failure)));
            let _ = sender.send(result);
        }
        Ok(SupervisorOutcome::Cancelled) => {}
        Err(_) => {
            let _ = sender.send(Err(DreamServiceError::Domain(
                "dream cycle supervisor panicked".into(),
            )));
        }
    }
}
