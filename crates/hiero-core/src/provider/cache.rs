use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, RwLock},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{ProviderError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCacheEntry {
    pub models: Vec<String>,
    pub cached_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedCache {
    entries: BTreeMap<String, ModelCacheEntry>,
}

#[derive(Debug, Clone)]
pub struct ModelCache {
    entries: Arc<RwLock<BTreeMap<String, ModelCacheEntry>>>,
    max_entries: usize,
    max_bytes: usize,
    ttl: Duration,
}

impl ModelCache {
    #[must_use]
    pub fn new(max_entries: usize, max_bytes: usize, ttl: Duration) -> Self {
        Self {
            entries: Arc::new(RwLock::new(BTreeMap::new())),
            max_entries,
            max_bytes,
            ttl,
        }
    }

    pub fn from_entries(
        max_entries: usize,
        max_bytes: usize,
        ttl: Duration,
        entries: BTreeMap<String, ModelCacheEntry>,
    ) -> Result<Self> {
        let cache = Self::new(max_entries, max_bytes, ttl);
        for (key, entry) in entries {
            cache.insert(&key, entry.models, entry.cached_at)?;
        }
        Ok(cache)
    }

    pub fn insert(
        &self,
        identity: &str,
        mut models: Vec<String>,
        cached_at: DateTime<Utc>,
    ) -> Result<()> {
        if identity.is_empty() {
            return Err(ProviderError::Config(
                "cache identity must not be empty".into(),
            ));
        }
        models.sort();
        models.dedup();
        if models.is_empty() {
            return Err(ProviderError::Config(
                "cache does not store empty or error results".into(),
            ));
        }

        let mut guard = self
            .entries
            .write()
            .map_err(|_| ProviderError::Config("model cache lock was poisoned".into()))?;
        let mut candidate = guard.clone();
        candidate.insert(identity.to_owned(), ModelCacheEntry { models, cached_at });
        if serialized_size(&BTreeMap::from([(
            identity.to_owned(),
            candidate[identity].clone(),
        )]))?
            > self.max_bytes
        {
            return Err(ProviderError::Config(
                "model cache entry exceeds serialized byte bound".into(),
            ));
        }
        while candidate.len() > self.max_entries || serialized_size(&candidate)? > self.max_bytes {
            let victim = candidate
                .iter()
                .min_by_key(|(key, value)| (value.cached_at, *key))
                .map(|(key, _)| key.clone())
                .ok_or_else(|| ProviderError::Config("model cache bounds are invalid".into()))?;
            candidate.remove(&victim);
        }
        *guard = candidate;
        Ok(())
    }

    pub fn get(&self, identity: &str, now: DateTime<Utc>) -> Option<Vec<String>> {
        let mut entries = self.entries.write().ok()?;
        let entry = entries.get(identity)?;
        let age = now.signed_duration_since(entry.cached_at);
        if age < chrono::Duration::zero() || age >= chrono::Duration::from_std(self.ttl).ok()? {
            entries.remove(identity);
            return None;
        }
        Some(entry.models.clone())
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let entries = self
            .entries
            .read()
            .map_err(|_| ProviderError::Config("model cache lock was poisoned".into()))?
            .clone();
        let bytes = serde_json::to_vec_pretty(&PersistedCache { entries })
            .map_err(|error| ProviderError::Config(error.to_string()))?;
        if bytes.len() > self.max_bytes {
            return Err(ProviderError::Config(
                "serialized model cache exceeds byte bound".into(),
            ));
        }
        super::catalog::secure_write(path.as_ref(), &bytes)
    }

    pub fn load(
        path: impl AsRef<Path>,
        max_entries: usize,
        max_bytes: usize,
        ttl: Duration,
        now: DateTime<Utc>,
    ) -> Result<Self> {
        let empty = || Self::new(max_entries, max_bytes, ttl);
        let contents = match super::catalog::secure_read_bounded(path.as_ref(), max_bytes) {
            Ok(Some(contents)) => contents,
            Ok(None) | Err(ProviderError::ResponseTooLarge { .. }) => return Ok(empty()),
            Err(ProviderError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
                return Ok(empty());
            }
            Err(error) => return Err(error),
        };
        let persisted: PersistedCache = match serde_json::from_str(&contents) {
            Ok(persisted) => persisted,
            Err(_) => return Ok(empty()),
        };
        let cache = empty();
        let ttl = chrono::Duration::from_std(ttl)
            .map_err(|_| ProviderError::Config("cache TTL is invalid".into()))?;
        for (key, entry) in persisted.entries {
            let age = now.signed_duration_since(entry.cached_at);
            if age >= chrono::Duration::zero()
                && age < ttl
                && cache.insert(&key, entry.models, entry.cached_at).is_err()
            {
                continue;
            }
        }
        Ok(cache)
    }

    #[cfg(test)]
    pub fn serialized_len(&self) -> Result<usize> {
        let entries = self
            .entries
            .read()
            .map_err(|_| ProviderError::Config("model cache lock was poisoned".into()))?;
        serialized_size(&entries)
    }
}

fn serialized_size(entries: &BTreeMap<String, ModelCacheEntry>) -> Result<usize> {
    serde_json::to_vec_pretty(&PersistedCache {
        entries: entries.clone(),
    })
    .map(|bytes| bytes.len())
    .map_err(|error| ProviderError::Config(error.to_string()))
}
