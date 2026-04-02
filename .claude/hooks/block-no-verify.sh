#!/usr/bin/env bash
# block-no-verify.sh — Claude Code PreToolUse hook for Bash
#
# Blocks agent bypass vectors for git guards:
#   --no-verify     — skips all lefthook hooks
#   --no-gpg-sign   — bypasses signing requirements
#   LEFTHOOK=0      — disables lefthook entirely
#   LEFTHOOK_EXCLUDE — selectively disables hooks
#   NUKE_GUARD_SKIP — bypasses nuke-guard safety check
#
# This hook ONLY runs on Claude Code Bash tool calls — humans in their
# terminal are unaffected. --no-verify remains the human escape hatch.
#
# Exit 0 = allow, exit 2 = block with message
#
# The hook receives JSON on stdin with the tool call details.

set -euo pipefail

# Source OTEL emission helper (best-effort, never fails)
HOOK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/otel-emit.sh
source "$HOOK_DIR/lib/otel-emit.sh" 2>/dev/null || true

INPUT=$(cat)

# Extract the command being run
COMMAND=$(echo "$INPUT" | grep -o '"command"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/"command"[[:space:]]*:[[:space:]]*"//' | sed 's/"$//')

# --- Block environment variable bypasses (any command, not just git) ---

# Block LEFTHOOK=0 or LEFTHOOK=false (disables all lefthook hooks)
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)LEFTHOOK=(0|false)'; then
  otel_emit_log "block-no-verify" "AX_BLOCK_GUARD_BYPASS" "WARN" "LEFTHOOK=0 is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_GUARD_BYPASS] BLOCKED: LEFTHOOK=0 is prohibited — this disables all git guards." >&2
  echo "Remediation: If a git hook is blocking you, use AskUserQuestion to escalate to the human operator." >&2
  exit 2
fi

# Block LEFTHOOK_EXCLUDE=... (selectively disables hooks)
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)LEFTHOOK_EXCLUDE='; then
  otel_emit_log "block-no-verify" "AX_BLOCK_GUARD_BYPASS" "WARN" "LEFTHOOK_EXCLUDE is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_GUARD_BYPASS] BLOCKED: LEFTHOOK_EXCLUDE is prohibited — this disables specific git guards." >&2
  echo "Remediation: If a git hook is blocking you, use AskUserQuestion to escalate to the human operator." >&2
  exit 2
fi

# Block NUKE_GUARD_SKIP=1 (bypasses nuke-guard safety)
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)NUKE_GUARD_SKIP=(1|true)'; then
  otel_emit_log "block-no-verify" "AX_BLOCK_GUARD_BYPASS" "WARN" "NUKE_GUARD_SKIP is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_GUARD_BYPASS] BLOCKED: NUKE_GUARD_SKIP is prohibited — this bypasses push safety checks." >&2
  echo "Remediation: If nuke-guard is blocking your push, use AskUserQuestion to escalate to the human operator." >&2
  exit 2
fi

# --- Block dangerous system commands ---

# Block gh api pushes to refs (bypasses all local hooks)
if echo "$COMMAND" | grep -qE 'gh\s+api.*refs'; then
  otel_emit_log "block-no-verify" "AX_BLOCK_GUARD_BYPASS" "WARN" "Direct GitHub API ref manipulation is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_GUARD_BYPASS] BLOCKED: Direct GitHub API ref manipulation is prohibited." >&2
  echo "Remediation: Use git push through a worktree so hooks can validate the push." >&2
  exit 2
fi

# Block chflags/chattr unlock (prevent agents from unlocking protected files)
if echo "$COMMAND" | grep -qE 'chflags.*(noschg|nouchg)|chattr.*-i'; then
  otel_emit_log "block-no-verify" "AX_BLOCK_GUARD_BYPASS" "WARN" "Unlocking protected files is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_GUARD_BYPASS] BLOCKED: Unlocking protected files is prohibited." >&2
  echo "Remediation: Only the human operator can unlock immutable files." >&2
  exit 2
fi

# --- Block git-specific bypass flags ---

# Skip remaining checks if not a git command
if ! echo "$COMMAND" | grep -qE '\bgit\b'; then
  exit 0
fi

# Block git config tampering (bare=true, identity spoofing)
if echo "$COMMAND" | grep -qE 'git\s+config\s+(core\.bare|user\.(email|name))'; then
  otel_emit_log "block-no-verify" "AX_BLOCK_GUARD_BYPASS" "WARN" "Modifying git core.bare or user identity is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_GUARD_BYPASS] BLOCKED: Modifying git core.bare or user identity is prohibited." >&2
  echo "Remediation: Use AskUserQuestion to escalate to the human operator." >&2
  exit 2
fi

# Block --no-verify (skips pre-commit, pre-push, etc.)
if echo "$COMMAND" | grep -qE '\-\-no-verify'; then
  otel_emit_log "block-no-verify" "AX_BLOCK_NO_VERIFY" "WARN" "--no-verify is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_NO_VERIFY] BLOCKED: --no-verify is prohibited." >&2
  echo "Git hooks exist to protect the codebase. Do not bypass them." >&2
  echo "Remediation: If a hook is blocking you, use AskUserQuestion to escalate to the human operator." >&2
  exit 2
fi

# Block --no-gpg-sign (can be used to bypass signing requirements)
if echo "$COMMAND" | grep -qE '\-\-no-gpg-sign'; then
  otel_emit_log "block-no-verify" "AX_BLOCK_NO_VERIFY" "WARN" "--no-gpg-sign is prohibited" \
    ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
    2>/dev/null || true
  echo "[AX_BLOCK_NO_VERIFY] BLOCKED: --no-gpg-sign is prohibited." >&2
  echo "Remediation: Use AskUserQuestion to escalate to the human operator." >&2
  exit 2
fi

# Block -n shorthand for --no-verify in git commit/push
if echo "$COMMAND" | grep -qE 'git\s+(commit|push)\s.*\s-[a-zA-Z]*n'; then
  # Be more careful: -n in git commit means --no-verify, but in other
  # commands it means different things. Only block for commit/push.
  if echo "$COMMAND" | grep -qE 'git\s+(commit|push)\s.*\b-n\b'; then
    otel_emit_log "block-no-verify" "AX_BLOCK_NO_VERIFY" "WARN" "-n (--no-verify shorthand) is prohibited" \
      ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
      2>/dev/null || true
    echo "[AX_BLOCK_NO_VERIFY] BLOCKED: -n (--no-verify shorthand) is prohibited for git commit/push." >&2
    echo "Remediation: Use AskUserQuestion to escalate to the human operator." >&2
    exit 2
  fi
fi

exit 0
