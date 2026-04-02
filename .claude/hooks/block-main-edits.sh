#!/usr/bin/env bash
# block-main-edits.sh — Claude Code PreToolUse hook for Write, Edit, and Bash
#
# Blocks file edits when on the main/master branch in the primary working
# directory. Clones (where .ale-clone marker exists) are allowed.
# Legacy worktrees (where .git is a file, not a directory) are also allowed
# during the transition period (remove in v2.1.0).
#
# For Write/Edit tools: checks the TARGET FILE PATH — if it's inside a
# clone, worktree, or outside the repo entirely, the edit is allowed.
# For Bash tool: only blocks commands containing file-writing patterns on
# main in the primary repo. Read-only commands (gh queries, git log/status,
# grep, cat, ls, docker compose ps, etc.) are allowed through.
#
# False-positive avoidance (Ref #296):
#   - Filesystem mutations (rm, mkdir, etc.) targeting paths outside the repo
#     are allowed (e.g., ~/.claude/teams/..., /tmp/...).
#   - git push --delete (remote branch cleanup) is allowed.
#   - git worktree remove (worktree cleanup) is allowed.
#   - ALE_AGENT_ROLE env var: tribe-lead and orchestrator roles get selective
#     bypass for ops-safe commands (docker, outside-repo cleanup, worktree
#     management). Repo file mutations remain blocked even for these roles.
#
# Tool detection uses the "tool_name" field from JSON input to reliably
# distinguish Bash from Write/Edit calls — avoids false positives when
# command extraction fails on complex JSON.
#
# This hook ONLY runs on Claude Code tool calls — humans in their terminal
# are unaffected.
#
# Exit 0 = allow, exit 2 = block with message

set -euo pipefail

# Source OTEL emission helper (best-effort, never fails)
HOOK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/otel-emit.sh
source "$HOOK_DIR/lib/otel-emit.sh" 2>/dev/null || true

# Read JSON input from stdin (tool call details)
INPUT=$(cat)

