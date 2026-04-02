#!/usr/bin/env bash
# Export Codex MCP review data to OTEL collector as structured log entries
#
# Usage: called as a PostToolUse hook for mcp__codex__* tool calls.
#        Stdin receives JSON with tool call details from Claude Code.
#
# Emits OTLP/HTTP log entries to the collector so review findings are
# queryable in HyperDX. Each entry includes session ID, branch, review
# verdict, findings summary, and timestamp.
#
# Configuration:
#   OTEL_EXPORTER_OTLP_ENDPOINT — collector endpoint (default: http://localhost:4318)
#   ALE_OTEL_REVIEW_EXPORT      — set to "0" to disable (default: enabled)
#
# GenAI Semantic Conventions (OTEL):
#   This hook emits standard gen_ai.* attributes alongside legacy custom
#   attributes for backward compatibility. See docs/OBSERVABILITY-STACK.md
#   for the full attribute mapping.
#
# Ref #55, #71

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_OTEL_REVIEW_EXPORT:-1}" = "0" ]; then
  exit 0
fi

OTEL_ENDPOINT="${OTEL_EXPORTER_OTLP_ENDPOINT:-http://localhost:4318}"
OTEL_LOGS_URL="${OTEL_ENDPOINT}/v1/logs"

# Read stdin (tool call JSON from Claude Code PostToolUse hook)
INPUT=$(cat)

# Only export on post-tool (we have the result), skip pre-tool
MODE="${1:-post}"
if [ "$MODE" = "pre" ]; then
  exit 0
fi

