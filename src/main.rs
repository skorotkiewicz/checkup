use anyhow::{Context, Result};
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use chrono::{Duration, Utc};
use clap::Parser;
use regex::Regex;
use std::{fs, path::PathBuf, sync::Arc};

mod format_html;
mod provider;

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

#[derive(Debug, Clone)]
pub struct RepoPath {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl RepoPath {
    pub fn parse(path: &str) -> Result<Self> {
        let re = Regex::new(r"^([^/]+)/([^/]+)/([^/]+)$").unwrap();

        if let Some(caps) = re.captures(path) {
            Ok(RepoPath {
                host: caps[1].to_string(),
                owner: caps[2].to_string(),
                repo: caps[3].to_string(),
            })
        } else {
            anyhow::bail!("Invalid repository path: {}", path)
        }
    }

    pub fn cache_key(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.repo)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub client: reqwest::Client,
    pub cache: CacheManager,
}

#[derive(Clone)]
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

    pub fn read_cache(&self, repo_path: &RepoPath) -> Result<Option<provider::CachedReleases>> {
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

        let latest = entries.into_iter().max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

        if let Some(entry) = latest {
            let content = fs::read_to_string(entry.path())?;
            let cached: provider::CachedReleases = serde_json::from_str(&content)?;

            let now = Utc::now();
            if now - cached.cached_at > self.cache_duration {
                return Ok(None);
            }

            return Ok(Some(cached));
        }

        Ok(None)
    }

    pub fn write_cache(
        &self,
        repo_path: &RepoPath,
        releases: Vec<provider::Release>,
    ) -> Result<()> {
        let cache_dir = self.get_cache_path(repo_path);
        fs::create_dir_all(&cache_dir)?;

        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let cache_file = cache_dir.join(format!("cache-{}.json", timestamp));

        let cached = provider::CachedReleases {
            releases,
            cached_at: Utc::now(),
            repo_path: repo_path.cache_key(),
        };

        let content = serde_json::to_string_pretty(&cached)?;
        fs::write(&cache_file, content)?;

        Ok(())
    }
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    fs::create_dir_all(&args.cache)?;

    let state = Arc::new(AppState {
        client: reqwest::Client::builder()
            .user_agent("checkup/0.1.0")
            .build()?,
        cache: CacheManager::new(args.cache.clone(), args.cache_hours),
    });

    let app = Router::new()
        .route("/github/*repo_path", get(provider::github::handler))
        .route("/gitlab/*repo_path", get(provider::gitlab::handler))
        .route("/forgejo/*forgejo_path", get(provider::forgejo::handler))
        .route("/cgit/*cgit_path", get(provider::cgit::handler))
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
