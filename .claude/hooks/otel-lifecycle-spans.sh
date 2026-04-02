#!/usr/bin/env bash
# otel-lifecycle-spans.sh -- Emit OTEL spans for orchestration lifecycle phases
#
# Instruments the dispatch->work->PR->merge pipeline with structured OTEL spans
# so we can compute orchestration metrics (cycle time, bottleneck detection, etc.).
#
# Usage:
#   otel-lifecycle-spans.sh start <span-type> [key=value ...]
#   otel-lifecycle-spans.sh end   <span-type> [key=value ...]
#   otel-lifecycle-spans.sh emit  <span-type> <duration_ms> [key=value ...]
#
# Span types:
#   tribe-dispatch    -- task decomposition time
#   squad-formation   -- worktree/clone creation, branch setup
#   agent-work        -- first tool call to last commit
#   pr-creation       -- agent completion to PR opened
#   review-merge      -- PR open to merge
#
# Required attributes (passed as key=value args):
#   agent_id, branch_name, team_id, task_id, model
#
# "start" writes a span context file (trace_id, span_id, start_time) to a temp dir.
# "end" reads the context file, calculates duration, and emits the span.
# "emit" sends a complete span with explicit duration (for cases where start/end
#   is impractical, e.g., computed from git timestamps).
#
# Span nesting (parent-child):
#   tribe-dispatch
#     -> squad-formation
#       -> agent-work
#       -> pr-creation
#   review-merge (standalone -- starts after PR is opened)
#
# Configuration:
#   OTEL_EXPORTER_OTLP_ENDPOINT -- collector endpoint (default: http://localhost:4318)
#   ALE_OTEL_LIFECYCLE_SPANS    -- set to "0" to disable (default: enabled)
#
# Span context files: $ALE_SPANS_DIR/<team_id>/<span-type>.json
#   Default: ~/.claude/spans/
#
# Ref #299

set -euo pipefail

# Allow disabling via env var
if [ "${ALE_OTEL_LIFECYCLE_SPANS:-1}" = "0" ]; then
  exit 0
fi

OTEL_ENDPOINT="${OTEL_EXPORTER_OTLP_ENDPOINT:-http://localhost:4318}"
OTEL_TRACES_URL="${OTEL_ENDPOINT}/v1/traces"
SPANS_DIR="${ALE_SPANS_DIR:-$HOME/.claude/spans}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Generate a random hex string of N bytes (2N hex chars)
rand_hex() {
  local bytes="${1:-16}"
  python3 -c "import os; print(os.urandom($bytes).hex())" 2>/dev/null
}

# Current time in nanoseconds (epoch)
now_ns() {
  python3 -c "import time; print(int(time.time() * 1e9))" 2>/dev/null
}

# Valid span types
VALID_TYPES="tribe-dispatch squad-formation agent-work pr-creation review-merge"

validate_span_type() {
  local span_type="$1"
  for t in $VALID_TYPES; do
    if [ "$t" = "$span_type" ]; then
      return 0
    fi
  done
  echo "ERROR: Invalid span type '$span_type'. Valid types: $VALID_TYPES" >&2
  exit 1
}

# Map span type to its parent span type for nesting
get_parent_span_type() {
  local span_type="$1"
  case "$span_type" in
    agent-work)      echo "squad-formation" ;;
    pr-creation)     echo "squad-formation" ;;
    squad-formation) echo "tribe-dispatch" ;;
    *)               echo "" ;;
  esac
}

# Extract a key=value pair from argument list
extract_attr() {
  local key="$1"
  shift
  for arg in "$@"; do
    case "$arg" in
      "${key}"=*) echo "${arg#*=}"; return ;;
    esac
  done
  echo ""
}

