---
name: ale:heal
description: Reconcile planning state with reality — find and fix stale issues, branches, squads, and docs
allowed-tools:
  - Bash
  - Read
  - Glob
  - Grep
  - AskUserQuestion
---
<objective>
Cross-reference GitHub issues, PRs, git branches, local artifacts, and planning docs to find
mismatches between planning state and reality. Present findings grouped by category, then
execute approved fixes interactively.

Three phases: AUDIT (gather state), FINDINGS (cross-reference), HEAL (interactive fixes).

Does NOT touch code, does NOT force-close without evidence, does NOT modify CLAUDE.md.
</objective>

<process>
## Phase 1: AUDIT — Gather State

Run all of the following in parallel to build a complete picture of the current state.

### 1a: GitHub State

```bash
# Repo identity
OWNER_REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)
REPO_SLUG=$(basename "$(git remote get-url origin 2>/dev/null || basename "$(pwd)")" .git)
echo "OWNER_REPO=$OWNER_REPO"
echo "REPO_SLUG=$REPO_SLUG"
```

```bash
# Open issues (with labels, milestone, linked PRs)
gh issue list --state open --limit 200 --json number,title,labels,milestone,state,body 2>/dev/null
```

```bash
# Closed issues (recent — last 60 days, to check for stale reopens)
gh issue list --state closed --limit 100 --json number,title,closedAt,state 2>/dev/null
```

```bash
# Open PRs
gh pr list --state open --limit 100 --json number,title,headRefName,state,isDraft,baseRefName,body,url 2>/dev/null
```

```bash
# Merged PRs (last 60 days — to correlate with closeable issues)
gh pr list --state merged --limit 100 --json number,title,headRefName,body,mergedAt,url 2>/dev/null
```

```bash
# Active milestones
gh api "repos/$OWNER_REPO/milestones" --jq '.[] | {number, title, state, open_issues, closed_issues, description}' 2>/dev/null
```

### 1b: Git State

```bash
# Local branches (feat/fix/worktree patterns)
git branch --format='%(refname:short) %(objectname:short) %(committerdate:relative) %(upstream:track)' 2>/dev/null | head -50
```

```bash
# Remote branches
git branch -r --format='%(refname:short) %(objectname:short) %(committerdate:relative)' 2>/dev/null | grep -vE 'HEAD|main$|master$' | head -50
```

```bash
# Branches already merged to origin/main
git branch -r --merged origin/main 2>/dev/null | grep -vE 'HEAD|main$|master$' | head -30
```

```bash
# Worktrees
git worktree list 2>/dev/null
```

### 1c: Planning Docs

```bash
# Team briefs and roadmaps
ls -la docs/*/TEAM-BRIEF.md docs/*/ROADMAP.md docs/*/SPLIT.md 2>/dev/null
```

```bash
# Planning markdown files in docs/
find docs/ -name '*.md' -type f 2>/dev/null | head -30
```

### 1d: Ale State

```bash
# Team registries scoped to this repo
REPO_SLUG=$(basename "$(git remote get-url origin 2>/dev/null || basename "$(pwd)")" .git)
for dir in ~/.claude/teams/*"$REPO_SLUG"*/; do
  [ -d "$dir" ] || continue
  team=$(basename "$dir")
  echo "TEAM: $team"
  cat "$dir/config.json" 2>/dev/null | head -20
done
```

```bash
# Task files scoped to this repo
REPO_SLUG=$(basename "$(git remote get-url origin 2>/dev/null || basename "$(pwd)")" .git)
for taskdir in ~/.claude/tasks/*"$REPO_SLUG"*/; do
  [ -d "$taskdir" ] || continue
  team=$(basename "$taskdir")
  echo "=== $team ==="
  for f in "$taskdir"/*.json; do
    [ -f "$f" ] || continue
    python3 -c "
import json, sys
try:
    t = json.load(open('$f'))
except (json.JSONDecodeError, ValueError, FileNotFoundError):
    print(f'  (corrupt: $f)', file=sys.stderr); sys.exit(0)
status = t.get('status', 'unknown')
subject = t.get('subject', '(no subject)')
owner = t.get('owner', '')
task_id = t.get('id', '?')
print(f'  #{task_id}: {subject} -- {status} ({owner})')
" 2>/dev/null
  done
done
```

```bash
# Heartbeat liveness check
REPO_SLUG=$(basename "$(git remote get-url origin 2>/dev/null || basename "$(pwd)")" .git)
~/.claude/hooks/heartbeat.sh check-all "$REPO_SLUG" 2>/dev/null
```

### 1e: Legacy Artifacts

```bash
# GSD files (legacy workflow — should not coexist with Ale)
find . -maxdepth 3 -name '.gsd-*.md' -o -name 'GSD-*.md' -o -name '.handoff-*.md' 2>/dev/null | head -20
```

```bash
# Old dump files
find . -path './.ale/dumps/*' -name 'DUMP-*.md' -type f 2>/dev/null | head -20
ls -la .ale/dumps/ 2>/dev/null
```

## Phase 2: FINDINGS — Cross-Reference and Identify Mismatches

Analyze the audit data and build a findings report organized by category.
For each finding, include evidence (the specific data that proves the mismatch).

### Category 1: Closeable Issues

An issue is closeable when a **merged PR** references it (via "Fixes #N", "Closes #N",
"Ref #N" in the PR title or body) but the issue is still open.

