#!/usr/bin/env bash
# lifecycle-spans.sh -- Emit OTEL lifecycle span events for the orchestration pipeline
#
# Records start/end events for 5 span types:
#   tribe_dispatch  -- task decomposition time (tribe lead breaking work into tasks)
#   squad_formation -- worktree/clone creation, branch setup
#   agent_work      -- first tool call to last commit
#   pr_creation     -- agent completion to PR opened
#   review_merge    -- PR open to merge
#
# Usage:
#   hooks/lifecycle-spans.sh <span_type> <phase> [key=value ...]
#
# Examples:
#   hooks/lifecycle-spans.sh tribe_dispatch start team_id=ale-workflow-auth task_id=8
#   hooks/lifecycle-spans.sh tribe_dispatch end team_id=ale-workflow-auth task_id=8
#   hooks/lifecycle-spans.sh squad_formation start agent_id=auth-worker branch_name=feat/auth
#   hooks/lifecycle-spans.sh agent_work start agent_id=auth-worker model=claude-opus-4-6
#   hooks/lifecycle-spans.sh pr_creation end agent_id=auth-worker pr_number=42
#   hooks/lifecycle-spans.sh review_merge start pr_number=42 branch_name=feat/auth
#
# All key=value pairs are forwarded to otel_emit_lifecycle. The required attributes
# (agent_id, branch_name, team_id, task_id, model) should be passed as key=value pairs.
# Missing attributes default to environment variables or "unknown".
#
# Configuration:
#   OTEL_EXPORTER_OTLP_ENDPOINT -- collector HTTP endpoint (no emission if unset)
#   ALE_OTEL_HOOK_EXPORT        -- set to "0" to disable
#   ALE_PARENT_SESSION_ID       -- parent session for correlation
#   ALE_TEAM_NAME               -- team name for correlation
#   ALE_AGENT_ROLE              -- agent role
#   ALE_ISSUE_REFS              -- issue references
#   ALE_SQUAD_BRANCH            -- squad branch name
#
# Exit 0 always (informational only, never blocks).
#
# Ref #299

set -euo pipefail

# Source the shared OTEL emit helper
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/otel-emit.sh
source "$SCRIPT_DIR/lib/otel-emit.sh"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

if [ $# -lt 2 ]; then
  echo "Usage: lifecycle-spans.sh <span_type> <phase> [key=value ...]" >&2
  echo "" >&2
  echo "Span types: tribe_dispatch, squad_formation, agent_work, pr_creation, review_merge" >&2
  echo "Phases: start, end" >&2
  exit 1
fi

SPAN_TYPE="$1"
PHASE="$2"
shift 2

# Validate span type
case "$SPAN_TYPE" in
  tribe_dispatch|squad_formation|agent_work|pr_creation|review_merge) ;;
  *)
    echo "Error: Unknown span type '$SPAN_TYPE'" >&2
    echo "Valid types: tribe_dispatch, squad_formation, agent_work, pr_creation, review_merge" >&2
    exit 1
    ;;
esac

# Validate phase
case "$PHASE" in
  start|end) ;;
  *)
    echo "Error: Unknown phase '$PHASE'. Must be 'start' or 'end'." >&2
    exit 1
    ;;
esac

# Build human-readable message based on span type and phase
case "${SPAN_TYPE}:${PHASE}" in
  tribe_dispatch:start)  MESSAGE="Task decomposition started" ;;
  tribe_dispatch:end)    MESSAGE="Task decomposition completed" ;;
  squad_formation:start) MESSAGE="Squad formation started (worktree/branch setup)" ;;
  squad_formation:end)   MESSAGE="Squad formation completed" ;;
  agent_work:start)      MESSAGE="Agent work started" ;;
  agent_work:end)        MESSAGE="Agent work completed" ;;
  pr_creation:start)     MESSAGE="PR creation started" ;;
  pr_creation:end)       MESSAGE="PR creation completed" ;;
  review_merge:start)    MESSAGE="Review and merge process started" ;;
  review_merge:end)      MESSAGE="Review and merge completed" ;;
esac

# Emit the lifecycle span event (all extra args forwarded as key=value pairs)
otel_emit_lifecycle "$SPAN_TYPE" "$PHASE" "$MESSAGE" "$@"

# Also record to local event log if ale-emit is available
ALE_EMIT="${ALE_DIR:-.ale}/bin/ale-emit"
if [ -x "$ALE_EMIT" ]; then
  "$ALE_EMIT" "lifecycle_${SPAN_TYPE}_${PHASE}" "$@" 2>/dev/null &
  disown 2>/dev/null || true
fi

exit 0
