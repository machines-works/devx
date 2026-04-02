#!/usr/bin/env bash
# session-preflight.sh -- Claude Code SessionStart hook
#
# Fires at the start of every Claude Code session.
# Runs a battery of quick environment checks and prints an informational
# summary. This hook NEVER blocks session start.
#
# Must be FAST (< 3 seconds total). All checks are best-effort.
#
# Configuration:
#   ALE_SESSION_PREFLIGHT  -- set to "0" to disable (default: enabled)
#
# Exit 0 = always allow (informational only, never blocks)

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_SESSION_PREFLIGHT:-1}" = "0" ]; then
  exit 0
fi

# Read stdin (hook context JSON from Claude Code)
INPUT=$(cat)

ISSUES=()
INFO=()

# ---------------------------------------------------------------------------
# 1. Stale clone/worktree detection
# ---------------------------------------------------------------------------
stale_check() {
  local stale_count=0
  local stale_paths=()
  local now
  now=$(date +%s)
  local threshold=86400  # 24 hours
  local found_any=false

  # --- Check clones in .claude/clones/ ---
  local repo_root
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
  if [ -n "$repo_root" ] && [ -d "$repo_root/.claude/clones" ]; then
    for clone_dir in "$repo_root/.claude/clones"/*/; do
      [ -d "$clone_dir" ] || continue
      [ -f "$clone_dir/.ale-clone" ] || continue
      found_any=true

      local last_commit_ts
      last_commit_ts=$(git -C "$clone_dir" log -1 --format='%ct' 2>/dev/null || echo "0")
      [ "$last_commit_ts" = "0" ] && continue

      local age=$(( now - last_commit_ts ))
      if [ "$age" -gt "$threshold" ]; then
        stale_count=$((stale_count + 1))
        stale_paths+=("$clone_dir")
      fi
    done
  fi

  # --- Legacy: check worktrees in .claude/worktrees/ ---
  if [ -n "$repo_root" ] && [ -d "$repo_root/.claude/worktrees" ]; then
    while IFS= read -r line; do
      local wt_path
      wt_path=$(echo "$line" | awk '{print $1}')
      [ -d "$wt_path" ] || continue
      # Only count worktrees under .claude/worktrees/
      case "$wt_path" in
        */.claude/worktrees/*) ;;
        *) continue ;;
      esac
      found_any=true

      local last_commit_ts
      last_commit_ts=$(git -C "$wt_path" log -1 --format='%ct' 2>/dev/null || echo "0")
      [ "$last_commit_ts" = "0" ] && continue

      local age=$(( now - last_commit_ts ))
      if [ "$age" -gt "$threshold" ]; then
        stale_count=$((stale_count + 1))
        stale_paths+=("$wt_path")
      fi
    done < <(git worktree list 2>/dev/null | tail -n +2)
  fi

  if [ "$stale_count" -gt 0 ]; then
    local paths_str
    paths_str=$(printf '%s, ' "${stale_paths[@]}")
    paths_str="${paths_str%, }"
    ISSUES+=("Stale clones/worktrees (>24h): ${stale_count} -- ${paths_str}")
  elif [ "$found_any" = true ]; then
    INFO+=("Clones: all fresh")
  else
    INFO+=("Clones: none active")
  fi
}

# ---------------------------------------------------------------------------
# 2. GitHub auth check
# ---------------------------------------------------------------------------
gh_auth_check() {
  if ! command -v gh >/dev/null 2>&1; then
    ISSUES+=("GitHub CLI: not installed")
    return
  fi

  local gh_output
  gh_output=$(gh auth status 2>&1) || true

  if echo "$gh_output" | grep -q "Logged in"; then
    INFO+=("GitHub CLI: authenticated")
  else
    ISSUES+=("GitHub CLI: not authenticated (run 'gh auth login')")
  fi
}

# ---------------------------------------------------------------------------
# 3. Orphaned squad branches
# ---------------------------------------------------------------------------
orphan_check() {
  local teams_dir="$HOME/.claude/teams"
  local orphaned=()

  while IFS= read -r branch; do
    # branch looks like: origin/squad/foo
    local squad_name
    squad_name=$(echo "$branch" | sed 's|.*squad/||')
    [ -z "$squad_name" ] && continue

    # Check if there's a live heartbeat for this squad
    local hb_file="$teams_dir/$squad_name/heartbeat.json"
    local is_alive=false

    if [ -f "$hb_file" ]; then
      local pid
      pid=$(python3 -c "import json; print(json.load(open('$hb_file'))['pid'])" 2>/dev/null || echo "")
      if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        is_alive=true
      fi
    fi

    if [ "$is_alive" = false ]; then
      orphaned+=("squad/$squad_name")
    fi
  done < <(git branch -r 2>/dev/null | grep 'origin/squad/' | sed 's/^[[:space:]]*//')

  if [ "${#orphaned[@]}" -gt 0 ]; then
    local orphan_str
    orphan_str=$(printf '%s, ' "${orphaned[@]}")
    orphan_str="${orphan_str%, }"
    ISSUES+=("Orphaned squad branches (no live team): ${#orphaned[@]} -- ${orphan_str}")
  fi
}

