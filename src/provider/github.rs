use super::{Asset, Release};
use crate::{
    AppState, RepoPath,
    format_html::{format_processing_html, format_releases_html, rename_to_latest},
    provider::CachedReleases,
};
use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

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
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    content_type: Option<String>,
    size: u64,
    download_count: u64,
}

pub async fn fetch_releases(client: &Client, owner: &str, repo: &str) -> Result<Vec<Release>> {
    let url = format!("https://api.github.com/repos/{}/{}/releases", owner, repo);

    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("User-Agent", "checkup/0.1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("GitHub API returned status: {}", response.status());
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

pub async fn handler(
    Path(repo_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, String)> {
    if let Some(pos) = repo_path.rfind('/') {
        let last_segment = &repo_path[pos + 1..];
        if last_segment.starts_with("latest") {
            let repo_part = &repo_path[..pos];
            let repo = parse_github_path(repo_part)?;
            let releases = fetch_blocking(&state, &repo).await?;

            if let Some(latest) = releases.first() {
                for asset in &latest.assets {
                    if rename_to_latest(&asset.name) == last_segment {
                        return Ok(Redirect::temporary(&asset.url).into_response());
                    }
                }
            }
            return Err((
                StatusCode::NOT_FOUND,
                format!("No asset matching '{}' found", last_segment),
            ));
        }
    }

    let (path_str, want_cache) = if repo_path.ends_with("/cache") {
        (repo_path.trim_end_matches("/cache").to_string(), true)
    } else {
        (repo_path.clone(), false)
    };

    let repo = parse_github_path(&path_str)?;
    let cache_key = repo.cache_key();

    match get_or_spawn_fetch(&state, &repo).await {
        Ok(FetchResult::Cached {
            releases,
            cached_at,
        }) => {
            if want_cache {
                let cached = super::CachedReleases {
                    releases,
                    cached_at,
                    repo_path: cache_key.clone(),
                };
                return Ok(Json(cached).into_response());
            }
            let html = format_releases_html(&releases, &cache_key, "github", Some(cached_at));
            Ok(Html(html).into_response())
        }
        Ok(FetchResult::Processing) => {
            let html = format_processing_html(&cache_key, "github");
            Ok(Html(html).into_response())
        }
        Err(e) => Err(e),
    }
}

fn parse_github_path(path: &str) -> Result<RepoPath, (StatusCode, String)> {
    let parts: Vec<&str> = path.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid path format. Use: /github/{owner}/{repo}".to_string(),
        ));
    }
    Ok(RepoPath {
        host: "github.com".to_string(),
        owner: parts[0].to_string(),
        repo: parts[1].to_string(),
    })
}

pub enum FetchResult {
    Cached {
        releases: Vec<Release>,
        cached_at: DateTime<Utc>,
    },
    Processing,
}

pub async fn get_or_spawn_fetch(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<FetchResult, (StatusCode, String)> {
    if let Ok(Some(cached)) =
        state
            .cache
            .read_cache::<CachedReleases>(&repo.host, &repo.owner, &repo.repo)
    {
        if !state.cache.is_expired(cached.cached_at) {
            return Ok(FetchResult::Cached {
                releases: cached.releases,
                cached_at: cached.cached_at,
            });
        }
    }

    let cache_key = repo.cache_key();

    if state.pending_repos.contains(&cache_key) {
        return Ok(FetchResult::Processing);
    }

    state.pending_repos.insert(cache_key.clone());

    let state = state.clone();
    let repo = repo.clone();
    tokio::spawn(async move {
        let result = fetch_and_cache(&state, &repo).await;
        state.pending_repos.remove(&cache_key);
        result
    });

    Ok(FetchResult::Processing)
}

async fn fetch_and_cache(state: &Arc<AppState>, repo: &RepoPath) -> Result<()> {
    let releases = fetch_releases(&state.client, &repo.owner, &repo.repo).await?;

    let cached = CachedReleases {
        releases,
        cached_at: Utc::now(),
        repo_path: repo.cache_key(),
    };
    let _ = state
        .cache
        .write_cache(&repo.host, &repo.owner, &repo.repo, &cached);
    Ok(())
}

async fn fetch_blocking(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<Vec<Release>, (StatusCode, String)> {
    if let Ok(Some(cached)) =
        state
            .cache
            .read_cache::<CachedReleases>(&repo.host, &repo.owner, &repo.repo)
    {
        if !state.cache.is_expired(cached.cached_at) {
            return Ok(cached.releases);
        }
    }

    let releases = fetch_releases(&state.client, &repo.owner, &repo.repo)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let cached = CachedReleases {
        releases: releases.clone(),
        cached_at: Utc::now(),
        repo_path: repo.cache_key(),
    };
    let _ = state
        .cache
        .write_cache(&repo.host, &repo.owner, &repo.repo, &cached);
    Ok(releases)
}
