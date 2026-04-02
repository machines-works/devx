#!/usr/bin/env bash
# Write or check team heartbeat files.
# Usage:
#   heartbeat.sh write <team-name>   -- write PID + timestamp + branch + status to heartbeat file
#   heartbeat.sh check <team-name>   -- check if the team's process is alive (legacy, backward compat)
#   heartbeat.sh check-all [filter]  -- check all teams, output status for each (legacy, backward compat)
#   heartbeat.sh read [team-name]    -- read one or all heartbeats with liveness status (active/stale/dead)
#
# Liveness thresholds (configurable via env vars):
#   ALE_HEARTBEAT_STALE_SECS  -- seconds before a heartbeat is considered stale (default: 300 = 5 min)
#   ALE_HEARTBEAT_DEAD_SECS   -- seconds before a heartbeat is considered dead  (default: 1800 = 30 min)
#
# Liveness categories:
#   active -- heartbeat < stale threshold AND PID is running
#   active (workers) -- PID dead but recent git commits on squad branches (git-activity fallback)
#   stale  -- heartbeat between stale and dead thresholds, OR PID not running but within dead threshold
#   dead   -- heartbeat > dead threshold OR PID not running and heartbeat > stale threshold
#
# Git-activity fallback (Phase 1 of #314):
#   When PID is dead, check for recent commits on branches matching the team name.
#   If recent commits exist, workers are still active even though the squad lead exited.
#   ALE_HEARTBEAT_GIT_ACTIVITY_SECS -- max age of git commits to consider "recent" (default: 300 = 5 min)

TEAMS_DIR="${ALE_TEAMS_DIR:-$HOME/.claude/teams}"

# Liveness thresholds (seconds)
STALE_THRESHOLD="${ALE_HEARTBEAT_STALE_SECS:-300}"   # 5 minutes
DEAD_THRESHOLD="${ALE_HEARTBEAT_DEAD_SECS:-1800}"    # 30 minutes
GIT_ACTIVITY_THRESHOLD="${ALE_HEARTBEAT_GIT_ACTIVITY_SECS:-300}"  # 5 minutes

# ---------------------------------------------------------------------------
# check_git_activity -- check for recent commits on branches matching a team name
# Returns "true" if recent git activity found, "false" otherwise
# ---------------------------------------------------------------------------
check_git_activity() {
  local team="$1"
  local threshold="${2:-$GIT_ACTIVITY_THRESHOLD}"

  # Need git available
  if ! command -v git &>/dev/null; then
    echo "false"
    return
  fi

  # Need to be in a git repo (or have one configured)
  if ! git rev-parse --git-dir &>/dev/null 2>&1; then
    echo "false"
    return
  fi

  # Look for recent commits on any branch containing the team name
  # This catches: squad/<team>, fix/<team>-*, feat/<team>-*, etc.
  local recent_count
  recent_count=$(git log --branches="*${team}*" --since="${threshold} seconds ago" \
    --format="%H" -1 2>/dev/null | wc -l | tr -d ' ')

  if [ "${recent_count:-0}" -gt 0 ]; then
    echo "true"
  else
    echo "false"
  fi
}

# ---------------------------------------------------------------------------
# write -- write a heartbeat file for a team
# ---------------------------------------------------------------------------
write_heartbeat() {
  local team="$1"
  if [ -z "$team" ]; then
    echo "Usage: heartbeat.sh write <team-name>" >&2
    return 1
  fi
  local hb_file="$TEAMS_DIR/$team/heartbeat.json"
  # Ensure directory exists
  mkdir -p "$(dirname "$hb_file")"
  # Detect current branch (best-effort)
  local branch
  branch=$(git branch --show-current 2>/dev/null || echo "")
  # PPID is the Claude Code node process (parent of this bash shell)
  cat > "$hb_file" <<EOF
{"pid": $PPID, "timestamp": $(date +%s), "started": "$(date -u +%Y-%m-%dT%H:%M:%SZ)", "branch": "$branch", "team": "$team", "status": "active"}
EOF
}

