use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use async_trait::async_trait;
use futures::future::join_all;
use hiero_core::{
    config::HieronymusConfig,
    db::{connect_url, migrate},
    domain::{AddCrystalInput, AddMemoryInput, CrystalStore, TranslationContext, WorkspaceStore},
    dreaming::{
        CycleOptions, DreamConfig, DreamPhaseError, DreamProviderResolver, DreamService,
        MaintenancePayload, PhaseProfile, WorkflowProfile, acquire_dream_cycle_lock,
        run_background_loop,
    },
    provider::{
        DreamOutput, DreamProvider, PassName, ProviderCatalog, ProviderDefaults, ProviderError,
        ProviderPassOutput, ProviderProfile,
    },
};
use serde_json::json;
use sqlx::{Row, SqlitePool};
use tempfile::TempDir;
use tokio::sync::{Notify, broadcast};

struct FakeProvider {
    failure: Option<PassName>,
    entered: Option<Arc<Notify>>,
    release: Option<Arc<Notify>>,
}

type RecordedCalls = Arc<Mutex<Vec<(TranslationContext, Vec<i64>)>>>;

struct RecordingProvider {
    calls: RecordedCalls,
}

#[async_trait]
impl DreamProvider for RecordingProvider {
    fn name(&self) -> &str {
        "recording"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<DreamOutput, ProviderError> {
        unreachable!("the service executes configured evidence passes")
    }

    async fn run_pass(
        &self,
        pass: PassName,
        context: &TranslationContext,
        memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<ProviderPassOutput, ProviderError> {
        let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        self.calls
            .lock()
            .unwrap()
            .push((context.clone(), ids.clone()));
        let value = match pass {
            PassName::CoverageAudit => json!({"covered_memory_ids": ids}),
            _ => json!({}),
        };
        Ok(ProviderPassOutput::direct(value))
    }
}

struct ReinforcementProvider {
    crystal_id: i64,
    strength_delta: f64,
    confidence_delta: f64,
}

struct ProcessBlockingProvider {
    entered: PathBuf,
    release: PathBuf,
}

struct FullOutputProvider {
    relation_source: i64,
    relation_target: i64,
}

struct BulkOutputProvider {
    count: usize,
}

struct ShutdownBlockingProvider {
    entered: Arc<Notify>,
    release: Arc<Notify>,
    current: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

struct ActiveCall(Arc<AtomicUsize>);

impl Drop for ActiveCall {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[async_trait]
impl DreamProvider for ShutdownBlockingProvider {
    fn name(&self) -> &str {
        "shutdown-blocking"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<DreamOutput, ProviderError> {
        unreachable!("the service executes configured evidence passes")
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<ProviderPassOutput, ProviderError> {
        let active = self.current.fetch_add(1, Ordering::AcqRel) + 1;
        self.maximum.fetch_max(active, Ordering::AcqRel);
        let _active = ActiveCall(self.current.clone());
        self.entered.notify_one();
        self.release.notified().await;
        let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        Ok(ProviderPassOutput::direct(match pass {
            PassName::CoverageAudit => json!({"covered_memory_ids": ids}),
            _ => json!({}),
        }))
    }
}

#[async_trait]
impl DreamProvider for BulkOutputProvider {
    fn name(&self) -> &str {
        "bulk-output"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<DreamOutput, ProviderError> {
        unreachable!("the service executes configured evidence passes")
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<ProviderPassOutput, ProviderError> {
        let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let value = match pass {
            PassName::KnowledgeCrystals => json!({
                "crystals": (0..self.count)
                    .map(|index| json!({
                        "crystal_type": "observation",
                        "title": format!("Bulk finding {index}"),
                        "text": format!("Bulk translated finding {index}."),
                        "source_credibility": "observation",
                        "rule_intent": "",
                        "confidence": 0.8,
                        "source_memory_ids": ids,
                    }))
                    .collect::<Vec<_>>()
            }),
            PassName::CoverageAudit => json!({"covered_memory_ids": ids}),
            _ => json!({}),
        };
        Ok(ProviderPassOutput::direct(value))
    }
}

#[async_trait]
impl DreamProvider for FullOutputProvider {
    fn name(&self) -> &str {
        "full-output"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<DreamOutput, ProviderError> {
        unreachable!("the service executes configured evidence passes")
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<ProviderPassOutput, ProviderError> {
        let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let value = match pass {
            PassName::Concepts => json!({
                "concepts": [{
                    "canonical_name": "Sense",
                    "facets": [["rendering", "Сенс", "ru"]],
                    "source_memory_ids": ids,
                }]
            }),
            PassName::TerminologyCandidates => json!({
                "concept_proposals": [{
                    "concept_text": "Sense",
                    "source_form": "Sense",
                    "canonical_rendering": "Сенс",
                    "source_memory_ids": ids,
                }]
            }),
            PassName::KnowledgeCrystals => json!({
                "crystals": [{
                    "crystal_type": "observation",
                    "title": "Stable finding",
                    "text": "A stable translated finding.",
                    "source_credibility": "observation",
                    "rule_intent": "",
                    "confidence": 0.8,
                    "source_memory_ids": ids,
                }]
            }),
            PassName::Relations => json!({
                "relations": [{
                    "source_id": self.relation_source,
                    "target_id": self.relation_target,
                    "relation": "related",
                    "source_memory_ids": ids,
                }]
            }),
            PassName::Reinforcement => json!({
                "reinforce": [{
                    "crystal_id": self.relation_source,
                    "strength_delta": 0.07,
                    "confidence_delta": 0.03,
                    "source_memory_ids": ids,
                }]
            }),
            PassName::CoverageAudit => json!({"covered_memory_ids": ids}),
            _ => json!({}),
        };
        Ok(ProviderPassOutput::direct(value))
    }
}

#[async_trait]
impl DreamProvider for ProcessBlockingProvider {
    fn name(&self) -> &str {
        "process-blocking"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<DreamOutput, ProviderError> {
        unreachable!("the service executes configured evidence passes")
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<ProviderPassOutput, ProviderError> {
        let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        if pass == PassName::KnowledgeCrystals {
            fs::write(&self.entered, b"entered").unwrap();
            while !self.release.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        let value = match pass {
            PassName::KnowledgeCrystals => json!({"crystals": []}),
            PassName::CoverageAudit => json!({"covered_memory_ids": memory_ids}),
            _ => json!({}),
        };
        Ok(ProviderPassOutput::direct(value))
    }
}

#[async_trait]
impl DreamProvider for ReinforcementProvider {
    fn name(&self) -> &str {
        "reinforcement"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<DreamOutput, ProviderError> {
        unreachable!("the service executes configured evidence passes")
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<ProviderPassOutput, ProviderError> {
        let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let value = match pass {
            PassName::Reinforcement => json!({
                "reinforce": [{
                    "crystal_id": self.crystal_id,
                    "strength_delta": self.strength_delta,
                    "confidence_delta": self.confidence_delta,
                    "source_memory_ids": memory_ids,
                }]
            }),
            PassName::CoverageAudit => json!({"covered_memory_ids": memory_ids}),
            _ => json!({}),
        };
        Ok(ProviderPassOutput::direct(value))
    }
}

#[async_trait]
impl DreamProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake"
    }

    async fn crystallize(
        &self,
        _context: &TranslationContext,
        _memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<DreamOutput, ProviderError> {
        unreachable!("the service executes configured evidence passes")
    }

    async fn run_pass(
        &self,
        pass: PassName,
        _context: &TranslationContext,
        memories: &[hiero_core::domain::ShortTermMemory],
    ) -> Result<ProviderPassOutput, ProviderError> {
        if let Some(entered) = &self.entered {
            entered.notify_one();
            self.release
                .as_ref()
                .expect("blocking fake has a release notification")
                .notified()
                .await;
        }
        if self.failure == Some(pass) {
            return Err(ProviderError::MalformedJson);
        }
        let memory_ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let value = match pass {
            PassName::KnowledgeCrystals => json!({
                "crystals": [{
                    "crystal_type": "observation",
                    "title": "Stable finding",
                    "text": "A stable translated finding.",
                    "source_credibility": "observation",
                    "rule_intent": "",
                    "confidence": 0.8,
                    "source_memory_ids": memory_ids,
                }]
            }),
            PassName::CoverageAudit => json!({"covered_memory_ids": memory_ids}),
            _ => json!({}),
        };
        Ok(ProviderPassOutput::direct(value))
    }
}

struct Fixture {
    _temp: TempDir,
    pool: SqlitePool,
    config: HieronymusConfig,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let config = HieronymusConfig::load(Some(temp.path().to_path_buf())).unwrap();
        config.ensure_directories().unwrap();
        let pool = connect_url(&format!("sqlite://{}", config.database_path().display()))
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO series(slug,title,default_source_language,default_target_language,created_at,updated_at)
             VALUES ('book','Book','en','ru',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();
        Self {
            _temp: temp,
            pool,
            config,
        }
    }

    async fn pending_memory(&self) -> i64 {
        self.pending_memory_for(TranslationContext::new("book", "en", "ru"))
            .await
    }

    async fn pending_memory_for(&self, context: TranslationContext) -> i64 {
        let workspace = WorkspaceStore::new(&self.pool);
        let session = workspace
            .start_session(&context, "translation", "1", "1")
            .await
            .unwrap();
        workspace
            .add_short_term(
                session.id,
                AddMemoryInput {
                    text: "The term remains stable.".into(),
                    ..AddMemoryInput::default()
                },
            )
            .await
            .unwrap();
        assert!(workspace.complete_session(session.id).await.unwrap());
        session.id
    }

    async fn add_series(&self, slug: &str, source_language: &str, target_language: &str) {
        sqlx::query(
            "INSERT INTO series(slug,title,default_source_language,default_target_language,created_at,updated_at)
             VALUES (?,?,?,?,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
        )
        .bind(slug)
        .bind(slug)
        .bind(source_language)
        .bind(target_language)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    fn service(&self, provider: Arc<dyn DreamProvider>, min_pending: usize) -> DreamService<'_> {
        self.service_with_limits(provider, min_pending, 10, 10)
    }

    fn service_with_limits(
        &self,
        provider: Arc<dyn DreamProvider>,
        min_pending: usize,
        per_cycle: usize,
        per_run: usize,
    ) -> DreamService<'_> {
        self.service_with_change_limit(provider, min_pending, per_cycle, per_run, 200)
    }

    fn service_with_change_limit(
        &self,
        provider: Arc<dyn DreamProvider>,
        min_pending: usize,
        per_cycle: usize,
        per_run: usize,
        max_changed: usize,
    ) -> DreamService<'_> {
        self.service_with_phases(
            provider,
            min_pending,
            per_cycle,
            per_run,
            max_changed,
            &[PassName::KnowledgeCrystals, PassName::CoverageAudit],
        )
    }

    fn service_with_phases(
        &self,
        provider: Arc<dyn DreamProvider>,
        min_pending: usize,
        per_cycle: usize,
        per_run: usize,
        max_changed: usize,
        enabled: &[PassName],
    ) -> DreamService<'_> {
        let resolver: Arc<dyn DreamProviderResolver> = Arc::new(
            move |_: &WorkflowProfile| -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
                Ok(provider.clone())
            },
        );
        let mut config = DreamConfig {
            enabled: true,
            min_pending_short_term_memories: min_pending,
            max_pending_short_term_memories: min_pending.max(10),
            max_short_term_memories_per_cycle: per_cycle,
            max_short_term_memories_per_run: per_run,
            max_changed_crystals_per_cycle: max_changed,
            ..DreamConfig::default()
        };
        for phase in PassName::ALL {
            config = config.with_phase(phase, PhaseProfile::default());
        }
        for phase in enabled {
            config = config.with_phase(
                *phase,
                PhaseProfile {
                    provider: "fake".into(),
                    model: "fixture".into(),
                    enabled: true,
                    max_records_per_pass: max_changed.max(10),
                },
            );
        }
        let config = Box::leak(Box::new(config));
        DreamService::new_with_catalog(&self.pool, &self.config, config, resolver, fake_catalog())
    }
}

fn fake() -> Arc<dyn DreamProvider> {
    Arc::new(FakeProvider {
        failure: None,
        entered: None,
        release: None,
    })
}

fn fake_catalog() -> ProviderCatalog {
    let mut catalog = ProviderCatalog::default();
    catalog
        .upsert(ProviderProfile::new(
            "fake",
            "Fake",
            "openai",
            "https://example.test/v1",
        ))
        .unwrap();
    catalog
}

#[tokio::test]
async fn run_cycle_persists_output_and_closes_all_audit_records() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let run = fixture
        .service(fake(), 1)
        .run_cycle(CycleOptions::default())
        .await
        .unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.input_count, 1);
    assert_eq!(run.created_crystal_count, 1);
    let archived: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM short_term_memories WHERE archived_at IS NOT NULL",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(archived, 1);
    let phases = sqlx::query(
        "SELECT status, completed_at FROM dream_phase_runs WHERE dream_run_id=? ORDER BY id",
    )
    .bind(run.id)
    .fetch_all(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(phases.len(), 2);
    assert!(
        phases
            .iter()
            .all(|row| row.get::<String, _>("status") == "completed")
    );
    assert!(
        phases
            .iter()
            .all(|row| row.get::<Option<String>, _>("completed_at").is_some())
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM dream_audit_entries WHERE dream_run_id=?",
        )
        .bind(run.id)
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        10
    );
}

#[tokio::test]
async fn service_resolves_blank_workflow_provider_and_model_from_catalog_defaults() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let mut catalog = ProviderCatalog::default();
    catalog
        .upsert(ProviderProfile::new(
            "remote",
            "Remote",
            "openai",
            "https://example.test/v1",
        ))
        .unwrap();
    catalog.set_defaults(ProviderDefaults {
        provider: "remote".into(),
        model: "catalog-model".into(),
    });
    let calls = Arc::new(Mutex::new(Vec::<WorkflowProfile>::new()));
    let provider: Arc<dyn DreamProvider> = fake();
    let captured = calls.clone();
    let resolver: Arc<dyn DreamProviderResolver> = Arc::new(
        move |workflow: &WorkflowProfile| -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
            captured.lock().unwrap().push(workflow.clone());
            Ok(provider.clone())
        },
    );
    let mut dream_config = DreamConfig {
        enabled: true,
        ..DreamConfig::default()
    };
    dream_config = dream_config.with_phase(
        PassName::CoverageAudit,
        PhaseProfile {
            provider: String::new(),
            model: String::new(),
            enabled: true,
            max_records_per_pass: 10,
        },
    );

    DreamService::new_with_catalog(
        &fixture.pool,
        &fixture.config,
        &dream_config,
        resolver,
        catalog,
    )
    .run_cycle(CycleOptions::default())
    .await
    .unwrap();

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [WorkflowProfile {
            phase: PassName::CoverageAudit,
            provider: "remote".into(),
            model: "catalog-model".into(),
            max_records_per_pass: 10,
        }]
    );
}

#[tokio::test]
async fn phase_failure_fails_open_phase_and_run_then_releases_cycle_lock() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let provider = Arc::new(FakeProvider {
        failure: Some(PassName::KnowledgeCrystals),
        entered: None,
        release: None,
    });
    let error = fixture
        .service(provider, 1)
        .run_cycle(CycleOptions::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("provider"));

    let rows = sqlx::query("SELECT status, completed_at FROM dream_runs ORDER BY id")
        .fetch_all(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<String, _>("status"), "failed");
    assert!(rows[0].get::<Option<String>, _>("completed_at").is_some());
    let phase = sqlx::query("SELECT status, completed_at FROM dream_phase_runs")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(phase.get::<String, _>("status"), "failed");
    assert!(phase.get::<Option<String>, _>("completed_at").is_some());
    acquire_dream_cycle_lock(&fixture.config, "after-failure", false).unwrap();
}

#[tokio::test]
async fn cancelled_cycle_closes_audit_records_and_releases_cycle_lock() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let service = fixture.service(
        Arc::new(FakeProvider {
            failure: None,
            entered: Some(entered.clone()),
            release: Some(release),
        }),
        1,
    );
    {
        let cycle = service.run_cycle(CycleOptions::default());
        tokio::pin!(cycle);
        tokio::select! {
            () = entered.notified() => {}
            result = &mut cycle => panic!("cycle completed before cancellation: {result:?}"),
        }
    }

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let open: i64 = sqlx::query_scalar(
                "SELECT
                   (SELECT count(*) FROM dream_runs WHERE status='running') +
                   (SELECT count(*) FROM dream_phase_runs WHERE status='running')",
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
            if open == 0 && acquire_dream_cycle_lock(&fixture.config, "after-cancel", false).is_ok()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let run = sqlx::query(
        "SELECT status,error,completed_at,input_count,created_crystal_count,proposal_count FROM dream_runs",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(run.get::<String, _>("status"), "failed");
    assert_eq!(run.get::<String, _>("error"), "dream cycle cancelled");
    assert!(run.get::<Option<String>, _>("completed_at").is_some());
    assert_eq!(run.get::<i64, _>("input_count"), 0);
    assert_eq!(run.get::<i64, _>("created_crystal_count"), 0);
    assert_eq!(run.get::<i64, _>("proposal_count"), 0);
    let phase = sqlx::query("SELECT status,error,completed_at FROM dream_phase_runs")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(phase.get::<String, _>("status"), "failed");
    assert_eq!(phase.get::<String, _>("error"), "dream cycle cancelled");
    assert!(phase.get::<Option<String>, _>("completed_at").is_some());
}

#[tokio::test]
async fn concurrent_lock_can_be_reported_as_a_skipped_closed_run() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let _guard = acquire_dream_cycle_lock(&fixture.config, "other-process", false).unwrap();
    let run = fixture
        .service(fake(), 1)
        .run_cycle(CycleOptions {
            skip_when_locked: true,
            ..CycleOptions::default()
        })
        .await
        .unwrap();

    assert_eq!(run.status, "skipped");
    assert!(run.completed_at.is_some());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dream_phase_runs")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn empty_input_completes_one_run_without_calling_a_provider_phase() {
    let fixture = Fixture::new().await;
    let run = fixture
        .service(fake(), 1)
        .run_cycle(CycleOptions::default())
        .await
        .unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.input_count, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dream_phase_runs WHERE dream_run_id=?",)
            .bind(run.id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn run_all_repeats_bounded_runs_until_no_pending_memory_remains() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    fixture.pending_memory().await;
    let run = fixture
        .service_with_limits(fake(), 1, 1, 2)
        .run_all(CycleOptions::default())
        .await
        .unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dream_runs WHERE status='completed'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM short_term_memories WHERE archived_at IS NULL",
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn service_bounds_and_orders_activations_before_pair_reinforcement() {
    let fixture = Fixture::new().await;
    let session_id = fixture.pending_memory().await;
    for index in 0..3 {
        let crystal_id = CrystalStore::new(&fixture.pool)
            .add(AddCrystalInput {
                text: format!("Distinct crystal number {index}"),
                title: format!("Crystal {index}"),
                scope_type: "series".into(),
                scope_key: "series:book".into(),
                series_slug: "book".into(),
                source_language: "en".into(),
                target_language: "ru".into(),
                status: "active".into(),
                ..AddCrystalInput::default()
            })
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO crystal_activations(crystal_id,session_id,recall_query,rank,score,reason,outcome,created_at)
             VALUES (?,?,'query',1,1.0,'useful result','useful',CURRENT_TIMESTAMP)",
        )
        .bind(crystal_id)
        .bind(session_id)
        .execute(&fixture.pool)
        .await
        .unwrap();
    }

    fixture
        .service_with_change_limit(fake(), 1, 10, 10, 2)
        .run_cycle(CycleOptions::default())
        .await
        .unwrap();

    let links = sqlx::query("SELECT source_crystal_id,target_crystal_id FROM crystal_links")
        .fetch_all(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].get::<i64, _>("source_crystal_id"), 1);
    assert_eq!(links[0].get::<i64, _>("target_crystal_id"), 2);
}

#[tokio::test]
async fn maintenance_is_deduplicated_deterministic_and_bounded() {
    let fixture = Fixture::new().await;
    let mut ids = Vec::new();
    for index in 0..3 {
        ids.push(
            CrystalStore::new(&fixture.pool)
                .add(AddCrystalInput {
                    text: format!("Maintenance crystal {index}"),
                    title: format!("Maintenance {index}"),
                    status: "active".into(),
                    ..AddCrystalInput::default()
                })
                .await
                .unwrap(),
        );
    }
    let service = fixture.service_with_change_limit(fake(), 1, 10, 10, 2);
    let result = service
        .apply_maintenance(
            &MaintenancePayload {
                reinforce: vec![ids[0], ids[0]],
                decay: vec![ids[1], ids[2]],
                ..MaintenancePayload::default()
            },
            7,
        )
        .await
        .unwrap();

    assert_eq!(result.reinforced, 1);
    assert_eq!(result.decayed, 1);
    assert_eq!(result.combined, 0);
    assert_eq!(result.superseded, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_events WHERE crystal_id=? AND event_type='maintenance_decay'",
        )
        .bind(ids[1])
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM memory_events WHERE crystal_id=?",)
            .bind(ids[2])
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn background_loop_runs_due_cycles_serially_and_stops_on_shutdown() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let service = fixture.service(fake(), 1);
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let controller = async {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let runs: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM dream_runs WHERE status='completed'")
                        .fetch_one(&fixture.pool)
                        .await
                        .unwrap();
                if runs == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        shutdown_tx.send(()).unwrap();
    };
    let (loop_result, ()) = tokio::join!(
        run_background_loop(service, Duration::from_millis(10), shutdown_rx),
        controller
    );
    loop_result.unwrap();

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dream_runs WHERE status='completed'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn scheduler_shutdown_waits_for_blocked_cycle_cancellation_and_audit_cleanup() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let entered = Arc::new(Notify::new());
    let current = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let service = fixture.service(
        Arc::new(ShutdownBlockingProvider {
            entered: entered.clone(),
            release: Arc::new(Notify::new()),
            current: current.clone(),
            maximum: maximum.clone(),
        }),
        1,
    );
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let controller = async {
        entered.notified().await;
        shutdown_tx.send(()).unwrap();
    };
    let (scheduler, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            run_background_loop(service, Duration::from_millis(10), shutdown_rx),
            controller
        )
    })
    .await
    .unwrap();
    scheduler.unwrap();

    assert_eq!(current.load(Ordering::Acquire), 0);
    assert_eq!(maximum.load(Ordering::Acquire), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dream_runs WHERE status='running'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
    acquire_dream_cycle_lock(&fixture.config, "after-scheduler-shutdown", false).unwrap();
}

#[tokio::test]
async fn each_run_uses_one_exact_series_and_language_context() {
    let fixture = Fixture::new().await;
    fixture.add_series("other", "ja", "de").await;
    let first_session = fixture
        .pending_memory_for(TranslationContext::new("book", "en", "ru"))
        .await;
    let second_session = fixture
        .pending_memory_for(TranslationContext::new("other", "ja", "de"))
        .await;
    let first_memory: i64 =
        sqlx::query_scalar("SELECT id FROM short_term_memories WHERE session_id=?")
            .bind(first_session)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let second_memory: i64 =
        sqlx::query_scalar("SELECT id FROM short_term_memories WHERE session_id=?")
            .bind(second_session)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn DreamProvider> = Arc::new(RecordingProvider {
        calls: calls.clone(),
    });
    let service = fixture.service_with_phases(provider, 1, 10, 10, 10, &[PassName::CoverageAudit]);

    let first = service.run_cycle(CycleOptions::default()).await.unwrap();
    assert_eq!(first.input_count, 1);
    let second = service.run_cycle(CycleOptions::default()).await.unwrap();
    assert_eq!(second.input_count, 1);

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0.series_slug, "book");
    assert_eq!(calls[0].0.source_language, "en");
    assert_eq!(calls[0].0.target_language, "ru");
    assert_eq!(calls[0].1, vec![first_memory]);
    assert_eq!(calls[1].0.series_slug, "other");
    assert_eq!(calls[1].0.source_language, "ja");
    assert_eq!(calls[1].0.target_language, "de");
    assert_eq!(calls[1].1, vec![second_memory]);
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_cycle_lock_does_not_block_the_async_runtime_thread() {
    let fixture = Fixture::new().await;
    let guard = acquire_dream_cycle_lock(&fixture.config, "holder", false).unwrap();
    let progress = Arc::new(AtomicUsize::new(0));
    let observed = progress.clone();
    let releaser = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while observed.load(Ordering::Acquire) < 3 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let ticks_before_release = observed.load(Ordering::Acquire);
        drop(guard);
        ticks_before_release
    });
    let service = fixture.service(fake(), 1);
    let heartbeat = async {
        let mut ticks = 0;
        while ticks < 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ticks += 1;
            progress.store(ticks, Ordering::Release);
        }
        ticks
    };
    let (run, ticks) = tokio::join!(
        service.run_cycle(CycleOptions {
            wait: true,
            ..CycleOptions::default()
        }),
        heartbeat
    );
    assert_eq!(releaser.join().unwrap(), 3);
    assert_eq!(ticks, 3);
    assert_eq!(run.unwrap().status, "completed");
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_a_waiting_cycle_does_not_leave_a_detached_lock_owner() {
    let fixture = Fixture::new().await;
    let guard = acquire_dream_cycle_lock(&fixture.config, "holder", false).unwrap();
    {
        let service = fixture.service(fake(), 1);
        let waiting = service.run_cycle(CycleOptions {
            wait: true,
            ..CycleOptions::default()
        });
        tokio::pin!(waiting);
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(25)) => {}
            result = &mut waiting => panic!("waiting cycle unexpectedly completed: {result:?}"),
        }
    }
    drop(guard);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if acquire_dream_cycle_lock(&fixture.config, "after-wait-cancel", false).is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dream_runs")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
}

