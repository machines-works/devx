---
name: ale:start
description: Start work — solo agent, lean squad, or full team with gates
argument-hint: "<description or milestone> [--solo | --team] [--spike] [--discovery] [--brief | --issues #N ... | --milestone <name-or-number>]"
allowed-tools:
  - Read
  - Bash
  - Task(squad-worker-mcp, codex-reviewer, pr-creator, Explore)
  - EnterWorktree
  - TeamCreate
  - TeamDelete
  - TaskCreate
  - TaskList
  - TaskGet
  - TaskUpdate
  - SendMessage
  - AskUserQuestion
  - mcp__codex__codex
  - mcp__codex__codex-reply
---
<objective>
Start work using one of three execution modes:

- **Solo** (`--solo`): Single autonomous agent, no team overhead. Best for bug fixes and small features.
- **Lean** (default): Delegate aggressively, skip gate ceremony. Auto-switches to solo if only 1 task.
- **Full squad** (`--team`): Full team with DAG execution, quality enforcement, and draft PR. For large coordinated work.

You are the **orchestrator**. You pick the mode, load context, and drive execution.
</objective>

<hard-rules>
## CRITICAL — Orchestrator Rules

1. **NEVER investigate code yourself.** Delegate to Explore sub-agents. Only short git/gh commands allowed.
2. **NEVER merge to main.** All changes go through PRs via pr-creator.
3. **NEVER use `--no-verify`.** NEVER bypass hooks (`LEFTHOOK=0`, `NUKE_GUARD_SKIP=1`).
4. **Respond to blockers immediately.** Acknowledge, triage (provide info / reassign / escalate to human), track resolution.
5. **Workers branch FROM squad branch** (`origin/squad/<team-name>`), not `origin/main` — except solo mode.
6. **Verify completion evidence.** Every worker must provide test evidence and review status before merge.
</hard-rules>

<context>
Input: $ARGUMENTS
Review Strategy: !`grep "review:" .claude/ale.config.yaml -A 5 2>/dev/null | grep "strategy:" | awk '{print $2}' || echo "codex"`

## Parse Mode

### Step 1: Extract the execution mode flag (first match wins)

| Flag | Mode | When to use |
|------|------|-------------|
| `--solo` | **solo** | Force single agent, no team |
| `--team` | **full squad** | Force full team with gates, waves, draft PR |
| *(no flag)* | **lean** | Default -- delegate aggressively, skip gate ceremony |

Remove the mode flag from `$ARGUMENTS` before parsing context flags below.

### Step 1a: Check for `--spike` and `--discovery` flags

Remove these flags from `$ARGUMENTS` before parsing context flags. Both are orthogonal
to execution mode and context flags.

- **`--spike`**: Set `SPIKE_MODE=true`. Branch prefix becomes `spike/`, workers produce
  evidence instead of production code, review gates skipped, no PR (issue comment instead).
- **`--discovery`**: Set `DISCOVERY_MODE=true`. Workers research and propose instead of building code.

### Step 2: Extract the context flag (from remaining arguments, first match wins)

- **`--brief`**: Load `docs/<milestone>/TEAM-BRIEF.md` and `docs/<milestone>/ROADMAP.md`.
  If either is missing, abort: run `/ale:new-brief <milestone>` first.
- **`--issues`**: Extract all `#N` tokens. Each issue becomes a task.
- **`--milestone`**: Fetch all open issues in that milestone. Each becomes a task.
- **No flag** (DEFAULT): Plain description. Derive tasks from description + codebase knowledge.

Mode and context flags are **orthogonal** -- any combination is valid (e.g.,
`--solo --issues #42`, `--team --milestone v0.1.0-beta`, `--spike --solo --issues #42`).
</context>

<session-correlation>
## Session Correlation (all modes)

Before spawning any worker, set these environment variables in the worker prompt so that
OTEL telemetry from child sessions can be correlated back to this parent session:

```
## Session Correlation
Set these environment variables before doing any work:
  export ALE_PARENT_SESSION_ID="<this session's CLAUDE_SESSION_ID or SESSION_ID>"
  export ALE_TEAM_NAME="<team-name>"
  export ALE_ISSUE_NUMBER="<issue-number if applicable, otherwise omit>"
```

These values propagate into OTEL spans via `hooks/lib/otel-emit.sh` and `hooks/session-preflight.sh`,
enabling parent-to-child session tracing in ClickHouse/HyperDX.

- `ALE_PARENT_SESSION_ID`: Read from `$CLAUDE_SESSION_ID` or `$SESSION_ID` in the current session
- `ALE_TEAM_NAME`: The scoped team name (e.g., `ale-workflow-sandbox-fixes`)
- `ALE_ISSUE_NUMBER`: The GitHub issue number being worked on (if applicable)
</session-correlation>

