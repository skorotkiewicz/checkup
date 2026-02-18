use crate::icons;
use crate::provider::Release;
use chrono::{DateTime, Utc};

fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

/// Split a filename into stem and known compound extension.
/// e.g., "bat-v0.26.1-x86_64.tar.gz" -> ("bat-v0.26.1-x86_64", ".tar.gz")
pub fn split_stem_ext(filename: &str) -> (&str, &str) {
    const KNOWN_EXTS: &[&str] = &[
        ".tar.gz.sha256",
        ".tar.xz.sha256",
        ".tar.bz2.sha256",
        ".tar.zst.sha256",
        ".zip.sha256",
        ".xz.sha256",
        ".gz.sha256",
        ".bz2.sha256",
        ".xz.asc",
        ".tar.gz",
        ".tar.xz",
        ".tar.bz2",
        ".tar.zst",
        ".xz",
        ".gz",
        ".bz2",
        ".zst",
        ".zip",
        ".sha256",
        ".sha512",
        ".exe",
        ".msi",
        ".deb",
        ".rpm",
    ];

    for ext in KNOWN_EXTS {
        if let Some(stem) = filename.strip_suffix(ext) {
            return (stem, ext);
        }
    }

    // Fallback: split on last dot, but only if suffix looks like a real file extension
    // (not a version fragment like ".2-linux-amd64" or ".2" in "forgejo-14.0.2-linux-amd64")
    if let Some(pos) = filename.rfind('.') {
        let suffix = &filename[pos + 1..];
        if !suffix.is_empty()
            && !suffix.contains('-')
            && !suffix.chars().all(|c| c.is_ascii_digit())
        {
            return (&filename[..pos], &filename[pos..]);
        }
    }
    (filename, "")
}

/// Rename an asset filename to a "latest" variant, stripping app name and version.
/// e.g., "bat-v0.26.1-x86_64-linux.tar.gz" -> "latest-x86_64-linux.tar.gz"
///       "forgejo-14.0.2-linux-amd64.xz"    -> "latest-linux-amd64.xz"
///       "bat_0.26.1_amd64.deb"             -> "latest_amd64.deb"
///       "linux-6.19.2.tar.gz"              -> "latest.tar.gz"
pub fn rename_to_latest(filename: &str) -> String {
    let (stem, ext) = split_stem_ext(filename);

    // Detect separator: use '_' if stem contains underscores but no dashes
    let sep = if stem.contains('-') { '-' } else { '_' };
    let parts: Vec<&str> = stem.split(sep).collect();

    // Find the version segment (optional "v" prefix + at least major.minor numeric)
    let version_idx = parts.iter().position(|p| {
        let p = p.strip_prefix('v').unwrap_or(p);
        let mut iter = p.split('.');
        iter.next().map_or(false, |s| s.parse::<u64>().is_ok())
            && iter.next().map_or(false, |s| s.parse::<u64>().is_ok())
    });

    let suffix_parts = match version_idx {
        // Drop everything up to and including the version
        Some(idx) => &parts[idx + 1..],
        // No version found: drop just the app name (first segment)
        None => &parts[1..],
    };

    if suffix_parts.is_empty() {
        format!("latest{}", ext)
    } else {
        format!(
            "latest{}{}{}",
            sep,
            suffix_parts.join(&sep.to_string()),
            ext
        )
    }
}

