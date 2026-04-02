#!/usr/bin/env bash
# Rogue agent detection — scan heartbeats for agents exceeding time, inactivity,
# or scope thresholds and emit structured alerts.
#
# Usage:
#   rogue-detector.sh scan [team-filter]   -- scan all teams (or filtered) for rogue signals
#   rogue-detector.sh clear <team-name>    -- clear alerts for a team
#   rogue-detector.sh alerts [team-filter] -- show current alerts (JSON array)
#
# Configuration (env vars or ale.config.yaml via ALE_ prefix):
#   ALE_ROGUE_ENABLED                -- "0" to disable (default: "1")
#   ALE_ROGUE_MAX_DURATION_DISPATCH  -- max seconds for dispatch agents (default: 1800)
#   ALE_ROGUE_MAX_DURATION_SQUAD     -- max seconds for squad workers (default: 3600)
#   ALE_ROGUE_MAX_DURATION_LEAD      -- max seconds for squad leads (default: 7200)
#   ALE_ROGUE_INACTIVITY_THRESHOLD   -- seconds without commits (default: 900)
#   ALE_ROGUE_INACTIVITY_GRACE       -- initial grace period seconds (default: 600)
#   ALE_ROGUE_AUTO_KILL              -- "1" to enable auto-kill at critical (default: "0")
#   ALE_ROGUE_AUTO_KILL_MULTIPLIER   -- multiplier for critical threshold (default: 1.5)
#
# Output: structured JSON alerts to stderr and to ~/.claude/teams/<team>/alerts.json
#
# Ref #185

set -euo pipefail

TEAMS_DIR="${ALE_TEAMS_DIR:-$HOME/.claude/teams}"

# ---------------------------------------------------------------------------
# Configuration defaults
# ---------------------------------------------------------------------------
ENABLED="${ALE_ROGUE_ENABLED:-1}"
MAX_DURATION_DISPATCH="${ALE_ROGUE_MAX_DURATION_DISPATCH:-1800}"   # 30 min
MAX_DURATION_SQUAD="${ALE_ROGUE_MAX_DURATION_SQUAD:-3600}"         # 60 min
MAX_DURATION_LEAD="${ALE_ROGUE_MAX_DURATION_LEAD:-7200}"           # 120 min
MAX_DURATION_DISCOVERY="${ALE_ROGUE_MAX_DURATION_DISCOVERY:-2700}" # 45 min
INACTIVITY_THRESHOLD="${ALE_ROGUE_INACTIVITY_THRESHOLD:-900}"      # 15 min
INACTIVITY_GRACE="${ALE_ROGUE_INACTIVITY_GRACE:-600}"              # 10 min
AUTO_KILL="${ALE_ROGUE_AUTO_KILL:-0}"
AUTO_KILL_MULTIPLIER="${ALE_ROGUE_AUTO_KILL_MULTIPLIER:-1.5}"

# Heartbeat liveness thresholds (reuse from heartbeat.sh)
STALE_THRESHOLD="${ALE_HEARTBEAT_STALE_SECS:-300}"
DEAD_THRESHOLD="${ALE_HEARTBEAT_DEAD_SECS:-1800}"

if [ "$ENABLED" = "0" ]; then
  exit 0
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

now_epoch() {
  date +%s
}

# Get max duration for an agent type
max_duration_for_type() {
  local agent_type="$1"
  case "$agent_type" in
    dispatch|ic)          echo "$MAX_DURATION_DISPATCH" ;;
    squad-lead|lead)      echo "$MAX_DURATION_LEAD" ;;
    discovery)            echo "$MAX_DURATION_DISCOVERY" ;;
    squad-worker|worker|*) echo "$MAX_DURATION_SQUAD" ;;
  esac
}

# Infer agent type from team name or heartbeat data
infer_agent_type() {
  local team="$1"
  local hb_agent_type="$2"  # from heartbeat JSON, may be empty

  # Explicit type from heartbeat takes priority
  if [ -n "$hb_agent_type" ] && [ "$hb_agent_type" != "null" ]; then
    echo "$hb_agent_type"
    return
  fi

  # Heuristic: team name patterns
  case "$team" in
    *-lead|*-orchestrator)  echo "squad-lead" ;;
    *-dispatch|dispatch-*)  echo "dispatch" ;;
    *-discovery)            echo "discovery" ;;
    *)                      echo "squad-worker" ;;
  esac
}

