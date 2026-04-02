---
name: ale:status
description: Show all active squads, ICs, branches, and worktree status
allowed-tools:
  - Read
  - Bash
  - Glob
  - Grep
---
<objective>
Tribe-level overview: show all active teams (Agent Teams), dispatched ICs, task progress,
and branch activity. Quick, read-only — no modifications.

Important: Do NOT use the TaskList tool — it is session-scoped and only returns tasks for
the calling session's team. Instead, read task JSON files directly from ~/.claude/tasks/
to get a complete cross-team view.
</objective>

<process>
## Step 0: Derive Repo Slug

Derive `$REPO_SLUG` using the Repo Slug pattern from `_patterns.md`.
Use throughout to scope all discovery to the current project.

## Step 1: Discover and Classify Teams

Find all team directories scoped to this repo, then classify each by liveness:

```bash
# List team directories matching this repo
for dir in ~/.claude/teams/*"$REPO_SLUG"*/; do
  [ -d "$dir" ] || continue
  team=$(basename "$dir")
  echo "TEAM: $team"
done
```

For each team found, check heartbeat liveness:
```bash
~/.claude/hooks/heartbeat.sh check-all "$REPO_SLUG" 2>/dev/null
```

The heartbeat output format is `team-name|status` where status is one of:
- `alive|Ns` — PID is running (active team)
- `dead|pid=N` — PID is not running (stale team)
- `no-heartbeat` — no heartbeat file (stale team)
- `corrupt` — heartbeat file unreadable (stale team)

For each team, also read `~/.claude/teams/<team>/config.json` to get:
- Team name and description
- Members (name, agentType, status)

## Step 2: Read Task Files from Filesystem

**Do NOT use the TaskList tool.** It is session-scoped and will return 0 tasks when run
from a session that is not part of the target team.

Instead, read task JSON files directly for each team scoped to this repo:

```bash
# For each team matching repo slug, read its task files
for taskdir in ~/.claude/tasks/*"$REPO_SLUG"*/; do
  [ -d "$taskdir" ] || continue
  team=$(basename "$taskdir")
  echo "=== $team ==="
  for f in "$taskdir"/*.json; do
    [ -f "$f" ] || continue
    # Each file is a task JSON with: id, subject, status, owner, blockedBy
    python3 -c "
import json, sys
try:
    t = json.load(open('$f'))
except (json.JSONDecodeError, ValueError, FileNotFoundError):
    print(f'  [?] (corrupt: $f)', file=sys.stderr); sys.exit(0)
status = t.get('status', 'unknown')
subject = t.get('subject', '(no subject)')
owner = t.get('owner', '')
task_id = t.get('id', '?')
blocked = t.get('blockedBy', [])
owner_str = f' ({owner})' if owner else ''
blocked_str = f' -- blocked by {blocked}' if blocked else ''
mark = 'x' if status == 'completed' else ' '
print(f'  [{mark}] #{task_id}: {subject}{owner_str} -- {status}{blocked_str}')
" 2>/dev/null
  done
done
```

Compute per-team task summary counts (completed, in_progress, pending, blocked).

## Step 3: Classify Teams

Using the heartbeat status from Step 1 and task counts from Step 2, classify each team:

- **alive** — heartbeat PID is running. Show in ACTIVE TEAMS section.
- **stale** — heartbeat PID is dead (or missing), but tasks/worktrees/branches remain with incomplete work. Show in STALE TEAMS section with task counts and cleanup suggestion.
- **done** — all tasks are completed (regardless of PID status). Show in COMPLETED TEAMS section.

A team is stale if its PID is dead AND it has incomplete tasks (pending or in_progress).
A team is done if ALL its tasks have status "completed" (or it has no tasks and PID is dead).

## Step 4: Discover Worktrees and Branches

```bash
git worktree list
git branch --format='%(refname:short) %(objectname:short) %(committerdate:relative) %(subject)' | grep -E '^(feat/|fix/|worktree-)' | head -40
```

## Step 5: Discover Team Briefs

```bash
ls -d docs/*/TEAM-BRIEF.md docs/*/SPLIT.md 2>/dev/null
```

