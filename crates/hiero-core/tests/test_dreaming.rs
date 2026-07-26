use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use hiero_core::{
    config::HieronymusConfig,
    db::{connect_url, migrate},
    domain::{AddCrystalInput, AddMemoryInput, CrystalStore, TranslationContext, WorkspaceStore},
    dreaming::{
        CycleOptions, DreamConfig, DreamPhaseError, DreamProviderResolver, DreamService,
        MaintenancePayload, PhaseProfile, WorkflowProfile, acquire_dream_cycle_lock,
        run_background_loop,
    },
    provider::{DreamOutput, DreamProvider, PassName, ProviderError, ProviderPassOutput},
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
        if self.failure == Some(pass) {
            return Err(ProviderError::MalformedJson);
        }
        if let Some(entered) = &self.entered {
            entered.notify_one();
            self.release
                .as_ref()
                .expect("blocking fake has a release notification")
                .notified()
                .await;
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
        let workspace = WorkspaceStore::new(&self.pool);
        let context = TranslationContext::new("book", "en", "ru");
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
        let config = Box::leak(Box::new(config));
        DreamService::new(&self.pool, &self.config, config, resolver)
    }
}

fn fake() -> Arc<dyn DreamProvider> {
    Arc::new(FakeProvider {
        failure: None,
        entered: None,
        release: None,
    })
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
        7
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
        .service_with_limits(fake(), 1, 1, 1)
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