<shared>
These patterns are used across modes. Each mode references them by name.

## Repo Slug

Derive `$REPO_SLUG` using the Repo Slug pattern from `_patterns.md`.
All team/branch names must be prefixed with `$REPO_SLUG-`.

## Context Loading

**Always read:** `CLAUDE.md`

**If `--brief`:**
Read `docs/<milestone>/TEAM-BRIEF.md` and `docs/<milestone>/ROADMAP.md`.
If either is missing, abort: "Run `/ale:new-brief <milestone>` first."

**If `--issues`:**
1. Fetch each issue: `gh issue view <N> --json number,title,body,labels,milestone`
2. Issue title = task summary. Issue body = primary task description (pass in full).
   Extract: Summary, Acceptance Criteria, Task Checklist, Files, Context.
3. Note issue number for PR linking.

**If `--milestone`:**
1. Number -> resolve via `gh api repos/$REPO_NWO/milestones/<number> --jq '.title'`. Name -> use directly.
2. Fetch: `gh issue list --milestone "$MILESTONE_NAME" --json number,title,body,labels --state open --limit 100`
3. If no issues, abort. Use milestone name as team name.

**If lightweight (no flag):**
Read referenced files. Break work into tasks. Derive team name from description.

## Branch Naming

- Feature: `feat/$REPO_SLUG-<slug>`. Bug fix: `fix/$REPO_SLUG-<slug>` (use `fix/` if issue has `bug` label).
- From issue: include number, e.g., `feat/$REPO_SLUG-42-<slug>`.
- **SPIKE_MODE**: use `spike/` prefix instead of `feat/`/`fix/`.
- Squad branches: `squad/<team-name>` (not `feat/`).
- Worker branches in squads: `feat/<team-name>-<slug>` (HYPHEN not slash).
- Workers branch FROM `origin/squad/<team-name>` in squad modes, `origin/main` in solo.

## Issue Comment (pre-work)

For each task with `issueNumber`:
```bash
gh issue comment <N> --body "Working on this -- branch: \`<branch-name>\`"
```

## Worker Spawn Config

Use the Task tool with `isolation: "worktree"`, `mode: "bypassPermissions"`, `subagent_type: "squad-worker-mcp"`.

**Teammate prompt**: Use the Teammate Prompt Template from `_patterns.md` with:
- `{base-branch}`: `origin/main` (solo) or `origin/squad/<team-name>` (lean/full)
- `{task-description}`: full task body including acceptance criteria
- Append: "When done, summarize: what changed, files modified, tests passing, Codex review status."

## Spike Mode Worker Addendum

**If SPIKE_MODE is true**, append to worker prompt:

```
## Spike Mode
This task is in --spike mode. Your goal is to VALIDATE a hypothesis with working code.

Rules:
- Build the MINIMUM code needed to prove or disprove the hypothesis
- This code is THROWAWAY -- optimize for speed and clarity, not production quality
- You MUST run your prototype and capture real output
- Skip Codex self-review -- this code is disposable

Your deliverable MUST include an Evidence section:
### Evidence
- **Hypothesis**: What you're testing
- **Prototype**: What you built (files, approach)
- **Result**: Actual output (paste terminal output, test results, benchmarks)
- **Conclusion**: Validated / Invalidated / Partially validated -- with reasoning

If you cannot run the prototype, say so explicitly with what WOULD need to happen.
```

## Completion Verification

**SPIKE_MODE true:** Skip review gate. Check for Evidence section (Hypothesis, Prototype,
Result, Conclusion). If missing: "Spike mode requires an Evidence section."

**SPIKE_MODE false:** Apply Review Verification Gate from `_patterns.md`. Check test
evidence and review evidence (strategy-dependent).

## Spike Completion

When SPIKE_MODE is true, instead of creating a PR:
1. Push: `git push origin <branch>`
2. If from issue: `gh issue comment <N>` with spike results, evidence, branch, files.
3. No PR, no merge.

## Normal PR Creation

When SPIKE_MODE is false:
1. Push: `git push origin <branch>`
2. Spawn pr-creator per PR Creation pattern in `_patterns.md`.
3. If `issueNumber`: `gh issue comment <N>` with PR URL.

## Blocker Handling (lean and full modes)

See Hard Rule #4. Acknowledge, triage (provide info / reassign / `AskUserQuestion`), track in summary.

## Lean Orchestrator Rule (lean and full modes)

See Hard Rule #1. Delegate investigation to Explore sub-agents. Only short git/gh commands.

## Squad Setup (lean and full modes)

