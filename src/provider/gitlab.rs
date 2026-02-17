use super::{Asset, Release};
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

use crate::{
    AppState, RepoPath,
    format_html::{extract_extension, format_releases_html},
    provider::CachedReleases,
};

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

pub async fn handler(
    Path(repo_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, String)> {
    // Check if requesting latest asset redirect
    if let Some(pos) = repo_path.rfind("/latest.") {
        let extension = &repo_path[pos + 8..];
        let repo_part = &repo_path[..pos];
        let repo = parse_gitlab_path(repo_part)?;
        let releases = get_or_fetch(&state, &repo).await?;

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

    // Check if requesting raw cache
    let (path_str, want_cache) = if repo_path.ends_with("/cache") {
        (repo_path.trim_end_matches("/cache").to_string(), true)
    } else {
        (repo_path.clone(), false)
    };

    let repo = parse_gitlab_path(&path_str)?;

    let cached_at = state
        .cache
        .read_cache::<CachedReleases>(&repo.host, &repo.owner, &repo.repo)
        .ok()
        .flatten()
        .map(|c| c.cached_at);
    let releases = get_or_fetch(&state, &repo).await?;

    if want_cache {
        let cached = CachedReleases {
            releases,
            cached_at: cached_at.unwrap_or_else(Utc::now),
            repo_path: repo.cache_key(),
        };
        return Ok(Json(cached).into_response());
    }

    let html = format_releases_html(&releases, &repo.cache_key(), "gitlab", cached_at);
    Ok(Html(html).into_response())
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

async fn get_or_fetch(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<Vec<Release>, (StatusCode, String)> {
    // Check cache first
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
