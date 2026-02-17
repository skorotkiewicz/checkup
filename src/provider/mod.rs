pub mod cgit;
pub mod forgejo;
pub mod github;
pub mod gitlab;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub download_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub published_at: DateTime<Utc>,
    pub html_url: String,
    pub body: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<Asset>,
    pub source_tarball: Option<String>,
    pub source_zipball: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedReleases {
    pub releases: Vec<Release>,
    pub cached_at: DateTime<Utc>,
    pub repo_path: String,
}
