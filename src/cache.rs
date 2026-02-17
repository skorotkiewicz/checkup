use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::de::DeserializeOwned;
use std::{fs, path::PathBuf};

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

    pub fn get_repo_dir(&self, host: &str, owner: &str, repo: &str) -> PathBuf {
        self.cache_dir
            .join("repo")
            .join(host)
            .join(owner)
            .join(repo)
    }

    pub fn read_timestamp(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Option<DateTime<Utc>>> {
        let repo_dir = self.get_repo_dir(host, owner, repo);
        let current_file = repo_dir.join(".current");

        if !current_file.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&current_file).context("Failed to read .current file")?;
        let timestamp = DateTime::parse_from_rfc3339(content.trim())
            .context("Failed to parse timestamp")?
            .with_timezone(&Utc);

        Ok(Some(timestamp))
    }

    pub fn write_timestamp(&self, host: &str, owner: &str, repo: &str) -> Result<()> {
        let repo_dir = self.get_repo_dir(host, owner, repo);
        fs::create_dir_all(&repo_dir)?;

        let current_file = repo_dir.join(".current");
        let timestamp = Utc::now().to_rfc3339();
        fs::write(&current_file, timestamp)?;

        Ok(())
    }

    pub fn read_json<T: DeserializeOwned>(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Option<T>> {
        let repo_dir = self.get_repo_dir(host, owner, repo);
        let json_file = repo_dir.join(".json");

        if !json_file.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&json_file).context("Failed to read .json file")?;
        let data: T = serde_json::from_str(&content).context("Failed to parse .json file")?;

        Ok(Some(data))
    }

    pub fn read_json_raw(&self, host: &str, owner: &str, repo: &str) -> Result<Option<String>> {
        let repo_dir = self.get_repo_dir(host, owner, repo);
        let json_file = repo_dir.join(".json");

        if !json_file.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&json_file).context("Failed to read .json file")?;
        Ok(Some(content))
    }

    pub fn write_json<T: serde::Serialize>(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
        data: &T,
    ) -> Result<()> {
        let repo_dir = self.get_repo_dir(host, owner, repo);
        fs::create_dir_all(&repo_dir)?;

        let json_file = repo_dir.join(".json");
        let content = serde_json::to_string_pretty(data)?;
        fs::write(&json_file, content)?;

        Ok(())
    }

    pub fn read_html(&self, host: &str, owner: &str, repo: &str) -> Result<Option<String>> {
        let repo_dir = self.get_repo_dir(host, owner, repo);
        let html_file = repo_dir.join("index.html");

        if !html_file.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&html_file).context("Failed to read index.html file")?;

        Ok(Some(content))
    }

    pub fn write_html(&self, host: &str, owner: &str, repo: &str, html: &str) -> Result<()> {
        let repo_dir = self.get_repo_dir(host, owner, repo);
        fs::create_dir_all(&repo_dir)?;

        let html_file = repo_dir.join("index.html");
        fs::write(&html_file, html)?;

        Ok(())
    }

    pub fn is_expired(&self, cached_at: DateTime<Utc>) -> bool {
        let now = Utc::now();
        now - cached_at > self.cache_duration
    }
}