# ---------------------------------------------------------------------------
# 4. Lefthook validation
# ---------------------------------------------------------------------------
lefthook_check() {
  local repo_root
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")

  if ! command -v lefthook >/dev/null 2>&1; then
    ISSUES+=("Lefthook: binary not found")
    return
  fi

  if [ -n "$repo_root" ] && [ -f "$repo_root/lefthook.yml" ]; then
    INFO+=("Lefthook: installed and configured")
  elif [ -n "$repo_root" ]; then
    ISSUES+=("Lefthook: binary found but lefthook.yml missing in repo root")
  fi
}

# ---------------------------------------------------------------------------
# 5. Pending actions from ale-react
# ---------------------------------------------------------------------------
pending_actions_check() {
  local repo_root
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
  [ -z "$repo_root" ] && return

  local actions_file="$repo_root/.ale/cache/pending-actions.jsonl"
  [ -f "$actions_file" ] || return

  local line_count
  line_count=$(wc -l < "$actions_file" 2>/dev/null | tr -d ' ')
  [ "$line_count" -eq 0 ] && return

  # Read up to 10 most recent actions
  local actions=()
  while IFS= read -r line; do
    local action_type
    action_type=$(echo "$line" | sed -n 's/.*"action":"\([^"]*\)".*/\1/p')
    [ -z "$action_type" ] && continue
    actions+=("$action_type")
  done < <(tail -10 "$actions_file")

  if [ "${#actions[@]}" -gt 0 ]; then
    local action_str
    action_str=$(printf '%s, ' "${actions[@]}")
    action_str="${action_str%, }"
    ISSUES+=("Pending actions ($line_count): $action_str")
  fi
}

# ---------------------------------------------------------------------------
# 6. Escalation files for human review
# ---------------------------------------------------------------------------
escalation_check() {
  local esc_dir="$HOME/.ale/escalations"
  [ -d "$esc_dir" ] || return

  local esc_count=0
  local esc_files=()
  for f in "$esc_dir"/esc-*.json; do
    [ -f "$f" ] || continue
    esc_count=$((esc_count + 1))
    esc_files+=("$(basename "$f")")
  done

  if [ "$esc_count" -gt 0 ]; then
    ISSUES+=("Blocker escalations ($esc_count): review $esc_dir")
  fi
}

# ---------------------------------------------------------------------------
# 7. Emit session_start event and inject computed state
# ---------------------------------------------------------------------------
state_injection() {
  # Find ale-emit and ale-state (check local .ale/bin, then project root)
  local ale_emit="" ale_state=""
  local repo_root
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")

  for base in ".ale/bin" "$repo_root/.ale/bin"; do
    [ -z "$base" ] && continue
    [ -x "$base/ale-emit" ] && ale_emit="$base/ale-emit"
    [ -x "$base/ale-state" ] && ale_state="$base/ale-state"
  done

  # Emit session_start event with context
  if [ -n "$ale_emit" ]; then
    local branch uncommitted unpushed
    branch=$(git symbolic-ref --short HEAD 2>/dev/null || echo "unknown")
    uncommitted=$(git status --short 2>/dev/null | grep -c '.' || echo "0")
    unpushed=$(git rev-list origin/main..HEAD --count 2>/dev/null || echo "0")

    "$ale_emit" session_start \
      branch="$branch" \
      uncommitted="$uncommitted" \
      unpushed="$unpushed" \
      source="${ALE_SESSION_SOURCE:-startup}" \
      2>/dev/null || true
  fi

  # Inject computed state summary into preflight output
  if [ -n "$ale_state" ]; then
    local summary
    summary=$("$ale_state" --summary 2>/dev/null || echo "")
    if [ -n "$summary" ]; then
      INFO+=("Project state: computed and injected below")
      STATE_SUMMARY="$summary"
    fi
  fi
}

