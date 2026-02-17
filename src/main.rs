use anyhow::Result;
use axum::{
    Router,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
};
use clap::Parser;
use regex::Regex;
use std::{fs, path::PathBuf, sync::Arc};

mod cache;
mod format_html;
mod icons;
mod provider;

#[derive(Parser, Debug)]
#[command(name = "checkup")]
#[command(about = "HTTP server for caching and serving repository releases", version, long_about = None)]
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
    pub cache: cache::CacheManager,
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
        cache: cache::CacheManager::new(args.cache.clone(), args.cache_hours),
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
