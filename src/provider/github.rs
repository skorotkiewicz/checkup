use super::{Asset, Release};
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

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
