#!/bin/bash
# ralph loop for devx — autonomous iteration with Claude Code
#
# Usage:
#   ./ralph.sh                  # default 10 iterations
#   ./ralph.sh 20               # 20 iterations
#   ./ralph.sh --resume         # resume from where we left off
#
# How it works:
#   1. Reads TASKS.md for the next uncompleted task
#   2. Spawns Claude Code with fresh context to implement it
#   3. Runs cargo test + cargo clippy to validate
#   4. If passing, marks the task done and commits
#   5. Loops until all tasks complete or max iterations reached
#
# State is persisted in:
#   - TASKS.md        — task list with [x] completion markers
#   - progress.txt    — append-only log of what happened each iteration
#   - git history     — each completed task = one commit

set -euo pipefail

MAX_ITERATIONS="${1:-10}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROGRESS_FILE="$SCRIPT_DIR/progress.txt"
TASKS_FILE="$SCRIPT_DIR/TASKS.md"

# ── Preflight ──────────────────────────────────────────────────

if ! command -v claude &>/dev/null; then
  echo "error: claude CLI not found. Install Claude Code first."
  exit 1
fi

if [[ ! -f "$TASKS_FILE" ]]; then
  echo "error: TASKS.md not found. Create it with tasks like:"
  echo ""
  echo "  - [ ] Add daemon mode (devx up -d)"
  echo "  - [ ] Add devx logs command"
  echo "  - [ ] Strip ANSI codes from log output"
  echo ""
  exit 1
fi

# ── Helpers ────────────────────────────────────────────────────

get_next_task() {
  grep -n '^\- \[ \]' "$TASKS_FILE" | head -1 | sed 's/^[0-9]*:- \[ \] //'
}

count_remaining() {
  grep -c '^\- \[ \]' "$TASKS_FILE" 2>/dev/null || echo 0
}

count_completed() {
  grep -c '^\- \[x\]' "$TASKS_FILE" 2>/dev/null || echo 0
}

mark_task_done() {
  local task="$1"
  # Escape special regex chars in the task description
  local escaped
  escaped=$(printf '%s' "$task" | sed 's/[[\.*^$()+?{|]/\\&/g')
  sed -i '' "s/- \[ \] ${escaped}/- [x] ${escaped}/" "$TASKS_FILE"
}

log_progress() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" >> "$PROGRESS_FILE"
}

# ── Main Loop ──────────────────────────────────────────────────

echo "ralph loop — devx"
echo "tasks remaining: $(count_remaining)"
echo "tasks completed: $(count_completed)"
echo "max iterations:  $MAX_ITERATIONS"
echo ""

for ((i = 1; i <= MAX_ITERATIONS; i++)); do
  TASK=$(get_next_task)

  if [[ -z "$TASK" ]]; then
    echo "all tasks complete!"
    log_progress "ALL TASKS COMPLETE after $((i - 1)) iterations"
    exit 0
  fi

  REMAINING=$(count_remaining)
  echo "━━━ iteration $i/$MAX_ITERATIONS ━━━ ($REMAINING remaining)"
  echo "task: $TASK"
  echo ""
  log_progress "iteration $i — starting: $TASK"

  # Build the prompt for Claude Code
  PROMPT=$(cat <<EOF
You are working on the devx project — a Rust CLI tool for local dev stack orchestration.

Your task for this iteration:
  $TASK

Context:
- Project root: $SCRIPT_DIR
- Read TASKS.md to understand the full roadmap
- Read progress.txt to see what previous iterations learned
- Read existing source code before making changes

Rules:
1. Implement ONLY this one task. Do not touch other tasks.
2. Run \`cargo test --all\` and \`cargo clippy -- -D warnings\` to validate.
3. If tests pass, commit with a descriptive message.
4. If you learn something useful for future iterations, append it to progress.txt.
5. Do NOT modify TASKS.md — the harness handles that.

When done, end your response with exactly: <done>COMPLETE</done>
If you cannot complete the task, end with: <done>BLOCKED: reason</done>
EOF
  )

  # Run Claude Code with fresh context
  OUTPUT=$(echo "$PROMPT" | claude --dangerously-skip-permissions -p 2>&1) || true

  # Check result
  if echo "$OUTPUT" | grep -q '<done>COMPLETE</done>'; then
    echo "✓ task completed"
    log_progress "iteration $i — COMPLETED: $TASK"

    # Verify tests actually pass
    if cargo test --all --quiet 2>&1 | tail -1 | grep -q "ok"; then
      mark_task_done "$TASK"
      echo ""
    else
      echo "⚠ tests failing after completion claim — not marking done"
      log_progress "iteration $i — tests failed despite completion claim"
    fi

  elif echo "$OUTPUT" | grep -q '<done>BLOCKED:'; then
    REASON=$(echo "$OUTPUT" | grep -o '<done>BLOCKED:.*</done>' | sed 's/<done>BLOCKED: *//;s/<\/done>//')
    echo "✗ blocked: $REASON"
    log_progress "iteration $i — BLOCKED: $TASK — $REASON"
  else
    echo "? no completion signal — moving on"
    log_progress "iteration $i — no signal: $TASK"
  fi

  echo ""
done

REMAINING=$(count_remaining)
if [[ "$REMAINING" -gt 0 ]]; then
  echo "max iterations reached. $REMAINING tasks remaining."
  log_progress "MAX ITERATIONS ($MAX_ITERATIONS) reached. $REMAINING tasks remaining."
  exit 1
else
  echo "all tasks complete!"
  exit 0
fi