#[test]
fn cancelling_many_waiters_does_not_occupy_blocking_workers() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(2)
        .build()
        .unwrap();
    runtime.block_on(async {
        let fixture = Fixture::new().await;
        let holder = acquire_dream_cycle_lock(&fixture.config, "holder", false).unwrap();
        let pool = Box::leak(Box::new(fixture.pool.clone()));
        let config = Box::leak(Box::new(fixture.config.clone()));
        let dream_config = Box::leak(Box::new(DreamConfig {
            enabled: true,
            min_pending_short_term_memories: 1,
            ..DreamConfig::default()
        }));
        let service = DreamService::new_with_catalog(
            pool,
            config,
            dream_config,
            Arc::new(|_: &WorkflowProfile| {
                Err(DreamPhaseError::InvalidInput("provider must not run"))
            }),
            fake_catalog(),
        );
        let mut waiters = Vec::new();
        for _ in 0..8 {
            let service = service.clone();
            waiters.push(tokio::spawn(async move {
                service
                    .run_cycle(CycleOptions {
                        wait: true,
                        ..CycleOptions::default()
                    })
                    .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        for waiter in &waiters {
            waiter.abort();
        }
        join_all(waiters).await;

        let sentinel = tokio::task::spawn_blocking(|| 42);
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), sentinel)
                .await
                .expect("cancelled waiters must not strand the blocking pool")
                .unwrap(),
            42
        );
        drop(holder);
    });
}

