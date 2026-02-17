use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::de::DeserializeOwned;
use std::{fs, path::PathBuf, time::SystemTime};

#[derive(Clone)]
pub struct CacheManager {
    pub cache_dir: PathBuf,
    pub cache_duration: Duration,
}

impl CacheManager {
    pub fn new(cache_dir: PathBuf, cache_hours: i64) -> Self {
        Self {
            cache_dir,
            cache_duration: Duration::hours(cache_hours),
        }
    }

    pub fn get_cache_path(&self, host: &str, owner: &str, repo: &str) -> PathBuf {
        self.cache_dir
            .join("repo")
            .join(host)
            .join(owner)
            .join(repo)
    }

    pub fn read_cache<T: DeserializeOwned>(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Option<T>> {
        let cache_dir = self.get_cache_path(host, owner, repo);

        if !cache_dir.exists() {
            return Ok(None);
        }

        let entries: Vec<_> = fs::read_dir(&cache_dir)
            .context("Failed to read cache directory")?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .collect();

        if entries.is_empty() {
            return Ok(None);
        }

        // Get the most recent cache file
        let latest = entries.into_iter().max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });

        if let Some(entry) = latest {
            let content = fs::read_to_string(entry.path())?;
            let cached: T = serde_json::from_str(&content)?;

            // Check if cache is expired by looking at the cached_at field
            // This is handled by the caller since T is generic
            return Ok(Some(cached));
        }

        Ok(None)
    }

    pub fn write_cache<T: serde::Serialize>(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
        data: &T,
    ) -> Result<()> {
        let cache_dir = self.get_cache_path(host, owner, repo);
        fs::create_dir_all(&cache_dir)?;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let cache_file = cache_dir.join(format!("cache-{}.json", timestamp));

        let content = serde_json::to_string_pretty(data)?;
        fs::write(&cache_file, content)?;

        Ok(())
    }

    pub fn is_expired(&self, cached_at: DateTime<Utc>) -> bool {
        let now = Utc::now();
        now - cached_at > self.cache_duration
    }
}