# ---------------------------------------------------------------------------
# check -- legacy check (backward compatible): alive|Ns / dead|pid=N / no-heartbeat / corrupt
# ---------------------------------------------------------------------------
check_heartbeat() {
  local team="$1"
  local hb_file="$TEAMS_DIR/$team/heartbeat.json"
  if [ ! -f "$hb_file" ]; then
    echo "no-heartbeat"
    return
  fi
  local pid
  pid=$(python3 -c "
import json
try: print(json.load(open('$hb_file'))['pid'])
except (json.JSONDecodeError, KeyError, ValueError, FileNotFoundError): pass
" 2>/dev/null)
  if [ -z "$pid" ]; then
    echo "corrupt"
    return
  fi
  if kill -0 "$pid" 2>/dev/null; then
    local ts
    ts=$(python3 -c "
import json, time
try:
    hb = json.load(open('$hb_file'))
    print(int(time.time() - hb['timestamp']))
except (json.JSONDecodeError, KeyError, ValueError, FileNotFoundError): pass
" 2>/dev/null)
    echo "alive|${ts}s"
  else
    # Git-activity fallback: check for recent commits before declaring dead
    local git_active
    git_active=$(check_git_activity "$team")
    if [ "$git_active" = "true" ]; then
      echo "alive|workers-active"
    else
      echo "dead|pid=$pid"
    fi
  fi
}

# ---------------------------------------------------------------------------
# check-all -- legacy check all teams (backward compatible)
# ---------------------------------------------------------------------------
check_all() {
  local filter="$1"
  for dir in "$TEAMS_DIR"/*/; do
    [ -d "$dir" ] || continue
    local team
    team=$(basename "$dir")
    # If filter provided, skip teams that don't match
    if [ -n "$filter" ] && [[ "$team" != *"$filter"* ]]; then
      continue
    fi
    local status
    status=$(check_heartbeat "$team")
    echo "$team|$status"
  done
}

# ---------------------------------------------------------------------------
# read -- read heartbeat(s) with threshold-based liveness status
# ---------------------------------------------------------------------------
# Output format (JSON):
#   Single team: {"team":"name","pid":123,"timestamp":N,"age":N,"branch":"...","status":"active|stale|dead","pid_alive":true|false}
#   All teams:   [{"team":"name",...}, ...]
#   No heartbeat file: {"team":"name","status":"no-heartbeat"}
#   Corrupt file:      {"team":"name","status":"corrupt"}

classify_liveness() {
  local age="$1"
  local pid_alive="$2"
  local git_active="${3:-false}"

  if [ "$pid_alive" = "false" ]; then
    # PID is dead -- check git-activity fallback before declaring dead
    if [ "$git_active" = "true" ]; then
      echo "active"
    elif [ "$age" -lt "$DEAD_THRESHOLD" ]; then
      echo "stale"
    else
      echo "dead"
    fi
  else
    # PID is alive -- classify by timestamp age
    if [ "$age" -lt "$STALE_THRESHOLD" ]; then
      echo "active"
    elif [ "$age" -lt "$DEAD_THRESHOLD" ]; then
      echo "stale"
    else
      echo "dead"
    fi
  fi
}

read_heartbeat() {
  local team="$1"
  local hb_file="$TEAMS_DIR/$team/heartbeat.json"

  if [ ! -f "$hb_file" ]; then
    echo "{\"team\":\"$team\",\"status\":\"no-heartbeat\"}"
    return
  fi

  # Parse heartbeat JSON in one python3 call for reliability
  local parsed
  parsed=$(python3 -c "
import json, time, sys, os
try:
    hb = json.load(open('$hb_file'))
    pid = int(hb['pid'])
    ts = int(hb.get('timestamp', 0))
    age = int(time.time() - ts)
    branch = hb.get('branch', '')
    team_name = hb.get('team', '$team')
    # Check PID liveness
    try:
        os.kill(pid, 0)
        pid_alive = True
    except (OSError, ProcessLookupError):
        pid_alive = False
    print(json.dumps({
        'pid': pid,
        'timestamp': ts,
        'age': age,
        'branch': branch,
        'team': team_name,
        'pid_alive': pid_alive
    }))
except (json.JSONDecodeError, KeyError, ValueError) as e:
    print('CORRUPT')
    sys.exit(0)
" 2>/dev/null)

  if [ "$parsed" = "CORRUPT" ] || [ -z "$parsed" ]; then
    echo "{\"team\":\"$team\",\"status\":\"corrupt\"}"
    return
  fi

  # Extract age and pid_alive from parsed JSON, then classify
  local age pid_alive
  age=$(echo "$parsed" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['age'])" 2>/dev/null)
  pid_alive=$(echo "$parsed" | python3 -c "import json,sys; d=json.load(sys.stdin); print(str(d['pid_alive']).lower())" 2>/dev/null)

  # Git-activity fallback: when PID is dead, check for recent commits
  local git_active="false"
  if [ "$pid_alive" = "false" ]; then
    git_active=$(check_git_activity "$team")
  fi

  local liveness
  liveness=$(classify_liveness "$age" "$pid_alive" "$git_active")

  # Inject the status and git_active flag into the parsed JSON
  echo "$parsed" | python3 -c "
import json, sys
d = json.load(sys.stdin)
d['status'] = '$liveness'
d['git_active'] = $( [ "$git_active" = "true" ] && echo "True" || echo "False" )
print(json.dumps(d))
" 2>/dev/null
}

read_all() {
  local filter="$1"
  local first=true

  echo "["
  for dir in "$TEAMS_DIR"/*/; do
    [ -d "$dir" ] || continue
    local team
    team=$(basename "$dir")
    # If filter provided, skip teams that don't match
    if [ -n "$filter" ] && [[ "$team" != *"$filter"* ]]; then
      continue
    fi
    if [ "$first" = true ]; then
      first=false
    else
      echo ","
    fi
    read_heartbeat "$team"
  done
  echo "]"
}

# ---------------------------------------------------------------------------
# Main dispatch
# ---------------------------------------------------------------------------
case "$1" in
  write)     write_heartbeat "$2" ;;
  check)     check_heartbeat "$2" ;;
  check-all) check_all "$2" ;;
  read)
    if [ -n "$2" ]; then
      read_heartbeat "$2"
    else
      read_all "$3"
    fi
    ;;
  *)
    echo "Usage: heartbeat.sh {write|check|check-all|read} [team-name]"
    ;;
esac
