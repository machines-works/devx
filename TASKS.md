# devx — task backlog

## Daemon mode
- [ ] Add `devx up -d` flag to run orchestrator as a background process (fork, detach from terminal, skip TUI)
- [ ] Write PID file to `/tmp/devx-<project>.pid` so `devx down` and `devx status` can find the daemon
- [ ] Add `devx logs` command that tails logs from the running daemon via the control socket
- [ ] Add `devx logs <service>` to tail a single service's logs
- [ ] Persist logs to `~/.devx/logs/<project>/` so they survive restarts and are available to `devx logs`

## TUI improvements
- [ ] Strip ANSI escape codes from log output before rendering in the TUI
- [ ] Show a visual indicator on the selected service row when logs are filtered to it

## Health checks
- [ ] Add Encore to framework detection with default health endpoint at port 9400

## Distribution
- [ ] Push latest changes to machines-works/devx and tag v0.1.0-alpha.1