1. **Enter Worktree FIRST**: `EnterWorktree` with name `squad-<team-name>`.
2. **Derive Repo Slug** per shared pattern.
3. **Verify Prerequisites**: `git status --short` and `git log --oneline -5`.
4. **Create Team**: `TeamCreate` with repo-scoped name, description, `agent_type: "orchestrator"`.
   Immediately write heartbeat: `[ -x ~/.claude/hooks/heartbeat.sh ] && ~/.claude/hooks/heartbeat.sh write "$TEAM_NAME"`
5. **Create Squad Branch**:
   ```bash
   git fetch origin
   git push origin origin/main:refs/heads/squad/<team-name>
   ```

## Squad Merge Pattern (lean and full modes)

For each completed worker branch:
1. `git fetch origin` -> `git checkout squad/<team-name>` -> `git merge origin/<worker-branch> --no-edit`
2. If merge conflict: try auto-resolve, else ask user
3. `git push origin squad/<team-name>`
After all merges: `git checkout -`

## Squad Shutdown

1. Shut down teammates: SendMessage `shutdown_request` to each
2. Clean up: `TeamDelete`

## Issue-Aware Task Creation (from issues)

- Issue title = task subject
- Include **full issue body** in task description
- Store `issueNumber` in metadata: `metadata: { "issueNumber": N }`
- Branch naming per shared Branch Naming
</shared>

<process>

---

## MODE A: Solo (`--solo`)

A single autonomous agent. No team, no task list, no squad branch, no gates.

### A0. Setup
Derive Repo Slug per shared pattern.

### A1. Load Context
Per shared Context Loading. For `--milestone` with multiple issues, pick highest-priority
or ask user which to tackle solo.

### A2. Create Branch
Per shared Branch Naming. Solo uses `origin/main` as base.

### A3. Comment on Issue
Per shared Issue Comment, if applicable.

### A4. Spawn Agent
Per shared Worker Spawn Config + Spike Mode Worker Addendum if applicable.
**Native strategy**: If diff < 200 lines, review inline. If larger, spawn `codex-reviewer`.

### A5. Verify Completion
Per shared Completion Verification.

### A6. Push & Create PR
Per shared Spike Completion or Normal PR Creation.

### A7. Report
**Spike**: branch, issue, evidence summary, files, next steps.
**Normal**: branch, PR URL, issue, commits, files, Codex status, next steps.

---

## MODE B: Lean (default -- no mode flag)

Delegate aggressively, minimal ceremony. No gates, no wave ordering.
If only 1 task, auto-switch to **Solo mode (MODE A)** from step A2.

### B0-B3. Setup
Per shared Squad Setup (enter worktree, repo slug, prerequisites, create team).

### B4. Load Context and Create Tasks
Per shared Context Loading. For `--issues`, derive team name from issues.

**Auto-switch**: If only 1 task, switch to Solo mode (A2 onward).

Create tasks per shared Issue-Aware Task Creation. **No gate tasks** -- use `blockedBy`
directly between work tasks if natural dependencies exist.

### B4b. Sync to GitHub Project (optional)
Per Project Board Update in `_patterns.md`. Best-effort.

### B5. Create Squad Branch
Per shared Squad Setup step 5.
**SPIKE_MODE**: workers use `spike/<team-name>-<slug>` instead of `feat/`.

### B6. Display Plan and Start
Show compact summary then immediately start.

### B7. Spawn Workers and Monitor
**Spawn all workers in parallel** per shared Worker Spawn Config.
Workers branch FROM `origin/squad/<team-name>`.
Per shared Issue Comment before spawning. Append Spike Mode Addendum if applicable.

**Native strategy + not spike**: spawn `codex-reviewer` alongside workers.

Monitor via teammate messages + periodic `TaskList`.
Per shared Completion Verification and Blocker Handling.

### B8. Merge and Create PR
**Spike**: Per shared Spike Completion for each worker.
**Normal**: Per shared Squad Merge Pattern (merge all at once). Then spawn pr-creator
with issue refs from `issueNumber` metadata.

### B9. Complete
**Spike**: summary (evidence, branches, files), next steps, shutdown.
**Normal**: `gh pr ready`, summary (tasks, commits, PR URL), next steps, shutdown.
Per shared Squad Shutdown.

---

## MODE C: Full Squad (`--team`)

Full team with wave gates, quality enforcement, wave-by-wave execution.

### C0-C3. Setup
Per shared Squad Setup (enter worktree, repo slug, prerequisites, create team).

### C4. Load Context and Create Tasks with Dependencies

Per shared Context Loading. For `--brief`, also extract wave plan from TEAM-BRIEF.md
and phases/dependencies from ROADMAP.md.

Track task IDs for wiring `blockedBy` dependencies.

**Wave Gate Tasks**: Between each wave, create a gate task owned by orchestrator.
Gates are the ONLY dependency for the next wave.

