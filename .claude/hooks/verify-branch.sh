#!/usr/bin/env bash
# verify-branch.sh — Claude Code PostToolUse hook for Bash
#
# Fires after Bash tool calls that contain `git commit`. Verifies that
# the current branch and clone/worktree root match the expected values set by
# the squad orchestrator via environment variables.
#
# Environment variables:
#   ALE_EXPECTED_BRANCH    — expected branch name (skip check if unset)
#   ALE_EXPECTED_CLONE     — expected clone root path (preferred, skip check if unset)
#   ALE_EXPECTED_WORKTREE  — legacy alias for ALE_EXPECTED_CLONE (deprecated)
#   ALE_VERIFY_BRANCH      — set to "0" to disable entirely
#   ALE_VERIFY_BRANCH_STRICT — set to "1" to block (exit 2) instead of warn
#
# Exit 0 = allow (default, warning mode)
# Exit 2 = block (strict mode only, on mismatch)

set -euo pipefail

# Allow disabling entirely via env var
if [ "${ALE_VERIFY_BRANCH:-}" = "0" ]; then
  exit 0
fi

# Read JSON input from stdin (PostToolUse tool result details)
INPUT=$(cat)

# Extract the command from the tool input
COMMAND=$(echo "$INPUT" | grep -o '"command"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/"command"[[:space:]]*:[[:space:]]*"//' | sed 's/"$//' || true)

# Only activate when the command contains `git commit`
if ! echo "$COMMAND" | grep -qE '\bgit\b.*\bcommit\b'; then
  exit 0
fi

# --- Post git-commit verification ---

MISMATCH=false
MESSAGES=""

# Check branch if ALE_EXPECTED_BRANCH is set
if [ -n "${ALE_EXPECTED_BRANCH:-}" ]; then
  ACTUAL_BRANCH=$(git branch --show-current 2>/dev/null || echo "")
  if [ "$ACTUAL_BRANCH" != "$ALE_EXPECTED_BRANCH" ]; then
    MISMATCH=true
    MESSAGES="${MESSAGES}Branch mismatch: expected '${ALE_EXPECTED_BRANCH}', got '${ACTUAL_BRANCH}'.\n"
  fi
fi

# Check clone/worktree root path
# Prefer ALE_EXPECTED_CLONE; fall back to legacy ALE_EXPECTED_WORKTREE
EXPECTED_ROOT="${ALE_EXPECTED_CLONE:-${ALE_EXPECTED_WORKTREE:-}}"
if [ -n "$EXPECTED_ROOT" ]; then
  ACTUAL_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
  if [ "$ACTUAL_ROOT" != "$EXPECTED_ROOT" ]; then
    MISMATCH=true
    MESSAGES="${MESSAGES}Clone root mismatch: expected '${EXPECTED_ROOT}', got '${ACTUAL_ROOT}'.\n"
  fi
fi

if [ "$MISMATCH" = false ]; then
  exit 0
fi

# Mismatch detected — warn or block depending on strict mode
if [ "${ALE_VERIFY_BRANCH_STRICT:-}" = "1" ]; then
  echo "ERROR: Post-commit branch/clone verification failed."
  printf "%b" "$MESSAGES"
  echo "Strict mode is enabled (ALE_VERIFY_BRANCH_STRICT=1)."
  echo "The commit landed on the wrong branch or clone."
  echo "Use 'git log --oneline -1' to inspect, then 'git reset HEAD~1' to undo if needed."
  exit 2
fi

# Warning mode (default) — alert but allow
echo "WARNING: Post-commit branch/clone verification mismatch."
printf "%b" "$MESSAGES"
echo "The commit may have landed on the wrong branch."
echo "Verify with 'git log --oneline -1' and 'git branch --show-current'."
exit 0
