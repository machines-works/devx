#!/usr/bin/env bash
# context-watchdog.sh -- Claude Code PostToolUse hook (all tools)
#
# Tracks approximate context budget usage by counting tool calls per session.
# When the count exceeds a configurable threshold, emits a warning suggesting
# the agent run /compact with a handoff summary.
#
# This is a heuristic -- each tool call adds to the context window (input JSON
# + output). After enough calls, the context window will be close to capacity.
# The pre-compact-dump hook handles the actual compaction event; this hook
# warns BEFORE that point so agents can compact proactively with a good
# handoff summary instead of relying on auto-compaction.
#
# State is tracked per-session in a temp file keyed by PPID (the Claude Code
# process). State files are cleaned up automatically by the OS (/tmp).
#
# Configuration (environment variables):
#   ALE_CONTEXT_WATCHDOG         -- set to "0" to disable (default: enabled)
#   ALE_WATCHDOG_WARN_THRESHOLD  -- tool call count to trigger first warning (default: 80)
#   ALE_WATCHDOG_CRITICAL_THRESHOLD -- tool call count for critical warning (default: 120)
#   ALE_WATCHDOG_REMIND_INTERVAL -- re-warn every N calls after threshold (default: 20)
#
# Configuration (ale.config.yaml):
#   context_watchdog.warn_threshold      -- overrides ALE_WATCHDOG_WARN_THRESHOLD
#   context_watchdog.critical_threshold  -- overrides ALE_WATCHDOG_CRITICAL_THRESHOLD
#   context_watchdog.remind_interval     -- overrides ALE_WATCHDOG_REMIND_INTERVAL
#
# Exit 0 = always allow (informational only, never blocks)

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_CONTEXT_WATCHDOG:-1}" = "0" ]; then
  exit 0
fi

# Read stdin (PostToolUse hook context JSON from Claude Code)
INPUT=$(cat)

# ---------------------------------------------------------------------------
# 1. Read config from ale.config.yaml (best-effort, fall back to env/defaults)
# ---------------------------------------------------------------------------
read_yaml_value() {
  local key="$1"
  local default="$2"
  local config_file=""

  # Try project-local config first, then global defaults
  if [ -f ".claude/ale.config.yaml" ]; then
    config_file=".claude/ale.config.yaml"
  elif [ -f "$HOME/.claude/ale/defaults.yaml" ]; then
    config_file="$HOME/.claude/ale/defaults.yaml"
  fi

  if [ -n "$config_file" ]; then
    local val
    val=$(grep -E "^\s*${key}:" "$config_file" 2>/dev/null | head -1 | sed 's/.*:\s*//' | tr -d ' "' || echo "")
    if [ -n "$val" ] && [ "$val" != "~" ] && [ "$val" != "null" ]; then
      echo "$val"
      return
    fi
  fi

  echo "$default"
}

# Resolve thresholds: env var > yaml config > hardcoded default
WARN_THRESHOLD="${ALE_WATCHDOG_WARN_THRESHOLD:-$(read_yaml_value warn_threshold 80)}"
CRITICAL_THRESHOLD="${ALE_WATCHDOG_CRITICAL_THRESHOLD:-$(read_yaml_value critical_threshold 120)}"
REMIND_INTERVAL="${ALE_WATCHDOG_REMIND_INTERVAL:-$(read_yaml_value remind_interval 20)}"

# ---------------------------------------------------------------------------
# 2. Track tool call count in a session-scoped temp file
# ---------------------------------------------------------------------------
# PPID is the Claude Code node process (parent of this bash shell).
# Each session gets its own counter file.
# ALE_WATCHDOG_SESSION_ID can override for testing (PPID is readonly in bash).
SESSION_KEY="${ALE_WATCHDOG_SESSION_ID:-$PPID}"
STATE_FILE="/tmp/ale-watchdog-${SESSION_KEY}.count"

