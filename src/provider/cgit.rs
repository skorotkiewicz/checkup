use super::{Asset, Release};
use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use scraper::{Html as ScraperHtml, Selector};
use std::sync::Arc;

use crate::{
    format_html::{extract_extension, format_releases_html},
    provider::CachedReleases,
    AppState, RepoPath,
};

pub async fn fetch_releases(client: &Client, host: &str, repo_path: &str) -> Result<Vec<Release>> {
    // cgit repos have paths like: /pub/scm/linux/kernel/git/stable/linux.git
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

    // Parse cgit tags table
    let row_selector = Selector::parse("table.list tr").unwrap();
    let tag_selector = Selector::parse("td:nth-child(1) a").unwrap();
    let download_selector = Selector::parse("td:nth-child(2) a").unwrap();
    let age_selector = Selector::parse("td:nth-child(4) span, td:nth-child(5) span").unwrap();

    for row in document.select(&row_selector).skip(1) {
        // Skip header row
        let tag_elem = row.select(&tag_selector).next();
        let download_elem = row.select(&download_selector).next();

        if let (Some(tag_elem), Some(download_elem)) = (tag_elem, download_elem) {
            let tag_name = tag_elem.text().collect::<String>().trim().to_string();
            let download_url = download_elem.value().attr("href").unwrap_or("").to_string();

            if tag_name.is_empty() || download_url.is_empty() {
                continue;
            }

            // Extract asset name from download URL
            let asset_name = download_url
                .rsplit('/')
                .next()
                .unwrap_or(&tag_name)
                .to_string();

            // Try to parse age for published_at
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
    // Check if requesting latest asset redirect
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
    let (path_str, want_cache) = if cgit_path.ends_with("/cache") {
        (cgit_path.trim_end_matches("/cache").to_string(), true)
    } else {
        (cgit_path.clone(), false)
    };

    // Parse: host/repo_path
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

    let html = format_releases_html(&releases, &repo.cache_key(), "cgit", cached_at);
    Ok(Html(html).into_response())
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

    let releases = fetch_releases(&state.client, &repo.host, &repo.repo)
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
