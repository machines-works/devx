#!/usr/bin/env bash
# pre-compact-dump.sh -- Claude Code PreCompact hook
#
# Fires before context compaction (~95% context window usage).
# Saves session state to a dump file and emits an OTEL log event
# so the session can be restored after compaction.
#
# This hook MUST be fast (< 2 seconds) -- compaction waits for it.
#
# Configuration:
#   ALE_PRE_COMPACT_DUMP       -- set to "0" to disable (default: enabled)
#   OTEL_EXPORTER_OTLP_ENDPOINT -- collector endpoint (default: http://localhost:4318)
#   ALE_OTEL_ENABLED           -- set to "0" to skip OTEL export (default: enabled)
#
# Exit 0 = allow compaction to proceed (this hook never blocks)

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_PRE_COMPACT_DUMP:-1}" = "0" ]; then
  exit 0
fi

# Read stdin (hook context JSON from Claude Code)
INPUT=$(cat)

# ---------------------------------------------------------------------------
# 1. Derive timestamp and slug
# ---------------------------------------------------------------------------
TIMESTAMP=$(date +%s)
TS_HUMAN=$(date -u +"%Y-%m-%d %H:%M UTC")
TS_SHORT=$(date -u +"%Y%m%d-%H%M%S")
SLUG="auto-compact-${TS_SHORT}"