## Step 6: Check OTEL Observability Stack

Run the OTEL Stack Check from `_patterns.md`.

## Step 6.5: Check Rogue Agent Alerts

Run the rogue detector to scan for misbehaving agents:

```bash
# Scan all teams scoped to this repo for rogue signals
~/.claude/hooks/rogue-detector.sh scan "$REPO_SLUG" 2>/tmp/ale-rogue-scan.log

# Read current alerts
ROGUE_ALERTS=$(~/.claude/hooks/rogue-detector.sh alerts "$REPO_SLUG" 2>/dev/null)
```

Parse the JSON alert array. Each alert has: `team`, `agent`, `branch`, `alert` (type), `severity`, `message`, `elapsed_sec`, `threshold_sec`, `timestamp`.

## Step 7: Display Status

```
====================================================
 TRIBE STATUS — $REPO_SLUG
====================================================

 ACTIVE TEAMS (alive — PID running)
----------------------------------------------------
 squad-name
   Members: worker-a, worker-b, worker-c
   Tasks: 3/8 completed | 3 in_progress | 2 blocked
     [x] #1: Phase 1: RBAC Extension (rbac-worker) — completed
     [x] #2: Phase 2: Fleet Management API (fleet-worker) — completed
     [ ] #3: Phase 3: Agent Scheduling (scheduling-worker) — in_progress
     [ ] #4: Phase 4: Persona Layout — blocked by 1,2,3
     ...

 STALE TEAMS (dead PID — work remains)
----------------------------------------------------
 old-squad-name  [stale — PID 12345 not running]
   Tasks: 2/5 completed | 1 in_progress | 2 pending
     [x] #1: Setup infrastructure — completed
     [x] #2: Implement auth — completed
     [ ] #3: Add tests — in_progress
     [ ] #4: Write docs — pending
     [ ] #5: Final review — pending
   Cleanup: restart with `/ale:start`, or remove with:
     rm -rf ~/.claude/teams/old-squad-name
     rm -rf ~/.claude/tasks/old-squad-name

 COMPLETED TEAMS
----------------------------------------------------
 finished-squad  [done — all 4 tasks completed]

 ICs (dispatch — no team, solo worktrees)
----------------------------------------------------
   fix/eval-regression — 2 commits, last: 30m ago
   feat/http-fetch-tool — 3 commits, last: 1h ago

 AVAILABLE BRIEFS (not started)
----------------------------------------------------
   docs/v1.2-ux/TEAM-BRIEF.md
   docs/v1.2-stability/TEAM-BRIEF.md

 OTEL STACK (coding agent telemetry)
----------------------------------------------------
   Local: UP (4/4 containers) | UI: http://localhost:8090
   — or —
   Local: DOWN — `mise run otel-up` to start

 ROGUE AGENT ALERTS
----------------------------------------------------
   [critical] my-squad — duration_exceeded — Agent running for 50m (limit: 30m for dispatch)
   [warning]  other-squad — inactivity — No commits for 18m on feat/other-squad-worker (threshold: 15m)
   [critical] rogue-squad — scope_divergence — Agent modified protected paths: .claude/settings.json

 STALE WORKTREES (no matching team or branch)
----------------------------------------------------
   worktree-agent-abc123 — N commits (orphaned)

 SUMMARY
----------------------------------------------------
   Teams: N active | N stale | N done
   ICs: N active | Worktrees: N total
====================================================
```

Classification rules:
- Teams in `~/.claude/teams/` with alive heartbeat -> Active Teams section
- Teams in `~/.claude/teams/` with dead/missing heartbeat + incomplete tasks -> Stale Teams section
- Teams in `~/.claude/teams/` with all tasks completed -> Completed Teams section
- Branches matching `fix/*` or `feat/*` without a team -> IC section
- Branches matching `worktree-agent-*` -> stale/orphaned
- `docs/*/TEAM-BRIEF.md` without matching team -> available briefs
- Alerts from `rogue-detector.sh alerts` -> Rogue Agent Alerts section (omit if empty)

If no activity at all:
> No active teams or ICs. Start one with `/ale:start <milestone>` or `/ale:start --solo <task>`.
</process>