Cross-reference:
- For each open issue, check if any merged PR references its number
- Check the PR body AND title for patterns: `Fixes #N`, `Closes #N`, `Resolves #N`, `Ref #N`
- Also check if the issue's linked branch has been merged

```
CLOSEABLE ISSUES
  #42: "Add retry logic" — closed by PR #55 (merged 3 days ago)
  #38: "Fix auth regression" — branch fix/auth-regression merged to main
```

### Category 2: Stale PRs

A PR is stale when:
- It is open/draft but its head branch has been deleted
- It is open but has had no commits in 30+ days
- It targets a branch that no longer exists (non-main base)

```
STALE PRs
  #60: "WIP: new feature" — draft, branch deleted
  #55: "Old refactor" — no commits in 45 days
```

### Category 3: Merged Branches

Branches (local or remote) that have been fully merged to main and can be deleted.
Exclude `main`, `master`, and the current branch.

```
MERGED BRANCHES
  Local: fix/old-bug, feat/done-feature
  Remote: origin/fix/old-bug, origin/feat/done-feature
```

### Category 4: Dead Squads

Teams in `~/.claude/teams/` whose heartbeat PID is dead (process not running)
and who have incomplete tasks.

```
DEAD SQUADS
  squad-ale-workflow-abc123 — PID 12345 not running, 2/5 tasks incomplete
    ~/.claude/teams/squad-ale-workflow-abc123/
    ~/.claude/tasks/squad-ale-workflow-abc123/
```

### Category 5: Orphaned Worktrees

Git worktrees with no matching active team or branch, or whose branch has been
merged/deleted.

```
ORPHANED WORKTREES
  /path/to/worktree-agent-abc123 — branch deleted
  /path/to/worktree-agent-def456 — team dead, tasks done
```

### Category 6: Stale Planning Docs

Planning documents (team briefs, roadmaps) for milestones that are complete
(all issues closed) or that reference branches/squads that no longer exist.

```
STALE PLANNING DOCS
  docs/v0.1-beta/TEAM-BRIEF.md — milestone v0.1.0-beta is 100% closed
  docs/shadow-mode/ROADMAP.md — all referenced issues are closed
```

### Category 7: Legacy Artifacts

GSD files, old handoff files, or other artifacts from deprecated workflows.

```
LEGACY ARTIFACTS
  .gsd-phase3.md — GSD workflow artifact (Ale replaces GSD)
  .handoff-e2e-sandbox.md — old handoff file
```

### Category 8: Memory Drift

Check MEMORY.md (or ClaudeFMD) for references to completed milestones,
merged branches, or resolved issues that are still mentioned as active/current.

This is informational only — the heal command flags drift but does NOT modify
memory files automatically. The user decides what to update.

```
MEMORY DRIFT
  MEMORY.md references "v0.1.0-beta milestone" — milestone is 100% closed
  MEMORY.md mentions "fix/auth-regression branch" — branch merged and deleted
```

### Findings Summary

```
======================================
 HEAL FINDINGS — $REPO_SLUG
======================================

 Closeable Issues:    N
 Stale PRs:           N
 Merged Branches:     N (N local, N remote)
 Dead Squads:         N
 Orphaned Worktrees:  N
 Stale Planning Docs: N
 Legacy Artifacts:    N
 Memory Drift:        N items

 Total actions available: N
======================================
```

Skip categories with 0 findings. If no findings at all:

```
All clear — planning state matches reality. Nothing to heal.
```

## Phase 3: HEAL — Interactive Approval and Execution

For each category with findings, present the proposed actions and ask for approval
using `AskUserQuestion`. Execute only approved actions.

**Process each category one at a time. Never batch all categories into a single question.**

### For each category:

1. Show the findings for this category
2. Show the proposed fix for each item
3. Ask: "Approve these N fixes? (yes / no / pick)" where:
   - **yes** — execute all proposed fixes in this category
   - **no** — skip this category entirely
   - **pick** — let the user select which items to fix (show numbered list)

### Fix Actions by Category

**Closeable Issues:**
```bash
gh issue close <number> --comment "Closed by /ale:heal — linked PR #<pr> was merged."
```

**Stale PRs:**
```bash
gh pr close <number> --comment "Closed by /ale:heal — branch deleted / stale."
```

**Merged Branches (local):**
```bash
git branch -d <branch-name>
```

**Merged Branches (remote):**
```bash
git push origin --delete <branch-name>
```

**Dead Squads:**
```bash
rm -rf ~/.claude/teams/<team-name>
rm -rf ~/.claude/tasks/<team-name>
```

**Orphaned Worktrees:**
```bash
git worktree remove --force <path>
```

**Stale Planning Docs:**
Present file paths and suggest archiving or deleting. Do not auto-delete — ask the user.

**Legacy Artifacts:**
```bash
rm <file-path>
# Only after explicit user approval per file
```

**Memory Drift:**
Report only. Tell the user which lines/sections are stale and suggest edits,
but do NOT modify memory files automatically. The user owns their memory.

### Completion Summary

After processing all categories, show what was done:

```
======================================
 HEAL COMPLETE — $REPO_SLUG
======================================

 Executed:
   Closed N issues
   Closed N stale PRs
   Deleted N local branches
   Deleted N remote branches
   Removed N dead squads
   Removed N orphaned worktrees

 Skipped:
   N stale planning docs (user skipped)
   N memory drift items (informational only)

 State is now reconciled.
======================================
```
</process>