# ---------------------------------------------------------------------------
# 8. Session correlation attributes (set by ale:start / ale:tribe) -- Ref #188
# ---------------------------------------------------------------------------
session_correlation_check() {
  local has_correlation=false

  if [ -n "${ALE_PARENT_SESSION_ID:-}" ]; then
    INFO+=("Session correlation: parent=${ALE_PARENT_SESSION_ID}")
    has_correlation=true
  fi
  if [ -n "${ALE_TEAM_NAME:-}" ]; then
    INFO+=("Team: ${ALE_TEAM_NAME}")
    has_correlation=true
  fi
  if [ -n "${ALE_ISSUE_NUMBER:-}" ]; then
    INFO+=("Issue: #${ALE_ISSUE_NUMBER}")
    has_correlation=true
  fi

  # Emit session_start with correlation via otel-emit if available
  if [ "$has_correlation" = true ]; then
    local otel_emit_path=""
    local repo_root
    repo_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
    for base in "$repo_root/hooks/lib" "$HOME/.claude/hooks/lib"; do
      if [ -f "$base/otel-emit.sh" ]; then
        otel_emit_path="$base/otel-emit.sh"
        break
      fi
    done

    if [ -n "$otel_emit_path" ]; then
      # shellcheck disable=SC1090
      source "$otel_emit_path"
      otel_emit_log "session-preflight" "SESSION_START" "INFO" \
        "Session started with parent correlation" || true
    fi
  fi
}

# ---------------------------------------------------------------------------
# 9. Budget status (set by orchestrator via env vars) -- Ref #186
# ---------------------------------------------------------------------------
budget_check() {
  local limit="${ALE_BUDGET_LIMIT:-}"
  local current="${ALE_BUDGET_CURRENT:-}"
  local warn_threshold="${ALE_BUDGET_WARN_THRESHOLD:-0.8}"

  # Only check if budget env vars are set
  if [ -z "$limit" ]; then
    return
  fi

  # Default current to 0 if not set
  current="${current:-0}"

  # Calculate percentage (using awk for float math)
  local pct
  pct=$(awk "BEGIN { if ($limit > 0) printf \"%.0f\", ($current / $limit) * 100; else print 0 }")

  local warn_amount
  warn_amount=$(awk "BEGIN { printf \"%.2f\", $limit * $warn_threshold }")

  if awk "BEGIN { exit !($current >= $limit) }"; then
    ISSUES+=("Budget EXCEEDED: \$${current} / \$${limit} (${pct}%) -- session may be stopped")
  elif awk "BEGIN { exit !($current >= $warn_amount) }"; then
    ISSUES+=("Budget warning: \$${current} / \$${limit} (${pct}%) -- approaching limit")
  else
    INFO+=("Budget: \$${current} / \$${limit} (${pct}%)")
  fi
}

