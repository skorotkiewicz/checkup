# Checkup API Documentation

## Base URL

```
http://localhost:3000
```

## Endpoints

### GET /github/{owner}/{repo}

Fetch releases for a GitHub repository. Returns an HTML page with a list of releases, including download links for all assets.

**URL Parameters**

| Parameter | Description | Example |
|-----------|-------------|---------|
| `owner` | Repository owner/organization | `rust-lang` |
| `repo` | Repository name | `rust` |

**Example Request**

```bash
curl http://localhost:3000/github/rust-lang/rust
```

**Response**

- **Content-Type**: `text/html`
- **Status**: `200 OK`

Returns an HTML page with:
- Latest release assets box at the top
- All releases with download links
- Asset sizes and download counts
- Release notes (collapsible)

**Error Responses**

| Status | Description |
|--------|-------------|
| `400 Bad Request` | Invalid repository path format |
| `500 Internal Server Error` | Failed to fetch releases from API |

---

### GET /gitlab/{owner}/{repo}

Fetch releases for a GitLab repository.

**Example Request**

```bash
curl http://localhost:3000/gitlab/gitlab-org/gitlab
```

**Response**

Same as GitHub endpoint - HTML page with releases.

---

### GET /forgejo/{host}/{owner}/{repo}

Fetch releases from any Forgejo-based instance (Codeberg, self-hosted Forgejo, Gitea).

**URL Parameters**

| Parameter | Description | Example |
|-----------|-------------|---------|
| `host` | Forgejo instance hostname | `codeberg.org` |
| `owner` | Repository owner/organization | `forgejo` |
| `repo` | Repository name | `forgejo` |

**Example Request**

```bash
curl http://localhost:3000/forgejo/codeberg.org/forgejo/forgejo
```

**Response**

Same as GitHub endpoint - HTML page with releases.

---

### GET /cgit/{host}/{repo_path}

Fetch releases from any cgit instance. cgit is a web interface for Git repositories used by many projects including the Linux kernel.

**URL Parameters**

| Parameter | Description | Example |
|-----------|-------------|---------|
| `host` | cgit instance hostname | `git.kernel.org` |
| `repo_path` | Full repository path | `pub/scm/linux/kernel/git/stable/linux.git` |

**Example Request**

```bash
curl http://localhost:3000/cgit/git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git
```

**Response**

Same as GitHub endpoint - HTML page with releases.

**Notes**

- cgit doesn't have a JSON API, so releases are parsed from HTML
- Only tag-based releases with downloadable archives are shown
- Release dates are extracted from the cgit page when available

---

### GET /github/{owner}/{repo}/cache

Get cached releases as JSON. If cache doesn't exist or is expired, fetches fresh data from the API.

**Example Request**

```bash
curl http://localhost:3000/github/rust-lang/rust/cache
```

**Response**

- **Content-Type**: `application/json`
- **Status**: `200 OK`

```json
{
  "releases": [
    {
      "tag_name": "v1.0.0",
      "name": "Release 1.0.0",
      "published_at": "2024-01-15T10:30:00Z",
      "html_url": "https://github.com/owner/repo/releases/tag/v1.0.0",
      "body": "Release notes...",
      "prerelease": false,
      "draft": false,
      "assets": [
        {
          "name": "app-1.0.0.tar.gz",
          "url": "https://github.com/owner/repo/releases/download/v1.0.0/app-1.0.0.tar.gz",
          "content_type": "application/gzip",
          "size": 1234567,
          "download_count": 1234
        }
      ],
      "source_tarball": null,
      "source_zipball": null
    }
  ],
  "cached_at": "2024-01-15T12:00:00Z",
  "repo_path": "github.com/owner/repo"
}
```

---

### GET /github/{owner}/{repo}/latest.{extension}

Redirect to the latest release asset matching the given extension. Perfect for scripts and CI/CD pipelines.

**URL Parameters**

| Parameter | Description | Example |
|-----------|-------------|---------|
| `extension` | File extension or asset suffix | `tar.gz`, `zip`, `AppImage`, `exe` |

**How Extension Matching Works**

The extension is extracted from the asset name:
- `latest.tar.gz` → matches `*.tar.gz` files
- `latest.zip` → matches `*.zip` files
- `latest.AppImage` → matches `*.AppImage` files
- `latest.exe` → matches `*.exe` files

**Example Requests**

```bash
# Download latest tar.gz
curl -L http://localhost:3000/github/owner/repo/latest.tar.gz

# Download latest AppImage
curl -L http://localhost:3000/github/owner/repo/latest.AppImage

# Download latest Windows executable
curl -L http://localhost:3000/github/owner/repo/latest.exe
```

**Response**

- **Status**: `307 Temporary Redirect`
- **Location**: Direct download URL from the release

**Error Responses**

| Status | Description |
|--------|-------------|
| `400 Bad Request` | Invalid repository path format |
| `404 Not Found` | No asset with matching extension found |

---

### GET /health

Health check endpoint.

**Example Request**

```bash
curl http://localhost:3000/health
```

**Response**

- **Status**: `200 OK`
- **Body**: `OK`

---

## Supported Platforms

| Platform | Endpoint | API Version | Notes |
|----------|----------|-------------|-------|
| GitHub | `/github/owner/repo` | REST API v3 | Full support including pre-release and draft flags |
| GitLab | `/gitlab/owner/repo` | REST API v4 | Full support |
| Forgejo | `/forgejo/host/owner/repo` | REST API v1 | Works with Codeberg and any Forgejo instance |
| Gitea | `/forgejo/host/owner/repo` | REST API v1 | Compatible with Forgejo endpoint |
| cgit | `/cgit/host/repo-path` | HTML parsing | Works with any cgit instance (e.g., Linux kernel) |

---

## Cache Behavior

### Cache Expiration

- Default: 24 hours
- Configurable via `--cache-hours` flag
- Expired cache is automatically refreshed on next request

### Cache Location

```
data/cache/
└── repo/
    ├── github.com/
    │   └── {owner}/
    │       └── {repo}/
    │           └── cache-{timestamp}.json
    ├── gitlab.com/
    │   └── {owner}/
    │       └── {repo}/
    │           └── cache-{timestamp}.json
    └── {forgejo-host}/
        └── {owner}/
            └── {repo}/
                └── cache-{timestamp}.json
```

---

## Rate Limits

| Platform | Unauthenticated | With Token |
|----------|-----------------|------------|
| GitHub | 60 requests/hour | 5000 requests/hour |
| GitLab | Varies by instance | Varies by instance |
| Forgejo | Varies by instance | Varies by instance |

**Recommendations for heavy usage:**
- Increase cache duration (`--cache-hours`)
- Run behind a reverse proxy with rate limiting
- Consider API tokens for higher limits

---

## Error Handling

All errors return a plain text response with an appropriate HTTP status code.

**Common Error Responses**

| Status | Description |
|--------|-------------|
| `400 Bad Request` | Invalid URL format or parameters |
| `404 Not Found` | Repository or asset not found |
| `500 Internal Server Error` | API request failed or server error |

---

## Examples

### Download Latest Release in Script

```bash
#!/bin/bash
# Always downloads the latest version
curl -L -o app.tar.gz http://localhost:3000/github/owner/repo/latest.tar.gz
```

### Get Release Info as JSON

```bash
# Get all releases as JSON
curl http://localhost:3000/github/owner/repo/cache | jq '.releases[0]'
```

### Use in CI/CD

```yaml
# GitHub Actions example
- name: Download latest tool
  run: |
    curl -L -o tool.tar.gz http://your-server:3000/github/owner/tool/latest.tar.gz
    tar xzf tool.tar.gz
```
