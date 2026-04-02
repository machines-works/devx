# Shared Prompt Patterns

Canonical patterns referenced by ale commands. Each pattern lives here once;
commands use it by name (e.g., "follow the Repo Slug pattern from _patterns.md").

---

## Repo Slug

Derive once at session start, reuse everywhere:

```bash
REPO_SLUG=$(basename "$(git remote get-url origin 2>/dev/null || basename "$(pwd)")" .git)
```

Gives e.g. `ale-workflow` from `git@github.com:Sharpi-AI/ale-workflow.git`.
Use for team names, branch prefixes, task/team directory scoping.

---

## OTEL Stack Check

```bash
docker compose -f ~/.claude/observability/docker-compose.yml ps --format '{{.Name}} {{.Status}}' 2>/dev/null || echo "OTEL stack: not running"
```

- 4 containers (ch-server, otel-collector, app, db) = UP
- Partial = DEGRADED (list which are down)
- Not running = DOWN -- suggest `mise run otel-up`
- HyperDX UI: http://localhost:8090

---

## Project Board Update

Read config once:
```bash
PROJECT_ENABLED=$(yq -r '.project.enabled // false' .claude/ale.config.yaml 2>/dev/null || echo "false")
PROJECT_NUM=$(grep 'number:' .claude/ale.config.yaml 2>/dev/null | head -1 | awk '{print $2}')
PROJECT_ORG=$(grep 'org:' .claude/ale.config.yaml 2>/dev/null | head -1 | awk '{print $2}' | tr -d '"')
```

If `PROJECT_ENABLED` is not true, skip all project board operations.

**Set status on an item** (In Progress, Done, Todo):
```bash
# 1. Find the item
gh project item-list $PROJECT_NUM --owner $PROJECT_ORG --format json
# 2. Get field metadata
gh project field-list $PROJECT_NUM --owner $PROJECT_ORG --format json
# 3. Update status
gh project item-edit --project-id <pid> --id <item-id> --field-id <fid> --single-select-option-id <option-id>
```

**Add an issue to the project:**
```bash
gh project item-add $PROJECT_NUM --owner $PROJECT_ORG --url "<issue-url>" --format json
```

**Create a draft item:**
```bash
gh project item-create $PROJECT_NUM --owner $PROJECT_ORG --title "<title>" --format json
```

All project board operations are **best-effort** -- log warnings on failure, never block.

---

## Teammate Prompt Template

The canonical template for spawning worker agents. Replace `{placeholders}` at spawn time.

The orchestrator MUST set these **session correlation environment variables** when spawning
sub-agents via the Task tool, so that child session telemetry is traceable to the parent:

| Env Var | Value | Required |
|---------|-------|----------|
| `ALE_PARENT_SESSION_ID` | Parent's `CLAUDE_SESSION_ID` | Yes |
| `ALE_TEAM_NAME` | Team/squad name | Yes |
| `ALE_AGENT_ROLE` | `tribe-lead`, `squad-lead`, or `worker` | Yes |
| `ALE_ISSUE_REFS` | GitHub issue number(s) being worked on | If applicable |
| `ALE_SQUAD_BRANCH` | Squad integration branch (e.g., `squad/team-name`) | If applicable |
| `ALE_WAVE_NUMBER` | Current wave number | If applicable |

These env vars are read by `hooks/session-correlation.sh` (SessionStart) and
`hooks/otel-review-export.sh` (PostToolUse) to emit correlated OTEL telemetry.