#[tokio::test]
async fn failed_audit_cleanup_poison_is_visible_and_recovers_before_unlock() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    sqlx::query(
        "CREATE TRIGGER injected_cleanup_failure
         BEFORE UPDATE OF status ON dream_runs
         WHEN OLD.status='running' AND NEW.status='failed'
         BEGIN SELECT RAISE(ABORT,'injected cleanup failure'); END",
    )
    .execute(&fixture.pool)
    .await
    .unwrap();
    let entered = Arc::new(Notify::new());
    {
        let service = fixture.service(
            Arc::new(FakeProvider {
                failure: None,
                entered: Some(entered.clone()),
                release: Some(Arc::new(Notify::new())),
            }),
            1,
        );
        let cycle = service.run_cycle(CycleOptions::default());
        tokio::pin!(cycle);
        tokio::select! {
            () = entered.notified() => {}
            result = &mut cycle => panic!("cycle completed before cancellation: {result:?}"),
        }
    }

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let error =
                acquire_dream_cycle_lock(&fixture.config, "must-stay-locked", false).unwrap_err();
            let owner = error.state.as_ref().map(|state| state.owner.as_str());
            let running: i64 =
                sqlx::query_scalar("SELECT count(*) FROM dream_runs WHERE status='running'")
                    .fetch_one(&fixture.pool)
                    .await
                    .unwrap();
            if owner == Some("audit-recovery") && running == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cleanup failure must enter a visible poison state");

    sqlx::query("DROP TRIGGER injected_cleanup_failure")
        .execute(&fixture.pool)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let failed: i64 =
                sqlx::query_scalar("SELECT count(*) FROM dream_runs WHERE status='failed'")
                    .fetch_one(&fixture.pool)
                    .await
                    .unwrap();
            if failed == 1
                && acquire_dream_cycle_lock(&fixture.config, "after-recovery", false).is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("audit recovery must close the run and release the lock");
}

