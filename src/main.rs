use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc, time::SystemTime};
use thiserror::Error;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;

#[derive(Parser, Debug)]
#[command(name = "checkup")]
#[command(about = "HTTP server for caching and serving repository releases", long_about = None)]
struct Args {
    /// Cache directory path
    #[arg(short, long, default_value = "data/cache")]
    cache: PathBuf,

    /// Cache expiration time in hours
    #[arg(short = 'e', long, default_value = "24")]
    cache_hours: i64,

    /// Server port
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Server host
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Invalid repository path: {0}")]
    InvalidRepoPath(String),
    #[error("Cache error: {0}")]
    CacheError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub download_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub published_at: DateTime<Utc>,
    pub html_url: String,
    pub body: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<Asset>,
    pub source_tarball: Option<String>,
    pub source_zipball: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedReleases {
    pub releases: Vec<Release>,
    pub cached_at: DateTime<Utc>,
    pub repo_path: String,
}

#[derive(Debug, Clone)]
pub struct RepoPath {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl RepoPath {
    pub fn parse(path: &str) -> Result<Self, AppError> {
        let re = Regex::new(r"^([^/]+)/([^/]+)/([^/]+)$").unwrap();

        if let Some(caps) = re.captures(path) {
            Ok(RepoPath {
                host: caps[1].to_string(),
                owner: caps[2].to_string(),
                repo: caps[3].to_string(),
            })
        } else {
            Err(AppError::InvalidRepoPath(path.to_string()))
        }
    }

    pub fn cache_key(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.repo)
    }
}

pub struct CacheManager {
    cache_dir: PathBuf,
    cache_duration: Duration,
}

impl CacheManager {
    pub fn new(cache_dir: PathBuf, cache_hours: i64) -> Self {
        Self {
            cache_dir,
            cache_duration: Duration::hours(cache_hours),
        }
    }

    pub fn get_cache_path(&self, repo_path: &RepoPath) -> PathBuf {
        self.cache_dir
            .join("repo")
            .join(&repo_path.host)
            .join(&repo_path.owner)
            .join(&repo_path.repo)
    }

    pub fn read_cache(&self, repo_path: &RepoPath) -> Result<Option<CachedReleases>> {
        let cache_dir = self.get_cache_path(repo_path);

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
            let cached: CachedReleases = serde_json::from_str(&content)?;

            // Check if cache is expired
            let now = Utc::now();
            if now - cached.cached_at > self.cache_duration {
                return Ok(None);
            }

            return Ok(Some(cached));
        }

        Ok(None)
    }

    pub fn write_cache(&self, repo_path: &RepoPath, releases: Vec<Release>) -> Result<()> {
        let cache_dir = self.get_cache_path(repo_path);
        fs::create_dir_all(&cache_dir)?;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let cache_file = cache_dir.join(format!("cache-{}.json", timestamp));

        let cached = CachedReleases {
            releases,
            cached_at: Utc::now(),
            repo_path: repo_path.cache_key(),
        };

        let content = serde_json::to_string_pretty(&cached)?;
        fs::write(&cache_file, content)?;

        Ok(())
    }
}

pub struct ReleaseFetcher {
    client: reqwest::Client,
}