# ---------------------------------------------------------------------------
# build_otlp_payload -- construct an OTLP trace payload via python3
# All span construction goes through this single function.
# ---------------------------------------------------------------------------
build_otlp_payload() {
  local trace_id="$1"
  local span_id="$2"
  local parent_span_id="$3"
  local span_type="$4"
  local start_time_ns="$5"
  local end_time_ns="$6"
  shift 6
  # Remaining args are key=value attribute pairs

  HOOK_TRACE_ID="$trace_id" HOOK_SPAN_ID="$span_id" \
    HOOK_PARENT_SPAN_ID="$parent_span_id" HOOK_SPAN_TYPE="$span_type" \
    HOOK_START_NS="$start_time_ns" HOOK_END_NS="$end_time_ns" \
    HOOK_KV_ATTRS="$*" \
    python3 << 'PYEOF'
import json, os

trace_id = os.environ["HOOK_TRACE_ID"]
span_id = os.environ["HOOK_SPAN_ID"]
parent_span_id = os.environ.get("HOOK_PARENT_SPAN_ID", "")
span_type = os.environ["HOOK_SPAN_TYPE"]
start_ns = os.environ["HOOK_START_NS"]
end_ns = os.environ["HOOK_END_NS"]
kv_attrs = os.environ.get("HOOK_KV_ATTRS", "")

# Parse key=value pairs into OTLP attributes
attributes = [
    {"key": "ale.span_type", "value": {"stringValue": span_type}},
    {"key": "ale.component", "value": {"stringValue": "orchestration-lifecycle"}},
]
for pair in kv_attrs.split():
    if "=" not in pair:
        continue
    k, v = pair.split("=", 1)
    # Skip team_id from attributes (it's used for context file scoping, not OTEL)
    # but still include it as an attribute for queryability
    attributes.append({"key": k, "value": {"stringValue": v}})

span = {
    "traceId": trace_id,
    "spanId": span_id,
    "name": "ale.orchestration." + span_type,
    "kind": 1,  # SPAN_KIND_INTERNAL
    "startTimeUnixNano": start_ns,
    "endTimeUnixNano": end_ns,
    "attributes": attributes,
    "status": {"code": 1}  # STATUS_CODE_OK
}

if parent_span_id:
    span["parentSpanId"] = parent_span_id

payload = {
    "resourceSpans": [{
        "resource": {
            "attributes": [
                {"key": "service.name", "value": {"stringValue": "claude-code"}},
                {"key": "ale.component", "value": {"stringValue": "orchestration-lifecycle"}}
            ]
        },
        "scopeSpans": [{
            "scope": {"name": "ale-workflow.orchestration"},
            "spans": [span]
        }]
    }]
}

print(json.dumps(payload))
PYEOF
}

# Send payload to OTEL collector (async, best-effort -- never blocks)
send_payload() {
  local payload="$1"
  if [ -z "$payload" ]; then
    return 0
  fi
  curl -sf -X POST "$OTEL_TRACES_URL" \
    -H "Content-Type: application/json" \
    -d "$payload" \
    --max-time 2 \
    >/dev/null 2>&1 &
}

# ---------------------------------------------------------------------------
# start -- begin a span (write context file for later "end")
# ---------------------------------------------------------------------------
cmd_start() {
  local span_type="$1"
  shift
  validate_span_type "$span_type"

  local team_id
  team_id=$(extract_attr "team_id" "$@")
  team_id="${team_id:-default}"

  local ctx_dir="$SPANS_DIR/$team_id"
  mkdir -p "$ctx_dir"

  # Check if parent span exists to inherit trace_id
  local parent_type trace_id="" parent_span_id=""
  parent_type=$(get_parent_span_type "$span_type")

  if [ -n "$parent_type" ] && [ -f "$ctx_dir/${parent_type}.json" ]; then
    trace_id=$(python3 -c "import json; print(json.load(open('$ctx_dir/${parent_type}.json'))['trace_id'])" 2>/dev/null || echo "")
    parent_span_id=$(python3 -c "import json; print(json.load(open('$ctx_dir/${parent_type}.json'))['span_id'])" 2>/dev/null || echo "")
  fi

  # Generate new IDs if no parent or parent read failed
  if [ -z "$trace_id" ]; then
    trace_id=$(rand_hex 16)
  fi
  local span_id start_time
  span_id=$(rand_hex 8)
  start_time=$(now_ns)

  # Write context file
  python3 -c "
import json
ctx = {
    'trace_id': '$trace_id',
    'span_id': '$span_id',
    'parent_span_id': '$parent_span_id',
    'start_time_ns': '$start_time',
    'span_type': '$span_type',
    'team_id': '$team_id'
}
with open('$ctx_dir/${span_type}.json', 'w') as f:
    json.dump(ctx, f)
" 2>/dev/null

  # Output context for callers that want it
  echo "{\"trace_id\":\"$trace_id\",\"span_id\":\"$span_id\"}"
}

