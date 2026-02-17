use super::{Asset, Release};
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;

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
