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
                match result {
                    Ok(()) | Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    Err(broadcast::error::RecvError::Lagged(_)) => return Ok(()),
                }
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
                service.run_due_with_lock(guard).await?;
            }
        }
    }
}
