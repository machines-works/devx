#!/usr/bin/env bash
# nuke-guard.sh — pre-push hook that blocks branches that would delete most of the codebase
#
# Prevents the classic agent-nuke: agent starts from orphan/empty tree,
# pushes a branch that on merge would delete everything.
#
# Usage: Add to lefthook.yml pre-push, or call directly from a git hook.
#   lefthook:
#     pre-push:
#       commands:
#         nuke-guard:
#           run: bash ~/.claude/hooks/nuke-guard.sh {push_head}
#
# Environment:
#   NUKE_GUARD_MAX_DELETE_PCT  — max % of tracked files that can be deleted (default: 50)
#   NUKE_GUARD_MAX_DELETIONS   — absolute max deleted lines before flagging (default: 10000)
#   NUKE_GUARD_SKIP            — set to "1" to bypass (escape hatch)

set -euo pipefail

# Config
MAX_DELETE_PCT="${NUKE_GUARD_MAX_DELETE_PCT:-50}"
MAX_DELETIONS="${NUKE_GUARD_MAX_DELETIONS:-10000}"

if [ "${NUKE_GUARD_SKIP:-0}" = "1" ]; then
  exit 0
fi

# Determine what we're pushing
# Use git rev-parse HEAD as the reliable default — lefthook's {push_head}
# template variable is NOT interpolated inside multiline `run: |` blocks,
# so $1 may be the literal string "{push_head}".
if [ -n "${1:-}" ] && ! echo "$1" | grep -qF '{'; then
  LOCAL_SHA="$1"
else
  # Fallback: resolve HEAD (works in both primary dir and worktrees)
  LOCAL_SHA=$(git rev-parse HEAD 2>/dev/null || echo "")
  if [ -z "$LOCAL_SHA" ]; then
    exit 0  # can't determine HEAD — skip check
  fi
fi

# Skip if pushing a delete (all zeros)
if echo "$LOCAL_SHA" | grep -qE '^0+$'; then
  exit 0
fi

# Find merge base with main
MAIN_REF="main"
if ! git rev-parse --verify "$MAIN_REF" >/dev/null 2>&1; then
  MAIN_REF="origin/main"
  if ! git rev-parse --verify "$MAIN_REF" >/dev/null 2>&1; then
    # Can't find main — skip check
    exit 0
  fi
fi

MERGE_BASE=$(git merge-base "$MAIN_REF" "$LOCAL_SHA" 2>/dev/null || echo "")

# CHECK 1: No common ancestor with main — orphan branch
if [ -z "$MERGE_BASE" ]; then
  echo ""
  echo "NUKE GUARD: BLOCKED"
  echo "Branch has NO common ancestor with $MAIN_REF."
  echo "This means merging would replace the entire codebase."
  echo "The branch was likely created from an orphan or re-initialized repo."
  echo ""
  echo "To fix: rebase onto main first: git rebase $MAIN_REF"
  echo "To bypass: NUKE_GUARD_SKIP=1 git push"
  exit 1
fi

# CHECK 2: Mass file deletions
TOTAL_FILES=$(git ls-tree -r --name-only "$MAIN_REF" 2>/dev/null | wc -l | tr -d ' ')
if [ "$TOTAL_FILES" -eq 0 ]; then
  exit 0  # empty main, nothing to protect
fi

DELETED_FILES=$(git diff --diff-filter=D --name-only "$MERGE_BASE".."$LOCAL_SHA" 2>/dev/null | wc -l | tr -d ' ')
if [ "$TOTAL_FILES" -gt 0 ]; then
  DELETE_PCT=$((DELETED_FILES * 100 / TOTAL_FILES))
else
  DELETE_PCT=0
fi

if [ "$DELETE_PCT" -gt "$MAX_DELETE_PCT" ]; then
  echo ""
  echo "NUKE GUARD: BLOCKED"
  echo "Branch deletes $DELETED_FILES/$TOTAL_FILES files (${DELETE_PCT}% — threshold: ${MAX_DELETE_PCT}%)."
  echo "This looks like an accidental codebase wipe."
  echo ""
  echo "To bypass: NUKE_GUARD_SKIP=1 git push"
  exit 1
fi

# CHECK 3: Mass line deletions vs additions (catches rewrites)
STATS=$(git diff --shortstat "$MERGE_BASE".."$LOCAL_SHA" 2>/dev/null || echo "")
INSERTIONS=$(echo "$STATS" | grep -oE '[0-9]+ insertion' | grep -oE '[0-9]+' || echo "0")
DELETIONS=$(echo "$STATS" | grep -oE '[0-9]+ deletion' | grep -oE '[0-9]+' || echo "0")

if [ "$DELETIONS" -gt "$MAX_DELETIONS" ] && [ "$DELETIONS" -gt $((INSERTIONS * 10)) ]; then
  echo ""
  echo "NUKE GUARD: BLOCKED"
  echo "Branch has +$INSERTIONS/-$DELETIONS lines (ratio > 10:1 deletions, threshold: $MAX_DELETIONS)."
  echo "This looks like an accidental mass deletion."
  echo ""
  echo "To bypass: NUKE_GUARD_SKIP=1 git push"
  exit 1
fi

# All checks passed
exit 0