# Check git activity on a branch
# Returns seconds since last commit, or -1 if branch not found
last_commit_age() {
  local branch="$1"
  if [ -z "$branch" ]; then
    echo "-1"
    return
  fi

  # Try to get the last commit timestamp on this branch
  local commit_ts
  commit_ts=$(git log -1 --format='%ct' "origin/$branch" 2>/dev/null || \
              git log -1 --format='%ct' "$branch" 2>/dev/null || \
              echo "")

  if [ -z "$commit_ts" ]; then
    echo "-1"
    return
  fi

  local now
  now=$(now_epoch)
  echo $(( now - commit_ts ))
}

# Write an alert to the team's alerts file
write_alert() {
  local team="$1"
  local alert_json="$2"
  local alerts_file="$TEAMS_DIR/$team/alerts.json"

  mkdir -p "$(dirname "$alerts_file")"

  # Read existing alerts, append new one, write back
  # Use python3 for reliable JSON manipulation
  local existing="[]"
  if [ -f "$alerts_file" ]; then
    existing=$(cat "$alerts_file" 2>/dev/null || echo "[]")
  fi

  NEW_ALERT="$alert_json" EXISTING="$existing" python3 -c "
import json, os, sys
try:
    existing = json.loads(os.environ.get('EXISTING', '[]'))
    if not isinstance(existing, list):
        existing = []
except (json.JSONDecodeError, ValueError):
    existing = []

try:
    new_alert = json.loads(os.environ.get('NEW_ALERT', '{}'))
except (json.JSONDecodeError, ValueError):
    sys.exit(0)

# Deduplicate: replace existing alert with same team+agent+alert type
deduped = [a for a in existing
           if not (a.get('team') == new_alert.get('team')
                   and a.get('agent') == new_alert.get('agent')
                   and a.get('alert') == new_alert.get('alert'))]
deduped.append(new_alert)
print(json.dumps(deduped, indent=2))
" > "$alerts_file" 2>/dev/null
}

# Build a JSON alert object
build_alert() {
  local team="$1"
  local agent="$2"
  local branch="$3"
  local alert_type="$4"
  local severity="$5"
  local message="$6"
  local elapsed="$7"
  local threshold="$8"

  local now
  now=$(now_epoch)

  cat <<EOF
{"team":"$team","agent":"$agent","branch":"$branch","alert":"$alert_type","severity":"$severity","message":"$message","elapsed_sec":$elapsed,"threshold_sec":$threshold,"timestamp":$now}
EOF
}