# ---------------------------------------------------------------------------
# 2. Gather git state (all best-effort -- never fail on unusual git states)
# ---------------------------------------------------------------------------
BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null || echo "unknown")
GIT_STATUS=$(git status --short 2>/dev/null | head -20 || echo "(unable to read git status)")
RECENT_COMMITS=$(git log --oneline -5 2>/dev/null || echo "(no commits)")
# List active clones (with .ale-clone marker), fall back to legacy worktree list
REPO_ROOT_FOR_CLONES=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
CLONES=""
if [ -n "$REPO_ROOT_FOR_CLONES" ] && [ -d "$REPO_ROOT_FOR_CLONES/.claude/clones" ]; then
  for clone_dir in "$REPO_ROOT_FOR_CLONES/.claude/clones"/*/; do
    [ -d "$clone_dir" ] && [ -f "$clone_dir/.ale-clone" ] || continue
    clone_branch=$(git -C "$clone_dir" branch --show-current 2>/dev/null || echo "unknown")
    CLONES="${CLONES}${clone_dir}  [${clone_branch}]
"
  done
fi
WORKTREES=$(git worktree list 2>/dev/null | tail -n +2 || echo "")
ISOLATION_LIST=""
if [ -n "$CLONES" ]; then
  ISOLATION_LIST="${CLONES}"
fi
if [ -n "$WORKTREES" ]; then
  ISOLATION_LIST="${ISOLATION_LIST}${WORKTREES}
"
fi
if [ -z "$ISOLATION_LIST" ]; then
  ISOLATION_LIST="(no active clones or worktrees)"
fi
REPO_NAME=$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" 2>/dev/null || echo "unknown")

# Count uncommitted changes
UNCOMMITTED=$(echo "$GIT_STATUS" | grep -c '.' 2>/dev/null || echo "0")
if [ "$UNCOMMITTED" = "0" ]; then
  STATUS_SUMMARY="Clean working tree"
else
  STATUS_SUMMARY="${UNCOMMITTED} uncommitted change(s)"
fi

# ---------------------------------------------------------------------------
# 3. Extract session info from hook input (best-effort)
# ---------------------------------------------------------------------------
SESSION_ID=$(echo "$INPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('session_id','unknown'))" 2>/dev/null || echo "unknown")
CONTEXT_PCT=$(echo "$INPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); cw=d.get('context_window',{}); print(100-int(cw.get('remaining_percentage',0)))" 2>/dev/null || echo "unknown")

# ---------------------------------------------------------------------------
# 4. Ensure docs/workflow/ directory exists
# ---------------------------------------------------------------------------
DUMP_DIR="docs/workflow"
mkdir -p "$DUMP_DIR" 2>/dev/null || true

# ---------------------------------------------------------------------------
# 5. Write the dump file (consistent with /ale:dump format)
# ---------------------------------------------------------------------------
DUMP_FILE="${DUMP_DIR}/DUMP-${SLUG}.md"

cat > "$DUMP_FILE" <<DUMPEOF
# Session Dump -- ${SLUG}

> Last updated: ${TS_HUMAN}
> Type: auto-compact (context compaction at ~${CONTEXT_PCT}%)

## Current Task
Auto-saved before context compaction. Review conversation history after restore.

## Key Decisions
- (Preserved automatically -- review conversation transcript for details)

## Files Changed
- (Check \`git status\` and \`git log\` below for recent activity)

## Blockers
- None captured (auto-dump)

## Next Steps
1. Run \`/ale:restore ${SLUG}\` to reload this context
2. Review the conversation history for in-progress work
3. Continue from where compaction interrupted

## Running Agents
(Check \`~/.claude/teams/\` for active agents)

## Branch State
- **Branch:** ${BRANCH}
- **Status:** ${STATUS_SUMMARY}
- **Working tree changes:**
\`\`\`
${GIT_STATUS}
\`\`\`
- **Recent commits:**
\`\`\`
${RECENT_COMMITS}
\`\`\`
- **Active clones/worktrees:**
\`\`\`
${ISOLATION_LIST}
\`\`\`

## Session Metadata
- **Session ID:** ${SESSION_ID}
- **Context usage:** ~${CONTEXT_PCT}%
- **Timestamp:** ${TS_HUMAN}
- **Repository:** ${REPO_NAME}
DUMPEOF

# ---------------------------------------------------------------------------
# 6. Update DUMP-INDEX.md if it exists (or create it)
# ---------------------------------------------------------------------------
INDEX_FILE="${DUMP_DIR}/DUMP-INDEX.md"

if [ ! -f "$INDEX_FILE" ]; then
  cat > "$INDEX_FILE" <<IDXEOF
# Dump Index

| Slug | Last Updated | Summary |
|------|-------------|---------|
| ${SLUG} | ${TS_HUMAN} | Auto-compact dump (context ~${CONTEXT_PCT}%) |
IDXEOF
else
  # Append a new row (simple append -- no dedup needed for auto-compact slugs)
  echo "| ${SLUG} | ${TS_HUMAN} | Auto-compact dump (context ~${CONTEXT_PCT}%) |" >> "$INDEX_FILE"
fi

# ---------------------------------------------------------------------------
# 7. Emit OTEL log event (async, best-effort -- mirrors otel-review-export.sh)
# ---------------------------------------------------------------------------
if [ "${ALE_OTEL_ENABLED:-1}" != "0" ]; then
  OTEL_ENDPOINT="${OTEL_EXPORTER_OTLP_ENDPOINT:-http://localhost:4318}"
  OTEL_LOGS_URL="${OTEL_ENDPOINT}/v1/logs"

  OTEL_PAYLOAD=$(python3 << PYEOF 2>/dev/null || true
import json, time

timestamp_ns = str(int(time.time() * 1e9))

log_record = {
    "timeUnixNano": timestamp_ns,
    "severityNumber": 13,
    "severityText": "WARN",
    "body": {
        "stringValue": json.dumps({
            "type": "compaction.started",
            "session_id": "${SESSION_ID}",
            "branch": "${BRANCH}",
            "context_usage_pct": "${CONTEXT_PCT}",
            "dump_file": "${DUMP_FILE}",
            "dump_slug": "${SLUG}",
            "repo": "${REPO_NAME}"
        })
    },
    "attributes": [
        {"key": "type", "value": {"stringValue": "compaction.started"}},
        {"key": "session_id", "value": {"stringValue": "${SESSION_ID}"}},
        {"key": "branch", "value": {"stringValue": "${BRANCH}"}},
        {"key": "context_usage_pct", "value": {"stringValue": "${CONTEXT_PCT}"}},
        {"key": "dump_file", "value": {"stringValue": "${DUMP_FILE}"}},
        {"key": "dump_slug", "value": {"stringValue": "${SLUG}"}},
        {"key": "repo", "value": {"stringValue": "${REPO_NAME}"}}
    ]
}

otlp_payload = {
    "resourceLogs": [{
        "resource": {
            "attributes": [
                {"key": "service.name", "value": {"stringValue": "claude-code"}},
                {"key": "ale.component", "value": {"stringValue": "pre-compact-dump"}}
            ]
        },
        "scopeLogs": [{
            "scope": {"name": "ale-workflow.pre-compact-dump"},
            "logRecords": [log_record]
        }]
    }]
}

print(json.dumps(otlp_payload))
PYEOF
)

  if [ -n "${OTEL_PAYLOAD:-}" ]; then
    curl -sf -X POST "$OTEL_LOGS_URL" \
      -H "Content-Type: application/json" \
      -d "$OTEL_PAYLOAD" \
      --max-time 2 \
      >/dev/null 2>&1 &
  fi
fi

# ---------------------------------------------------------------------------
# 8. Emit session_compact event via ale-emit (best-effort, async)
# ---------------------------------------------------------------------------

# Find ale-emit (check local .ale/bin, then repo root .ale/bin)
ALE_EMIT=""
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || echo "")
for base in ".ale/bin" "$REPO_ROOT/.ale/bin"; do
  [ -z "$base" ] && continue
  if [ -x "$base/ale-emit" ]; then
    ALE_EMIT="$base/ale-emit"
    break
  fi
done

if [ -n "$ALE_EMIT" ]; then
  # Count unpushed commits (best-effort)
  UNPUSHED=$(git rev-list origin/main..HEAD --count 2>/dev/null || echo "0")
  LAST_COMMIT=$(git log -1 --format='%H %s' 2>/dev/null | head -c 80 || echo "unknown")

  "$ALE_EMIT" session_compact \
    branch="$BRANCH" \
    uncommitted="$UNCOMMITTED" \
    unpushed="$UNPUSHED" \
    dump_slug="$SLUG" \
    dump_file="$DUMP_FILE" \
    context_pct="$CONTEXT_PCT" \
    last_commit="$LAST_COMMIT" \
    2>/dev/null &
fi

# ---------------------------------------------------------------------------
# 9. Notify the user
# ---------------------------------------------------------------------------
echo ""
echo "Context compaction imminent -- auto-dumped state to ${DUMP_FILE}"
echo "Restore with: /ale:restore ${SLUG}"
echo ""

# Always allow compaction to proceed
exit 0
