---
name: ale:review
description: Review a branch diff using the configured review strategy (codex or native)
argument-hint: "<branch name> [--quick | --thorough | --model opus|sonnet]"
allowed-tools:
  - Read
  - Bash
  - Task
---
<objective>
Review a branch's changes against main using the configured review strategy.

- **codex** (default): Spawns a codex-reviewer agent that sends the diff to Codex MCP for analysis.
- **native**: Performs an inline diff review directly without Codex MCP.

Both strategies report structured findings (verdict, issues with severity, suggestions, test gaps).

Use for: any branch that needs a second-opinion code review before merge.
</objective>

<context>
Input: $ARGUMENTS
Review Strategy: !`grep "review:" .claude/ale.config.yaml -A 5 2>/dev/null | grep "strategy:" | awk '{print $2}' || echo "codex"`
Review Timeout: !`grep "review:" .claude/ale.config.yaml -A 5 2>/dev/null | grep "timeout:" | awk '{print $2}' || echo "120"`
Review Model: !`grep "review:" .claude/ale.config.yaml -A 5 2>/dev/null | grep "model:" | awk '{print $2}' || echo ""`
Project Description: !`grep "description:" .claude/ale.config.yaml 2>/dev/null | head -1 | sed 's/.*description: *//' | tr -d '"' || echo ""`
Codex Project Context: !`grep "codex:" .claude/ale.config.yaml -A 3 2>/dev/null | grep "project_context:" | sed 's/.*project_context: *//' | tr -d '"' || echo ""`
Codex Extra Checks: !`grep "codex:" .claude/ale.config.yaml -A 10 2>/dev/null | grep -A 20 "extra_checks:" | grep "^    - " | sed 's/^    - //' | tr -d '"' || echo ""`

## Parse Flags

Extract flags from `$ARGUMENTS` (first match wins, then remove from args):

| Flag | Effect |
|------|--------|
| `--quick` | Use faster model (sonnet) for review |
| `--thorough` | Use Opus for review (highest quality) |
| `--model opus\|sonnet` | Explicit model selection |

The remaining `$ARGUMENTS` after flag removal is the branch name.

**Model resolution order** (first non-empty wins):
1. `--quick` -> sonnet, `--thorough` -> opus, `--model <X>` -> X
2. `review.model` from config (preprocessed above)
3. Empty (let Codex use its default)
</context>

<process>
## Step 1: Validate Input

If no branch name in `$ARGUMENTS` (after flag removal), list recent branches and ask:
```bash
git branch --sort=-committerdate --format='%(refname:short) (%(committerdate:relative))' | head -10
```

Verify the branch exists:
```bash
git rev-parse --verify $BRANCH
```

## Step 2: Gather Branch Context

Find the merge-base so we only review the branch's own changes:
```bash
MERGE_BASE=$(git merge-base main $BRANCH)
git log $MERGE_BASE..$BRANCH --oneline
git diff $MERGE_BASE..$BRANCH --stat
```

Note the size: if total lines changed > 500, flag as a large review.

## Step 2.5: Check for Conflicts

```bash
git merge-tree $(git merge-base main $BRANCH) main $BRANCH
```

If the output contains conflict markers (`<<<<<<<`), warn:
> **Warning:** This branch has merge conflicts with main. The review will proceed,
> but the branch needs a rebase/merge before it can be merged.

List the conflicting files.

## Step 3: Review Code

### If Review Strategy is "codex":

Use the Task tool to spawn a codex-reviewer agent in the background:

```
subagent_type: "codex-reviewer"
run_in_background: true
```

Tell the user: "Starting code review in background. This typically takes 30-60 seconds..."