#[tokio::test]
async fn audit_recovery_publication_failure_is_retained_with_cleanup_failure() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    sqlx::query(
        "CREATE TRIGGER injected_cleanup_failure
         BEFORE UPDATE OF status ON dream_runs
         WHEN OLD.status='running' AND NEW.status='failed'
         BEGIN SELECT RAISE(ABORT,'injected cleanup failure'); END",
    )
    .execute(&fixture.pool)
    .await
    .unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let service = fixture.service(
        Arc::new(FakeProvider {
            failure: Some(PassName::KnowledgeCrystals),
            entered: Some(entered.clone()),
            release: Some(release.clone()),
        }),
        1,
    );
    let cycle = service.run_cycle(CycleOptions::default());
    tokio::pin!(cycle);
    tokio::select! {
        () = entered.notified() => {}
        result = &mut cycle => panic!("cycle completed before publication injection: {result:?}"),
    }
    let state_link = fixture.config.data_root.join("dream-cycle-state-link");
    fs::hard_link(fixture.config.dream_cycle_state_path(), &state_link).unwrap();
    release.notify_one();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let error =
                acquire_dream_cycle_lock(&fixture.config, "must-stay-locked", false).unwrap_err();
            if error.is_already_running() && error.state.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    fs::remove_file(state_link).unwrap();
    sqlx::query("DROP TRIGGER injected_cleanup_failure")
        .execute(&fixture.pool)
        .await
        .unwrap();

    let error = cycle.await.unwrap_err().to_string();
    assert!(error.contains("audit cleanup failed"));
    assert!(error.contains("audit recovery state publication failed"));
}

