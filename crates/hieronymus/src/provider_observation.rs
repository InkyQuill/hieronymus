//! Ephemeral, content-free observations of real generation attempts.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProviderKey {
    pub profile: String,
    pub model: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOutcome {
    Success,
    Unavailable,
    Authentication,
    RateLimited,
    InvalidResponse,
}

pub trait ProviderObserver: Send + Sync {
    /// Reconcile configuration at request start without relabeling an old client.
    fn starting(&self) {}
    fn completed(&self, key: &ProviderKey, sequence: u64, outcome: ProviderOutcome);
}

/// Each logical generation receives one process-wide sequence before HTTP I/O.
/// Retries belong to that generation; completion order cannot reorder health.
#[derive(Clone)]
pub(crate) struct Observation {
    pub observer: Arc<dyn ProviderObserver>,
    pub key: ProviderKey,
}

impl std::fmt::Debug for Observation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Observation").finish_non_exhaustive()
    }
}

impl Observation {
    pub fn start(&self) -> u64 {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        self.observer.starting();
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    }

    pub fn finish(&self, sequence: u64, outcome: ProviderOutcome) {
        self.observer.completed(&self.key, sequence, outcome);
    }
}