# Extract fields and build OTLP payload using python3 (available on macOS)
# Pass INPUT via env var since python heredoc uses stdin
EXPORT_PAYLOAD=$(HOOK_INPUT="$INPUT" python3 << 'PYEOF'
import json, sys, time, re, os

input_data = os.environ.get("HOOK_INPUT", "")
if not input_data:
    sys.exit(0)

try:
    data = json.loads(input_data)
except (json.JSONDecodeError, ValueError):
    sys.exit(0)

tool_name = data.get("tool_name", "")

# Only export codex tool calls (codex, codex-reply)
if not tool_name.startswith("mcp__codex__"):
    sys.exit(0)

# Extract useful fields from the hook payload
session_id = data.get("session_id", "unknown")
tool_use_id = data.get("tool_use_id", "")
tool_input = data.get("tool_input", {})
tool_result = data.get("tool_result", "")
cwd = data.get("cwd", "")

# Session correlation attributes (set by parent orchestrator via env vars)
parent_session_id = os.environ.get("ALE_PARENT_SESSION_ID", "")
team_name = os.environ.get("ALE_TEAM_NAME", "")
agent_role = os.environ.get("ALE_AGENT_ROLE", "")
issue_refs = os.environ.get("ALE_ISSUE_REFS", "")

# Try to determine the branch from cwd (worktree name or git)
branch = ""
if cwd:
    parts = cwd.split("/")
    for i, p in enumerate(parts):
        if p in ("worktrees", "clones") and i + 1 < len(parts):
            branch = parts[i + 1]
            break

# Extract prompt text to detect review-related calls
prompt = ""
if isinstance(tool_input, dict):
    prompt = tool_input.get("prompt", tool_input.get("instructions", ""))
elif isinstance(tool_input, str):
    prompt = tool_input

# Determine if this is a review call (vs general codex usage)
is_review = False
review_keywords = ["review", "diff", "check for", "logic error", "security", "test gap"]
prompt_lower = prompt.lower() if prompt else ""
for kw in review_keywords:
    if kw in prompt_lower:
        is_review = True
        break

# Parse the result to extract verdict and findings
result_text = ""
if isinstance(tool_result, str):
    result_text = tool_result
elif isinstance(tool_result, dict):
    result_text = tool_result.get("text", tool_result.get("content", json.dumps(tool_result)))

verdict = "unknown"
verdict_patterns = ["approve", "request-changes", "needs-discussion"]
result_lower = result_text.lower() if result_text else ""
for v in verdict_patterns:
    if v in result_lower:
        verdict = v
        break

# Count issues by severity
critical_count = len(re.findall(r"\[critical\]", result_lower))
warning_count = len(re.findall(r"\[warning\]", result_lower))
nit_count = len(re.findall(r"\[nit\]", result_lower))
total_issues = critical_count + warning_count + nit_count

# Truncate result for the summary (keep it under 4KB for OTEL)
findings_summary = result_text[:4000] if result_text else ""

# Build OTLP log export payload
timestamp_ns = str(int(time.time() * 1e9))

# GenAI semantic convention attributes (OTEL standard)
# See: opentelemetry.io/docs/specs/semconv/gen-ai
genai_attrs = [
    {"key": "gen_ai.operation.name", "value": {"stringValue": "execute_tool"}},
    {"key": "gen_ai.system", "value": {"stringValue": "anthropic"}},
    {"key": "gen_ai.request.model", "value": {"stringValue": "codex"}},
    {"key": "gen_ai.conversation.id", "value": {"stringValue": session_id}},
    {"key": "gen_ai.agent.name", "value": {"stringValue": "ale-workflow"}},
]

log_record = {
    "timeUnixNano": timestamp_ns,
    "severityNumber": 9,
    "severityText": "INFO",
    "body": {
        "stringValue": json.dumps({
            "type": "codex_review",
            "session_id": session_id,
            "parent_session_id": parent_session_id,
            "team_name": team_name,
            "agent_role": agent_role,
            "issue_refs": issue_refs,
            "tool_name": tool_name,
            "tool_use_id": tool_use_id,
            "branch": branch,
            "is_review": is_review,
            "verdict": verdict,
            "issues": {
                "critical": critical_count,
                "warning": warning_count,
                "nit": nit_count,
                "total": total_issues
            },
            "findings_summary": findings_summary,
            "cwd": cwd
        })
    },
    "attributes":
        genai_attrs + [
        # Legacy custom attributes (backward compat)
        {"key": "type", "value": {"stringValue": "codex_review"}},
        {"key": "session_id", "value": {"stringValue": session_id}},
        {"key": "tool_name", "value": {"stringValue": tool_name}},
        {"key": "tool_use_id", "value": {"stringValue": tool_use_id}},
        {"key": "branch", "value": {"stringValue": branch}},
        {"key": "is_review", "value": {"boolValue": is_review}},
        {"key": "verdict", "value": {"stringValue": verdict}},
        {"key": "issues.critical", "value": {"intValue": str(critical_count)}},
        {"key": "issues.warning", "value": {"intValue": str(warning_count)}},
        {"key": "issues.nit", "value": {"intValue": str(nit_count)}},
        {"key": "issues.total", "value": {"intValue": str(total_issues)}},
        {"key": "cwd", "value": {"stringValue": cwd}},
        # Session correlation attributes (from parent orchestrator env vars)
        {"key": "parent_session_id", "value": {"stringValue": parent_session_id}},
        {"key": "team_name", "value": {"stringValue": team_name}},
        {"key": "agent_role", "value": {"stringValue": agent_role}},
        {"key": "issue_refs", "value": {"stringValue": issue_refs}},
    ]
}

otlp_payload = {
    "resourceLogs": [{
        "resource": {
            "attributes": [
                {"key": "service.name", "value": {"stringValue": "claude-code"}},
                {"key": "gen_ai.system", "value": {"stringValue": "anthropic"}},
                {"key": "ale.component", "value": {"stringValue": "codex-review-hook"}}
            ]
        },
        "scopeLogs": [{
            "scope": {"name": "ale-workflow.codex-review"},
            "logRecords": [log_record]
        }]
    }]
}

print(json.dumps(otlp_payload))
PYEOF
) || exit 0

# Skip if python produced no output (not a codex call, or parse error)
if [ -z "$EXPORT_PAYLOAD" ]; then
  exit 0
fi

# Send to OTEL collector (async, best-effort — don't block the hook)
curl -sf -X POST "$OTEL_LOGS_URL" \
  -H "Content-Type: application/json" \
  -d "$EXPORT_PAYLOAD" \
  --max-time 2 \
  >/dev/null 2>&1 &

exit 0
