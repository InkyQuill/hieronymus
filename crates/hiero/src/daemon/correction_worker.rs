//! Supervised correction recovery at startup and every sixty seconds, including
//! idle daemons. Prepared output never needs provider admission or a new call.
use chrono::{DateTime, Utc};
use hieronymus::{
    consolidation::{
        ConsolidationError, ConsolidationLease, ConsolidationStore, DraftPreparationError,
        FailureKind, finish_result_tx, prepare_correction_draft, select_correction_context,
    },
    data_root::HieronymusConfig,
    db::open_migrated,
    dream_config::load_dream_config,
    dream_locks::{DreamLockError, dream_cycle_lock},
    dream_output::parse_decisions,
    dream_providers::LlmDreamProvider,
    provider_config::{ProviderCatalog, load_provider_catalog},
};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub type CorrectionClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;
pub type CorrectionCall = Box<dyn Fn(&Value) -> Result<Value, String>>;
pub type CorrectionSource = Arc<dyn Fn() -> CorrectionProvider + Send + Sync>;
/// Created inside the worker so provider clients need not cross threads.
pub struct CorrectionProvider {
    pub slot: String,
    pub fingerprint: String,
    pub call: Result<CorrectionCall, String>,
}
impl CorrectionProvider {
    /// Catalog map key is identity. Fingerprints contain no credentials, URL
    /// query, user info, or display label; changes never reset recovery budgets.
    pub fn from_catalog(catalog: ProviderCatalog) -> Self {
        let slot: String = if catalog.defaults.provider.trim().is_empty() {
            "default".into()
        } else {
            catalog.defaults.provider.trim().into()
        };
        let profile = (!catalog.defaults.provider.trim().is_empty())
            .then(|| catalog.providers.get(&slot))
            .flatten();
        let fingerprint = if profile.is_none() {
            "unconfigured".into()
        } else {
            format!("{:x}",Sha256::digest(serde_json::json!({
            "slot":slot,"model":catalog.defaults.model.trim(),
            "type":profile.map(|p|p.provider_type()),
            "timeout":profile.map(|p|p.timeout_seconds()),
            "endpoint":profile.and_then(|p| url::Url::parse(p.url()).ok()).map(|mut url| { let _=url.set_username("");let _=url.set_password(None);url.set_query(None);url.set_fragment(None);url.to_string() }),
        }).to_string()))
        };
        let call = profile
            .ok_or_else(|| "provider_unavailable".to_string())
            .and_then(|profile| {
                LlmDreamProvider::new(slot.clone(), profile.clone(), catalog.defaults.model)
                    .map_err(|_| "provider_unavailable".to_string())
            })
            .map(|provider| {
                Box::new(move |context: &Value| {
                    provider
                        .run_correction(context)
                        .map_err(|_| "provider_request".into())
                }) as CorrectionCall
            });
        Self {
            slot,
            fingerprint,
            call,
        }
    }
}