# Check if a given path is inside an isolated environment (clone or legacy
# worktree) or outside any repo. Traverses from the path upward looking for
# .ale-clone marker or .git file (legacy worktree signal).
# Returns 0 (true) if the path is safe to edit.
# Returns 1 (false) if the path is inside the primary repo (.git is a dir,
# no .ale-clone marker found).
path_is_isolated() {
  local target="$1"

  # Resolve relative paths against CWD
  if [[ "$target" != /* ]]; then
    target="$(pwd)/$target"
  fi

  # If the target is a directory, use it; otherwise use its parent
  local dir
  if [ -d "$target" ]; then
    dir="$target"
  else
    dir="$(dirname "$target")"
  fi

  # Traverse upward looking for isolation markers
  while [ "$dir" != "/" ]; do
    if [ -f "$dir/.ale-clone" ]; then
      # .ale-clone marker found → this is a clone → allow
      return 0
    fi
    if [ -f "$dir/.git" ] && [ ! -d "$dir/.git" ]; then
      # .git is a file → legacy worktree → allow (transition period)
      return 0
    fi
    if [ -d "$dir/.git" ]; then
      # .git is a directory with no .ale-clone above it → primary repo → block
      return 1
    fi
    dir="$(dirname "$dir")"
  done

  # Reached / without finding .git → path is outside any repo → allow
  return 0
}

# Check if a given path is outside the current repo root.
# Returns 0 (true) if the path does NOT start with the repo root.
# Returns 1 (false) if the path is inside the repo or cannot be determined.
path_is_outside_repo() {
  local target="$1"
  local repo_root
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null || echo "")

  # If we can't determine the repo root, assume inside (safe default)
  if [ -z "$repo_root" ]; then
    return 1
  fi

  # Resolve the repo root to its physical path (handles symlinks like /tmp -> /private/tmp)
  if [ -d "$repo_root" ]; then
    repo_root=$(cd "$repo_root" && pwd -P)
  fi

  # Expand ~ to $HOME for comparison
  if [[ "$target" == "~/"* ]]; then
    target="$HOME/${target#\~/}"
  elif [[ "$target" == "~" ]]; then
    target="$HOME"
  fi

  # Resolve relative paths against CWD (using physical path to handle symlinks)
  if [[ "$target" != /* ]]; then
    target="$(pwd -P)/$target"
  fi

  # If the target directory exists, resolve symlinks for accurate comparison
  local target_dir
  if [ -d "$target" ]; then
    target_dir=$(cd "$target" && pwd -P)
    target="$target_dir"
  elif [ -d "$(dirname "$target")" ]; then
    target_dir=$(cd "$(dirname "$target")" && pwd -P)
    target="$target_dir/$(basename "$target")"
  fi

  # Check if the target starts with the repo root
  if [[ "$target" == "$repo_root"* ]]; then
    return 1  # inside repo
  fi

  return 0  # outside repo
}

# Check if ALL path-like arguments in a filesystem mutation command target
# paths outside the repo. Returns 0 if all targets are outside the repo
# (safe to allow), 1 otherwise.
# Usage: all_targets_outside_repo "rm -rf /path/to/something"
all_targets_outside_repo() {
  local cmd="$1"
  local found_path=false
  local all_outside=true

  # Split command into words and check each non-flag argument
  local -a words
  read -ra words <<< "$cmd"

  # Skip the first word (the command itself, e.g., rm, mkdir)
  for word in "${words[@]:1}"; do
    # Skip flags (single or double dash)
    if [[ "$word" == -* ]]; then
      continue
    fi

    # Skip non-path arguments: chmod modes (+x, 755, u+rw), operators (&&, ||, ;)
    if [[ "$word" == +* ]] || [[ "$word" =~ ^[0-9]+$ ]] || [[ "$word" == "&&" ]] || [[ "$word" == "||" ]] || [[ "$word" == ";" ]]; then
      continue
    fi

    # This looks like a path argument
    found_path=true
    if ! path_is_outside_repo "$word"; then
      all_outside=false
      break
    fi
  done

  # If no paths found, assume targeting current dir (inside repo)
  if [ "$found_path" = false ]; then
    return 1
  fi

  if [ "$all_outside" = true ]; then
    return 0
  fi
  return 1
}

# Clone detection: .ale-clone marker = isolated clone = allow writes.
# Legacy worktree detection: .git as file (not dir) = allow writes.
# Primary repo: .git as directory with no .ale-clone = block writes.
if [ -f ".ale-clone" ]; then
  exit 0  # clone — allow
fi
if [ -f ".git" ] && [ ! -d ".git" ]; then
  exit 0  # legacy worktree — allow (transition period, remove in v2.1.0)
fi
if [ ! -d ".git" ]; then
  exit 0  # not a git repo at all — allow
fi

# Check current branch
BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "")

if [ "$BRANCH" != "main" ] && [ "$BRANCH" != "master" ]; then
  exit 0
fi

# --- We are on main/master in the primary repo ---

# Detect tool type from tool_name field in JSON input.
# This is more reliable than inferring from input fields, because a failed
# extraction of "command" would otherwise fall through to the Write/Edit blocker.
TOOL_NAME=$(echo "$INPUT" | grep -o '"tool_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/"tool_name"[[:space:]]*:[[:space:]]*"//' | sed 's/"$//' || true)

# Extract file_path for Write/Edit tool calls
FILE_PATH=$(echo "$INPUT" | grep -o '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/"file_path"[[:space:]]*:[[:space:]]*"//' | sed 's/"$//' || true)

# For Write/Edit tool calls, check the target path instead of CWD
if [ -n "$FILE_PATH" ]; then
  if path_is_isolated "$FILE_PATH"; then
    exit 0
  fi
fi

# Detect if this is a Bash tool call by checking tool_name or the presence of a "command" field.
# Use tool_name when available (reliable), fall back to field detection.
IS_BASH=false
if [ "$TOOL_NAME" = "Bash" ]; then
  IS_BASH=true
elif echo "$INPUT" | grep -q '"command"'; then
  IS_BASH=true
fi

if [ "$IS_BASH" = true ]; then
  # Extract command value. The grep pattern stops at the first unescaped quote,
  # which truncates commands containing escaped quotes (e.g., echo \"hello\").
  # This is acceptable — truncated commands are checked against write patterns,
  # and the truncated prefix is sufficient for detection. If extraction fails
  # entirely (empty COMMAND), we default to ALLOW since most Bash calls are
  # read-only (git status, gh queries, grep, etc.).
  COMMAND=$(echo "$INPUT" | grep -o '"command"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/"command"[[:space:]]*:[[:space:]]*"//' | sed 's/"$//' || true)

  # If we couldn't extract the command at all, default to allow.
  # The alternative (blocking) would cause false positives on every Bash call
  # with complex JSON that our grep can't parse.
  if [ -z "$COMMAND" ]; then
    exit 0
  fi

  # Role-based bypass for Bash commands: tribe-lead and orchestrator agents
  # need to run ops/maintenance commands (docker, cleanup, worktree teardown)
  # on main. Workers remain fully blocked. This bypass is SELECTIVE — it only
  # allows ops-safe patterns, NOT arbitrary repo file mutations.
  # Only applies to Bash — Write/Edit tools are always blocked on main
  # regardless of role (tribe leads coordinate, they don't implement).
  # Ref #296, #311, constitutional audit E6/G6.
  AGENT_ROLE="${ALE_AGENT_ROLE:-}"
  IS_OPS_ROLE=false
  if [ "$AGENT_ROLE" = "tribe-lead" ] || [ "$AGENT_ROLE" = "orchestrator" ]; then
    IS_OPS_ROLE=true
  fi

  # Extract cd target if command starts with "cd /path &&" or "cd /path ;"
  # When an agent runs "cd /worktree && git add", the session CWD is still on
  # main, but the actual write targets the worktree. We check the cd target
  # with the same path_is_isolated() logic used for Write/Edit tools.
  CD_TARGET=""
  if [[ "$COMMAND" =~ ^cd[[:space:]]+([^[:space:]\&\;]+) ]]; then
    CD_TARGET="${BASH_REMATCH[1]}"
    # Resolve relative paths
    if [[ "$CD_TARGET" != /* ]]; then
      CD_TARGET="$(pwd)/$CD_TARGET"
    fi
  fi

  # If command targets an isolated environment (clone/worktree) via cd, allow it
  if [ -n "$CD_TARGET" ] && path_is_isolated "$CD_TARGET"; then
    exit 0
  fi

  # Only block commands that contain file-writing patterns.
  # Allow non-write commands (git, gh, npm, test runners, etc.) to pass through.

  # --- Whitelisted git operations (Ref #296) ---
  # git push --delete / git push origin --delete: remote branch cleanup, not a local write
  if echo "$COMMAND" | grep -qE '\bgit\b\s+push\b.*--delete\b'; then
    exit 0
  fi
  # git worktree remove: worktree cleanup, not a repo file write
  if echo "$COMMAND" | grep -qE '\bgit\b\s+worktree\s+remove\b'; then
    exit 0
  fi

  # Strip harmless redirects before checking write patterns:
  #   N>/dev/null, N>>/dev/null  — fd redirect to /dev/null (not a real file write)
  #   N>&M                       — fd duplication (e.g., 2>&1)
  SANITIZED=$(echo "$COMMAND" | sed -E 's/[0-9]*>>[&]?\/dev\/null//g; s/[0-9]*>[&]?\/dev\/null//g; s/[0-9]+>&[0-9]+//g')

  BLOCKED=false

  # cat > or cat >> (file write via cat redirection)
  if echo "$SANITIZED" | grep -qE '\bcat\b.*>{1,2}'; then
    BLOCKED=true
  fi

  # echo/printf > or >> (file write via echo/printf redirection)
  if echo "$SANITIZED" | grep -qE '\b(echo|printf)\b.*>{1,2}'; then
    BLOCKED=true
  fi

  # tee (writes stdin to files)
  if echo "$SANITIZED" | grep -qE '\btee\b'; then
    BLOCKED=true
  fi

  # sed -i (in-place file edit)
  if echo "$SANITIZED" | grep -qE '\bsed\b\s+-i'; then
    BLOCKED=true
  fi

  # awk with redirect (awk '...' > file)
  if echo "$SANITIZED" | grep -qE '\bawk\b.*>{1,2}'; then
    BLOCKED=true
  fi

  # dd (can write to files)
  if echo "$SANITIZED" | grep -qE '\bdd\b\s.*of='; then
    BLOCKED=true
  fi

  # cp / mv into tracked paths (file creation/overwrite)
  # Only block if targeting paths inside the repo (Ref #296).
  if echo "$SANITIZED" | grep -qE '\b(cp|mv)\b\s'; then
    if ! all_targets_outside_repo "$COMMAND"; then
      BLOCKED=true
    fi
  fi

  # rm / mkdir / touch / chmod / chown (filesystem mutations)
  # Only block if targeting paths inside the repo (Ref #296).
  if echo "$COMMAND" | grep -qE '\b(rm|mkdir|touch|chmod|chown)\b\s'; then
    if ! all_targets_outside_repo "$COMMAND"; then
      BLOCKED=true
    fi
  fi

  # git write operations (add, commit, push, merge, rebase, reset, stash)
  if echo "$COMMAND" | grep -qE '\bgit\b\s+(add|commit|push|merge|rebase|reset|stash|cherry-pick|revert|tag|checkout\s+-b)\b'; then
    BLOCKED=true
  fi

  # install command (Unix install — copies files with permissions)
  # Exclude package manager install commands (bun/npm/yarn/pnpm/pip/cargo/gem/go install)
  if echo "$SANITIZED" | grep -qE '\binstall\b' && ! echo "$SANITIZED" | grep -qE '(bun|npm|yarn|pnpm|pip|cargo|gem|go)\s+install'; then
    BLOCKED=true
  fi

  # Heredoc redirect (cat <<EOF > file, cat <<'EOF' > file)
  if echo "$SANITIZED" | grep -qE "<<['\"]?[A-Za-z].*>{1,2}"; then
    BLOCKED=true
  fi

  # Bare redirect at start of command or after pipe/semicolon (> file)
  if echo "$SANITIZED" | grep -qE '(^|[|;&])\s*>{1,2}\s*\S'; then
    BLOCKED=true
  fi

  # Role-based ops-safe bypass (Ref #311): tribe-lead and orchestrator roles
  # can run ops commands that would otherwise be blocked, as long as the command
  # is an ops-safe pattern (docker, filesystem mutations outside repo, worktree
  # cleanup). Actual repo file mutations remain blocked even for these roles.
  if [ "$BLOCKED" = true ] && [ "$IS_OPS_ROLE" = true ]; then
    OPS_SAFE=false

    # docker compose / docker-compose commands (start, stop, up, down, rm, etc.)
    if echo "$COMMAND" | grep -qE '\b(docker[[:space:]]+compose|docker-compose)\b'; then
      OPS_SAFE=true
    fi

    # Filesystem mutations (rm, mkdir, touch, chmod, chown) targeting ONLY
    # paths outside the repo — these are ops cleanup, not repo edits
    if echo "$COMMAND" | grep -qE '\b(rm|mkdir|touch|chmod|chown)\b\s'; then
      if all_targets_outside_repo "$COMMAND"; then
        OPS_SAFE=true
      fi
    fi

    # git worktree add/remove/prune — worktree lifecycle management
    if echo "$COMMAND" | grep -qE '\bgit\b\s+worktree\s+(add|remove|prune)\b'; then
      OPS_SAFE=true
    fi

    # git push --delete — remote branch cleanup
    if echo "$COMMAND" | grep -qE '\bgit\b\s+push\b.*--delete\b'; then
      OPS_SAFE=true
    fi

    # git branch -d/-D — local branch cleanup
    if echo "$COMMAND" | grep -qE '\bgit\b\s+branch\b\s+-(d|D)\b'; then
      OPS_SAFE=true
    fi

    if [ "$OPS_SAFE" = true ]; then
      exit 0
    fi
  fi

  if [ "$BLOCKED" = true ]; then
    otel_emit_log "block-main-edits" "AX_BLOCK_MAIN_BASH" "WARN" \
      "Cannot write files via Bash while on main branch." \
      ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "$COMMAND" | head -c 200 | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
      2>/dev/null || true
    echo "[AX_BLOCK_MAIN_BASH] BLOCKED: Cannot write files via Bash while on main branch." >&2
    echo "This guard protects repository state — it blocks file writes, not just git commits." >&2

    # Pattern-specific guidance: suggest the best no-write alternative for each pattern
    if echo "$COMMAND" | grep -qE '\bsed\b\s+-i'; then
      echo "Tip: Use the Edit tool instead of sed -i — it does not require a branch." >&2
    elif echo "$COMMAND" | grep -qE '\b(cp|mv)\b\s'; then
      echo "Tip: cp/mv modify repo files. Create a clone with /ale:fix first to work in an isolated branch." >&2
    elif echo "$COMMAND" | grep -qE '\btee\b|>{1,2}'; then
      echo "Tip: If passing content to a CLI tool, use stdin instead (e.g., --body-file - or echo '...' | cmd)." >&2
    elif echo "$COMMAND" | grep -qE '\bgit\b\s+(add|commit|push)\b'; then
      echo "Tip: git write commands require a feature branch. Create a clone with /ale:fix first." >&2
    elif echo "$COMMAND" | grep -qE '\b(rm|mkdir|touch)\b\s'; then
      echo "Tip: Filesystem mutations are blocked on main. Create a clone with /ale:fix first." >&2
    else
      echo "Tip: If you only need to pass content to a CLI tool, use stdin instead (e.g., --body-file - or echo '...' | cmd)." >&2
    fi

    echo "Remediation: Create a clone with /ale:fix <description> to work in an isolated branch." >&2
    echo "Docs: https://github.com/Sharpi-AI/ale-workflow#clone-isolation" >&2
    exit 2
  fi

  # Not a write pattern — allow through
  exit 0
fi

# This is a Write or Edit tool call — block entirely on main.
otel_emit_log "block-main-edits" "AX_BLOCK_MAIN_WRITE" "WARN" \
  "Cannot edit files while on main branch." \
  ",{\"key\": \"hook.command\", \"value\": {\"stringValue\": \"$(printf '%s' "${FILE_PATH:-unknown}" | sed 's/\\/\\\\/g; s/"/\\"/g')\"}}" \
  2>/dev/null || true
echo "[AX_BLOCK_MAIN_WRITE] BLOCKED: Cannot edit files while on main branch." >&2
echo "This guard protects repository state — it blocks file writes, not just git commits." >&2
echo "Note: Files outside the repo (e.g., /tmp) are not blocked — only repo-tracked paths." >&2
echo "Remediation: Create a clone with /ale:fix <description> to work in an isolated branch." >&2
echo "Docs: https://github.com/Sharpi-AI/ale-workflow#clone-isolation" >&2
exit 2