Agent prompt — include the resolved model, timeout, project context, and diff context:
```
You are a Codex branch reviewer. Review the diff of branch <branch> against main and report findings.

## Config
- Timeout: <timeout>s — if Codex does not respond within this time, report what you have and note the timeout.
- Model: <resolved model or "default"> — pass as the `model` parameter to mcp__codex__codex ONLY if non-empty.

## Setup

Use the merge-base to only review changes introduced by the branch:
```bash
MERGE_BASE=$(git merge-base main <branch>)
```

1. Get the diff: `git diff $MERGE_BASE..<branch>`
2. Get commit log: `git log $MERGE_BASE..<branch> --oneline`
3. Check diff size with `git diff $MERGE_BASE..<branch> --stat`

## Review via Codex MCP

Read the project context for the review prompt:
- Check `.claude/ale.config.yaml` for `codex.project_context`
- If not set, fall back to `project.description` from the same file
- If neither exists, use "software project"

Also check for:
- `codex.extra_checks` — additional review items to append
- `review.prompt_template` — if set, use this as the review prompt instead of the default below

### Default Review Prompt

Call `mcp__codex__codex` with:
- `model` parameter: ONLY if a non-empty model was resolved (from --quick/--thorough/--model/config). Otherwise omit it.
- Prompt:

"Review this code diff. Project context: <project context from config>.

Base branch: main
PR branch: <branch>
Commits: <commit log summary>

Check for:
- Logic errors, bugs, or incorrect behavior
- Missing error handling or edge cases
- Test coverage gaps (are new/changed paths tested?)
- Security issues (injection, auth bypass, hardcoded secrets, OWASP top 10)
- API contract consistency
- Language/framework conventions
<if codex.extra_checks configured, add each as a bullet>

Diff:
<full diff>"

### Custom Prompt Template

If `review.prompt_template` is set in config, use it instead. The template can reference these variables:
- `{project_context}` — resolved project context string
- `{branch}` — branch name
- `{commits}` — commit log
- `{diff}` — full diff
- `{extra_checks}` — formatted extra checks (one per line, bulleted)

### Timeout Handling

Set a timeout of <timeout> seconds (from config, default 120s) for the Codex call using Bash timeout parameter.

If the call times out:
1. Surface whatever partial results are available
2. Report clearly: "**Review timed out** after <timeout>s. Partial results shown above."
3. Suggest: retry with `--quick` for faster results, or increase `review.timeout` in config

If the diff exceeds ~500 lines, split by file group and make multiple Codex calls.
Use `mcp__codex__codex-reply` with the threadId for follow-up questions.

## Report

After Codex responds, synthesize findings:

**Verdict:** approve | request-changes | needs-discussion

### Issues
(numbered, with severity: critical/warning/nit, file:line, description)

### Suggestions
(optional improvements)

### Test Gaps
(missing test coverage)

### Security/Auth Flags
(any auth/secrets/infra that need human review)

If no issues: "Verdict: approve. No issues found."
```

### If Review Strategy is "native":

Perform an inline diff review without Codex MCP.

1. Get the full diff:
   ```bash
   MERGE_BASE=$(git merge-base main $BRANCH)
   DIFF=$(git diff $MERGE_BASE..$BRANCH)
   ```

2. Get commit context:
   ```bash
   git log $MERGE_BASE..$BRANCH --oneline
   ```

3. Read project conventions from `.claude/ale.config.yaml`:
   - `codex.project_context` or `project.description` for project context
   - `codex.extra_checks` for additional review items

4. Read the diff and analyze it yourself. Check for:
   - Logic errors, bugs, or incorrect behavior
   - Missing error handling or edge cases
   - Test coverage gaps
   - Security issues (injection, auth bypass, hardcoded secrets)
   - API contract consistency
   - Language/framework conventions
   - Any items from `codex.extra_checks`

5. If the diff is large (>500 lines), spawn an Explore agent to help analyze sections.

6. Output findings in the same structured format:

   **Verdict:** approve | request-changes | needs-discussion

   ### Issues
   (numbered, with severity: critical/warning/nit, file:line, description)

   ### Suggestions
   (optional improvements)

   ### Test Gaps
   (missing test coverage)

   ### Security/Auth Flags
   (any auth/secrets/infra that need human review)

   If no issues: "Verdict: approve. No issues found."

## Step 4: Display Results

When the reviewer agent completes (or the native review finishes), display:

```
## Branch Review: <branch>
**Model:** <model used or "default">
**Duration:** <elapsed time>

### Reviewer Findings
<structured report>

### Next Steps
- To merge: `git merge <branch>` (from main)
- For more detail: run `/ale:review <branch>` again
- For faster review: `/ale:review <branch> --quick`
- For deeper review: `/ale:review <branch> --thorough`
```

If the review failed (Codex unavailable, diff too large, timeout, or other error), suggest:
- Manual review with `git diff $(git merge-base main <branch>)..<branch>`
- Retry: `/ale:review <branch>`
- If timeout: `/ale:review <branch> --quick` or increase `review.timeout` in config
- If using codex strategy and Codex is unavailable, suggest setting `review.strategy: native` in `.claude/ale.config.yaml`
</process>
