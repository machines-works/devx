#!/usr/bin/env bash
# Log Codex MCP tool calls to JSONL
# Usage: called by Claude Code hooks with "pre" or "post" as $1
# Stdin receives JSON with tool call details

set -euo pipefail

LOG_DIR="$HOME/.claude/logs"
LOG_FILE="$LOG_DIR/codex-mcp-calls.jsonl"
mkdir -p "$LOG_DIR"

MODE="${1:-unknown}"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Read stdin (tool call JSON from Claude Code hooks)
INPUT=$(cat)

# Enrich with timestamp and mode, write JSONL
echo "$INPUT" | jq -c --arg ts "$TIMESTAMP" --arg mode "$MODE" '. + {timestamp: $ts, hook_mode: $mode}' >> "$LOG_FILE" 2>/dev/null || \
  echo "{\"timestamp\":\"$TIMESTAMP\",\"hook_mode\":\"$MODE\",\"raw\":\"parse_error\"}" >> "$LOG_FILE"
