#!/usr/bin/env bash
# session-stop.sh -- Claude Code Stop hook
#
# Fires when a Claude Code session ends (user quits, timeout, etc.).
# Emits a session_pause event so the next session can pick up context.
#
# This hook MUST be fast (< 2 seconds) -- it runs during shutdown.
#
# Configuration:
#   ALE_SESSION_STOP  -- set to "0" to disable (default: enabled)
#
# Exit 0 = always allow (never blocks shutdown)

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_SESSION_STOP:-1}" = "0" ]; then
  exit 0
fi

# Read stdin (hook context JSON from Claude Code)
INPUT=$(cat)

# ---------------------------------------------------------------------------
# 1. Find ale-emit
# ---------------------------------------------------------------------------
ALE_EMIT=""
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
for base in ".ale/bin" "$REPO_ROOT/.ale/bin"; do
  [ -z "$base" ] && continue
  if [ -x "$base/ale-emit" ]; then
    ALE_EMIT="$base/ale-emit"
    break
  fi
done

if [ -z "$ALE_EMIT" ]; then
  exit 0
fi

# ---------------------------------------------------------------------------
# 2. Gather session state
# ---------------------------------------------------------------------------
BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "unknown")
UNCOMMITTED=$(git status --short 2>/dev/null | grep -c '.' || echo "0")
UNPUSHED=$(git rev-list origin/main..HEAD --count 2>/dev/null || echo "0")
LAST_COMMIT=$(git log -1 --format='%H %s' 2>/dev/null | head -c 80 || echo "unknown")

# Extract session ID from hook input (best-effort)
SESSION_ID=$(echo "$INPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('session_id','unknown'))" 2>/dev/null || echo "unknown")

# ---------------------------------------------------------------------------
# 3. Emit session_pause event
# ---------------------------------------------------------------------------
"$ALE_EMIT" session_pause \
  branch="$BRANCH" \
  uncommitted="$UNCOMMITTED" \
  unpushed="$UNPUSHED" \
  last_commit="$LAST_COMMIT" \
  session_id="$SESSION_ID" \
  2>/dev/null || true

# Always allow shutdown
exit 0
