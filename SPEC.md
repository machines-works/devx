# devx — Full Specification

One command to run your entire dev stack.

## CLI

```
devx <command> [options]
```

### Global options

| Flag | Short | Description |
|------|-------|-------------|
| `--project <NAME>` | `-p` | Override the effective project instance id (the key for the socket/PID/log triple and the singleton guard). Honored by `up`, `down`, `restart`, `status`, and `logs`. `check`/`trust`/`init` ignore it (they touch no socket/PID). Default: `[project].name`, or `{name}-{worktree-dir}` inside a linked git worktree. |

The `DEVX_PROJECT` environment variable does the same thing as `--project`, at lower precedence: a non-empty `DEVX_PROJECT` is used only when `--project` is absent. See [Project Identity / Instance Resolution](#project-identity--instance-resolution) for the full precedence chain.

### `devx up [--daemon] [SERVICES...]`

Start all services (or a filtered subset).

| Flag | Short | Description |
|------|-------|-------------|
| `--daemon` | `-d` | Run in background without TUI |

**TUI mode** (default): Interactive terminal dashboard with service status, logs, health indicators, keyboard shortcuts.

**Daemon mode** (`-d`): Fork to background. Parent prints PID and log file path, then exits. Child runs orchestrator headless, writing events to a log file.

Prevents multiple instances **per resolved id** — if a daemon is already running for the effective instance id, exits with an error. The singleton guard is scoped to the resolved id, not the bare `[project].name`. This means two named instances launched from the same checkout (`devx up -d --project shimizu` and `devx up -d --project sharpi`) coexist, each with its own socket/PID/log.

**Startup sequence:**
1. Find `devx.toml` by walking up from cwd
2. Validate config (services exist, cmds non-empty, deps valid)
3. Start infrastructure if `[infra]` defined (`docker compose up -d`)
4. Resolve dependency order (topological sort into waves)
5. Allocate ports (actual + proxy where configured)
6. Generate TLS leaf certificate (signed by local CA)
7. Start service proxies (per-service reverse proxy)
8. Start vhost proxy (domain-based routing) if any service has `domain`
9. Spawn processes in dependency wave order
10. Start file watchers (per-service directories + devx.toml)
11. Start control socket at `/tmp/devx-{id}.sock` (where `{id}` is the [resolved instance id](#project-identity--instance-resolution), not necessarily the bare project name)

### `devx down`

Stop a running devx instance via the control socket. Cleans up PID file. Prints log file location if it exists.

### `devx restart <service>`

Restart a specific service. Allocates a new port, updates proxy atomically.

### `devx status`

When daemon is running: queries live state via control socket. Displays table:

```
SERVICE              STATE        PORT     PROXY        UPTIME
------------------------------------------------------------
api                  healthy      34521    8000         5m23s
web                  starting     41002    3000         2s
```

When not running: prints "devx is not running".

### `devx logs [-f] [-s SERVICE] [-n LINES]`

Tail logs from a running (or stopped) daemon.

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--follow` | `-f` | false | Follow output (like `tail -f`) |
| `--service` | `-s` | all | Filter by service name |
| `--lines` | `-n` | 50 | Number of lines to show |

Follow mode polls every 200ms, exits when daemon stops.

### `devx check`

Validate `devx.toml` and check infrastructure. Reports service count. If `[infra]` is defined, verifies all Docker Compose containers are running.

### `devx trust`

Trust the devx local CA in the system certificate store.
- **macOS**: `sudo security add-trusted-cert` into System keychain
- **Linux**: Copies to `/usr/local/share/ca-certificates/` and runs `update-ca-certificates`

---

## Configuration

File: `devx.toml` in project root.

### `[project]`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | String | yes | Default project identifier. The effective instance id used for socket/PID/log keys is `name`, overridable by `--project`/`DEVX_PROJECT`, and auto-suffixed with the worktree directory name inside a linked git worktree. See [Project Identity / Instance Resolution](#project-identity--instance-resolution). |

### `[infra]`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `compose` | String | yes | Path to docker-compose file (relative to project root) |

If defined, `devx up` runs `docker compose up -d` before starting services. `devx check` verifies all compose services are in "running" state.

### `[proxy]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `tls` | bool | `true` | Enable TLS on all proxies |

### `[services.<name>]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `cmd` | String | *required* | Command to execute (run via `sh -c`) |
| `dir` | String | project root | Working directory (relative to project root) |
| `port` | u16 | none | Preferred port. If set, creates a reverse proxy on this port. |
| `health` | String | none | Health check URL. Supports `${port}` interpolation. |
| `domain` | String | none | Virtual host domain for vhost proxy routing. |
| `depends_on` | [String] | `[]` | Services that must start before this one. |
| `watch` | bool | `true` | Auto-restart on file changes in service directory. |
| `env_file` | String | none | Path to `.env` file to load. |

### `[services.<name>.env]`

Key-value environment variables. Supports interpolation:
- `${port}` — actual allocated port for this service
- `${proxy:SERVICE}` — proxy port for another service

---

## Port Allocation

Every service gets an **actual port** — a random free port allocated by the OS.

If a service defines `port` (the preferred port), devx creates a **reverse proxy** on that port forwarding to the actual port. If the preferred port is busy, the proxy binds to a fallback random port.

This means:
- Services always bind to random ports (no conflicts)
- Stable URLs via proxy (e.g., `localhost:8000` always works)
- Port changes on restart are transparent to clients

### Deterministic per-worktree port offset

Before allocation, every configured `port` is shifted by a **deterministic per-worktree offset**, so the same checkout always lands on the same preferred ports and concurrent worktrees don't fight over them.

| | Context | Offset | Effect on preferred port `P` |
|---|---------|--------|------------------------------|
| (a) | Primary checkout / non-git dir | `0` | `P` (unchanged, byte-for-byte) |
| (b) | Linked git worktree | stable `N ∈ {stride, 2·stride, …, SLOTS·stride}` | `P + N` |

- The offset is resolved **once per `devx up` invocation** (alongside the [instance id](#project-identity--instance-resolution)) and threaded into the orchestrator, so every service in that run shifts consistently.
- The worktree offset is derived from the **worktree directory basename** (`git rev-parse --show-toplevel` — the same stable, path-based key the instance id uses), hashed (FNV-1a) into a non-zero slot and scaled by the stride. It is **stateless** — no registry file; the same path always yields the same offset across runs, machines, and devx versions.
- It is keyed on the worktree, **independent of `--project`/`DEVX_PROJECT`** (those name the instance; the offset separates the ports). It is also orthogonal to the actual-vs-proxy split above: the offset moves the *preferred* port; actual ports stay OS-random.
- Constants: `WORKTREE_PORT_STRIDE = 10`, `WORKTREE_PORT_SLOTS = 16` (offsets `10..=160`). Kept small so offset ports stay near the base and remain bookmarkable.

**Backward-compatibility guarantee (load-bearing):** in the primary checkout (or any non-git directory) the offset is `0`, so preferred ports are passed through **unchanged**. `git::is_worktree()` returns `false` there (errors swallowed), exactly as for the instance-id fallback, so existing projects keep their exact ports.

**Collision behavior:** two distinct worktrees can hash to the same slot. devx does not detect or avoid this — the preferred ports simply match, and the standard fall-back-to-free behavior (below) keeps both instances bootable; one loses its stable URL for that session. The offset is also a `saturating_add`, so a preferred port near `u16::MAX` clamps instead of wrapping.

### Proxy behavior

- HTTP/1.1 with header case preservation
- WebSocket upgrade support
- TLS termination (when enabled)
- 502 Bad Gateway if target unavailable
- Atomic port updates on service restart (lock-free via `AtomicU16`)

### Vhost proxy

If any service has `domain`, devx starts a vhost proxy:
- HTTPS: port 443 (fallback 8443)
- HTTP: port 80 (fallback 8080)
- Routes by `Host` header
- Auto-adds branch-prefixed domains (e.g., branch `fix-auth` → `fix-auth.api.localhost`)

---

## TLS

### Certificate Authority

Stored at `~/.config/devx/ca.key` and `~/.config/devx/ca.pem`. Generated once, persisted.

- ECDSA P256 / SHA256
- 10-year validity
- CN="devx local CA", O="devx"

### Leaf Certificate

Generated fresh on every `devx up`.

- 365-day validity
- SANs: all configured domains + `*.localhost` + `localhost` + `127.0.0.1`

---

## Health Checks

When `health` URL is configured:
- Polls every **2 seconds**
- Max **30 retries** (60s total timeout)
- Success: HTTP 2xx response
- Between retries, verifies process is still alive

State transitions: `Starting` → `Healthy` (on 2xx) or `Unhealthy` (30 retries exhausted) or `Failed` (process exited).

When no health URL: waits **3 seconds**, then marks `Healthy`.

---

## File Watching

Each service directory is watched recursively (300ms debounce). File changes trigger automatic service restart.

**Ignored directories:** `node_modules`, `target`, `.git`, `__pycache__`, `.next`, `.nuxt`, `dist`, `.cache`

`devx.toml` is watched separately (500ms debounce). Changes trigger config hot-reload: added services start, removed services stop, changed services restart.

---

## Framework Auto-Detection

devx detects frameworks from the command string and `package.json`:

| Framework | Detection | Port injection |
|-----------|-----------|----------------|
| Vite/Astro | `vite`/`astro` in cmd or package.json | `--port PORT --host` |
| Next.js | `next` in cmd or package.json | `PORT` env var |
| Node/Express | `package.json` exists | `PORT` env var |
| Go | `go run` in cmd | — |
| Uvicorn | `uvicorn` in cmd | `--port PORT` |
| Gunicorn | `gunicorn` in cmd | `-b 127.0.0.1:PORT` |

Port injection is skipped if the command already contains `${port}` or `--port`.

---

## Dependency Resolution

Uses Kahn's topological sort. Services are started in **waves** — all services in a wave can start simultaneously.

Cycles are detected and reported as errors.

When filtering services (`devx up api web`), transitive dependencies are automatically included.

---

## Process Management

Each service runs via `sh -c "<cmd>"` in a new session (`setsid`). The child PID becomes the process group leader.

**Stop sequence:**
1. SIGTERM to process group → 5s grace period
2. SIGKILL to process group → 2s reap timeout

This ensures all child processes (including those spawned by the service) are terminated.

---

## Control Socket

Unix domain socket at `/tmp/devx-{id}.sock`, where `{id}` is the [resolved instance id](#project-identity--instance-resolution). JSON over newline-delimited protocol. The `status` reply echoes the effective id in its `project` field, so `devx status` output always matches the socket it answered on.

### Commands

**shutdown**
```json
→ {"cmd":"shutdown"}
← {"ok":true}
```

**restart**
```json
→ {"cmd":"restart","service":"api"}
← {"ok":true}
```

**status**
```json
→ {"cmd":"status"}
← {"services":[{"name":"api","state":"healthy","port":34521,"proxy_port":8000,"uptime_secs":323}],"project":"myapp"}
```

### Error responses
```json
{"error":"unknown command"}
{"error":"restart requires a 'service' field"}
{"error":"invalid json: ..."}
```

---

## Daemon Mode

### File paths

| File | Path |
|------|------|
| PID | `/tmp/devx-{id}.pid` |
| Log | `/tmp/devx-{id}.log` |
| Socket | `/tmp/devx-{id}.sock` |

`{id}` is the **effective instance id**, resolved once per invocation. Precedence (first match wins): (a) `--project`/`-p` flag, (b) non-empty `DEVX_PROJECT` env var, (c) inside a linked git worktree, `{name}-{worktree-dir}`, (d) otherwise `[project].name` verbatim. See [Project Identity / Instance Resolution](#project-identity--instance-resolution).

### Daemonization

1. `fork()` — parent prints PID and exits
2. `setsid()` — detach from terminal
3. `dup2()` — redirect stdout/stderr to log file
4. Write PID file

### Log format

```
[2024-01-15 10:30:45] [api] Starting on port 34521
[2024-01-15 10:30:45] [api] [stderr] warning: something
[2024-01-15 10:30:47] [api] state: healthy
[2024-01-15 10:30:47] [api] proxy :8000 -> :34521
[2024-01-15 10:30:47] [devx] vhost https :8443 domains: api.localhost, web.localhost
[2024-01-15 10:30:47] [devx] all services started
```

### Signal handling

SIGTERM triggers graceful shutdown — stops all services, cleans up PID file and socket.

---

## Project Identity / Instance Resolution

Every name-keyed FS/OS resource (`/tmp/devx-{id}.sock`, `.pid`, `.log`, and the singleton liveness probe) is keyed by an **effective instance id**, resolved exactly once per CLI invocation and threaded into every subcommand that touches the control socket (`up`, `down`, `restart`, `status`, `logs`). Resolving it once guarantees all subcommands of one invocation agree on which instance they target.

### Precedence

First match wins:

| | Source | Resolved id |
|---|--------|-------------|
| (a) | `--project`/`-p` flag | `sanitize(flag)` |
| (b) | `DEVX_PROJECT` env var (non-empty after trim) | `sanitize(env)` |
| (c) | Linked git worktree (`git-dir != git-common-dir`) | `{name}-{sanitize(worktree-dir-basename)}` |
| (d) | Fallback | `[project].name` **verbatim** (no sanitization) |

`sanitize` lowercases the string and replaces every character that is not alphanumeric or `-` with `-` (the same rule used by branch-prefixed vhost domains). Two distinct inputs can collapse to the same id (e.g. `feat_x` and `feat/x` both become `feat-x`); this is intentional and documented.

### Path, not branch

The worktree suffix (c) is derived from the **worktree directory basename** (`git rev-parse --show-toplevel`), not the branch name. The path is stable for the checkout's lifetime, so `up` and a later `down`/`status` in the same worktree re-derive the same id even after `git checkout` to another branch. Branch-prefixed vhost domains (the human-facing URL) remain branch-keyed and are a separate, unchanged concern.

### Backward-compatibility guarantee

In the primary worktree (or any non-git directory) with no `--project` and no `DEVX_PROJECT`, resolution reaches path (d) and returns `[project].name` **byte-for-byte** — no sanitization, no suffix. This is identical to pre-feature behavior, so existing projects keep their exact socket/PID/log paths. `git::is_worktree()` returns `false` for a primary checkout and for non-git directories (errors are swallowed), so path (c) cannot fire there.

### Transition caveat

A daemon started by a pre-feature build from inside a linked worktree used the bare `[project].name`. After upgrading, the same worktree resolves to the auto-suffixed id, so the old daemon becomes invisible to bare commands. Stop it once via the old id (`devx down --project <oldname>`) before relying on the new auto-suffixed id.

---

## Constants

| Item | Value |
|------|-------|
| Health check interval | 2s |
| Health check max retries | 30 |
| No-health-check delay | 3s |
| Stop grace period (SIGTERM → SIGKILL) | 5s |
| Process reap timeout | 2s |
| File watch debounce | 300ms |
| Config reload debounce | 500ms |
| Event channel capacity | 8192 |
| Command channel capacity | 64 |
| CA validity | 3650 days |
| Leaf cert validity | 365 days |
| Log follow poll interval | 200ms |
| Default log tail lines | 50 |

---

## Requirements

- Rust 1.85+ (edition 2024)
- macOS or Linux
- Docker (optional, for `[infra]`)
