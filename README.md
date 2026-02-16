# Checkup - Repository Release Tracker

A simple HTTP server for fetching and caching releases from GitHub and GitLab repositories with parallel processing support.

## Features

- **Multi-platform Support**: Fetch releases from GitHub, GitLab, and any Forgejo instance
- **Smart Caching**: Local file-based cache with configurable expiration
- **Parallel Processing**: Multi-core cache warming using Rayon
- **Simple HTTP API**: RESTful endpoints for easy integration
- **HTML Output**: Human-readable release listings with download links

## Installation

### From Source

```bash
git clone <repository-url>
cd checkup
cargo build --release
```

The binary will be available at `target/release/checkup`.

## Usage

### Starting the Server

```bash
# Basic usage with defaults
checkup

# Custom cache directory and expiration
checkup --cache /path/to/cache --cache-hours 12

# Custom host and port
checkup --host 0.0.0.0 --port 8080
```

### Command Line Options

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--cache` | `-c` | `data/cache` | Cache directory path |
| `--cache-hours` | `-e` | `24` | Cache expiration time in hours |
| `--port` | `-p` | `3000` | Server port |
| `--host` | | `127.0.0.1` | Server host |
| `--help` | `-h` | | Show help message |

## API Endpoints

### GET /repo/{host}/{owner}/{repo}

Fetch releases for a GitHub or GitLab repository.

**Example:**
```
GET http://localhost:3000/repo/github.com/rust-lang/rust
GET http://localhost:3000/repo/gitlab.com/gitlab-org/gitlab
```

**Response:** HTML page with list of releases

### GET /forgejo/{host}/{owner}/{repo}

Fetch releases from any Forgejo-based instance (Codeberg, self-hosted Forgejo, etc.).

**Example:**
```
GET http://localhost:3000/forgejo/codeberg.org/forgejo/forgejo
GET http://localhost:3000/forgejo/git.nextcloud.com/nextcloud/server
```

**Response:** HTML page with list of releases

### GET /health

Health check endpoint.

**Response:** `OK` with status 200

### GET /cache/*

Browse cached files directly.

## Cache Structure

```
data/cache/
└── repo/
    ├── github.com/
    │   └── {owner}/
    │       └── {repo}/
    │           └── cache-{timestamp}.json
    └── gitlab.com/
        └── {owner}/
            └── {repo}/
                └── cache-{timestamp}.json
```

### Cache File Format

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
      "draft": false
    }
  ],
  "cached_at": "2024-01-15T12:00:00Z",
  "repo_path": "github.com/owner/repo"
}
```

## How It Works

1. **Request Flow:**
   - Client requests `/repo/{host}/{owner}/{repo}`
   - Server checks if valid cache exists
   - If cache is valid (not expired), returns cached data
   - If no cache or expired, fetches from API and caches result

2. **Cache Warming:**
   - On startup, scans existing cache directories
   - Uses Rayon for parallel processing across CPU cores
   - Refreshes expired caches in parallel

3. **Parallel Processing:**
   - Cache warming uses `rayon` for multi-core parallelism
   - Multiple repositories refreshed simultaneously
   - Improves startup time for large cache directories

## Examples

### GitHub Repository

```bash
curl http://localhost:3000/repo/github.com/rust-lang/rust
```

### GitLab Repository

```bash
curl http://localhost:3000/repo/gitlab.com/gitlab-org/gitlab
```

### Codeberg (Forgejo) Repository

```bash
curl http://localhost:3000/forgejo/codeberg.org/forgejo/forgejo
```

### Self-hosted Forgejo Instance

```bash
curl http://localhost:3000/forgejo/git.example.com/owner/repo
```

### Check Health

```bash
curl http://localhost:3000/health
```

## Supported Platforms

| Platform | API Used | Endpoint | Notes |
|----------|----------|----------|-------|
| GitHub | REST API v3 | `/repo/github.com/...` | Full support including pre-release and draft flags |
| GitLab | REST API v4 | `/repo/gitlab.com/...` | Full support |
| Forgejo | REST API v1 | `/forgejo/{host}/...` | Works with Codeberg and any Forgejo instance |
| Gitea | REST API v1 | `/forgejo/{host}/...` | Compatible with Forgejo endpoint |

## Rate Limits

- **GitHub API**: 60 requests/hour unauthenticated, 5000/hour with token
- **GitLab API**: Varies by instance

For heavy usage, consider:
- Using GitHub/GitLab API tokens
- Increasing cache duration
- Running behind a reverse proxy with rate limiting

## Development

### Build

```bash
cargo build
```

### Run in Development

```bash
cargo run
```

### Run Tests

```bash
cargo test
```

## Dependencies

- `axum` - Web framework
- `tokio` - Async runtime
- `reqwest` - HTTP client
- `serde` - Serialization
- `chrono` - Date/time handling
- `rayon` - Parallel processing
- `clap` - CLI argument parsing
- `anyhow` / `thiserror` - Error handling

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR on the repository.