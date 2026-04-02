#!/usr/bin/env bash
# session-correlation.sh -- Emit OTEL telemetry events with session correlation attributes
#
# Fires as a SessionStart hook. Emits a session_start log entry with attributes
# that allow correlating this session to its parent tribe/squad session.
#
# Correlation attributes are passed via environment variables set by the
# orchestrator when spawning sub-agents:
#   ALE_PARENT_SESSION_ID  -- session ID of the parent (tribe lead or squad lead)
#   ALE_TEAM_NAME          -- name of the team this session belongs to
#   ALE_AGENT_ROLE         -- role of this agent (tribe-lead, squad-lead, worker)
#   ALE_ISSUE_REFS         -- comma-separated issue numbers (e.g., "42,45,188")
#   ALE_SQUAD_BRANCH       -- squad branch name (e.g., squad/ale-workflow-otel)
#   ALE_WAVE_NUMBER        -- current wave number (for full squad mode)
#
# These env vars are set by the orchestrator (ale:start, ale:tribe, ale:dispatch)
# in the teammate prompt's Branch Verification section and propagated through
# Claude Code's environment inheritance.
#
# Configuration:
#   ALE_SESSION_CORRELATION  -- set to "0" to disable (default: enabled)
#   OTEL_EXPORTER_OTLP_ENDPOINT -- collector endpoint (default: http://localhost:4318)
#
# Exit 0 = always allow (informational only, never blocks)
#
# Ref #188

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_SESSION_CORRELATION:-1}" = "0" ]; then
  exit 0
fi

OTEL_ENDPOINT="${OTEL_EXPORTER_OTLP_ENDPOINT:-http://localhost:4318}"
OTEL_LOGS_URL="${OTEL_ENDPOINT}/v1/logs"

# Read stdin (hook context JSON from Claude Code)
INPUT=$(cat)

# Extract session_id from hook input
SESSION_ID=$(echo "$INPUT" | python3 -c "
import json, sys
try:
    data = json.load(sys.stdin)
    print(data.get('session_id', ''))
except (json.JSONDecodeError, ValueError):
    pass
" 2>/dev/null || echo "")

if [ -z "$SESSION_ID" ]; then
  exit 0
fi

# Read correlation attributes from environment (set by parent orchestrator)
PARENT_SESSION_ID="${ALE_PARENT_SESSION_ID:-}"
TEAM_NAME="${ALE_TEAM_NAME:-}"
AGENT_ROLE="${ALE_AGENT_ROLE:-}"
ISSUE_REFS="${ALE_ISSUE_REFS:-}"
SQUAD_BRANCH="${ALE_SQUAD_BRANCH:-}"
WAVE_NUMBER="${ALE_WAVE_NUMBER:-}"

# Detect branch (best-effort)
BRANCH=$(git branch --show-current 2>/dev/null || echo "")

# Build OTLP log entry with correlation attributes
EXPORT_PAYLOAD=$(python3 << PYEOF
import json, time

timestamp_ns = str(int(time.time() * 1e9))

session_id = "$SESSION_ID"
parent_session_id = "$PARENT_SESSION_ID"
team_name = "$TEAM_NAME"
agent_role = "$AGENT_ROLE"
issue_refs = "$ISSUE_REFS"
squad_branch = "$SQUAD_BRANCH"
wave_number = "$WAVE_NUMBER"
branch = "$BRANCH"

# Determine hierarchy depth
if parent_session_id:
    # Check if this is a worker (has parent) vs a squad lead (is a parent)
    hierarchy_depth = 2 if agent_role == "worker" else 1
else:
    hierarchy_depth = 0  # tribe lead or solo session

# Build structured body
body = {
    "type": "session_start",
    "session_id": session_id,
    "parent_session_id": parent_session_id,
    "team_name": team_name,
    "agent_role": agent_role,
    "issue_refs": issue_refs,
    "squad_branch": squad_branch,
    "wave_number": wave_number,
    "branch": branch,
    "hierarchy_depth": hierarchy_depth,
}

# Build attributes list
attributes = [
    {"key": "type", "value": {"stringValue": "session_start"}},
    {"key": "session_id", "value": {"stringValue": session_id}},
    {"key": "parent_session_id", "value": {"stringValue": parent_session_id}},
    {"key": "team_name", "value": {"stringValue": team_name}},
    {"key": "agent_role", "value": {"stringValue": agent_role}},
    {"key": "issue_refs", "value": {"stringValue": issue_refs}},
    {"key": "squad_branch", "value": {"stringValue": squad_branch}},
    {"key": "branch", "value": {"stringValue": branch}},
    {"key": "hierarchy_depth", "value": {"intValue": str(hierarchy_depth)}},
]

if wave_number:
    attributes.append(
        {"key": "wave_number", "value": {"intValue": str(wave_number)}}
    )

log_record = {
    "timeUnixNano": timestamp_ns,
    "severityNumber": 9,
    "severityText": "INFO",
    "body": {"stringValue": json.dumps(body)},
    "attributes": attributes,
}

otlp_payload = {
    "resourceLogs": [{
        "resource": {
            "attributes": [
                {"key": "service.name", "value": {"stringValue": "claude-code"}},
                {"key": "ale.component", "value": {"stringValue": "session-correlation"}},
            ]
        },
        "scopeLogs": [{
            "scope": {"name": "ale-workflow.session-correlation"},
            "logRecords": [log_record],
        }],
    }]
}

print(json.dumps(otlp_payload))
PYEOF
) || exit 0

if [ -z "$EXPORT_PAYLOAD" ]; then
  exit 0
fi

# Send to OTEL collector (async, best-effort -- do not block session start)
curl -sf -X POST "$OTEL_LOGS_URL" \
  -H "Content-Type: application/json" \
  -d "$EXPORT_PAYLOAD" \
  --max-time 2 \
  >/dev/null 2>&1 &

# Always allow session start
exit 0