#### Issue-Aware Wave Ordering
Derive from labels: `wave:1/2/3` -> explicit waves. `blocked-by:#N` -> dependencies.
`priority:high/medium/low` -> ordering within wave. No wave labels? Derive from priority
(high->W1, medium->W2, low->W3, none->W1).

Create tasks per shared Issue-Aware Task Creation plus:
- Wave N work tasks have no cross-wave dependencies
- Gate N blocked by ALL Wave N tasks, owned by orchestrator
- Wave N+1 blocked by Gate N
- Gate naming: `Gate N: Quality check after Wave N`

Task descriptions include: full issue body or phase goal, quality gates, shared file notes.

### C4b. Sync to GitHub Project (optional)
Per Project Board Update in `_patterns.md`. Best-effort.

### C5. Create Squad Branch
Per shared Squad Setup step 5. **No draft PR yet** -- created after first wave merge (C7d).

### C6. Display Plan and Start
Show summary (squad name, branch, goal, wave plan with gates) then start Wave 1.
Mission statement: `Squad <team-name> on squad/<team-name> -- <goal in 10 words>`

**DAG display format:**
```
DAG: 7 tasks | 3 ready | max 6 workers

  #1 Add JWT middleware          (ready)
  #2 Add user model             (ready)
  #3 Add login endpoint         (depends: #1, #2)
  #4 Add protected routes       (depends: #3)
  #5 Add role-based permissions  (depends: #2)
  #6 Add rate limiting          (ready)
  #7 Add admin dashboard        (depends: #4, #5)
```

### C7. Execute DAG

For each wave:

#### C7a. Spawn Teammates
Per shared Worker Spawn Config. Spawn all in parallel. Workers branch FROM squad branch.
Per shared Issue Comment before spawning. Append Spike Mode Addendum if applicable.

**Native strategy + not spike**: spawn `codex-reviewer` with prompt:
"Review each worker's diff: `git diff origin/squad/<team-name>...origin/<worker-branch>`.
Check for: logic errors, bugs, edge cases, security, test gaps.
Verdict: approve/request-changes. Format: Issues (severity, file:line), Suggestions."

#### C7b. Monitor Progress
Per shared Completion Verification and Blocker Handling.
For native strategy, verify reviewer verdicts. Address "request-changes" before merging.

#### C7c. Report Wave Status
```
Wave N complete:
  <task> -- completed by <owner> (Tests: passed/BLOCKED, Codex: clean/N iterations/skipped)
```
Update project board if enabled. If any failed: ask user Retry, Skip, or Abort.

#### C7c.1 Wave Quality Gate

**Spike**: verify Evidence sections only. Mark gate completed when all have Evidence.

**Normal**:
1. Detect changes per worker -- classify using `gate_patterns` from config
2. Run applicable gates from `quality_gates` config
3. All passed -> mark gate completed. Any failed -> ask user: Retry, Skip, Abort.
   Verify review evidence per Review Verification Gate in `_patterns.md`.

#### C7d. Merge Worker Branches
**Spike**: Per shared Spike Completion. No merge, no PR.

**Normal**: Prerequisite: gate completed. Per shared Squad Merge Pattern.
**First wave only**: create draft PR via pr-creator with `--draft`.
Store PR number/URL. Skip on subsequent waves.
If `issueNumber`: comment on issue with PR URL.
Update PR body after each wave -- first verify PR is still open:
```bash
PR_STATE=$(gh pr view <PR-number> --json state --jq .state 2>/dev/null)
[ "$PR_STATE" = "MERGED" ] || [ "$PR_STATE" = "CLOSED" ] && echo "PR closed -- skipping edit."
```

#### C7e. Shutdown Wave Teammates
Send `shutdown_request` to completed wave teammates before spawning new ones.

### C8. Squad Complete

**Spike**: summary (evidence, branches, files), milestone progress if applicable,
next steps (review evidence, build for real). Per shared Squad Shutdown.

**Normal**:
1. Finalize PR via pr-creator (handles stale/closed detection)
2. `gh pr ready <PR-number>`
3. Summary: phases, commits, files, PR URL
4. Milestone progress if applicable:
   ```bash
   gh api repos/$REPO_NWO/milestones/<number> --jq '{title, open: .open_issues, closed: .closed_issues}'
   ```
   Include in PR body: "Milestone: <name> -- N/M issues completed"
5. Next steps (review PR, merge)
6. Per shared Squad Shutdown

<checkpoint>
## REMEMBER — Before Finalizing

Re-read the Hard Rules at the top. Verify: no code investigation (delegated to Explore), no direct merges to main, all blockers addressed, completion evidence collected from every worker, workers branched from squad branch.
</checkpoint>
</process>
