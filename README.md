# devx

One command to run your entire dev stack.

devx reads a `devx.toml` from your project root, auto-discovers free ports, runs a reverse proxy for stable URLs, and shows everything in a terminal dashboard.

## Features

- **Auto port discovery** — services bind to free ports, no more conflicts
- **Reverse proxy** — stable URLs (localhost:8000) even when actual ports change
- **Local domains** — route by Host header (arpid.localhost, web.localhost)
- **TUI dashboard** — status, logs, health checks in one terminal
- **Dependency ordering** — services start in the right order
- **Health checks** — polls health endpoints, shows readiness in real-time

## Install

```bash
curl -fsSL https://devx.machines.works/install.sh | sh
```

Or build from source:

```bash
cargo install --path .
```

Then run in any project with a `devx.toml`:

```bash
devx up
```

## Configuration

Create a `devx.toml` in your project root:

```toml
[project]
name = "myapp"

[services.api]
cmd = "go run ./cmd/api"
port = 8000
domain = "api.localhost"
health = "http://localhost:${port}/health"

[services.api.env]
HTTP_PORT = "${port}"
DATABASE_URL = "postgres://localhost:5432/mydb"

[services.web]
cmd = "npm run dev -- --port ${port}"
port = 3000
domain = "web.localhost"
depends_on = ["api"]

[services.web.env]
VITE_API_URL = "http://localhost:${proxy:api}"
```

## Variable Interpolation

- `${port}` — the actual allocated port for this service
- `${proxy:NAME}` — the proxy port for another service

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `q` | Quit all services |
| `r` | Restart focused service |
| `f` | Filter logs to focused service |
| `Tab` | Cycle through services |
| `↑↓` | Scroll logs |
| `Esc` | Clear log filter |

## Requirements

- Rust 1.85+ (edition 2024)
- Docker (for infra pre-flight checks)

## Deploy

The landing page at [devx.machines.works](https://devx.machines.works) is a static site in `site/`. Deploy manually:

```bash
vercel deploy --prod site/
```

## Release

Tag a version to trigger the release workflow:

```bash
git tag v0.0.2-alpha
git push origin v0.0.2-alpha
```

Builds binaries for darwin-arm64, linux-x86_64, and linux-arm64.

## Status

Alpha. Built by [Machines Works](https://machines.works).

## License

TBD