# Read current count (or start at 0)
if [ -f "$STATE_FILE" ]; then
  COUNT=$(cat "$STATE_FILE" 2>/dev/null || echo "0")
  # Validate it's a number
  if ! echo "$COUNT" | grep -qE '^[0-9]+$'; then
    COUNT=0
  fi
else
  COUNT=0
fi

# Increment
COUNT=$((COUNT + 1))

# Write back
echo "$COUNT" > "$STATE_FILE"

# ---------------------------------------------------------------------------
# 3. Check thresholds and emit warnings
# ---------------------------------------------------------------------------

# No warning needed yet
if [ "$COUNT" -lt "$WARN_THRESHOLD" ]; then
  exit 0
fi

# Determine if we should warn on this call (first threshold hit, or reminder interval)
SHOULD_WARN=false

if [ "$COUNT" -eq "$WARN_THRESHOLD" ]; then
  SHOULD_WARN=true
elif [ "$COUNT" -eq "$CRITICAL_THRESHOLD" ]; then
  SHOULD_WARN=true
elif [ "$COUNT" -gt "$WARN_THRESHOLD" ]; then
  # After first warning, remind every REMIND_INTERVAL calls
  CALLS_SINCE_WARN=$(( (COUNT - WARN_THRESHOLD) % REMIND_INTERVAL ))
  if [ "$CALLS_SINCE_WARN" -eq 0 ]; then
    SHOULD_WARN=true
  fi
fi

if [ "$SHOULD_WARN" = false ]; then
  exit 0
fi

# ---------------------------------------------------------------------------
# 4. Emit context_warning event via ale-emit (best-effort, non-blocking)
# ---------------------------------------------------------------------------
WARN_LEVEL="warn"
if [ "$COUNT" -ge "$CRITICAL_THRESHOLD" ]; then
  WARN_LEVEL="critical"
fi

# Find ale-emit
ALE_EMIT=""
for p in ".ale/bin/ale-emit" "$(git rev-parse --show-toplevel 2>/dev/null)/.ale/bin/ale-emit"; do
  if [ -x "$p" ] 2>/dev/null; then ALE_EMIT="$p"; break; fi
done
if [ -n "$ALE_EMIT" ]; then
  "$ALE_EMIT" context_warning level="$WARN_LEVEL" tool_calls="$COUNT" 2>/dev/null &
  disown 2>/dev/null || true
fi

# ---------------------------------------------------------------------------
# 5. Print the appropriate warning to the agent
# ---------------------------------------------------------------------------

# Extract context window info if available in the hook input
REMAINING_PCT=$(echo "$INPUT" | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)
    cw = d.get('context_window', {})
    print(cw.get('remaining_percentage', ''))
except:
    print('')
" 2>/dev/null || echo "")

CONTEXT_INFO=""
if [ -n "$REMAINING_PCT" ]; then
  CONTEXT_INFO=" (context remaining: ~${REMAINING_PCT}%)"
fi

echo ""

if [ "$COUNT" -ge "$CRITICAL_THRESHOLD" ]; then
  echo "CONTEXT WATCHDOG: CRITICAL -- ${COUNT} tool calls this session${CONTEXT_INFO}"
  echo "Context window is likely near capacity. Run /compact NOW with a handoff summary."
  echo ""
  echo "Suggested /compact message:"
  echo "  /compact Handoff: [describe current task state, what's done, what's next,"
  echo "  key decisions made, files changed, and any blockers]"
  echo ""
  echo "This preserves your progress across the context compaction boundary."
elif [ "$COUNT" -ge "$WARN_THRESHOLD" ]; then
  echo "CONTEXT WATCHDOG: WARNING -- ${COUNT} tool calls this session${CONTEXT_INFO}"
  echo "Consider running /compact soon to preserve context before auto-compaction."
  echo ""
  echo "Tip: Include a handoff summary with /compact so you can resume cleanly:"
  echo "  /compact Working on [task]. Done: [x, y]. Next: [z]. Key files: [a, b]."
fi

echo ""

# Always allow -- this hook is informational only
exit 0
