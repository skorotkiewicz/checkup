use super::{Asset, Release};
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

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

pub async fn fetch_releases(
    client: &Client,
    host: &str,
    owner: &str,
    repo: &str,
) -> Result<Vec<Release>> {
    let url = format!("https://{}/api/v1/repos/{}/{}/releases", host, owner, repo);

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("User-Agent", "checkup/0.1.0")
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Forgejo API ({}) returned status: {}",
            host,
            response.status()
        );
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
