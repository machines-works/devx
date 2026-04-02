#!/usr/bin/env bash
# inject-traceparent.sh — Claude Code PreToolUse hook for mcp__codex__* tools
#
# Injects W3C TRACEPARENT context into Codex MCP calls for distributed tracing.
# When Claude Code has OTEL telemetry enabled, this hook:
#   1. Reads the current TRACEPARENT from the hook environment (if available)
#   2. Falls back to constructing one from session_id + tool_use_id
#   3. Writes the traceparent to ~/.claude/otel/traceparent for MCP server pickup
#   4. Emits an OTEL correlation log linking Claude Code and Codex trace contexts
#
# The hook always allows the tool call (exit 0). Tracing is best-effort.
#
# Configuration:
#   ALE_TRACEPARENT_INJECT  — set to "0" to disable (default: enabled)
#   OTEL_EXPORTER_OTLP_ENDPOINT — collector endpoint (default: http://localhost:4318)
#
# Ref #89

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_TRACEPARENT_INJECT:-1}" = "0" ]; then
  exit 0
fi

# Read stdin (PreToolUse JSON from Claude Code)
INPUT=$(cat)

# Extract tool name — only act on mcp__codex__* calls
TOOL_NAME=$(echo "$INPUT" | python3 -c "
import json, sys
try:
    data = json.load(sys.stdin)
    print(data.get('tool_name', ''))
except (json.JSONDecodeError, KeyError, ValueError):
    print('')
" 2>/dev/null || echo "")

if [[ "$TOOL_NAME" != mcp__codex__* ]]; then
  exit 0
fi

# --- Resolve TRACEPARENT ---
#
# Priority:
#   1. TRACEPARENT env var (Claude Code may propagate its OTEL context)
#   2. Construct from session_id + tool_use_id (deterministic, correlatable)
#
# W3C TRACEPARENT format: 00-<trace-id-32hex>-<parent-id-16hex>-<flags-2hex>

TRACEPARENT_DIR="$HOME/.claude/otel"
TRACEPARENT_FILE="$TRACEPARENT_DIR/traceparent"
mkdir -p "$TRACEPARENT_DIR"

RESOLVED_TRACEPARENT=$(HOOK_INPUT="$INPUT" python3 << 'PYEOF'
import hashlib, json, os, sys

input_data = os.environ.get("HOOK_INPUT", "")
if not input_data:
    sys.exit(0)

try:
    data = json.loads(input_data)
except (json.JSONDecodeError, ValueError):
    sys.exit(0)

# Priority 1: TRACEPARENT already in environment
traceparent = os.environ.get("TRACEPARENT", "")
if traceparent:
    print(traceparent)
    sys.exit(0)

# Priority 2: Construct from session_id + tool_use_id
# This creates a deterministic trace-id that can be correlated in the collector
session_id = data.get("session_id", "")
tool_use_id = data.get("tool_use_id", "")

if not session_id:
    sys.exit(0)

# trace-id: 32 hex chars derived from session_id (stable per session)
trace_id = hashlib.md5(session_id.encode()).hexdigest()

# parent-id: 16 hex chars derived from tool_use_id (unique per call)
if tool_use_id:
    parent_id = hashlib.md5(tool_use_id.encode()).hexdigest()[:16]
else:
    # Fallback: use first 16 chars of trace_id
    parent_id = trace_id[:16]

# flags: 01 = sampled
traceparent = f"00-{trace_id}-{parent_id}-01"
print(traceparent)
PYEOF
) || true

if [ -z "$RESOLVED_TRACEPARENT" ]; then
  exit 0
fi

# Write traceparent to file for MCP server wrapper scripts to read
echo "$RESOLVED_TRACEPARENT" > "$TRACEPARENT_FILE"

# --- Emit OTEL correlation log ---
# Links Claude Code's session to the traceparent injected into Codex
OTEL_ENDPOINT="${OTEL_EXPORTER_OTLP_ENDPOINT:-http://localhost:4318}"
OTEL_LOGS_URL="${OTEL_ENDPOINT}/v1/logs"

CORRELATION_PAYLOAD=$(HOOK_INPUT="$INPUT" RESOLVED_TP="$RESOLVED_TRACEPARENT" python3 << 'PYEOF'
import json, os, sys, time

input_data = os.environ.get("HOOK_INPUT", "")
traceparent = os.environ.get("RESOLVED_TP", "")

if not input_data or not traceparent:
    sys.exit(0)

try:
    data = json.loads(input_data)
except (json.JSONDecodeError, ValueError):
    sys.exit(0)

session_id = data.get("session_id", "unknown")
tool_name = data.get("tool_name", "")
tool_use_id = data.get("tool_use_id", "")
cwd = data.get("cwd", "")

# Extract branch from worktree path
branch = ""
if cwd:
    parts = cwd.split("/")
    for i, p in enumerate(parts):
        if p == "worktrees" and i + 1 < len(parts):
            branch = parts[i + 1]
            break

# Parse traceparent components
tp_parts = traceparent.split("-")
trace_id = tp_parts[1] if len(tp_parts) >= 3 else ""
parent_id = tp_parts[2] if len(tp_parts) >= 3 else ""

timestamp_ns = str(int(time.time() * 1e9))

log_record = {
    "timeUnixNano": timestamp_ns,
    "severityNumber": 9,
    "severityText": "INFO",
    "body": {
        "stringValue": json.dumps({
            "type": "traceparent_injection",
            "session_id": session_id,
            "tool_name": tool_name,
            "tool_use_id": tool_use_id,
            "traceparent": traceparent,
            "trace_id": trace_id,
            "parent_id": parent_id,
            "branch": branch,
            "source": "env" if os.environ.get("TRACEPARENT") else "derived"
        })
    },
    "traceId": trace_id,
    "spanId": parent_id,
    "attributes": [
        {"key": "type", "value": {"stringValue": "traceparent_injection"}},
        {"key": "session_id", "value": {"stringValue": session_id}},
        {"key": "tool_name", "value": {"stringValue": tool_name}},
        {"key": "traceparent", "value": {"stringValue": traceparent}},
        {"key": "traceparent.source", "value": {"stringValue": "env" if os.environ.get("TRACEPARENT") else "derived"}},
        {"key": "branch", "value": {"stringValue": branch}},
    ]
}

otlp_payload = {
    "resourceLogs": [{
        "resource": {
            "attributes": [
                {"key": "service.name", "value": {"stringValue": "claude-code"}},
                {"key": "ale.component", "value": {"stringValue": "traceparent-inject-hook"}}
            ]
        },
        "scopeLogs": [{
            "scope": {"name": "ale-workflow.traceparent-inject"},
            "logRecords": [log_record]
        }]
    }]
}

print(json.dumps(otlp_payload))
PYEOF
) || true

# Send correlation log to OTEL collector (async, best-effort)
if [ -n "${CORRELATION_PAYLOAD:-}" ]; then
  curl -sf -X POST "$OTEL_LOGS_URL" \
    -H "Content-Type: application/json" \
    -d "$CORRELATION_PAYLOAD" \
    --max-time 2 \
    >/dev/null 2>&1 &
fi

# Always allow the tool call
exit 0
