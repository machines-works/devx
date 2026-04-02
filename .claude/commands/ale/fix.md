---
name: ale:fix
description: Start a quick-fix agent in an isolated worktree
argument-hint: "<description of what to fix>"
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
  - Task
  - EnterWorktree
  - AskUserQuestion
---
<objective>
Start a focused quick-fix session. This is for individual bug fixes, regressions, and small
improvements — not epic work.

You work directly with the user (not autonomously). The user drives; you investigate, debug, and fix.
</objective>

<context>
Fix description: $ARGUMENTS
</context>

<process>
## Step 1: Assess Scope

Read the fix description. Classify it:

**Small fix** (1-3 files):
- Typo, config fix, single-file bug
- Use worktree, quick turnaround

**Medium fix** (3-10 files):
- Regression, multi-file bug, small feature
- Use worktree, more investigation needed

**Large fix** (10+ files):
- Should probably be a squad or solo agent instead
- Suggest: "This looks large enough for a dedicated agent. Want me to proceed as a fix,
  or escalate to `/ale:start --solo` (single agent) or `/ale:start` (squad)?"

## Step 2: Set Up Worktree

Create an isolated worktree for the fix:
- Use `EnterWorktree` with name: "fix-<slug>" (e.g., "fix-login-error")
- This creates an isolated copy — safe to edit freely without affecting main

## Step 3: Investigate

Based on the fix description:
1. Find the relevant files (Glob/Grep)
2. Read the code around the problem
3. Identify root cause
4. Propose a fix to the user

Do NOT immediately start editing. Show the user what you found and your proposed approach first.

## Step 4: Fix

After user approves the approach:
1. Make the changes
2. Run quality gates from `.claude/ale.config.yaml` → `quality_gates` (or check CLAUDE.md)
3. Show the user the diff

## Step 5: Commit & PR (if user asks)

Only commit when the user explicitly asks. Use:
```
fix(<scope>): <description>

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
```

Push the branch and create a PR:
```bash
git push origin HEAD
gh pr create --base main --head $(git branch --show-current) \
  --title "fix(<scope>): <description>" \
  --body "## Summary
<what was fixed>

Generated with [Claude Code](https://claude.com/claude-code) + [ALE Workflow](https://github.com/Sharpi-AI/ale-workflow)"
```
The user merges PRs on GitHub — never merge directly to main.
</process>
