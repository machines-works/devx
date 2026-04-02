---
name: ale:issue
description: Create a GitHub issue with validated labels and multiline body — no temp files
argument-hint: "<title> [--label <name>]... [--body <markdown>]"
allowed-tools:
  - Bash
  - Read
---
<objective>
Create a GitHub issue from within a Claude Code session. Accepts title, labels, and a
multiline markdown body. Uses `--body-file -` with piped stdin so no temp files are
needed. Validates labels before creation and reports the issue URL on success.

Examples:
- `/ale:issue Add dark mode support`
- `/ale:issue Fix login timeout --label bug --label priority:high`
- `/ale:issue Add CLI init command --label cli --body "## Context\nNeeds stack detection.\n\n## Acceptance Criteria\n- Detects Node, Go, Python, Rust"`
</objective>

<context>
Arguments: $ARGUMENTS
</context>

<process>
## Step 1: Parse Arguments

Parse `$ARGUMENTS` into three parts:

1. **Title** — all positional words before the first `--label` or `--body` flag.
2. **Labels** — every `--label <name>` pair (can appear multiple times).
3. **Body** — the value after `--body`, which may contain `\n` for newlines.

Rules:
- Title is required. If empty, display usage and stop.
- Labels are optional. Collect into a list.
- Body is optional. If not provided, the issue is created with an empty body.
- `\n` sequences in the body should be interpreted as real newlines.

If `$ARGUMENTS` is empty, show usage:

```
Usage: /ale:issue <title> [--label <name>]... [--body "<markdown>"]

Examples:
  /ale:issue Add dark mode support
  /ale:issue Fix login timeout --label bug --label priority:high
  /ale:issue Add CLI init command --label cli --body "## Context\nNeeds stack detection."
```

## Step 2: Validate Labels

If any labels were provided, validate them against the repo's actual labels:

```bash
gh label list --json name --jq '.[].name'
```

Compare each requested label against this list. If any label does not exist:

- Display the invalid label(s) clearly
- Show a short list of available labels (up to 20) to help the user pick the right one
- **Stop** — do not create the issue

Example output for invalid labels:

```
Label "pririty:high" not found in this repo.

Available labels (showing first 20):
  bug, cli, enhancement, integration, observability, priority:high, priority:medium, skill, workflow

Did you mean "priority:high"?
```

If all labels are valid, proceed.

## Step 3: Create the Issue

Build and run the `gh issue create` command. Use printf + pipe to `--body-file -` so
multiline bodies work without temp files.

**If body is provided:**

```bash
printf '%s' "<body with newlines expanded>" | gh issue create \
  --title "<title>" \
  --label "<label1>" --label "<label2>" \
  --body-file -
```

**If no body:**

```bash
gh issue create --title "<title>" --label "<label1>" --label "<label2>" --body ""
```

Important:
- Quote the title to preserve spaces.
- Each label gets its own `--label` flag.
- Use `printf '%s'` (not `echo`) to handle special characters safely.
- Expand `\n` in the body string to actual newlines before piping.

If the command fails, display the error from `gh` and stop.

## Step 4: Report

On success, `gh issue create` prints the issue URL. Display a summary:

```
Issue created: <URL>

  Title:  <title>
  Labels: <label1>, <label2>
  Body:   <first 80 chars of body>...
```

Suggest next steps:
- `View: gh issue view <number>`
- `Start solo: /ale:start --solo #<number>`
- `Start squad: /ale:start --issues #<number>`
</process>