pub fn format_releases_html(
    releases: &[Release],
    repo_path: &str,
    route_prefix: &str,
    cached_at: Option<DateTime<Utc>>,
) -> String {
    let cache_info = cached_at
        .map(|t| {
            format!(
                "<p><em>Cached at: {}</em></p>",
                t.format("%Y-%m-%d %H:%M:%S UTC")
            )
        })
        .unwrap_or_default();

    // Latest assets box at the top
    let latest_assets_box = if let Some(latest) = releases.first() {
        if !latest.assets.is_empty() {
            let assets_list = latest
                .assets
                .iter()
                .map(|a| {
                    let size_info = if a.size > 0 {
                        format!(" <span style='color: #666;'>({})</span>", format_size(a.size))
                    } else {
                        String::new()
                    };
                    let icon = icons::get_file_icon(&a.name, 18);
                    let latest_name = rename_to_latest(&a.name);
                    let path_for_url = if route_prefix == "github" || route_prefix == "gitlab" {
                        // Strip the host part (e.g., "github.com/owner/repo" -> "owner/repo")
                        repo_path.splitn(2, '/').nth(1).unwrap_or(repo_path).to_string()
                    } else if route_prefix == "cgit" {
                        repo_path.replace("//", "/")
                    } else {
                        repo_path.to_string()
                    };

                    let latest_url = format!("/{}/{}/{}", route_prefix, path_for_url, latest_name);
                    format!(
                        r#"<div style="padding: 10px; margin: 6px 0; color: #777; background: #fff; border: 1px solid #28a745; border-radius: 6px; display: flex; justify-content: space-between; align-items: center;">
                            <div style="display: flex; align-items: center; gap: 6px;"><span style="display: flex; flex-shrink: 0;">{}</span> <a href="{}" style="font-weight: 600; color: #0366d6; font-size: 1.05em;">{}</a>{}</div>
                            <div>
                                <a href="{}" style="background: #28a745; color: white; padding: 6px 12px; border-radius: 4px; text-decoration: none; font-weight: 500; display: inline-flex; align-items: center; gap: 4px;">{} Download</a>
                            </div>
                        </div>"#,
                        icon, a.url, a.name, size_info, latest_url, icons::DOWNLOAD(16)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let version_name = latest.name.as_ref().unwrap_or(&latest.tag_name);
            format!(
                r#"<div style="margin-bottom: 30px; padding: 20px; background: linear-gradient(135deg, #f0fff4 0%, #e6ffed 100%); border: 2px solid #28a745; border-radius: 12px;">
                    <h2 style="margin: 0 0 5px 0; color: #28a745; display: flex; align-items: center; gap: 6px; font-size: 1.2em;">{} Latest Release: {}</h2>
                    <p style="margin: 0 0 15px 0; color: #666; font-size: 0.9em;">Published: {} • {} files</p>
                    <div>
                        {}
                    </div>
                </div>"#,
                icons::STAR(16),
                version_name,
                latest.published_at.format("%Y-%m-%d %H:%M:%S UTC"),
                latest.assets.len(),
                assets_list
            )
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let releases_html = releases
        .iter()
        .enumerate()
        .map(|(idx, r)| {
            let latest_badge = if idx == 0 {
                &format!(r#" <span style="background: #28a745; color: white; padding: 2px 8px; border-radius: 3px; font-size: 0.8em; font-weight: bold; display: inline-flex; align-items: center; gap: 4px;">{} Latest</span>"#, icons::STAR(12))
            } else {
                ""
            };
            let prerelease_badge = if r.prerelease {
                r#" <span style="background: #f0ad4e; padding: 2px 6px; border-radius: 3px; font-size: 0.8em;">Pre-release</span>"#
            } else {
                ""
            };
            let draft_badge = if r.draft {
                r#" <span style="background: #777; padding: 2px 6px; border-radius: 3px; font-size: 0.8em;">Draft</span>"#
            } else {
                ""
            };
            let name = r.name.as_ref().unwrap_or(&r.tag_name);

            // Format assets - show prominently at the top
            let assets_html = if !r.assets.is_empty() {
                let assets_list = r
                    .assets
                    .iter()
                    .map(|a| {
                        let size_info = if a.size > 0 {
                            format!(" <span style='color: #666;'>({})</span>", format_size(a.size))
                        } else {
                            String::new()
                        };
                        let download_info = if a.download_count > 0 {
                            format!(" <span style='color: #28a745; display: inline-flex; align-items: center; gap: 2px;'>{} {}</span>", icons::DOWNLOAD(16), a.download_count)
                        } else {
                            String::new()
                        };
                        let icon = icons::get_file_icon(&a.name, 16);
                        format!(
                            r#"<div style="padding: 8px; color: #777; margin: 4px 0; background: #fff; border: 1px solid #e1e4e8; border-radius: 6px; display: flex; align-items: center; gap: 6px;">
                                <span style="display: flex; flex-shrink: 0;">{}</span>
                                <a href="{}" style="font-weight: 500; color: #0366d6;">{}</a>{}{}
                            </div>"#,
                            icon, a.url, a.name, size_info, download_info
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                format!(
                    r#"<div style="margin: 15px 0;">
                        <strong style="font-size: 1.1em; display: inline-flex; align-items: center; gap: 4px;">{} Downloads ({} files):</strong>
                        <div style="margin-top: 8px;">
                            {}
                        </div>
                    </div>"#,
                    icons::PACKAGE(16),
                    r.assets.len(),
                    assets_list
                )
            } else {
                String::new()
            };

            // Body text - collapsible/hidden by default
            let body_html = if let Some(body) = &r.body {
                if !body.is_empty() {
                    let body_preview = body.lines().take(3).collect::<Vec<_>>().join("<br>");
                    format!(
                        r#"<details style="margin-top: 10px;">
                            <summary style="cursor: pointer; color: #777; font-weight: 500; display: inline-flex; align-items: center; gap: 4px;">{} Show release notes</summary>
                            <div style="margin-top: 10px; padding: 10px; background: #f6f8fa; border-radius: 6px; white-space: pre-wrap; font-size: 0.9em;">{}</div>
                        </details>"#,
                        icons::NOTE(16), body_preview
                    )
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            format!(
                r#"<li style="margin-bottom: 25px; padding: 20px; background: #fff; border: 1px solid #e1e4e8; border-radius: 8px; list-style: none;">
                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 10px;">
                        <strong style="font-size: 1.3em;"><a href="{}" target="_blank" style="color: #0366d6;">{}</a></strong>{}{}{}
                    </div>
                    <small style="color: #586069; display: inline-flex; align-items: center; gap: 4px;">{} Published: {}</small>
                    {}
                    {}
                </li>"#,
                r.html_url,
                name,
                latest_badge,
                prerelease_badge,
                draft_badge,
                icons::CALENDAR(16),
                r.published_at.format("%Y-%m-%d %H:%M:%S UTC"),
                assets_html,
                body_html
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Releases - {}</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; }}
        h1 {{ color: #333; }}
        h2 {{ margin: 0; }}
        ul {{ list-style-type: none; padding: 0; }}
        li {{ border-bottom: 1px solid #eee; padding: 15px 0; }}
        a {{ color: #0366d6; text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
        small {{ color: #666; }}
        p {{ color: #444; margin: 5px 0; }}
    </style>
</head>
<body>
    <h1>Releases for {}</h1>
    {}
    {}
    <h2 style="margin-top: 30px; color: #333; display: flex; align-items: center; gap: 6px;">{} All Releases</h2>
    <ul>
        {}
    </ul>
</body>
</html>"#,
        repo_path,
        repo_path,
        cache_info,
        latest_assets_box,
        icons::NOTE(18),
        releases_html
    )
}

pub fn format_processing_html(repo_path: &str, _route_prefix: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Processing - {}</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; }}
        .container {{ text-align: center; padding: 60px 20px; }}
        .spinner {{
            width: 50px; height: 50px;
            border: 4px solid #e1e4e8;
            border-top: 4px solid #0366d6;
            border-radius: 50%;
            animation: spin 1s linear infinite;
            margin: 0 auto 20px;
        }}
        @keyframes spin {{ 0% {{ transform: rotate(0deg); }} 100% {{ transform: rotate(360deg); }} }}
        h1 {{ color: #333; margin-bottom: 10px; }}
        p {{ color: #666; font-size: 1.1em; }}
        .refresh-btn {{
            display: inline-block;
            margin-top: 20px;
            padding: 12px 24px;
            background: #0366d6;
            color: white;
            border-radius: 6px;
            text-decoration: none;
            font-weight: 500;
        }}
        .refresh-btn:hover {{ background: #0257b3; }}
        code {{ background: #f6f8fa; padding: 2px 6px; border-radius: 4px; }}
    </style>
    <meta http-equiv="refresh" content="5">
</head>
<body>
    <div class="container">
        <div class="spinner"></div>
        <h1>Repository Processing</h1>
        <p>The repository <code>{}</code> is being fetched for the first time.</p>
        <p>Please wait a few seconds... This page will auto-refresh.</p>
       <a onClick="window.location.reload();" class="refresh-btn">Refresh Now</a>
    </div>
</body>
</html>"#,
        repo_path, repo_path
    )
}

pub fn format_error_html(repo_path: &str, error_message: &str, _route_prefix: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Error - {}</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; }}
        .container {{ text-align: center; padding: 60px 20px; }}
        .error-icon {{ font-size: 64px; margin-bottom: 20px; }}
        h1 {{ color: #d73a49; margin-bottom: 10px; }}
        p {{ color: #666; font-size: 1.1em; }}
        .error-box {{
            background: #fff5f5;
            border: 1px solid #f5c6cb;
            border-radius: 6px;
            padding: 15px;
            margin: 20px auto;
            max-width: 600px;
            text-align: left;
            color: #721c24;
            font-family: monospace;
            font-size: 0.9em;
        }}
        code {{ background: #f6f8fa; padding: 2px 6px; border-radius: 4px; }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-icon">&#10060;</div>
        <h1>Failed to Fetch Repository</h1>
        <p>The repository <code>{}</code> could not be fetched.</p>
        <div class="error-box">{}</div>
        <p>Check if the repository exists and is accessible.</p>
    </div>
</body>
</html>"#,
        repo_path, repo_path, error_message
    )
}
