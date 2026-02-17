use super::{Asset, Release};
use crate::{
    AppState, RepoPath,
    format_html::{
        format_error_html, format_processing_html, format_releases_html, rename_to_latest,
    },
    provider::CachedReleases,
};
use anyhow::Result;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;

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

pub async fn fetch_releases(client: &Client, owner: &str, repo: &str) -> Result<Vec<Release>> {
    let encoded_path = urlencoding::encode(&format!("{}/{}", owner, repo));
    let url = format!(
        "https://gitlab.com/api/v4/projects/{}/releases",
        encoded_path
    );

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("User-Agent", "checkup/0.1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("GitLab API returned status: {}", response.status());
    }

    let gitlab_releases: Vec<GitLabRelease> = response.json().await?;

    Ok(gitlab_releases
        .into_iter()
        .map(|r| {
            let mut assets: Vec<Asset> = Vec::new();

            for source in r.assets.sources {
                assets.push(Asset {
                    name: format!("{}.{}", r.tag_name, source.format.to_lowercase()),
                    url: source.url,
                    content_type: Some(format!("application/{}", source.format.to_lowercase())),
                    size: 0,
                    download_count: 0,
                });
            }

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

pub async fn handler(
    Path(repo_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, String)> {
    if let Some(pos) = repo_path.rfind('/') {
        let last_segment = &repo_path[pos + 1..];
        if last_segment.starts_with("latest") {
            let repo_part = &repo_path[..pos];
            let repo = parse_gitlab_path(repo_part)?;
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

    let (path_str, want_json) = if repo_path.ends_with("/.json") {
        (repo_path.trim_end_matches("/.json").to_string(), true)
    } else {
        (repo_path.clone(), false)
    };

    let repo = parse_gitlab_path(&path_str)?;
    let cache_key = repo.cache_key();

    if want_json {
        if let Some(json_content) = state
            .cache
            .read_json_raw(&repo.host, &repo.owner, &repo.repo)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        {
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json_content))
                .unwrap());
        }
        return Err((
            StatusCode::NOT_FOUND,
            "No cached data available".to_string(),
        ));
    }

    match get_or_spawn_fetch(&state, &repo).await {
        Ok(FetchResult::Cached) => {
            if let Some(html) = state
                .cache
                .read_html(&repo.host, &repo.owner, &repo.repo)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            {
                return Ok(Html(html).into_response());
            }
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to read cached HTML".to_string(),
            ))
        }
        Ok(FetchResult::Processing) => {
            let html = format_processing_html(&cache_key, "gitlab");
            Ok(Html(html).into_response())
        }
        Ok(FetchResult::Error(err)) => {
            let html = format_error_html(&cache_key, &err, "gitlab");
            Ok(Html(html).into_response())
        }
        Err(e) => Err(e),
    }
}

fn parse_gitlab_path(path: &str) -> Result<RepoPath, (StatusCode, String)> {
    let parts: Vec<&str> = path.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid path format. Use: /gitlab/{owner}/{repo}".to_string(),
        ));
    }
    Ok(RepoPath {
        host: "gitlab.com".to_string(),
        owner: parts[0].to_string(),
        repo: parts[1].to_string(),
    })
}

pub enum FetchResult {
    Cached,
    Processing,
    Error(String),
}

pub async fn get_or_spawn_fetch(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<FetchResult, (StatusCode, String)> {
    if let Ok(Some(cached_at)) = state
        .cache
        .read_timestamp(&repo.host, &repo.owner, &repo.repo)
    {
        if !state.cache.is_expired(cached_at) {
            return Ok(FetchResult::Cached);
        }
    }

    let cache_key = repo.cache_key();

    if let Some(error) = state.failed_repos.get(&cache_key) {
        return Ok(FetchResult::Error(error.clone()));
    }

    if state.pending_repos.contains(&cache_key) {
        return Ok(FetchResult::Processing);
    }

    state.pending_repos.insert(cache_key.clone());

    let state = state.clone();
    let repo = repo.clone();
    tokio::spawn(async move {
        let result = fetch_and_cache(&state, &repo).await;
        state.pending_repos.remove(&cache_key);
        if let Err(e) = result {
            state.failed_repos.insert(cache_key.clone(), e.to_string());
        }
    });

    Ok(FetchResult::Processing)
}

async fn fetch_and_cache(state: &Arc<AppState>, repo: &RepoPath) -> Result<()> {
    let releases = fetch_releases(&state.client, &repo.owner, &repo.repo).await?;
    let cached_at = Utc::now();
    let cache_key = repo.cache_key();

    let cached = CachedReleases {
        releases: releases.clone(),
        cached_at,
        repo_path: cache_key.clone(),
    };

    let html = format_releases_html(&releases, &cache_key, "gitlab", Some(cached_at));

    state
        .cache
        .write_timestamp(&repo.host, &repo.owner, &repo.repo)?;
    state
        .cache
        .write_json(&repo.host, &repo.owner, &repo.repo, &cached)?;
    state
        .cache
        .write_html(&repo.host, &repo.owner, &repo.repo, &html)?;

    state.failed_repos.remove(&cache_key);

    Ok(())
}

async fn fetch_blocking(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<Vec<Release>, (StatusCode, String)> {
    if let Ok(Some(cached_at)) = state
        .cache
        .read_timestamp(&repo.host, &repo.owner, &repo.repo)
    {
        if !state.cache.is_expired(cached_at) {
            if let Some(cached) = state
                .cache
                .read_json::<CachedReleases>(&repo.host, &repo.owner, &repo.repo)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            {
                return Ok(cached.releases);
            }
        }
    }

    let releases = fetch_releases(&state.client, &repo.owner, &repo.repo)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let cached_at = Utc::now();
    let cache_key = repo.cache_key();

    let cached = CachedReleases {
        releases: releases.clone(),
        cached_at,
        repo_path: cache_key.clone(),
    };

    let html = format_releases_html(&releases, &cache_key, "gitlab", Some(cached_at));

    let _ = state
        .cache
        .write_timestamp(&repo.host, &repo.owner, &repo.repo);
    let _ = state
        .cache
        .write_json(&repo.host, &repo.owner, &repo.repo, &cached);
    let _ = state
        .cache
        .write_html(&repo.host, &repo.owner, &repo.repo, &html);

    Ok(releases)
}
