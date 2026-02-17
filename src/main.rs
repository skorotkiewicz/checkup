mod format_html;
mod provider;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::get,
    Router,
};
use chrono::{Duration, Utc};
use clap::Parser;
use format_html::{extract_extension, format_releases_html};
use provider::{CachedReleases, Release};
use regex::Regex;

use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;

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
    #[error("Provider error: {0}")]
    ProviderError(#[from] anyhow::Error),
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
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
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
        provider::github::fetch_releases(&self.client, owner, repo)
            .await
            .map_err(AppError::ProviderError)
    }

    pub async fn fetch_gitlab_releases(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Release>, AppError> {
        provider::gitlab::fetch_releases(&self.client, owner, repo)
            .await
            .map_err(AppError::ProviderError)
    }

    pub async fn fetch_forgejo_releases(
        &self,
        host: &str,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Release>, AppError> {
        provider::forgejo::fetch_releases(&self.client, host, owner, repo)
            .await
            .map_err(AppError::ProviderError)
    }

    pub async fn fetch_cgit_releases(
        &self,
        host: &str,
        repo_path: &str,
    ) -> Result<Vec<Release>, AppError> {
        provider::cgit::fetch_releases(&self.client, host, repo_path)
            .await
            .map_err(AppError::ProviderError)
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

// GitHub handler
async fn get_github_releases(
    Path(repo_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, String)> {
    // Check if requesting latest asset redirect
    if let Some(pos) = repo_path.rfind("/latest.") {
        let extension = &repo_path[pos + 8..];
        let repo_part = &repo_path[..pos];
        let repo =
            RepoPath::parse(repo_part).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let releases = get_or_fetch_github_releases(&state, &repo).await?;

        if let Some(latest) = releases.first() {
            for asset in &latest.assets {
                let asset_ext = extract_extension(&asset.name);
                if asset_ext == extension {
                    return Ok(Redirect::temporary(&asset.url).into_response());
                }
            }
        }
        return Err((
            StatusCode::NOT_FOUND,
            format!("No asset with extension '{}' found", extension),
        ));
    }

    let (path_str, want_cache) = if repo_path.ends_with("/cache") {
        (repo_path.trim_end_matches("/cache").to_string(), true)
    } else {
        (repo_path.clone(), false)
    };

    let repo = RepoPath::parse(&path_str).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let cached_at = state
        .cache
        .read_cache(&repo)
        .ok()
        .flatten()
        .map(|c| c.cached_at);
    let releases = get_or_fetch_github_releases(&state, &repo).await?;

    if want_cache {
        let cached = CachedReleases {
            releases,
            cached_at: cached_at.unwrap_or_else(|| Utc::now()),
            repo_path: repo.cache_key(),
        };
        return Ok(Json(cached).into_response());
    }

    let html = format_releases_html(&releases, &repo.cache_key(), "github", cached_at);
    Ok(Html(html).into_response())
}

async fn get_or_fetch_github_releases(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<Vec<Release>, (StatusCode, String)> {
    if let Ok(Some(cached)) = state.cache.read_cache(repo) {
        return Ok(cached.releases);
    }

    {
        let pending = state.pending_cache.read().await;
        if pending.contains_key(&repo.cache_key()) {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Already fetching this repository".to_string(),
            ));
        }
    }

    {
        let mut pending = state.pending_cache.write().await;
        pending.insert(repo.cache_key(), true);
    }

    let result = state
        .fetcher
        .fetch_github_releases(&repo.owner, &repo.repo)
        .await;

    {
        let mut pending = state.pending_cache.write().await;
        pending.remove(&repo.cache_key());
    }

    let releases = result.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state.cache.write_cache(repo, releases.clone());

    Ok(releases)
}

// Forgejo handler
async fn get_forgejo_releases(
    Path(forgejo_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, String)> {
    if let Some(pos) = forgejo_path.rfind("/latest.") {
        let extension = &forgejo_path[pos + 8..];
        let repo_part = &forgejo_path[..pos];
        let parts: Vec<&str> = repo_part.splitn(3, '/').collect();
        if parts.len() != 3 {
            return Err((StatusCode::BAD_REQUEST, "Invalid path format".to_string()));
        }
        let repo = RepoPath {
            host: parts[0].to_string(),
            owner: parts[1].to_string(),
            repo: parts[2].to_string(),
        };
        let releases = get_or_fetch_forgejo_releases(&state, &repo).await?;

        if let Some(latest) = releases.first() {
            for asset in &latest.assets {
                let asset_ext = extract_extension(&asset.name);
                if asset_ext == extension {
                    return Ok(Redirect::temporary(&asset.url).into_response());
                }
            }
        }
        return Err((
            StatusCode::NOT_FOUND,
            format!("No asset with extension '{}' found", extension),
        ));
    }

    let (path_str, want_cache) = if forgejo_path.ends_with("/cache") {
        (forgejo_path.trim_end_matches("/cache").to_string(), true)
    } else {
        (forgejo_path.clone(), false)
    };

    let parts: Vec<&str> = path_str.splitn(3, '/').collect();
    if parts.len() != 3 {
        return Err((StatusCode::BAD_REQUEST, "Invalid path format".to_string()));
    }
    let repo = RepoPath {
        host: parts[0].to_string(),
        owner: parts[1].to_string(),
        repo: parts[2].to_string(),
    };
    let cached_at = state
        .cache
        .read_cache(&repo)
        .ok()
        .flatten()
        .map(|c| c.cached_at);
    let releases = get_or_fetch_forgejo_releases(&state, &repo).await?;

    if want_cache {
        let cached = CachedReleases {
            releases,
            cached_at: cached_at.unwrap_or_else(|| Utc::now()),
            repo_path: repo.cache_key(),
        };
        return Ok(Json(cached).into_response());
    }

    let html = format_releases_html(&releases, &repo.cache_key(), "forgejo", cached_at);
    Ok(Html(html).into_response())
}

async fn get_or_fetch_forgejo_releases(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<Vec<Release>, (StatusCode, String)> {
    if let Ok(Some(cached)) = state.cache.read_cache(repo) {
        return Ok(cached.releases);
    }

    {
        let pending = state.pending_cache.read().await;
        if pending.contains_key(&repo.cache_key()) {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Already fetching this repository".to_string(),
            ));
        }
    }

    {
        let mut pending = state.pending_cache.write().await;
        pending.insert(repo.cache_key(), true);
    }

    let result = state
        .fetcher
        .fetch_forgejo_releases(&repo.host, &repo.owner, &repo.repo)
        .await;

    {
        let mut pending = state.pending_cache.write().await;
        pending.remove(&repo.cache_key());
    }

    let releases = result.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state.cache.write_cache(repo, releases.clone());

    Ok(releases)
}

// cgit handler
async fn get_cgit_releases(
    Path(cgit_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, String)> {
    if let Some(pos) = cgit_path.rfind("/latest.") {
        let extension = &cgit_path[pos + 8..];
        let repo_part = &cgit_path[..pos];
        let parts: Vec<&str> = repo_part.splitn(2, '/').collect();
        if parts.len() != 2 {
            return Err((StatusCode::BAD_REQUEST, "Invalid path format".to_string()));
        }
        let repo = RepoPath {
            host: parts[0].to_string(),
            owner: String::new(),
            repo: parts[1].to_string(),
        };
        let releases = get_or_fetch_cgit_releases(&state, &repo).await?;

        if let Some(latest) = releases.first() {
            for asset in &latest.assets {
                let asset_ext = extract_extension(&asset.name);
                if asset_ext == extension {
                    return Ok(Redirect::temporary(&asset.url).into_response());
                }
            }
        }
        return Err((
            StatusCode::NOT_FOUND,
            format!("No asset with extension '{}' found", extension),
        ));
    }

    let (path_str, want_cache) = if cgit_path.ends_with("/cache") {
        (cgit_path.trim_end_matches("/cache").to_string(), true)
    } else {
        (cgit_path.clone(), false)
    };

    let parts: Vec<&str> = path_str.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err((StatusCode::BAD_REQUEST, "Invalid path format".to_string()));
    }
    let repo = RepoPath {
        host: parts[0].to_string(),
        owner: String::new(),
        repo: parts[1].to_string(),
    };
    let cached_at = state
        .cache
        .read_cache(&repo)
        .ok()
        .flatten()
        .map(|c| c.cached_at);
    let releases = get_or_fetch_cgit_releases(&state, &repo).await?;

    if want_cache {
        let cached = CachedReleases {
            releases,
            cached_at: cached_at.unwrap_or_else(|| Utc::now()),
            repo_path: repo.cache_key(),
        };
        return Ok(Json(cached).into_response());
    }

    let html = format_releases_html(&releases, &repo.cache_key(), "cgit", cached_at);
    Ok(Html(html).into_response())
}

async fn get_or_fetch_cgit_releases(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<Vec<Release>, (StatusCode, String)> {
    if let Ok(Some(cached)) = state.cache.read_cache(repo) {
        return Ok(cached.releases);
    }

    {
        let pending = state.pending_cache.read().await;
        if pending.contains_key(&repo.cache_key()) {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Already fetching this repository".to_string(),
            ));
        }
    }

    {
        let mut pending = state.pending_cache.write().await;
        pending.insert(repo.cache_key(), true);
    }

    let result = state
        .fetcher
        .fetch_cgit_releases(&repo.host, &repo.repo)
        .await;

    {
        let mut pending = state.pending_cache.write().await;
        pending.remove(&repo.cache_key());
    }

    let releases = result.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state.cache.write_cache(repo, releases.clone());

    Ok(releases)
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    fs::create_dir_all(&args.cache)?;

    let state = Arc::new(AppState::new(args.cache.clone(), args.cache_hours));

    let app = Router::new()
        .route("/github/*repo_path", get(get_github_releases))
        .route("/forgejo/*forgejo_path", get(get_forgejo_releases))
        .route("/cgit/*cgit_path", get(get_cgit_releases))
        .route("/health", get(health_check))
        .route("/", get(|| async { Html(include_str!("index.html")) }))
        .with_state(state);

    let addr = format!("{}:{}", args.host, args.port);
    println!("Server listening on http://{}", addr);
    println!("Cache directory: {:?}", args.cache);
    println!("Cache expiration: {} hours", args.cache_hours);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
