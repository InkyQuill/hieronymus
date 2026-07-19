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
        provider: &str,
        mut models: Vec<String>,
        cached_at: DateTime<Utc>,
    ) -> Result<()> {
        if provider.is_empty() {
            return Err(ProviderError::Config(
                "cache provider must not be empty".into(),
            ));
        }
        models.sort();
        models.dedup();
        if models.is_empty() {
            return Err(ProviderError::Config(
                "cache does not store empty or error results".into(),
            ));
        }
        if entry_size(provider, &models) > self.max_bytes {
            return Err(ProviderError::Config(
                "model cache entry exceeds byte bound".into(),
            ));
        }
        let mut entries = self
            .entries
            .write()
            .map_err(|_| ProviderError::Config("model cache lock was poisoned".into()))?;
        entries.insert(provider.to_owned(), ModelCacheEntry { models, cached_at });
        while entries.len() > self.max_entries || total_size(&entries) > self.max_bytes {
            let victim = entries
                .iter()
                .min_by_key(|(key, value)| (value.cached_at, *key))
                .map(|(key, _)| key.clone())
                .ok_or_else(|| ProviderError::Config("model cache bounds are invalid".into()))?;
            entries.remove(&victim);
        }
        Ok(())
    }
    pub fn get(&self, provider: &str, now: DateTime<Utc>) -> Option<Vec<String>> {
        let mut entries = self.entries.write().ok()?;
        let entry = entries.get(provider)?;
        let age = now.signed_duration_since(entry.cached_at);
        if age < chrono::Duration::zero() || age >= chrono::Duration::from_std(self.ttl).ok()? {
            entries.remove(provider);
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
        let path = path.as_ref();
        let Some(contents) = super::catalog::secure_read(path)? else {
            return Ok(Self::new(max_entries, max_bytes, ttl));
        };
        if contents.len() > max_bytes {
            return Ok(Self::new(max_entries, max_bytes, ttl));
        }
        let persisted: PersistedCache = serde_json::from_str(&contents)
            .map_err(|_| ProviderError::Config("model cache is malformed".into()))?;
        let cache = Self::new(max_entries, max_bytes, ttl);
        for (key, entry) in persisted.entries {
            if now.signed_duration_since(entry.cached_at) >= chrono::Duration::zero()
                && now.signed_duration_since(entry.cached_at)
                    < chrono::Duration::from_std(ttl)
                        .map_err(|_| ProviderError::Config("cache TTL is invalid".into()))?
            {
                let _ = cache.insert(&key, entry.models, entry.cached_at);
            }
        }
        Ok(cache)
    }
}

fn entry_size(provider: &str, models: &[String]) -> usize {
    provider
        .len()
        .saturating_add(models.iter().map(String::len).sum::<usize>())
}
fn total_size(entries: &BTreeMap<String, ModelCacheEntry>) -> usize {
    entries
        .iter()
        .map(|(key, value)| entry_size(key, &value.models))
        .sum()
}
