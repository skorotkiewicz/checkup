# Checkup - Repository Release Tracker

A simple HTTP server for fetching and caching releases from GitHub and GitLab repositories with parallel processing support.

## Installation

### From Source

```bash
git clone https://github.com/skorotkiewicz/checkup
cd checkup
cargo build --release
```

## Usage

```bash
./target/release/checkup --cache data/ --port 3000
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --cache` | `data/cache` | Cache directory |
| `-e, --cache-hours` | `24` | Cache expiration (hours) |
| `-p, --port` | `3000` | Server port |
| `--host` | `127.0.0.1` | Server host |

## Quick Examples

```bash
# GitHub
curl http://localhost:3000/repo/github.com/rust-lang/rust

# GitLab
curl http://localhost:3000/repo/gitlab.com/gitlab-org/gitlab

# Codeberg (Forgejo)
curl http://localhost:3000/forgejo/codeberg.org/forgejo/forgejo

# cgit
curl http://localhost:3000/cgit/git.zx2c4.com/cgit

# Get latest asset
curl -L http://localhost:3000/repo/github.com/owner/repo/latest.tar.gz

# Get cached JSON
curl http://localhost:3000/repo/github.com/owner/repo/cache
```

## Features

- **Multi-platform**: GitHub, GitLab, Forgejo, Gitea, cgit
- **Smart caching**: Configurable expiration
- **Latest downloads**: Consistent URLs for latest releases
- **JSON API**: Programmatic access to cached data
- **Concurrent processing**: Async cache warming

## Documentation

- [API.md](API.md) - Full API documentation

## License

MIT
