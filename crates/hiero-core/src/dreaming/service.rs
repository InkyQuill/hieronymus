use std::{
    collections::BTreeSet,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use chrono::Utc;
use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

use crate::{
    config::HieronymusConfig,
    db::{CrystalActivationRecord, DreamRunRecord, MemoryEventRecord},
    domain::{
        AddCrystalInput, CrystalStore, ScoreDelta, TranslationContext, WorkspaceStore,
        add_crystal_in_transaction, apply_score_delta,
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
    pool: SqlitePool,
    config: HieronymusConfig,
    dream_config: DreamConfig,
    resolver: Arc<dyn DreamProviderResolver>,
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
            lifetime: PhantomData,
        }
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
        let config = self.config.clone();
        let owner = owner.to_owned();
        tokio::task::spawn_blocking(move || acquire_dream_cycle_lock(&config, &owner, wait))
            .await
            .map_err(|_| DreamServiceError::Domain("dream lock worker failed".into()))?
            .map_err(Into::into)
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
            run_id,
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
            lifetime: PhantomData,
        }
    }

    async fn execute_supervised(
        &self,
        request: SupervisedRun,
        run_id: &Arc<AtomicI64>,
    ) -> Result<SupervisedResult> {
        match request {
            SupervisedRun::Cycle(opts) => self
                .run_unlocked(
                    self.dream_config.max_short_term_memories_per_cycle,
                    opts.ignore_minimum,
                    &opts.trigger_type,
                    Some(run_id),
                )
                .await
                .map(SupervisedResult::Run),
            SupervisedRun::All(opts) => {
                let mut remaining = self.dream_config.max_short_term_memories_per_run;
                let mut last = self
                    .run_unlocked(
                        self.dream_config
                            .max_short_term_memories_per_cycle
                            .min(remaining),
                        opts.ignore_minimum,
                        &opts.trigger_type,
                        Some(run_id),
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
    ) -> Result<DreamRunRecord> {
        let pending = self.pending_count().await?;
        let cycle_id = self.next_cycle_id().await?;
        let provider = self.provider_label();
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
        if !ignore_minimum && pending < self.dream_config.min_pending_short_term_memories as i64 {
            DreamAuditStore::new(&self.pool)
                .complete_run(run.id, DreamRunCompletion::new(0, 0, 0))
                .await?;
            clear_run_id(shared_run_id);
            return self.read_run(run.id).await;
        }

        let outcome = self.execute_run(run.id, cycle_id, limit).await;
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
                .execute_phase(&workflow, context.clone(), memories.clone())
                .await?;
            let count = phase_output_count(&output);
            DreamAuditStore::new(&self.pool)
                .complete_phase(phase.id, count as i64)
                .await?;
            outputs.push(output);
        }
        require_complete_coverage(&outputs, &memories)?;
        self.run_algorithmic_phases(cycle_id, &memories, &session_ids)
            .await?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let (created, proposals) = self
            .apply_outputs(
                &mut transaction,
                run_id,
                cycle_id,
                &context,
                &memories,
                &outputs,
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
        execute_provider_passes(
            &self.pool,
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
        transaction: &mut SqliteConnection,
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
                        sqlx::query("INSERT OR IGNORE INTO crystal_links(source_crystal_id,target_crystal_id,link_type) VALUES (?,?,?)")
                            .bind(relation.source_id)
                            .bind(relation.target_id)
                            .bind(relation.relation.trim())
                            .execute(&mut *transaction)
                            .await?;
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
                .run(&self.pool, working)
                .await?;
        let mut changed = reconsolidated.len();

        let activations = self
            .bounded_activations(session_ids, cycle_id, limit.saturating_sub(changed))
            .await?;
        let outcomes = LinkReinforcer::new(cycle_id)
            .run(&self.pool, activations.clone())
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
        let remaining = limit.saturating_sub(changed);
        if remaining > 0 {
            DecayManager
                .run(
                    &self.pool,
                    DecayScope {
                        after_id: 0,
                        current_cycle: cycle_id,
                        stale_before_cycle: cycle_id,
                        recalled_ids,
                        linked_ids,
                        limit: remaining,
                    },
                )
                .await?;
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
        let mut separated = select.separated(",");
        for id in &ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(") AND outcome='useful' AND cycle_id IS NULL");
        select
            .push(" ORDER BY crystal_id,id LIMIT ")
            .push_bind(limit as i64);
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let activations = select
            .build_query_as::<CrystalActivationRecord>()
            .fetch_all(&mut *transaction)
            .await?;
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

fn canonical_directed_pairs(pairs: &[(i64, i64)]) -> Vec<(i64, i64)> {
    pairs
        .iter()
        .copied()
        .filter(|(old_id, new_id)| *old_id > 0 && *new_id > 0 && old_id != new_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

struct ChangeBudget {
    limit: usize,
    ids: BTreeSet<i64>,
}

impl ChangeBudget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            ids: BTreeSet::new(),
        }
    }

    fn reserve(&mut self, ids: &[i64]) -> bool {
        let additional = ids.iter().filter(|id| !self.ids.contains(id)).count();
        if self.ids.len().saturating_add(additional) > self.limit {
            return false;
        }
        self.ids.extend(ids.iter().copied());
        true
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
    guard: DreamCycleGuard,
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
    if open_run_id != 0 {
        let cleanup = runtime.block_on(DreamAuditStore::new(&service.pool).fail_run(
            open_run_id,
            DreamRunCompletion::new(0, 0, 0),
            match &outcome {
                Ok(SupervisorOutcome::Cancelled) => "dream cycle cancelled",
                Ok(SupervisorOutcome::Complete(_)) => "dream cycle cleanup failed",
                Err(_) => "dream cycle supervisor panicked",
            },
        ));
        if let Err(error) = cleanup {
            // Fail closed: releasing the OS guard with a running audit row would
            // allow another process to overlap an indeterminate cycle.
            std::mem::forget(guard);
            let _ = sender.send(Err(error.into()));
            return;
        }
        run_id.store(0, Ordering::Release);
    }

    drop(guard);
    match outcome {
        Ok(SupervisorOutcome::Complete(result)) => {
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