```
You are a teammate in the {team-name} squad. Your name is {name}.

## Branch Verification (MANDATORY -- RUN BEFORE ANYTHING ELSE)
Run these commands NOW, before reading any other instructions:
  git fetch origin
  git checkout -b {worker-branch} origin/{base-branch}
  CURRENT=$(git branch --show-current)
  echo "Current branch: $CURRENT"
  echo "Expected branch: {worker-branch}"
If $CURRENT does not match `{worker-branch}`:
  -> STOP IMMEDIATELY. Do not proceed.
  -> Send a message to the team lead: "Branch mismatch: on $CURRENT, expected {worker-branch}"
  -> Wait for instructions.

## Your Mission
{task-description}

{if-issue}
## Issue Reference
This task is from GitHub issue #{N}: {title}
Include "Ref #{N}" in your commit messages.
The orchestrator handles issue comments -- do NOT comment on the issue yourself.
{/if-issue}

## Context
Read CLAUDE.md first, then: {context-files}
If any hook blocks you, read the error and follow its instructions. If you cannot
proceed, message the team lead -- do NOT try workarounds.

The squad-worker agent definition handles all workflow rules -- quality gates,
Codex self-review, shared file coordination, and communication protocol.
Follow the Blocker Protocol in the squad-worker agent definition -- if blocked, report immediately, never silently pivot.

## When Done
When your work is complete:
1. Call `TaskUpdate(taskId, status: completed)` FIRST to mark your task done in the shared task list.
2. THEN send your completion summary to the team lead via `SendMessage`.
The task list is the source of truth for progress -- always update it before messaging.

{if-discovery}
## Discovery Mode
This task is in --discovery mode. Follow EXPLORATORY triage: research and propose a design,
do NOT write implementation code. Your deliverable is a design comment on the issue.
{/if-discovery}

## Session Correlation
Set these environment variables before doing any work:
  export ALE_PARENT_SESSION_ID="{parent-session-id}"
  export ALE_TEAM_NAME="{team-name}"
  {if-issue}export ALE_ISSUE_NUMBER="{issue-number}"{/if-issue}

## Squad Context (generated at spawn time)
- Recent merges to main: {recent-main-commits}
- Active squads: {active-squad-branches}
- Your dependencies: {task-dependencies}
- Shared files: {shared-files-overlap}
```

**Parameters:**
| Placeholder | Source |
|---|---|
| `{team-name}` | Team or tribe name |
| `{name}` | Worker name (e.g., "rbac-worker") |
| `{worker-branch}` | `feat/<team>-<slug>` or `fix/<team>-<slug>` |
| `{base-branch}` | `origin/main` (solo/dispatch) or `origin/squad/<team>` (squad) |
| `{task-description}` | Full task body including acceptance criteria |
| `{context-files}` | Relevant file paths from issue body or brief |
| `{recent-main-commits}` | `git log origin/main --since='2 hours ago' --oneline` |
| `{active-squad-branches}` | `git branch -r \| grep squad/` |
| `{task-dependencies}` | From TaskList blockedBy/blocks |
| `{shared-files-overlap}` | From ale.config.yaml shared_files |
| `{parent-session-id}` | `$CLAUDE_SESSION_ID` or `$SESSION_ID` from the spawning session |
| `{issue-number}` | GitHub issue number (if task comes from an issue) |

---

## PR Creation via pr-creator

All commands delegate PR creation to the **pr-creator** sub-agent instead of inline `gh pr create`.

```
Agent(subagent_type: "pr-creator", prompt: "
  branch: {branch}
  issue_refs: {issue-numbers}
  summary: {what-changed}
  review_notes: {codex-status}
  {extra-flags}
")
```

The pr-creator handles: deduplication (checks existing PRs), title formatting,
`Closes #N` linking, body structure, and stale-PR detection.

**NEVER merge or cherry-pick to main.** All changes go through PRs.

---

## Review Verification Gate

The orchestrator verifies worker completion evidence. The squad-worker agent definition
contains the full review process; the orchestrator only checks the output.

**What to check in the worker's completion summary:**

1. **Test Evidence** (mandatory):
   - Actual output (bats, curl, CLI) = accept
   - BLOCKED with reason = evaluate, escalate if needed
   - Missing = reject, send back: "Show me test output before I accept this"

2. **Review Evidence** (strategy-dependent):
   - **codex strategy**: Codex status (`clean`/`N iterations`/`skipped`/`self-review (codex unavailable)`) + call count
   - **native strategy**: `native (reviewer handles)` is valid -- orchestrator spawns reviewer
   - `self-review (codex unavailable)` = worker completed the Self-Review Checklist because
     Codex MCP was down. Acceptable, but note it for the PR body.

Workers that push without any review evidence when strategy is "codex" are in workflow violation.
The squad-worker agent definition enforces the review process; orchestrators enforce verification.

---

## Lifecycle Span Emission

Orchestrators and agents emit lifecycle span events to track the dispatch->work->PR->merge
pipeline. Each span has a start and end event, emitted via `hooks/lifecycle-spans.sh`.

**Span types and where to emit them:**