pub fn start(
    config: HieronymusConfig,
    workers: &super::workers::WorkerGroup,
) -> Result<(), String> {
    let provider_config = config.clone();
    start_with_source(
        config,
        workers,
        Arc::new(Utc::now),
        Arc::new(move || {
            CorrectionProvider::from_catalog(
                load_provider_catalog(&provider_config).unwrap_or_default(),
            )
        }),
    )
}
/// Production and tests use the same startup/idle loop and WorkerGroup ownership.
pub fn start_with_source(
    config: HieronymusConfig,
    workers: &super::workers::WorkerGroup,
    clock: CorrectionClock,
    source: CorrectionSource,
) -> Result<(), String> {
    workers.spawn(Box::new(move |stop| {
        let mut next = clock();
        while !stop.load(Ordering::Acquire) {
            let now = clock();
            if now >= next {
                // One tick after a forward jump; backward time delays work.
                next = now + chrono::Duration::seconds(60);
                match std::panic::catch_unwind(AssertUnwindSafe(|| {
                    tick(&config, &stop, &clock, &source)
                })) {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => {
                        eprintln!("correction worker: local storage or invariant failure")
                    }
                    Err(_) => {
                        eprintln!("correction worker: interrupted tick; lease recovery scheduled")
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }))
}
/// Execute one bounded provider attempt, after provider-free prepared recovery.
pub fn tick(
    config: &HieronymusConfig,
    stop: &AtomicBool,
    clock: &CorrectionClock,
    source: &CorrectionSource,
) -> Result<(), String> {
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut db = open_migrated(&config.database_path()).map_err(|e| e.to_string())?;
    // Reclaim prior crashes even when admission is busy. This does not admit
    // a new attempt or advance the persisted provider recovery budget.
    ConsolidationStore::new(&mut db)
        .recovery_tick(clock())
        .map_err(|e| e.to_string())?;
    let series = ConsolidationStore::new(&mut db)
        .eligible_series(clock())
        .map_err(|e| e.to_string())?;
    let mut completed_prepared = false;
    for id in &series {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(lease) = ConsolidationStore::new(&mut db)
            .lease_prepared(clock(), *id)
            .map_err(|e| e.to_string())?
        {
            complete(&mut db, &lease, clock()).map_err(|e| e.to_string())?;
            completed_prepared = true;
        }
    }
    if completed_prepared || series.is_empty() || stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let _admission = match dream_cycle_lock(config, "correction consolidation") {
        Ok(lock) => lock,
        Err(DreamLockError::AlreadyRunning { .. }) => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let provider = source();
    ConsolidationStore::new(&mut db)
        .assign_provider(&provider.slot, &provider.fingerprint, clock())
        .map_err(|e| e.to_string())?;
    for id in series {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(lease) = ConsolidationStore::new(&mut db)
            .lease_next(clock(), id)
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        if lease.canonical_output.is_some() {
            complete(&mut db, &lease, clock()).map_err(|e| e.to_string())?;
            return Ok(());
        }
        let result = evaluate(&mut db, config, &lease, &provider, clock, stop);
        match result {
            Ok(true) => complete(&mut db, &lease, clock()).map_err(|e| e.to_string())?,
            Ok(false) => {}
            Err((kind, code)) => {
                ConsolidationStore::new(&mut db)
                    .fail(&lease.token, kind, code, clock())
                    .map_err(|e| e.to_string())?;
            }
        }
        return Ok(());
    }
    Ok(())
}
fn evaluate(
    db: &mut Connection,
    config: &HieronymusConfig,
    lease: &ConsolidationLease,
    provider: &CorrectionProvider,
    clock: &CorrectionClock,
    stop: &AtomicBool,
) -> Result<bool, (FailureKind, &'static str)> {
    let limits = load_dream_config(config)
        .map_err(|_| (FailureKind::Transient, "configuration_unavailable"))?;
    let selection = select_correction_context(db, lease, &limits)
        .map_err(|_| (FailureKind::Deterministic, "local_selection"))?;
    if clock() >= lease.expires_at {
        return Ok(false);
    }
    let call = provider
        .call
        .as_ref()
        .map_err(|_| (FailureKind::Transient, "provider_unavailable"))?;
    let value =
        call(selection.projection()).map_err(|_| (FailureKind::Transient, "provider_request"))?;
    if stop.load(Ordering::Acquire) {
        return Ok(false);
    } // lease survives for bounded recovery
    let draft = parse_decisions(value).map_err(|_| (FailureKind::Transient, "provider_schema"))?;
    prepare_correction_draft(db, lease, selection, draft, clock()).map_err(
        |error| match error {
            DraftPreparationError::Provider(_) => (FailureKind::Transient, "provider_policy"),
            DraftPreparationError::Local(_) => (FailureKind::Deterministic, "local_preparation"),
        },
    )?;
    Ok(true)
}
fn complete(
    db: &mut Connection,
    lease: &ConsolidationLease,
    now: DateTime<Utc>,
) -> Result<(), ConsolidationError> {
    let outcome = {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = finish_result_tx(&tx, &lease.result_id, &lease.token, now);
        match outcome {
            Ok(_) => {
                tx.commit()?;
                Ok(())
            }
            Err(error) => Err(error),
        }
    };
    if let Err(error) = outcome {
        // Already prepared bytes are never silently re-evaluated or hot-looped.
        if !matches!(error, ConsolidationError::ExpiredLease) {
            ConsolidationStore::new(db).fail(
                &lease.token,
                FailureKind::Deterministic,
                "local_prepared_result",
                now,
            )?;
        }
        return Err(error);
    }
    Ok(())
}
