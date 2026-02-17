use super::{Asset, Release};
use crate::{
    AppState, RepoPath,
    format_html::{format_processing_html, format_releases_html, rename_to_latest},
    provider::CachedReleases,
};
use anyhow::Result;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use scraper::{Html as ScraperHtml, Selector};
use std::sync::Arc;

pub async fn fetch_releases(client: &Client, host: &str, repo_path: &str) -> Result<Vec<Release>> {
    let url = format!("https://{}/{}/refs/tags", host, repo_path);

    let response = client
        .get(&url)
        .header("Accept", "text/html")
        .header("User-Agent", "checkup/0.1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("cgit ({}) returned status: {}", host, response.status());
    }

    let html = response.text().await?;
    let document = ScraperHtml::parse_document(&html);

    let mut releases = Vec::new();

    let row_selector = Selector::parse("table.list tr").unwrap();
    let tag_selector = Selector::parse("td:nth-child(1) a").unwrap();
    let download_selector = Selector::parse("td:nth-child(2) a").unwrap();
    let age_selector = Selector::parse("td:nth-child(4) span, td:nth-child(5) span").unwrap();

    for row in document.select(&row_selector).skip(1) {
        let tag_elem = row.select(&tag_selector).next();
        let download_elem = row.select(&download_selector).next();

        if let (Some(tag_elem), Some(download_elem)) = (tag_elem, download_elem) {
            let tag_name = tag_elem.text().collect::<String>().trim().to_string();
            let download_url = download_elem.value().attr("href").unwrap_or("").to_string();

            if tag_name.is_empty() || download_url.is_empty() {
                continue;
            }

            let asset_name = download_url
                .rsplit('/')
                .next()
                .unwrap_or(&tag_name)
                .to_string();

            let published_at = row
                .select(&age_selector)
                .next()
                .and_then(|el| {
                    el.value()
                        .attr("title")
                        .and_then(|t| DateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S %z").ok())
                        .map(|dt| dt.with_timezone(&Utc))
                })
                .unwrap_or_else(Utc::now);

            let full_download_url = if download_url.starts_with("http") {
                download_url
            } else {
                format!("https://{}{}", host, download_url)
            };

            let html_url = format!("https://{}/{}/tag/?h={}", host, repo_path, tag_name);

            releases.push(Release {
                tag_name: tag_name.clone(),
                name: Some(tag_name.clone()),
                published_at,
                html_url,
                body: None,
                prerelease: false,
                draft: false,
                assets: vec![Asset {
                    name: asset_name,
                    url: full_download_url,
                    content_type: Some("application/gzip".to_string()),
                    size: 0,
                    download_count: 0,
                }],
                source_tarball: None,
                source_zipball: None,
            });
        }
    }

    Ok(releases)
}

pub async fn handler(
    Path(cgit_path): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, String)> {
    if let Some(pos) = cgit_path.rfind('/') {
        let last_segment = &cgit_path[pos + 1..];
        if last_segment.starts_with("latest") {
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

    let (path_str, want_json) = if cgit_path.ends_with("/.json") {
        (cgit_path.trim_end_matches("/.json").to_string(), true)
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
        return Err((StatusCode::NOT_FOUND, "No cached data available".to_string()));
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
            let html = format_processing_html(&cache_key, "cgit");
            Ok(Html(html).into_response())
        }
        Err(e) => Err(e),
    }
}

pub enum FetchResult {
    Cached,
    Processing,
}

pub async fn get_or_spawn_fetch(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<FetchResult, (StatusCode, String)> {
    if let Ok(Some(cached_at)) = state.cache.read_timestamp(&repo.host, &repo.owner, &repo.repo) {
        if !state.cache.is_expired(cached_at) {
            return Ok(FetchResult::Cached);
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
    let releases = fetch_releases(&state.client, &repo.host, &repo.repo).await?;
    let cached_at = Utc::now();
    let cache_key = repo.cache_key();

    let cached = CachedReleases {
        releases: releases.clone(),
        cached_at,
        repo_path: cache_key.clone(),
    };

    let html = format_releases_html(&releases, &cache_key, "cgit", Some(cached_at));

    state.cache.write_timestamp(&repo.host, &repo.owner, &repo.repo)?;
    state.cache.write_json(&repo.host, &repo.owner, &repo.repo, &cached)?;
    state.cache.write_html(&repo.host, &repo.owner, &repo.repo, &html)?;

    Ok(())
}

async fn fetch_blocking(
    state: &Arc<AppState>,
    repo: &RepoPath,
) -> Result<Vec<Release>, (StatusCode, String)> {
    if let Ok(Some(cached_at)) = state.cache.read_timestamp(&repo.host, &repo.owner, &repo.repo) {
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

    let releases = fetch_releases(&state.client, &repo.host, &repo.repo)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let cached_at = Utc::now();
    let cache_key = repo.cache_key();

    let cached = CachedReleases {
        releases: releases.clone(),
        cached_at,
        repo_path: cache_key.clone(),
    };

    let html = format_releases_html(&releases, &cache_key, "cgit", Some(cached_at));

    let _ = state.cache.write_timestamp(&repo.host, &repo.owner, &repo.repo);
    let _ = state.cache.write_json(&repo.host, &repo.owner, &repo.repo, &cached);
    let _ = state.cache.write_html(&repo.host, &repo.owner, &repo.repo, &html);

    Ok(releases)
}