#[test]
fn runtime_shutdown_still_finishes_cancelled_audit_before_unlock() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = runtime.block_on(Fixture::new());
    runtime.block_on(fixture.pending_memory());
    let entered = Arc::new(Notify::new());
    let provider: Arc<dyn DreamProvider> = Arc::new(FakeProvider {
        failure: None,
        entered: Some(entered.clone()),
        release: Some(Arc::new(Notify::new())),
    });
    let resolver: Arc<dyn DreamProviderResolver> = Arc::new(
        move |_: &WorkflowProfile| -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
            Ok(provider.clone())
        },
    );
    let mut dream_config = DreamConfig {
        enabled: true,
        min_pending_short_term_memories: 1,
        max_pending_short_term_memories: 10,
        max_short_term_memories_per_cycle: 10,
        max_short_term_memories_per_run: 10,
        ..DreamConfig::default()
    };
    for phase in PassName::ALL {
        dream_config = dream_config.with_phase(phase, PhaseProfile::default());
    }
    for phase in [PassName::KnowledgeCrystals, PassName::CoverageAudit] {
        dream_config = dream_config.with_phase(
            phase,
            PhaseProfile {
                provider: "fake".into(),
                model: "fixture".into(),
                enabled: true,
                max_records_per_pass: 10,
            },
        );
    }
    let pool = Box::leak(Box::new(fixture.pool.clone()));
    let config = Box::leak(Box::new(fixture.config.clone()));
    let dream_config = Box::leak(Box::new(dream_config));
    let service =
        DreamService::new_with_catalog(pool, config, dream_config, resolver, fake_catalog());
    runtime.spawn(async move {
        let _ = service.run_cycle(CycleOptions::default()).await;
    });
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
    });
    drop(runtime);

    let verifier = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    verifier.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let open: i64 = sqlx::query_scalar(
                    "SELECT
                       (SELECT count(*) FROM dream_runs WHERE status='running') +
                       (SELECT count(*) FROM dream_phase_runs WHERE status='running')",
                )
                .fetch_one(&fixture.pool)
                .await
                .unwrap();
                if open == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    acquire_dream_cycle_lock(&fixture.config, "after-runtime-shutdown", false).unwrap();
}

#[tokio::test]
async fn public_run_due_acquires_the_cycle_lock_itself() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let guard = acquire_dream_cycle_lock(&fixture.config, "holder", false).unwrap();
    let error = fixture.service(fake(), 1).run_due().await.unwrap_err();
    assert!(error.to_string().contains("already running"));
    drop(guard);

    let run = fixture.service(fake(), 1).run_due().await.unwrap().unwrap();
    assert_eq!(run.status, "completed");
}

#[tokio::test]
async fn run_all_obeys_per_cycle_and_aggregate_per_run_limits() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    fixture.pending_memory().await;
    fixture.pending_memory().await;

    fixture
        .service_with_limits(fake(), 1, 1, 2)
        .run_all(CycleOptions::default())
        .await
        .unwrap();

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM dream_runs WHERE status='completed'")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM short_term_memories WHERE archived_at IS NULL",
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn run_all_shares_one_long_term_record_budget_across_cycles() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    fixture.pending_memory().await;
    let provider: Arc<dyn DreamProvider> = Arc::new(BulkOutputProvider { count: 2 });
    let resolver: Arc<dyn DreamProviderResolver> = Arc::new(
        move |_: &WorkflowProfile| -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
            Ok(provider.clone())
        },
    );
    let mut dream_config = DreamConfig {
        enabled: true,
        max_short_term_memories_per_cycle: 1,
        max_short_term_memories_per_run: 2,
        max_changed_crystals_per_cycle: 20,
        max_total_affected_crystals: 20,
        max_long_term_records_affected_per_run: 3,
        ..DreamConfig::default()
    };
    for phase in PassName::ALL {
        dream_config = dream_config.with_phase(phase, PhaseProfile::default());
    }
    for phase in [PassName::KnowledgeCrystals, PassName::CoverageAudit] {
        dream_config = dream_config.with_phase(
            phase,
            PhaseProfile {
                provider: "fake".into(),
                model: "fixture".into(),
                enabled: true,
                max_records_per_pass: 10,
            },
        );
    }

    DreamService::new_with_catalog(
        &fixture.pool,
        &fixture.config,
        &dream_config,
        resolver,
        fake_catalog(),
    )
    .run_all(CycleOptions::default())
    .await
    .unwrap();

    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crystals WHERE title LIKE 'Bulk finding %'"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap()
            <= 3
    );
}