# ---------------------------------------------------------------------------
# end -- finish a span (read context, calculate duration, emit)
# ---------------------------------------------------------------------------
cmd_end() {
  local span_type="$1"
  shift
  validate_span_type "$span_type"

  local team_id
  team_id=$(extract_attr "team_id" "$@")
  team_id="${team_id:-default}"

  local ctx_file="$SPANS_DIR/$team_id/${span_type}.json"
  if [ ! -f "$ctx_file" ]; then
    echo "ERROR: No start context found for span '$span_type' (team: $team_id)" >&2
    echo "Hint: Call 'otel-lifecycle-spans.sh start $span_type team_id=$team_id' first" >&2
    exit 1
  fi

  local end_time
  end_time=$(now_ns)

  # Read span context
  local trace_id span_id parent_span_id start_time_ns
  trace_id=$(python3 -c "import json; print(json.load(open('$ctx_file'))['trace_id'])" 2>/dev/null)
  span_id=$(python3 -c "import json; print(json.load(open('$ctx_file'))['span_id'])" 2>/dev/null)
  parent_span_id=$(python3 -c "import json; print(json.load(open('$ctx_file')).get('parent_span_id',''))" 2>/dev/null)
  start_time_ns=$(python3 -c "import json; print(json.load(open('$ctx_file'))['start_time_ns'])" 2>/dev/null)

  if [ -z "$trace_id" ] || [ -z "$span_id" ] || [ -z "$start_time_ns" ]; then
    echo "ERROR: Corrupt span context for '$span_type' (team: $team_id)" >&2
    rm -f "$ctx_file"
    exit 1
  fi

  # Build and send payload
  local payload
  payload=$(build_otlp_payload "$trace_id" "$span_id" "$parent_span_id" \
    "$span_type" "$start_time_ns" "$end_time" "$@") || exit 0

  send_payload "$payload"

  # Clean up context file
  rm -f "$ctx_file"
}

# ---------------------------------------------------------------------------
# emit -- send a complete span with explicit duration (no start/end files)
# ---------------------------------------------------------------------------
cmd_emit() {
  local span_type="$1"
  local duration_ms="$2"
  shift 2
  validate_span_type "$span_type"

  local team_id
  team_id=$(extract_attr "team_id" "$@")
  team_id="${team_id:-default}"

  local ctx_dir="$SPANS_DIR/$team_id"

  # Check for parent span to inherit trace context
  local parent_type trace_id="" parent_span_id=""
  parent_type=$(get_parent_span_type "$span_type")

  if [ -n "$parent_type" ] && [ -f "$ctx_dir/${parent_type}.json" ]; then
    trace_id=$(python3 -c "import json; print(json.load(open('$ctx_dir/${parent_type}.json'))['trace_id'])" 2>/dev/null || echo "")
    parent_span_id=$(python3 -c "import json; print(json.load(open('$ctx_dir/${parent_type}.json'))['span_id'])" 2>/dev/null || echo "")
  fi

  if [ -z "$trace_id" ]; then
    trace_id=$(rand_hex 16)
  fi
  local span_id end_ns start_ns
  span_id=$(rand_hex 8)
  end_ns=$(now_ns)
  start_ns=$(python3 -c "print(int($end_ns) - ($duration_ms * 1000000))" 2>/dev/null)

  local payload
  payload=$(build_otlp_payload "$trace_id" "$span_id" "$parent_span_id" \
    "$span_type" "$start_ns" "$end_ns" "$@") || exit 0

  send_payload "$payload"
}

# ---------------------------------------------------------------------------
# Main dispatch
# ---------------------------------------------------------------------------
CMD="${1:-}"
if [ -z "$CMD" ]; then
  echo "Usage: otel-lifecycle-spans.sh {start|end|emit} <span-type> [key=value ...]" >&2
  exit 1
fi
shift

case "$CMD" in
  start) cmd_start "$@" ;;
  end)   cmd_end "$@" ;;
  emit)  cmd_emit "$@" ;;
  *)
    echo "Usage: otel-lifecycle-spans.sh {start|end|emit} <span-type> [key=value ...]" >&2
    exit 1
    ;;
esac
