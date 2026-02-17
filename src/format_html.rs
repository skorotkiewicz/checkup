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

/// Extract extension from asset name, removing version numbers
/// e.g., "v0.1.0.tar.gz" -> "tar.gz", "grab-linux-x86_64" -> "grab-linux-x86_64"
/// "package-1.0.0.zip" -> "zip", "app-v2.0.0.AppImage" -> "AppImage"
pub fn extract_extension(name: &str) -> String {
    // Common double extensions
    let double_extensions = [".tar.gz", ".tar.bz2", ".tar.xz"];

    for ext in double_extensions {
        if name.ends_with(ext) {
            return ext[1..].to_string(); // Remove leading dot
        }
    }

    // Single extension
    if let Some(pos) = name.rfind('.') {
        name[pos + 1..].to_string()
    } else {
        // No extension, use the whole name
        name.to_string()
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
                    let icon = if a.name.ends_with(".exe") || a.name.ends_with(".msi") {
                        "🪟"
                    } else if a.name.ends_with(".deb") || a.name.ends_with(".rpm") {
                        "🐧"
                    } else if a.name.ends_with(".dmg") || a.name.contains("darwin") || a.name.contains("macos") {
                        "🍎"
                    } else if a.name.ends_with(".AppImage") {
                        "📦"
                    } else if a.name.ends_with(".tar.gz") || a.name.ends_with(".tgz") {
                        "🗜️"
                    } else if a.name.ends_with(".zip") {
                        "🗜️"
                    } else if a.name.ends_with(".jar") {
                        "☕"
                    } else if a.name.contains("source") || a.name.contains("src") {
                        "📄"
                    } else {
                        "📎"
                    };
                    let extension = extract_extension(&a.name);
                    let path_for_url = if route_prefix == "github" || route_prefix == "gitlab" {
                        // Strip the host part (e.g., "github.com/owner/repo" -> "owner/repo")
                        repo_path.splitn(2, '/').nth(1).unwrap_or(repo_path).to_string()
                    } else if route_prefix == "cgit" {
                        repo_path.replace("//", "/")
                    } else {
                        repo_path.to_string()
                    };

                    let latest_url = format!("/{}/{}/latest.{}", route_prefix, path_for_url, extension);
                    format!(
                        r#"<div style="padding: 10px; margin: 6px 0; background: #fff; border: 1px solid #28a745; border-radius: 6px; display: flex; justify-content: space-between; align-items: center;">
                            <div>{} <a href="{}" style="font-weight: 600; color: #0366d6; font-size: 1.05em;">{}</a>{}</div>
                            <div>
                                <a href="{}" style="background: #28a745; color: white; padding: 6px 12px; border-radius: 4px; text-decoration: none; font-weight: 500;">⬇ Download</a>
                            </div>
                        </div>"#,
                        icon, a.url, a.name, size_info, latest_url
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let version_name = latest.name.as_ref().unwrap_or(&latest.tag_name);
            format!(
                r#"<div style="margin-bottom: 30px; padding: 20px; background: linear-gradient(135deg, #f0fff4 0%, #e6ffed 100%); border: 2px solid #28a745; border-radius: 12px;">
                    <h2 style="margin: 0 0 5px 0; color: #28a745;">⭐ Latest Release: {}</h2>
                    <p style="margin: 0 0 15px 0; color: #666; font-size: 0.9em;">Published: {} • {} files</p>
                    <div>
                        {}
                    </div>
                </div>"#,
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
                r#" <span style="background: #28a745; color: white; padding: 2px 8px; border-radius: 3px; font-size: 0.8em; font-weight: bold;">⭐ Latest</span>"#
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
                            format!(" <span style='color: #28a745;'>⬇ {}</span>", a.download_count)
                        } else {
                            String::new()
                        };
                        let icon = if a.name.ends_with(".exe") || a.name.ends_with(".msi") {
                            "🪟"
                        } else if a.name.ends_with(".deb") || a.name.ends_with(".rpm") {
                            "🐧"
                        } else if a.name.ends_with(".dmg") || a.name.contains("darwin") || a.name.contains("macos") {
                            "🍎"
                        } else if a.name.ends_with(".AppImage") {
                            "📦"
                        } else if a.name.ends_with(".tar.gz") || a.name.ends_with(".tgz") {
                            "🗜️"
                        } else if a.name.ends_with(".zip") {
                            "🗜️"
                        } else if a.name.ends_with(".jar") {
                            "☕"
                        } else if a.name.contains("source") || a.name.contains("src") {
                            "📄"
                        } else {
                            "📎"
                        };
                        format!(
                            r#"<div style="padding: 8px; margin: 4px 0; background: #fff; border: 1px solid #e1e4e8; border-radius: 6px;">
                                {} <a href="{}" style="font-weight: 500; color: #0366d6;">{}</a>{}{}
                            </div>"#,
                            icon, a.url, a.name, size_info, download_info
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                format!(
                    r#"<div style="margin: 15px 0;">
                        <strong style="font-size: 1.1em;">📦 Downloads ({} files):</strong>
                        <div style="margin-top: 8px;">
                            {}
                        </div>
                    </div>"#,
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
                            <summary style="cursor: pointer; color: #0366d6; font-weight: 500;">📝 Show release notes</summary>
                            <div style="margin-top: 10px; padding: 10px; background: #f6f8fa; border-radius: 6px; white-space: pre-wrap; font-size: 0.9em;">{}</div>
                        </details>"#,
                        body_preview
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
                    <small style="color: #586069;">📅 Published: {}</small>
                    {}
                    {}
                </li>"#,
                r.html_url,
                name,
                latest_badge,
                prerelease_badge,
                draft_badge,
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
    <h2 style="margin-top: 30px; color: #333;">📋 All Releases</h2>
    <ul>
        {}
    </ul>
</body>
</html>"#,
        repo_path, repo_path, cache_info, latest_assets_box, releases_html
    )
}