#[tokio::test]
async fn activation_overflow_remains_unclaimed_until_the_next_cycle() {
    let fixture = Fixture::new().await;
    let workspace = WorkspaceStore::new(&fixture.pool);
    let session = workspace
        .start_session(
            &TranslationContext::new("book", "en", "ru"),
            "translation",
            "1",
            "1",
        )
        .await
        .unwrap();
    for text in ["first", "second"] {
        workspace
            .add_short_term(
                session.id,
                AddMemoryInput {
                    text: text.into(),
                    ..AddMemoryInput::default()
                },
            )
            .await
            .unwrap();
    }
    assert!(workspace.complete_session(session.id).await.unwrap());
    for index in 0..3 {
        let crystal_id = CrystalStore::new(&fixture.pool)
            .add(AddCrystalInput {
                title: format!("overflow {index}"),
                text: format!("overflow crystal {index}"),
                status: "active".into(),
                ..AddCrystalInput::default()
            })
            .await
            .unwrap();
        sqlx::query("INSERT INTO crystal_activations(crystal_id,session_id,recall_query,rank,score,reason,outcome,created_at) VALUES (?,?,'q',1,1.0,'useful','useful',CURRENT_TIMESTAMP)")
            .bind(crystal_id)
            .bind(session.id)
            .execute(&fixture.pool)
            .await
            .unwrap();
    }
    let service = fixture.service_with_change_limit(fake(), 1, 1, 10, 2);
    service.run_cycle(CycleOptions::default()).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crystal_activations WHERE cycle_id IS NULL"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
    service.run_cycle(CycleOptions::default()).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crystal_activations WHERE cycle_id IS NULL"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn exhausted_change_budget_skips_decay_instead_of_using_default_batch_size() {
    let fixture = Fixture::new().await;
    let reinforced = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "reinforce".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    let decay = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "must not decay".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    let before: (f64, f64) = sqlx::query_as("SELECT strength,confidence FROM crystals WHERE id=?")
        .bind(decay)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    let result = fixture
        .service_with_change_limit(fake(), 1, 10, 10, 1)
        .apply_maintenance(
            &MaintenancePayload {
                reinforce: vec![reinforced],
                decay: vec![decay],
                ..MaintenancePayload::default()
            },
            44,
        )
        .await
        .unwrap();
    assert_eq!(result.reinforced, 1);
    assert_eq!(result.decayed, 0);
    assert_eq!(
        sqlx::query_as::<_, (f64, f64)>("SELECT strength,confidence FROM crystals WHERE id=?")
            .bind(decay)
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn algorithmic_zero_remaining_budget_never_expands_to_default_decay_limit() {
    let fixture = Fixture::new().await;
    let session_id = fixture.pending_memory().await;
    let source = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "The term remains stable.".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    sqlx::query(
        "UPDATE short_term_memories SET source_crystal_id=?,kind='working_copy' WHERE session_id=?",
    )
    .bind(source)
    .bind(session_id)
    .execute(&fixture.pool)
    .await
    .unwrap();
    let decay_candidate = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "unrelated stale crystal".into(),
            strength: 0.5,
            confidence: 0.5,
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    fixture
        .service_with_phases(fake(), 1, 10, 10, 1, &[PassName::CoverageAudit])
        .run_cycle(CycleOptions::default())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_events WHERE crystal_id=? AND event_type='cycle_decay'"
        )
        .bind(decay_candidate)
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn directed_supersede_preserves_old_to_new_when_old_id_is_greater() {
    let fixture = Fixture::new().await;
    let new_id = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "replacement".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    let old_id = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "old".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    assert!(old_id > new_id);
    let result = fixture
        .service_with_change_limit(fake(), 1, 10, 10, 2)
        .apply_maintenance(
            &MaintenancePayload {
                supersede: vec![(old_id, new_id), (old_id, new_id)],
                ..MaintenancePayload::default()
            },
            45,
        )
        .await
        .unwrap();
    assert_eq!(result.superseded, 1);
    let statuses: Vec<String> = sqlx::query_scalar("SELECT status FROM crystals ORDER BY id")
        .fetch_all(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(statuses, vec!["active", "superseded"]);
}

#[tokio::test]
async fn no_op_inputs_still_consume_the_aggregate_maintenance_budget() {
    let fixture = Fixture::new().await;
    let floor_id = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "at floor".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    sqlx::query("UPDATE crystals SET strength=0,confidence=0 WHERE id=?")
        .bind(floor_id)
        .execute(&fixture.pool)
        .await
        .unwrap();
    let replacement = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "replacement".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    let old = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "old".into(),
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    let result = fixture
        .service_with_change_limit(fake(), 1, 10, 10, 1)
        .apply_maintenance(
            &MaintenancePayload {
                decay: vec![floor_id],
                supersede: vec![(old, replacement)],
                ..MaintenancePayload::default()
            },
            46,
        )
        .await
        .unwrap();
    assert_eq!(result.decayed, 0);
    assert_eq!(result.superseded, 0);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM crystals WHERE id=?")
            .bind(old)
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        "active"
    );
}

#[tokio::test]
async fn provider_reinforcement_applies_the_exact_validated_deltas() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let crystal_id = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            text: "reinforcement target".into(),
            strength: 0.4,
            confidence: 0.5,
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    sqlx::query("UPDATE crystals SET created_cycle=999,last_reinforced_cycle=999 WHERE id=?")
        .bind(crystal_id)
        .execute(&fixture.pool)
        .await
        .unwrap();
    let provider: Arc<dyn DreamProvider> = Arc::new(ReinforcementProvider {
        crystal_id,
        strength_delta: 0.07,
        confidence_delta: -0.04,
    });
    fixture
        .service_with_phases(
            provider,
            1,
            10,
            10,
            10,
            &[PassName::Reinforcement, PassName::CoverageAudit],
        )
        .run_cycle(CycleOptions::default())
        .await
        .unwrap();
    let scores: (f64, f64) = sqlx::query_as("SELECT strength,confidence FROM crystals WHERE id=?")
        .bind(crystal_id)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert!(
        (scores.0 - 0.47).abs() < f64::EPSILON,
        "unexpected scores: {scores:?}"
    );
    assert!(
        (scores.1 - 0.46).abs() < f64::EPSILON,
        "unexpected scores: {scores:?}"
    );
    let event: (f64, f64, bool) = sqlx::query_as(
        "SELECT strength_delta,confidence_delta,applied FROM memory_events WHERE crystal_id=? ORDER BY id DESC LIMIT 1",
    )
    .bind(crystal_id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(event, (0.07, -0.04, true));
}

#[tokio::test]
async fn output_persistence_and_source_archive_are_one_retry_safe_transaction() {
    for trigger in [
        "CREATE TRIGGER injected_failure BEFORE INSERT ON concepts BEGIN SELECT RAISE(ABORT,'injected concepts failure'); END",
        "CREATE TRIGGER injected_failure BEFORE INSERT ON concept_facets BEGIN SELECT RAISE(ABORT,'injected facets failure'); END",
        "CREATE TRIGGER injected_failure BEFORE INSERT ON concept_proposals BEGIN SELECT RAISE(ABORT,'injected proposals failure'); END",
        "CREATE TRIGGER injected_failure BEFORE INSERT ON crystals WHEN NEW.title='Stable finding' BEGIN SELECT RAISE(ABORT,'injected crystals failure'); END",
        "CREATE TRIGGER injected_failure BEFORE INSERT ON crystal_sources BEGIN SELECT RAISE(ABORT,'injected provenance failure'); END",
        "CREATE TRIGGER injected_failure BEFORE INSERT ON crystal_links BEGIN SELECT RAISE(ABORT,'injected relations failure'); END",
        "CREATE TRIGGER injected_failure BEFORE INSERT ON memory_events WHEN NEW.event_type='provider_reinforcement' BEGIN SELECT RAISE(ABORT,'injected reinforcement failure'); END",
        "CREATE TRIGGER injected_failure BEFORE UPDATE OF archived_at ON short_term_memories BEGIN SELECT RAISE(ABORT,'injected archive failure'); END",
    ] {
        let fixture = Fixture::new().await;
        fixture.pending_memory().await;
        let relation_source = CrystalStore::new(&fixture.pool)
            .add(AddCrystalInput {
                text: "relation source".into(),
                strength: 0.4,
                confidence: 0.5,
                status: "active".into(),
                ..AddCrystalInput::default()
            })
            .await
            .unwrap();
        let relation_target = CrystalStore::new(&fixture.pool)
            .add(AddCrystalInput {
                text: "relation target".into(),
                status: "active".into(),
                ..AddCrystalInput::default()
            })
            .await
            .unwrap();
        sqlx::query(
            "UPDATE crystals SET created_cycle=999,last_reinforced_cycle=999 WHERE id IN (?,?)",
        )
        .bind(relation_source)
        .bind(relation_target)
        .execute(&fixture.pool)
        .await
        .unwrap();
        let service = || {
            fixture.service_with_phases(
                Arc::new(FullOutputProvider {
                    relation_source,
                    relation_target,
                }),
                1,
                10,
                10,
                10,
                &[
                    PassName::Concepts,
                    PassName::TerminologyCandidates,
                    PassName::KnowledgeCrystals,
                    PassName::Relations,
                    PassName::Reinforcement,
                    PassName::CoverageAudit,
                ],
            )
        };
        sqlx::query(trigger).execute(&fixture.pool).await.unwrap();
        service()
            .run_cycle(CycleOptions::default())
            .await
            .unwrap_err();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM crystals WHERE title='Stable finding'"
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM concepts")
                .fetch_one(&fixture.pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM concept_proposals")
                .fetch_one(&fixture.pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystal_links")
                .fetch_one(&fixture.pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memory_events WHERE event_type='provider_reinforcement'"
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM short_term_memories WHERE archived_at IS NULL"
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
            1
        );
        sqlx::query("DROP TRIGGER injected_failure")
            .execute(&fixture.pool)
            .await
            .unwrap();
        service().run_cycle(CycleOptions::default()).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM crystals WHERE title='Stable finding'"
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM concepts")
                .fetch_one(&fixture.pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM concept_proposals")
                .fetch_one(&fixture.pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystal_links")
                .fetch_one(&fixture.pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM memory_events WHERE event_type='provider_reinforcement'"
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn late_persistence_failure_retries_algorithmic_work_exactly_once() {
    let fixture = Fixture::new().await;
    let session_id = fixture.pending_memory().await;
    let source = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            title: "working source".into(),
            text: "The term remains stable.".into(),
            strength: 0.4,
            confidence: 0.5,
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    sqlx::query(
        "UPDATE short_term_memories
         SET source_crystal_id=?,kind='working_copy'
         WHERE session_id=?",
    )
    .bind(source)
    .bind(session_id)
    .execute(&fixture.pool)
    .await
    .unwrap();
    let mut recalled = Vec::new();
    for (title, text) in [
        ("first recall", "A red lantern."),
        ("second recall", "A distant winter river."),
    ] {
        let crystal_id = CrystalStore::new(&fixture.pool)
            .add(AddCrystalInput {
                title: title.into(),
                text: text.into(),
                strength: 0.4,
                confidence: 0.5,
                status: "active".into(),
                ..AddCrystalInput::default()
            })
            .await
            .unwrap();
        recalled.push(crystal_id);
        sqlx::query(
            "INSERT INTO crystal_activations(
                crystal_id,session_id,recall_query,rank,score,reason,outcome,created_at
             ) VALUES (?,?,'query',1,1.0,'useful','useful',CURRENT_TIMESTAMP)",
        )
        .bind(crystal_id)
        .bind(session_id)
        .execute(&fixture.pool)
        .await
        .unwrap();
    }
    let stale = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            title: "stale".into(),
            text: "An unrelated stale observation.".into(),
            strength: 0.5,
            confidence: 0.5,
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    let passive = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            title: "passive".into(),
            text: "A passively reinforced memory.".into(),
            strength: 0.4,
            confidence: 0.5,
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO memory_events(crystal_id,event_type,source_role,evidence,strength_delta,confidence_delta,applied,created_at)
         VALUES (?,'recalled_again','recall','',0.02,0,0,CURRENT_TIMESTAMP)",
    )
    .bind(passive)
    .execute(&fixture.pool)
    .await
    .unwrap();
    for (name, status, confidence) in [
        ("Duplicate Name", "candidate", 0.9),
        (" duplicate name ", "established", 0.6),
    ] {
        sqlx::query(
            "INSERT INTO concepts(canonical_name,description,scope_type,scope_key,status,confidence,created_at,updated_at)
             VALUES (?,'','series','series:book',?,?,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
        )
        .bind(name)
        .bind(status)
        .bind(confidence)
        .execute(&fixture.pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "CREATE TRIGGER injected_late_failure
         BEFORE INSERT ON crystals
         WHEN NEW.title='Stable finding'
         BEGIN SELECT RAISE(ABORT,'injected late failure'); END",
    )
    .execute(&fixture.pool)
    .await
    .unwrap();
    let service = || fixture.service_with_change_limit(fake(), 1, 10, 10, 4);
    service()
        .run_cycle(CycleOptions::default())
        .await
        .unwrap_err();

    let algorithm_events_before: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT crystal_id,event_type,cycle_id
         FROM memory_events
         WHERE event_type IN ('reconsolidated_in_place','cycle_decay','combined_into')
         ORDER BY id",
    )
    .fetch_all(&fixture.pool)
    .await
    .unwrap();
    assert!(
        algorithm_events_before
            .iter()
            .any(|event| event.0 == source && event.1 == "reconsolidated_in_place")
    );
    assert!(
        algorithm_events_before
            .iter()
            .any(|event| event.0 == stale && event.1 == "cycle_decay")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crystal_links
             WHERE source_crystal_id=? AND target_crystal_id=?",
        )
        .bind(recalled[0])
        .bind(recalled[1])
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM concept_merge_proposals WHERE status='pending'"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM memory_events
             WHERE crystal_id=? AND event_type='recalled_again' AND applied=1"
        )
        .bind(passive)
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
    let scores_before: Vec<(i64, f64, f64)> =
        sqlx::query_as("SELECT id,strength,confidence FROM crystals ORDER BY id")
            .fetch_all(&fixture.pool)
            .await
            .unwrap();
    let activation_cycles_before: Vec<Option<i64>> =
        sqlx::query_scalar("SELECT cycle_id FROM crystal_activations ORDER BY id")
            .fetch_all(&fixture.pool)
            .await
            .unwrap();

    sqlx::query("DROP TRIGGER injected_late_failure")
        .execute(&fixture.pool)
        .await
        .unwrap();
    service().run_due().await.unwrap().unwrap();

    assert_eq!(
        sqlx::query_as::<_, (i64, String, i64)>(
            "SELECT crystal_id,event_type,cycle_id
             FROM memory_events
             WHERE event_type IN ('reconsolidated_in_place','cycle_decay','combined_into')
             ORDER BY id",
        )
        .fetch_all(&fixture.pool)
        .await
        .unwrap(),
        algorithm_events_before
    );
    assert_eq!(
        sqlx::query_as::<_, (i64, f64, f64)>(
            "SELECT id,strength,confidence FROM crystals
             WHERE title!='Stable finding' ORDER BY id",
        )
        .fetch_all(&fixture.pool)
        .await
        .unwrap(),
        scores_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT cycle_id FROM crystal_activations ORDER BY id",
        )
        .fetch_all(&fixture.pool)
        .await
        .unwrap(),
        activation_cycles_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM crystals WHERE title='Stable finding'",)
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM short_term_memories WHERE archived_at IS NULL",
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM concept_merge_proposals")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn cancelling_in_flight_persistence_rolls_back_outputs_and_retry_converges() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let stale = CrystalStore::new(&fixture.pool)
        .add(AddCrystalInput {
            title: "cancelled-cycle decay".into(),
            text: "A stale crystal changed before persistence cancellation.".into(),
            strength: 0.5,
            confidence: 0.5,
            status: "active".into(),
            ..AddCrystalInput::default()
        })
        .await
        .unwrap();
    {
        let service = fixture.service_with_phases(
            Arc::new(BulkOutputProvider { count: 1000 }),
            1,
            10,
            10,
            1000,
            &[PassName::KnowledgeCrystals, PassName::CoverageAudit],
        );
        let cycle = service.run_cycle(CycleOptions::default());
        tokio::pin!(cycle);
        tokio::select! {
            () = async {
                loop {
                    let completed: i64 = sqlx::query_scalar(
                        "SELECT count(*) FROM dream_audit_entries
                         WHERE event_type='algorithm_batch_completed'",
                    )
                    .fetch_one(&fixture.pool)
                    .await
                    .unwrap();
                    if completed == 1 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            } => {}
            result = &mut cycle => panic!("cycle completed before persistence cancellation: {result:?}"),
        }
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let open: i64 = sqlx::query_scalar(
                "SELECT
                   (SELECT count(*) FROM dream_runs WHERE status='running') +
                   (SELECT count(*) FROM dream_phase_runs WHERE status='running')",
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
            if open == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let run = sqlx::query("SELECT status,error,completed_at FROM dream_runs")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(run.get::<String, _>("status"), "failed");
    assert_eq!(run.get::<String, _>("error"), "dream cycle cancelled");
    assert!(run.get::<Option<String>, _>("completed_at").is_some());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM dream_phase_runs WHERE status='completed' AND completed_at IS NOT NULL"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        2
    );
    acquire_dream_cycle_lock(&fixture.config, "after-persistence-cancel", false).unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crystals WHERE title LIKE 'Bulk finding %'"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM short_term_memories WHERE archived_at IS NULL"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        1
    );
    let decay_before_retry: (f64, f64, i64) = sqlx::query_as(
        "SELECT crystals.strength,crystals.confidence,count(memory_events.id)
         FROM crystals
         LEFT JOIN memory_events
           ON memory_events.crystal_id=crystals.id
          AND memory_events.event_type='cycle_decay'
         WHERE crystals.id=?
         GROUP BY crystals.id",
    )
    .bind(stale)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(decay_before_retry.2, 1);
    fixture
        .service_with_phases(
            Arc::new(BulkOutputProvider { count: 1000 }),
            1,
            10,
            10,
            1000,
            &[PassName::KnowledgeCrystals, PassName::CoverageAudit],
        )
        .run_cycle(CycleOptions::default())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_as::<_, (f64, f64, i64)>(
            "SELECT crystals.strength,crystals.confidence,count(memory_events.id)
             FROM crystals
             LEFT JOIN memory_events
               ON memory_events.crystal_id=crystals.id
              AND memory_events.event_type='cycle_decay'
             WHERE crystals.id=?
             GROUP BY crystals.id",
        )
        .bind(stale)
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        decay_before_retry
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM crystals WHERE title LIKE 'Bulk finding %'"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        500
    );
}

#[tokio::test]
#[ignore]
async fn dream_service_process_helper() {
    let Ok(data_root) = std::env::var("HIERO_DREAM_TEST_DATA_ROOT") else {
        return;
    };
    let action = std::env::var("HIERO_DREAM_TEST_ACTION").unwrap();
    let entered = PathBuf::from(std::env::var("HIERO_DREAM_TEST_ENTERED").unwrap());
    let release = PathBuf::from(std::env::var("HIERO_DREAM_TEST_RELEASE").unwrap());
    let result = PathBuf::from(std::env::var("HIERO_DREAM_TEST_RESULT").unwrap());
    let config = HieronymusConfig::load(Some(PathBuf::from(data_root))).unwrap();
    config.ensure_directories().unwrap();
    let pool = connect_url(&format!("sqlite://{}", config.database_path().display()))
        .await
        .unwrap();
    migrate(&pool).await.unwrap();
    let fixture_config = Box::leak(Box::new(config));
    let dream_config = Box::leak(Box::new({
        let mut config = DreamConfig {
            enabled: true,
            min_pending_short_term_memories: 1,
            max_pending_short_term_memories: 10,
            max_short_term_memories_per_cycle: 10,
            max_short_term_memories_per_run: 10,
            ..DreamConfig::default()
        };
        for phase in PassName::ALL {
            config = config.with_phase(phase, PhaseProfile::default());
        }
        for phase in [PassName::KnowledgeCrystals, PassName::CoverageAudit] {
            config = config.with_phase(
                phase,
                PhaseProfile {
                    provider: "fake".into(),
                    model: "fixture".into(),
                    enabled: true,
                    max_records_per_pass: 10,
                },
            );
        }
        config
    }));
    let provider: Arc<dyn DreamProvider> = if action == "holder" {
        Arc::new(ProcessBlockingProvider { entered, release })
    } else {
        fake()
    };
    let resolver: Arc<dyn DreamProviderResolver> = Arc::new(
        move |_: &WorkflowProfile| -> Result<Arc<dyn DreamProvider>, DreamPhaseError> {
            Ok(provider.clone())
        },
    );
    let service = DreamService::new_with_catalog(
        &pool,
        fixture_config,
        dream_config,
        resolver,
        fake_catalog(),
    );
    let run = service
        .run_cycle(CycleOptions {
            owner: action.clone(),
            skip_when_locked: action == "contender",
            ..CycleOptions::default()
        })
        .await
        .unwrap();
    fs::write(result, run.status).unwrap();
}

#[tokio::test]
async fn two_process_services_cannot_overlap_one_data_root_cycle() {
    let fixture = Fixture::new().await;
    fixture.pending_memory().await;
    let root = fixture._temp.path();
    let entered = root.join("holder-entered");
    let release = root.join("holder-release");
    let holder_result = root.join("holder-result");
    let contender_result = root.join("contender-result");
    let executable = std::env::current_exe().unwrap();
    let base = |action: &str, result: &std::path::Path| {
        let mut command = Command::new(&executable);
        command
            .arg("--exact")
            .arg("dream_service_process_helper")
            .arg("--ignored")
            .arg("--nocapture")
            .env("HIERO_DREAM_TEST_DATA_ROOT", root)
            .env("HIERO_DREAM_TEST_ACTION", action)
            .env("HIERO_DREAM_TEST_ENTERED", &entered)
            .env("HIERO_DREAM_TEST_RELEASE", &release)
            .env("HIERO_DREAM_TEST_RESULT", result);
        command
    };
    let mut holder = base("holder", &holder_result).spawn().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !entered.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let contender = base("contender", &contender_result).output().unwrap();
    assert!(
        contender.status.success(),
        "contender failed: {}",
        String::from_utf8_lossy(&contender.stderr)
    );
    assert_eq!(fs::read_to_string(&contender_result).unwrap(), "skipped");
    fs::write(&release, b"release").unwrap();
    assert!(holder.wait().unwrap().success());
    assert_eq!(fs::read_to_string(holder_result).unwrap(), "completed");
}