# ---------------------------------------------------------------------------
# scan -- check all teams for rogue signals
# ---------------------------------------------------------------------------
scan() {
  local filter="${1:-}"
  local alerts_found=0
  local teams_scanned=0
  local now
  now=$(now_epoch)

  for dir in "$TEAMS_DIR"/*/; do
    [ -d "$dir" ] || continue
    local team
    team=$(basename "$dir")

    # Apply filter if provided
    if [ -n "$filter" ] && [[ "$team" != *"$filter"* ]]; then
      continue
    fi

    local hb_file="$dir/heartbeat.json"
    if [ ! -f "$hb_file" ]; then
      continue
    fi

    teams_scanned=$((teams_scanned + 1))

    # Parse heartbeat
    local hb_data
    hb_data=$(HB_FILE="$hb_file" python3 -c "
import json, os, sys
try:
    hb = json.load(open(os.environ['HB_FILE']))
    pid = int(hb.get('pid', 0))
    ts = int(hb.get('timestamp', 0))
    started = hb.get('started', '')
    branch = hb.get('branch', '')
    agent_type = hb.get('agent_type', '')
    task_id = hb.get('task_id', '')
    # Check PID liveness
    pid_alive = False
    if pid > 0:
        try:
            os.kill(pid, 0)
            pid_alive = True
        except (OSError, ProcessLookupError):
            pass
    import time
    age = int(time.time() - ts) if ts > 0 else 0
    # Calculate elapsed from 'started' if available, else from timestamp
    elapsed = age
    if started:
        from datetime import datetime
        try:
            start_dt = datetime.fromisoformat(started.replace('Z', '+00:00'))
            elapsed = int(time.time() - start_dt.timestamp())
        except (ValueError, TypeError):
            pass
    print(json.dumps({
        'pid': pid, 'pid_alive': pid_alive, 'age': age, 'elapsed': elapsed,
        'branch': branch, 'agent_type': agent_type, 'task_id': task_id
    }))
except Exception as e:
    print('{}')
" 2>/dev/null)

    if [ -z "$hb_data" ] || [ "$hb_data" = "{}" ]; then
      continue
    fi

    # Extract fields
    local pid pid_alive age elapsed branch agent_type_raw agent_type
    pid=$(echo "$hb_data" | python3 -c "import json,sys; print(json.load(sys.stdin).get('pid',0))" 2>/dev/null)
    pid_alive=$(echo "$hb_data" | python3 -c "import json,sys; print(str(json.load(sys.stdin).get('pid_alive',False)).lower())" 2>/dev/null)
    age=$(echo "$hb_data" | python3 -c "import json,sys; print(json.load(sys.stdin).get('age',0))" 2>/dev/null)
    elapsed=$(echo "$hb_data" | python3 -c "import json,sys; print(json.load(sys.stdin).get('elapsed',0))" 2>/dev/null)
    branch=$(echo "$hb_data" | python3 -c "import json,sys; print(json.load(sys.stdin).get('branch',''))" 2>/dev/null)
    agent_type_raw=$(echo "$hb_data" | python3 -c "import json,sys; print(json.load(sys.stdin).get('agent_type',''))" 2>/dev/null)

    # Skip dead agents (already handled by heartbeat liveness)
    if [ "$pid_alive" = "false" ]; then
      # Clear alerts for dead agents
      clear_alerts "$team" 2>/dev/null || true
      continue
    fi

    agent_type=$(infer_agent_type "$team" "$agent_type_raw")
    local max_duration
    max_duration=$(max_duration_for_type "$agent_type")

    # --- Signal 1: Duration exceeded ---
    if [ "$elapsed" -gt 0 ] && [ "$max_duration" -gt 0 ]; then
      local warn_threshold=$(( max_duration * 80 / 100 ))
      local critical_threshold
      critical_threshold=$(echo "$max_duration $AUTO_KILL_MULTIPLIER" | python3 -c "
import sys
parts = sys.stdin.read().split()
print(int(float(parts[0]) * float(parts[1])))
" 2>/dev/null || echo $(( max_duration * 3 / 2 )))

      if [ "$elapsed" -ge "$critical_threshold" ]; then
        local alert
        alert=$(build_alert "$team" "$team" "$branch" "duration_exceeded" "critical" \
          "Agent running for $((elapsed / 60))m (critical: $((critical_threshold / 60))m max for $agent_type)" \
          "$elapsed" "$critical_threshold")
        write_alert "$team" "$alert"
        echo "$alert" >&2
        alerts_found=$((alerts_found + 1))

        # Auto-kill if enabled and not a squad lead
        if [ "$AUTO_KILL" = "1" ] && [ "$agent_type" != "squad-lead" ] && [ "$agent_type" != "lead" ]; then
          if [ "$pid" -gt 0 ] && kill -0 "$pid" 2>/dev/null; then
            echo "{\"action\":\"auto_kill\",\"team\":\"$team\",\"pid\":$pid,\"reason\":\"duration_critical\",\"timestamp\":$(now_epoch)}" >&2
            kill -TERM "$pid" 2>/dev/null || true
          fi
        fi
      elif [ "$elapsed" -ge "$max_duration" ]; then
        local alert
        alert=$(build_alert "$team" "$team" "$branch" "duration_exceeded" "alert" \
          "Agent running for $((elapsed / 60))m (limit: $((max_duration / 60))m for $agent_type)" \
          "$elapsed" "$max_duration")
        write_alert "$team" "$alert"
        echo "$alert" >&2
        alerts_found=$((alerts_found + 1))
      elif [ "$elapsed" -ge "$warn_threshold" ]; then
        local alert
        alert=$(build_alert "$team" "$team" "$branch" "duration_exceeded" "warning" \
          "Agent approaching time limit: $((elapsed / 60))m of $((max_duration / 60))m ($agent_type)" \
          "$elapsed" "$max_duration")
        write_alert "$team" "$alert"
        echo "$alert" >&2
        alerts_found=$((alerts_found + 1))
      fi
    fi

    # --- Signal 2: Inactivity (no recent commits) ---
    # Skip if agent is in grace period or is a discovery task
    if [ "$agent_type" != "discovery" ] && [ "$elapsed" -gt "$INACTIVITY_GRACE" ]; then
      local commit_age
      commit_age=$(last_commit_age "$branch")

      if [ "$commit_age" -gt "$INACTIVITY_THRESHOLD" ] 2>/dev/null; then
        local alert
        alert=$(build_alert "$team" "$team" "$branch" "inactivity" "warning" \
          "No commits for $((commit_age / 60))m on $branch (threshold: $((INACTIVITY_THRESHOLD / 60))m)" \
          "$commit_age" "$INACTIVITY_THRESHOLD")
        write_alert "$team" "$alert"
        echo "$alert" >&2
        alerts_found=$((alerts_found + 1))
      fi
      # commit_age == -1 means branch not found -- skip, not an error
    fi

    # --- Signal 3: Scope divergence ---
    # Only check if we can determine the base branch and have a branch to check
    if [ -n "$branch" ]; then
      # Check for modifications to protected paths
      local protected_changes
      protected_changes=$(git diff --name-only "origin/main...$branch" 2>/dev/null | \
        grep -E '^(\.claude/|\.github/workflows/|lefthook\.yml)' 2>/dev/null | head -5 || echo "")

      if [ -n "$protected_changes" ]; then
        local alert
        alert=$(build_alert "$team" "$team" "$branch" "scope_divergence" "critical" \
          "Agent modified protected paths: $(echo "$protected_changes" | tr '\n' ', ' | sed 's/,$//')" \
          "$elapsed" "0")
        write_alert "$team" "$alert"
        echo "$alert" >&2
        alerts_found=$((alerts_found + 1))
      fi
    fi
  done

  # Summary to stdout
  echo "{\"scanned\":$teams_scanned,\"alerts\":$alerts_found,\"timestamp\":$(now_epoch)}"
}

# ---------------------------------------------------------------------------
# clear -- remove alerts for a team
# ---------------------------------------------------------------------------
clear_alerts() {
  local team="$1"
  if [ -z "$team" ]; then
    echo "Usage: rogue-detector.sh clear <team-name>" >&2
    return 1
  fi
  local alerts_file="$TEAMS_DIR/$team/alerts.json"
  if [ -f "$alerts_file" ]; then
    echo "[]" > "$alerts_file"
  fi
  echo "{\"cleared\":\"$team\",\"timestamp\":$(now_epoch)}"
}

# ---------------------------------------------------------------------------
# alerts -- show current alerts
# ---------------------------------------------------------------------------
show_alerts() {
  local filter="${1:-}"
  local first=true

  echo "["
  for dir in "$TEAMS_DIR"/*/; do
    [ -d "$dir" ] || continue
    local team
    team=$(basename "$dir")

    if [ -n "$filter" ] && [[ "$team" != *"$filter"* ]]; then
      continue
    fi

    local alerts_file="$dir/alerts.json"
    if [ ! -f "$alerts_file" ]; then
      continue
    fi

    # Read and emit each alert (flatten into top-level array)
    ALERTS_FILE="$alerts_file" python3 -c "
import json, os, sys
try:
    alerts = json.load(open(os.environ['ALERTS_FILE']))
    if not isinstance(alerts, list):
        sys.exit(0)
    for a in alerts:
        prefix = ',' if not ${first} else ''
        first_placeholder = False
        print(prefix + json.dumps(a))
except (json.JSONDecodeError, ValueError, FileNotFoundError):
    pass
" 2>/dev/null

    # After first team with alerts, subsequent entries need commas
    # (handled by python above via the first variable)
    if [ -f "$alerts_file" ]; then
      local count
      count=$(python3 -c "
import json, os
try:
    alerts = json.load(open(os.environ.get('ALERTS_FILE', '')))
    print(len(alerts) if isinstance(alerts, list) else 0)
except: print(0)
" 2>/dev/null)
      if [ "${count:-0}" -gt 0 ]; then
        first=false
      fi
    fi
  done
  echo "]"
}

# ---------------------------------------------------------------------------
# Main dispatch
# ---------------------------------------------------------------------------
case "${1:-}" in
  scan)    scan "${2:-}" ;;
  clear)   clear_alerts "${2:-}" ;;
  alerts)  show_alerts "${2:-}" ;;
  *)
    echo "Usage: rogue-detector.sh {scan|clear|alerts} [team-name-filter]" >&2
    echo "" >&2
    echo "Commands:" >&2
    echo "  scan [filter]    Scan teams for rogue agent signals" >&2
    echo "  clear <team>     Clear alerts for a team" >&2
    echo "  alerts [filter]  Show current alerts (JSON)" >&2
    exit 1
    ;;
esac