# ---------------------------------------------------------------------------
# 10. Session context injection (computed state for additionalContext)
# ---------------------------------------------------------------------------
session_context_injection() {
  local repo_root
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
  [ -z "$repo_root" ] && return

  local ale_dir=""
  for base in ".ale" "$repo_root/.ale"; do
    [ -d "$base" ] && ale_dir="$base" && break
  done

  # Gather git context
  local branch uncommitted unpushed main_drift
  branch=$(git symbolic-ref --short HEAD 2>/dev/null || echo "unknown")
  uncommitted=$(git status --short 2>/dev/null | grep -c '.' || echo "0")
  unpushed=$(git rev-list origin/main..HEAD --count 2>/dev/null || echo "0")
  main_drift=$(git rev-list HEAD..origin/main --count 2>/dev/null || echo "0")

  # Check file overlap with main for conflict risk
  local overlap_files=""
  if [ "$main_drift" -gt 0 ]; then
    local my_files main_files
    my_files=$(git diff --name-only origin/main...HEAD 2>/dev/null | sort || echo "")
    main_files=$(git diff --name-only HEAD...origin/main 2>/dev/null | sort || echo "")
    if [ -n "$my_files" ] && [ -n "$main_files" ]; then
      overlap_files=$(comm -12 <(echo "$my_files") <(echo "$main_files") 2>/dev/null | head -5 || echo "")
    fi
  fi

  # Recent events for this branch from .ale/events/
  local last_event=""
  if [ -n "$ale_dir" ] && [ -d "$ale_dir/events" ]; then
    local latest_file
    latest_file=$(ls -1 "$ale_dir/events"/*.jsonl 2>/dev/null | sort | tail -1 || true)
    if [ -n "$latest_file" ] && [ -f "$latest_file" ]; then
      last_event=$(grep "\"branch\":\"$branch\"" "$latest_file" 2>/dev/null | tail -1 || echo "")
      if [ -n "$last_event" ]; then
        local evt_type evt_ts
        evt_type=$(echo "$last_event" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
        evt_ts=$(echo "$last_event" | sed -n 's/.*"ts":"\([^"]*\)".*/\1/p')
        last_event="${evt_type} at ${evt_ts}"
      fi
    fi
  fi

  # Check for pending actions from ale-react
  local pending_actions=""
  if [ -n "$ale_dir" ] && [ -f "$ale_dir/cache/pending-actions.jsonl" ]; then
    local action_count
    action_count=$(wc -l < "$ale_dir/cache/pending-actions.jsonl" 2>/dev/null | tr -d ' ')
    if [ "$action_count" -gt 0 ]; then
      local recent_actions=()
      while IFS= read -r line; do
        local action_type
        action_type=$(echo "$line" | sed -n 's/.*"action":"\([^"]*\)".*/\1/p')
        [ -z "$action_type" ] && continue
        recent_actions+=("$action_type")
      done < <(tail -5 "$ale_dir/cache/pending-actions.jsonl")
      if [ "${#recent_actions[@]}" -gt 0 ]; then
        pending_actions=$(printf '%s, ' "${recent_actions[@]}")
        pending_actions="${pending_actions%, }"
      fi
    fi
  fi

  # Check for escalations
  local escalation_count=0
  local esc_dir="$HOME/.ale/escalations"
  if [ -d "$esc_dir" ]; then
    for f in "$esc_dir"/esc-*.json; do
      [ -f "$f" ] || continue
      escalation_count=$((escalation_count + 1))
    done
  fi

  # Build the session context block
  local context_lines=()
  context_lines+=("## Session Context (auto-generated)")
  context_lines+=("- Branch: ${branch} (${uncommitted} uncommitted files, ${unpushed} unpushed commits)")

  if [ "$main_drift" -gt 0 ]; then
    if [ -n "$overlap_files" ]; then
      context_lines+=("- Main drift: ${main_drift} commits behind, FILE OVERLAP DETECTED:")
      while IFS= read -r f; do
        [ -n "$f" ] && context_lines+=("  - $f")
      done <<< "$overlap_files"
    else
      context_lines+=("- Main drift: ${main_drift} commits behind, no file overlap")
    fi
  else
    context_lines+=("- Main drift: up to date")
  fi

  if [ -n "$pending_actions" ]; then
    context_lines+=("- Pending: ${pending_actions}")
  else
    context_lines+=("- Pending: none")
  fi

  if [ "$escalation_count" -gt 0 ]; then
    context_lines+=("- Escalations: ${escalation_count} (review ${esc_dir})")
  else
    context_lines+=("- Escalations: none")
  fi

  if [ -n "$last_event" ]; then
    context_lines+=("- Last event on this branch: ${last_event}")
  fi

  SESSION_CONTEXT=$(printf '%s\n' "${context_lines[@]}")
}

# ---------------------------------------------------------------------------
# Run all checks (best-effort -- never fail)
# ---------------------------------------------------------------------------
STATE_SUMMARY=""
SESSION_CONTEXT=""
stale_check                || true
gh_auth_check              || true
orphan_check               || true
lefthook_check             || true
pending_actions_check      || true
escalation_check           || true
state_injection            || true
session_correlation_check  || true
budget_check               || true
session_context_injection  || true

# ---------------------------------------------------------------------------
# Output summary
# ---------------------------------------------------------------------------
if [ "${#ISSUES[@]}" -eq 0 ] && [ "${#INFO[@]}" -eq 0 ] && [ -z "$STATE_SUMMARY" ] && [ -z "$SESSION_CONTEXT" ]; then
  # Nothing to report
  exit 0
fi

echo ""
echo "ALE Pre-flight Check"
echo "===================="

if [ "${#INFO[@]}" -gt 0 ]; then
  for item in "${INFO[@]}"; do
    echo "  [ok] $item"
  done
fi

if [ "${#ISSUES[@]}" -gt 0 ]; then
  for item in "${ISSUES[@]}"; do
    echo "  [!!] $item"
  done
fi

# Print state summary if available
if [ -n "$STATE_SUMMARY" ]; then
  echo ""
  echo "$STATE_SUMMARY"
fi

# Print session context (the primary output for agent context injection)
if [ -n "$SESSION_CONTEXT" ]; then
  echo ""
  echo "$SESSION_CONTEXT"
fi

echo ""

# Always allow session start
exit 0
