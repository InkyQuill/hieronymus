use std::time::Duration;

use chrono::Utc;
use tokio::sync::broadcast;

use crate::domain::complete_stale_sessions;

use super::{DreamService, DreamServiceError};

pub async fn run_background_loop(
    service: DreamService<'_>,
    interval: Duration,
    mut shutdown: broadcast::Receiver<()>,
) -> Result<(), DreamServiceError> {
    if interval.is_zero() {
        return Err(DreamServiceError::InvalidInput(
            "scheduler interval must be greater than zero",
        ));
    }
    let stale_after = chrono::Duration::from_std(interval)
        .map_err(|_| DreamServiceError::InvalidInput("scheduler interval is out of range"))?;
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result = shutdown.recv() => {
                acknowledge_shutdown(result);
                return Ok(());
            }
            _ = ticks.tick() => {
                complete_stale_sessions(service.pool(), Utc::now() - stale_after)
                    .await
                    .map_err(|_| DreamServiceError::InvalidInput("stale sessions could not be completed"))?;
                let guard = match service.acquire_lock("autostart", false).await {
                    Ok(guard) => guard,
                    Err(DreamServiceError::Lock(error)) if error.is_already_running() => continue,
                    Err(error) => return Err(error),
                };
                let mut cycle = Box::pin(service.run_due_with_lock(guard));
                tokio::select! {
                    result = &mut cycle => {
                        result?;
                    }
                    result = shutdown.recv() => {
                        acknowledge_shutdown(result);
                        drop(cycle);
                        // The supervisor retains the OS lock until cancellation
                        // audit cleanup is durable. Reacquisition is the explicit
                        // completion acknowledgement for scheduler shutdown.
                        let cleanup_ack = service.acquire_lock("scheduler-shutdown", true).await?;
                        drop(cleanup_ack);
                        if service.take_cleanup_failure() {
                            return Err(DreamServiceError::Domain(
                                "audit cleanup did not complete before the scheduler shutdown deadline".into(),
                            ));
                        }
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn acknowledge_shutdown(_result: Result<(), broadcast::error::RecvError>) {}
