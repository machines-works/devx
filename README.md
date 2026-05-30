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

## Named instances & worktrees

By default devx keys its control socket, PID file, and log file on `[project].name`
(`/tmp/devx-{name}.{sock,pid,log}`). You can run several independent instances of the
same `devx.toml` by overriding that **instance id**:

```bash
devx up -d --project shimizu     # /tmp/devx-shimizu.sock
devx up -d --project sharpi      # /tmp/devx-sharpi.sock — coexists with the above
```

The id is resolved once per command and honored by `up`, `down`, `restart`, `status`,
and `logs`. Resolution precedence (first match wins):

1. `--project`/`-p <NAME>` flag
2. `DEVX_PROJECT=<NAME>` environment variable (non-empty)
3. **Inside a linked git worktree**, devx auto-derives `{name}-{worktree-dir}` so each
   worktree gets its own instance automatically — no flag needed.
4. Otherwise the bare `[project].name` (unchanged default behavior).

Use the same override on follow-up commands; a bare `devx status`/`devx down` resolves
to the default id and will not see an instance you started under `--project`:

```bash
devx up -d --project sharpi
devx status --project sharpi     # or: DEVX_PROJECT=sharpi devx status
devx down   --project sharpi
```

**Transition caveat:** a daemon started by a pre-feature build from inside a linked
worktree used the bare `[project].name`. After upgrading, that worktree resolves to the
new auto-suffixed id, so the old daemon becomes invisible to bare commands. Stop it once
via the old name (`devx down --project <oldname>`) before relying on the new id.

### Deterministic per-worktree ports

A linked worktree doesn't just get its own instance id — it also gets its own **stable
set of preferred ports**, so each checkout has a fixed, bookmarkable
`http://localhost:<port>` that survives restarts.

Without this, two worktrees both want the same preferred port (e.g. `3001`); the first
wins and the second's reverse proxy falls back to a *random* free port, so its URL
changes on every boot. devx fixes that by adding a deterministic offset to every
service's preferred port:

- **Primary checkout (or any non-git dir):** offset `0` — preferred ports are used
  unchanged, byte-for-byte. Existing setups behave exactly as before.
- **Linked git worktree:** a stable offset derived from the worktree directory name,
  in `{10, 20, …, 160}` (stride 10, 16 slots). So a service on base port `3001` lands
  on `3011`, `3021`, … depending on the worktree — and the *same* worktree always gets
  the *same* ports.

```
                   commerce   bff    frontend
primary checkout   9001       3101   3001        # offset 0
worktree "wt-devx" 9001+N     3101+N 3001+N      # stable N ∈ {10..160}
```

The mapping is **stateless** — no registry file to allocate or clean up; the offset is
hashed from the path each run. On the rare hash collision (two worktrees → same slot),
the preferred ports match and the proxy's normal fall-back-to-free behavior keeps both
bootable (one just loses its stable URL for that session). The offset is logged at
startup (`[devx] per-worktree port offset +N`) and reflected in the `PROXY` column of
`devx status`.

This keeps every instance on plain `localhost` over HTTP — no certificates, no DNS
vhosts, no gateway — which is what makes concurrent worktrees Clerk-friendly.

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