| Span Type | Start | End | Emitter |
|-----------|-------|-----|---------|
| `tribe_dispatch` | Before task decomposition | After all tasks created | Tribe lead / `ale:start` |
| `squad_formation` | Before worktree/branch setup | After squad branch created + workers spawned | `ale:start` / `ale:tribe` |
| `agent_work` | After branch checkout | Before pushing branch | Squad worker |
| `pr_creation` | Before spawning pr-creator | After PR URL returned | Orchestrator |
| `review_merge` | After PR created | After PR merged | Orchestrator |

**Required attributes** (pass as key=value pairs):

| Attribute | Description |
|-----------|-------------|
| `agent_id` | Agent name (e.g., "auth-worker", "tribe-lead") |
| `branch_name` | Branch being worked on |
| `team_id` | Team name (e.g., "ale-workflow-auth") |
| `task_id` | Task ID from TaskList |
| `model` | Model used (e.g., "claude-opus-4-6") |

**Emission example:**
```bash
# At start of task decomposition
hooks/lifecycle-spans.sh tribe_dispatch start \
  agent_id=tribe-lead team_id="$TEAM_NAME" model=claude-opus-4-6

# After tasks created
hooks/lifecycle-spans.sh tribe_dispatch end \
  agent_id=tribe-lead team_id="$TEAM_NAME" task_count=5 model=claude-opus-4-6
```

**Nesting:** Spans nest naturally through session correlation:
- `tribe_dispatch` is the root span (emitted by tribe lead)
- `squad_formation` is a child (same team_id, emitted by squad lead)
- `agent_work` is a child of `squad_formation` (linked via parent_session_id)
- `pr_creation` follows `agent_work` (same agent_id)
- `review_merge` follows `pr_creation` (same branch_name)

All emissions are best-effort -- never block orchestration on OTEL failures.

---

## Anti-Hallucination

Ground every claim in tool output. Never state facts you did not read in this session.

1. **Source-grounding**: Do not assert versions, URLs, config values, error messages, or metrics
   unless you read them from a tool result. If you cannot verify a claim, say "(unverified)."
2. **Read-before-claim**: Before reporting any data point, confirm you have a tool result
   (file read, API response, CLI output) from this session that contains it.
3. **Cite-or-flag**: Tag data points with their source — `(from file read)`, `(from CLI output)`,
   `(from API response)`. No source available → mark `(unverified)`.
## Silent Pivot Prevention

A **silent pivot** occurs when an agent substitutes a weaker method for what was requested,
without disclosing the substitution. This produces false confidence -- the lead thinks the
task was done properly, but it was not.

### Definition

Silent pivot = doing Y when asked to do X, and reporting as if you did X.

Key distinction: **transparent alternatives are fine.** If you tell the lead "I cannot do X,
but I can do Y which covers [subset/different angle]" and get approval, that is not a silent
pivot. The violation is the lack of disclosure.

### Common Silent Pivot Patterns

These are the most frequently observed pivots from production agent runs:

| Asked to do | Silent pivot | Why it is wrong |
|---|---|---|
| Run browser tests | `curl` + HTML parsing | Does not test JS rendering, clicks, or client-side state |
| Visual regression check | Text-diff CSS/HTML | Does not catch visual regressions (layout, overlap, color) |
| Load test an endpoint | Time one `curl` | Does not test concurrency, connection pooling, or degradation |
| Verify DB migration runs | Read the SQL file | Does not catch runtime errors, constraint violations, data issues |
| Test OAuth flow end-to-end | Check config values exist | Does not verify token exchange, redirect handling, session creation |
| Verify email sending | Check SMTP config is set | Does not verify email actually arrives or renders correctly |

### Escalation Path

When an agent detects it cannot perform the requested verification:

1. **Recognize the gap.** Before starting any verification, ask: "Can I actually do what is being asked, or am I about to substitute something weaker?"
2. **Report the blocker.** Use the Missing Capability blocker format from the squad-worker agent definition.
3. **Propose alternatives transparently.** If a partial verification has value, say so -- but name what it does NOT cover.
4. **Wait for lead decision.** The lead may: accept the alternative, reassign the task, provide the missing tool/access, or descope the requirement.

### For Orchestrators

When reviewing worker completion summaries, watch for these signals of a possible silent pivot:

- Worker was asked to test X but evidence shows a different method (e.g., task says "browser test", evidence shows `curl` output)
- Verification evidence does not match the specificity of the acceptance criteria
- Worker says "verified" but the tool used cannot actually verify what was claimed

If you suspect a silent pivot, ask the worker directly: "Did you actually run [specific method], or did you use an alternative approach?" Do not accept vague confirmation.