impl ReleaseFetcher {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("checkup/0.1.0")
                .build()
                .unwrap(),
        }
    }

    pub async fn fetch_github_releases(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Release>, AppError> {
        let url = format!("https://api.github.com/repos/{}/{}/releases", owner, repo);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(AppError::CacheError(format!(
                "GitHub API returned status: {}",
                response.status()
            )));
        }

        let github_releases: Vec<GitHubRelease> = response.json().await?;

        Ok(github_releases
            .into_iter()
            .map(|r| {
                let mut assets: Vec<Asset> = r
                    .assets
                    .into_iter()
                    .map(|a| Asset {
                        name: a.name,
                        url: a.browser_download_url,
                        content_type: a.content_type,
                        size: a.size,
                        download_count: a.download_count,
                    })
                    .collect();

                // Add source archives as assets
                if let Some(tarball) = r.tarball_url {
                    assets.push(Asset {
                        name: format!("{}.tar.gz", r.tag_name),
                        url: tarball,
                        content_type: Some("application/gzip".to_string()),
                        size: 0,
                        download_count: 0,
                    });
                }
                if let Some(zipball) = r.zipball_url {
                    assets.push(Asset {
                        name: format!("{}.zip", r.tag_name),
                        url: zipball,
                        content_type: Some("application/zip".to_string()),
                        size: 0,
                        download_count: 0,
                    });
                }

                Release {
                    tag_name: r.tag_name,
                    name: r.name,
                    published_at: r.published_at,
                    html_url: r.html_url,
                    body: r.body,
                    prerelease: r.prerelease,
                    draft: r.draft,
                    assets,
                    source_tarball: None,
                    source_zipball: None,
                }
            })
            .collect())
    }

    pub async fn fetch_gitlab_releases(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Release>, AppError> {
        let encoded_path = urlencoding::encode(&format!("{}/{}", owner, repo));
        let url = format!(
            "https://gitlab.com/api/v4/projects/{}/releases",
            encoded_path
        );

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(AppError::CacheError(format!(
                "GitLab API returned status: {}",
                response.status()
            )));
        }

        let gitlab_releases: Vec<GitLabRelease> = response.json().await?;

        Ok(gitlab_releases
            .into_iter()
            .map(|r| {
                let mut assets = Vec::new();

                // Add sources (tar.gz, zip, etc.)
                for source in r.assets.sources {
                    assets.push(Asset {
                        name: format!("{}.{}", r.tag_name, source.format.to_lowercase()),
                        url: source.url,
                        content_type: Some(format!("application/{}", source.format.to_lowercase())),
                        size: 0,
                        download_count: 0,
                    });
                }

                // Add links (external binaries, etc.)
                for link in r.assets.links {
                    assets.push(Asset {
                        name: link.name,
                        url: link.url,
                        content_type: None,
                        size: 0,
                        download_count: 0,
                    });
                }

                Release {
                    tag_name: r.tag_name,
                    name: Some(r.name),
                    published_at: r.released_at,
                    html_url: r._links.self_url,
                    body: Some(r.description),
                    prerelease: false,
                    draft: false,
                    assets,
                    source_tarball: None,
                    source_zipball: None,
                }
            })
            .collect())
    }

    pub async fn fetch_releases(&self, repo_path: &RepoPath) -> Result<Vec<Release>, AppError> {
        match repo_path.host.as_str() {
            "github.com" => {
                self.fetch_github_releases(&repo_path.owner, &repo_path.repo)
                    .await
            }
            "gitlab.com" => {
                self.fetch_gitlab_releases(&repo_path.owner, &repo_path.repo)
                    .await
            }
            _ => Err(AppError::InvalidRepoPath(format!(
                "Unsupported host: {}. Use /forgejo/{} for Forgejo-based hosts.",
                repo_path.host,
                repo_path.cache_key()
            ))),
        }
    }

    pub async fn fetch_forgejo_releases(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Release>, AppError> {
        let url = format!("https://{}/api/v1/repos/{}/{}/releases", host, owner, repo);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(AppError::CacheError(format!(
                "Forgejo API ({}) returned status: {}",
                host,
                response.status()
            )));
        }

        let forgejo_releases: Vec<ForgejoRelease> = response.json().await?;

        Ok(forgejo_releases
            .into_iter()
            .map(|r| {
                let mut assets: Vec<Asset> = r
                    .assets
                    .into_iter()
                    .map(|a| Asset {
                        name: a.name,
                        url: a.browser_download_url,
                        content_type: None,
                        size: a.size.unwrap_or(0),
                        download_count: a.download_count.unwrap_or(0),
                    })
                    .collect();

                // Add source archives
                if let Some(tarball) = r.tarball_url {
                    assets.push(Asset {
                        name: format!("{}.tar.gz", r.tag_name),
                        url: tarball,
                        content_type: Some("application/gzip".to_string()),
                        size: 0,
                        download_count: 0,
                    });
                }
                if let Some(zipball) = r.zipball_url {
                    assets.push(Asset {
                        name: format!("{}.zip", r.tag_name),
                        url: zipball,
                        content_type: Some("application/zip".to_string()),
                        size: 0,
                        download_count: 0,
                    });
                }

                Release {
                    tag_name: r.tag_name,
                    name: Some(r.name),
                    published_at: r.published_at,
                    html_url: r.html_url,
                    body: Some(r.body),
                    prerelease: r.prerelease,
                    draft: r.draft,
                    assets,
                    source_tarball: None,
                    source_zipball: None,
                }
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    content_type: Option<String>,
    size: u64,
    download_count: u64,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    published_at: DateTime<Utc>,
    html_url: String,
    body: Option<String>,
    prerelease: bool,
    draft: bool,
    assets: Vec<GitHubAsset>,
    tarball_url: Option<String>,
    zipball_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitLabAsset {
    name: Option<String>,
    url: Option<String>,
    #[serde(default)]
    external: bool,
}

#[derive(Debug, Deserialize)]
struct GitLabRelease {
    tag_name: String,
    name: String,
    released_at: DateTime<Utc>,
    #[serde(rename = "_links")]
    _links: GitLabLinks,
    description: String,
    #[serde(default)]
    assets: GitLabAssets,
}

#[derive(Debug, Deserialize, Default)]
struct GitLabAssets {
    #[serde(default)]
    sources: Vec<GitLabSource>,
    #[serde(default)]
    links: Vec<GitLabLink>,
}

#[derive(Debug, Deserialize)]
struct GitLabSource {
    format: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct GitLabLink {
    name: String,
    url: String,
    external: bool,
}

#[derive(Debug, Deserialize)]
struct ForgejoRelease {
    tag_name: String,
    name: String,
    published_at: DateTime<Utc>,
    html_url: String,
    body: String,
    prerelease: bool,
    draft: bool,
    #[serde(default)]
    assets: Vec<ForgejoAsset>,
    tarball_url: Option<String>,
    zipball_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ForgejoAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
    download_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GitLabLinks {
    #[serde(rename = "self")]
    self_url: String,
}

mod urlencoding {
    pub fn encode(s: &str) -> String {
        url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }
}

pub struct AppState {
    cache: CacheManager,
    fetcher: ReleaseFetcher,
    pending_cache: Arc<RwLock<HashMap<String, bool>>>,
}

impl AppState {
    pub fn new(cache_dir: PathBuf, cache_hours: i64) -> Self {
        Self {
            cache: CacheManager::new(cache_dir, cache_hours),
            fetcher: ReleaseFetcher::new(),
            pending_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

pub fn format_releases_html(
    releases: &[Release],
    repo_path: &str,
    cached_at: Option<DateTime<Utc>>,
) -> String {
    let cache_info = cached_at
        .map(|t| {
            format!(
                "<p><em>Cached at: {}</em></p>",
                t.format("%Y-%m-%d %H:%M:%S UTC")
            )
        })
        .unwrap_or_default();

    let releases_html = releases
        .iter()
        .map(|r| {
            let prerelease_badge = if r.prerelease {
                r#" <span style="background: #f0ad4e; padding: 2px 6px; border-radius: 3px; font-size: 0.8em;">Pre-release</span>"#
            } else {
                ""
            };
            let draft_badge = if r.draft {
                r#" <span style="background: #777; padding: 2px 6px; border-radius: 3px; font-size: 0.8em;">Draft</span>"#
            } else {
                ""
            };
            let name = r.name.as_ref().unwrap_or(&r.tag_name);

            // Format assets - show prominently at the top
            let assets_html = if !r.assets.is_empty() {
                let assets_list = r
                    .assets
                    .iter()
                    .map(|a| {
                        let size_info = if a.size > 0 {
                            format!(" <span style='color: #666;'>({})</span>", format_size(a.size))
                        } else {
                            String::new()
                        };
                        let download_info = if a.download_count > 0 {
                            format!(" <span style='color: #28a745;'>⬇ {}</span>", a.download_count)
                        } else {
                            String::new()
                        };
                        let icon = if a.name.ends_with(".exe") || a.name.ends_with(".msi") {
                            "🪟"
                        } else if a.name.ends_with(".deb") || a.name.ends_with(".rpm") {
                            "🐧"
                        } else if a.name.ends_with(".dmg") || a.name.contains("darwin") || a.name.contains("macos") {
                            "🍎"
                        } else if a.name.ends_with(".AppImage") {
                            "📦"
                        } else if a.name.ends_with(".tar.gz") || a.name.ends_with(".tgz") {
                            "🗜️"
                        } else if a.name.ends_with(".zip") {
                            "🗜️"
                        } else if a.name.ends_with(".jar") {
                            "☕"
                        } else if a.name.contains("source") || a.name.contains("src") {
                            "📄"
                        } else {
                            "📎"
                        };
                        format!(
                            r#"<div style="padding: 8px; margin: 4px 0; background: #fff; border: 1px solid #e1e4e8; border-radius: 6px;">
                                {} <a href="{}" style="font-weight: 500; color: #0366d6;">{}</a>{}{}
                            </div>"#,
                            icon, a.url, a.name, size_info, download_info
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                format!(
                    r#"<div style="margin: 15px 0;">
                        <strong style="font-size: 1.1em;">📦 Downloads ({} files):</strong>
                        <div style="margin-top: 8px;">
                            {}
                        </div>
                    </div>"#,
                    r.assets.len(),
                    assets_list
                )
            } else {
                String::new()
            };

            // Body text - collapsible/hidden by default
            let body_html = if let Some(body) = &r.body {
                if !body.is_empty() {
                    let body_preview = body.lines().take(3).collect::<Vec<_>>().join("<br>");
                    format!(
                        r#"<details style="margin-top: 10px;">
                            <summary style="cursor: pointer; color: #0366d6; font-weight: 500;">📝 Show release notes</summary>
                            <div style="margin-top: 10px; padding: 10px; background: #f6f8fa; border-radius: 6px; white-space: pre-wrap; font-size: 0.9em;">{}</div>
                        </details>"#,
                        body_preview
                    )
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            format!(
                r#"<li style="margin-bottom: 25px; padding: 20px; background: #fff; border: 1px solid #e1e4e8; border-radius: 8px; list-style: none;">
                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 10px;">
                        <strong style="font-size: 1.3em;"><a href="{}" target="_blank" style="color: #0366d6;">{}</a></strong>{}{}
                    </div>
                    <small style="color: #586069;">📅 Published: {}</small>
                    {}
                    {}
                </li>"#,
                r.html_url,
                name,
                prerelease_badge,
                draft_badge,
                r.published_at.format("%Y-%m-%d %H:%M:%S UTC"),
                assets_html,
                body_html
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Releases - {}</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; }}
        h1 {{ color: #333; }}
        ul {{ list-style-type: none; padding: 0; }}
        li {{ border-bottom: 1px solid #eee; padding: 15px 0; }}
        a {{ color: #0366d6; text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
        small {{ color: #666; }}
        p {{ color: #444; margin: 5px 0; }}
    </style>
</head>
<body>
    <h1>Releases for {}</h1>
    {}
    <ul>
        {}
    </ul>
</body>
</html>"#,
        repo_path, repo_path, cache_info, releases_html
    )
}

async fn get_repo_releases(
    Path(repo_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, String)> {
    let repo = RepoPath::parse(&repo_path).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Check cache first
    if let Ok(Some(cached)) = state.cache.read_cache(&repo) {
        let html =
            format_releases_html(&cached.releases, &repo.cache_key(), Some(cached.cached_at));
        return Ok(Html(html));
    }

    // Check if we're already fetching this repo
    {
        let pending = state.pending_cache.read().await;
        if pending.contains_key(&repo.cache_key()) {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Already fetching releases for this repository. Please try again in a moment."
                    .to_string(),
            ));
        }
    }

    // Mark as pending
    {
        let mut pending = state.pending_cache.write().await;
        pending.insert(repo.cache_key(), true);
    }

    // Fetch releases
    let result = state.fetcher.fetch_releases(&repo).await;

    // Remove from pending
    {
        let mut pending = state.pending_cache.write().await;
        pending.remove(&repo.cache_key());
    }

    let releases = result.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Write to cache
    if let Err(e) = state.cache.write_cache(&repo, releases.clone()) {
        eprintln!("Failed to write cache: {}", e);
    }

    let html = format_releases_html(&releases, &repo.cache_key(), None);
    Ok(Html(html))
}

async fn get_forgejo_releases(
    Path(forgejo_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, String)> {
    // Parse: host/owner/repo
    let parts: Vec<&str> = forgejo_path.splitn(3, '/').collect();
    if parts.len() != 3 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid path format. Use: /forgejo/{host}/{owner}/{repo}".to_string(),
        ));
    }

    let repo = RepoPath {
        host: parts[0].to_string(),
        owner: parts[1].to_string(),
        repo: parts[2].to_string(),
    };

    // Check cache first
    if let Ok(Some(cached)) = state.cache.read_cache(&repo) {
        let html =
            format_releases_html(&cached.releases, &repo.cache_key(), Some(cached.cached_at));
        return Ok(Html(html));
    }

    // Check if we're already fetching this repo
    {
        let pending = state.pending_cache.read().await;
        if pending.contains_key(&repo.cache_key()) {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Already fetching releases for this repository. Please try again in a moment."
                    .to_string(),
            ));
        }
    }

    // Mark as pending
    {
        let mut pending = state.pending_cache.write().await;
        pending.insert(repo.cache_key(), true);
    }

    // Fetch releases from Forgejo
    let result = state
        .fetcher
        .fetch_forgejo_releases(&repo.host, &repo.owner, &repo.repo)
        .await;

    // Remove from pending
    {
        let mut pending = state.pending_cache.write().await;
        pending.remove(&repo.cache_key());
    }

    let releases = result.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Write to cache
    if let Err(e) = state.cache.write_cache(&repo, releases.clone()) {
        eprintln!("Failed to write cache: {}", e);
    }

    let html = format_releases_html(&releases, &repo.cache_key(), None);
    Ok(Html(html))
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

fn warm_cache_parallel(
    cache_manager: &CacheManager,
    fetcher: &ReleaseFetcher,
    cache_dir: &PathBuf,
) -> Result<()> {
    println!("Warming cache for existing repositories...");

    let repo_cache_dir = cache_dir.join("repo");
    if !repo_cache_dir.exists() {
        println!("No cached repositories found.");
        return Ok(());
    }

    let mut repos_to_warm = Vec::new();

    // Collect all repos that need warming
    for host_entry in fs::read_dir(&repo_cache_dir)? {
        let host_entry = host_entry?;
        if !host_entry.file_type()?.is_dir() {
            continue;
        }
        let host = host_entry.file_name().to_string_lossy().to_string();

        for owner_entry in fs::read_dir(host_entry.path())? {
            let owner_entry = owner_entry?;
            if !owner_entry.file_type()?.is_dir() {
                continue;
            }
            let owner = owner_entry.file_name().to_string_lossy().to_string();

            for repo_entry in fs::read_dir(owner_entry.path())? {
                let repo_entry = repo_entry?;
                if !repo_entry.file_type()?.is_dir() {
                    continue;
                }
                let repo = repo_entry.file_name().to_string_lossy().to_string();

                let repo_path = RepoPath {
                    host: host.clone(),
                    owner: owner.clone(),
                    repo,
                };

                // Check if cache is expired
                if cache_manager.read_cache(&repo_path)?.is_none() {
                    repos_to_warm.push(repo_path);
                }
            }
        }
    }

    if repos_to_warm.is_empty() {
        println!("All caches are up to date.");
        return Ok(());
    }

    println!("Found {} repositories to warm.", repos_to_warm.len());

    // Use rayon for parallel processing
    let fetcher = fetcher;
    let cache_manager = cache_manager;

    // Create a runtime for async operations
    let rt = tokio::runtime::Runtime::new()?;

    let results: Vec<_> = repos_to_warm
        .par_iter()
        .map(|repo| {
            println!(
                "Warming cache for: {}/{}/{}",
                repo.host, repo.owner, repo.repo
            );

            rt.block_on(async {
                match fetcher.fetch_releases(repo).await {
                    Ok(releases) => {
                        if let Err(e) = cache_manager.write_cache(repo, releases) {
                            Err(format!("Failed to write cache: {}", e))
                        } else {
                            Ok(repo.cache_key())
                        }
                    }
                    Err(e) => Err(format!("Failed to fetch: {}", e)),
                }
            })
        })
        .collect();

    let successful: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let failed: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();

    println!("Cache warming complete:");
    println!("  Successful: {}", successful.len());
    println!("  Failed: {}", failed.len());

    for error in failed {
        println!("  Error: {}", error);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Create cache directory if it doesn't exist
    fs::create_dir_all(&args.cache)?;

    let state = Arc::new(AppState::new(args.cache.clone(), args.cache_hours));

    // Warm cache on startup using parallel processing
    let cache_manager = CacheManager::new(args.cache.clone(), args.cache_hours);
    let fetcher = ReleaseFetcher::new();

    if let Err(e) = warm_cache_parallel(&cache_manager, &fetcher, &args.cache) {
        eprintln!("Cache warming failed: {}", e);
    }

    let app = Router::new()
        .route("/repo/*repo_path", get(get_repo_releases))
        .route("/forgejo/*forgejo_path", get(get_forgejo_releases))
        .route("/health", get(health_check))
        .route("/", get(|| async { Html(include_str!("index.html")) }))
        .nest_service("/cache", ServeDir::new(&args.cache))
        .with_state(state);

    let addr = format!("{}:{}", args.host, args.port);
    println!("Server listening on http://{}", addr);
    println!("Cache directory: {:?}", args.cache);
    println!("Cache expiration: {} hours", args.cache_hours);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
