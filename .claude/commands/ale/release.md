---
name: ale:release
description: Create a release — version bump, tag, GitHub release (from worktree)
argument-hint: "<version, e.g. v0.8.0>"
allowed-tools:
  - Read
  - Bash
  - Edit
  - Write
  - AskUserQuestion
---
<objective>
Create a versioned release of the project. Bumps version numbers, updates CHANGELOG.md,
creates a git tag, pushes from a temporary worktree (bypassing lefthook's block-main-push
guard), and creates a GitHub release.

All release operations happen in a temporary worktree so the user's working directory is
never modified and lefthook guards are respected.
</objective>

<context>
Version: $ARGUMENTS
</context>

<process>
## Step 1: Validate Version

Parse and validate the version from `$ARGUMENTS`.

1. Extract the version string. It must match `vN.N.N` (semver with `v` prefix), with optional
   pre-release suffix (e.g., `v1.0.0-beta`, `v0.8.0-rc.1`).
2. If the version is missing or invalid, abort:
   ```
   Invalid version "$ARGUMENTS". Expected format: v0.8.0 (semver with v prefix).
   ```
3. Strip the `v` prefix for use in files that store bare version numbers (e.g., `package.json`
   stores `0.8.0`, not `v0.8.0`). Keep the `v`-prefixed form for tags and display.

## Step 2: Pre-Flight Checks

Run these checks before proceeding:

1. **Clean working directory:**
   ```bash
   git status --short
   ```
   If there are uncommitted changes, warn the user and ask whether to proceed.

2. **Tag does not already exist:**
   ```bash
   git fetch origin --tags
   git tag -l "$VERSION"
   ```
   If the tag already exists, abort: "Tag `$VERSION` already exists. Aborting."

3. **Remote is reachable:**
   ```bash
   git ls-remote --exit-code origin >/dev/null 2>&1
   ```
   If unreachable, abort with an error.

4. **gh CLI is available:**
   ```bash
   gh --version >/dev/null 2>&1
   ```
   If missing, warn that the GitHub release step will be skipped.

## Step 3: Create Temporary Worktree

Create a temporary worktree from `origin/main` for the release work. This avoids
lefthook's `block-primary-commit` and `block-main-push` guards.

```bash
RELEASE_DIR=$(mktemp -d)
git fetch origin
git worktree add "$RELEASE_DIR" origin/main --detach
```

All subsequent git operations happen inside `$RELEASE_DIR` until cleanup.

**Important:** Check out a real branch in the worktree so commits are not detached:
```bash
cd "$RELEASE_DIR"
git checkout -b release/$VERSION
```

## Step 4: Bump Version Numbers

In the worktree, find and update version strings:

1. **`cli/package.json`** (always present):
   ```bash
   # Read current version
   grep '"version"' cli/package.json
   ```
   Update the `"version"` field to the bare version (without `v` prefix).

2. **Other version files** (check if they exist and update if found):
   - `package.json` (root, if present and has a `version` field)
   - `version.go`, `version.py`, `version.rs` (if present)
   - `pyproject.toml` (if present, update `version = "..."`)
   - `Cargo.toml` (if present, update `version = "..."`)

   Use a simple search to find version files:
   ```bash
   grep -rl '"version":\|version =' --include='*.json' --include='*.toml' --include='*.yaml' . 2>/dev/null | grep -v node_modules | grep -v .git | head -10
   ```

   For each file found, read it, check if it contains a project version (not a dependency
   version), and update it. Ask the user if unsure whether a file should be updated.

## Step 5: Update CHANGELOG.md

1. **If `CHANGELOG.md` exists** in the worktree root:
   - Read the file
   - Find the `## [Unreleased]` section
   - Insert a new version header below `## [Unreleased]`:
     ```
     ## [Unreleased]

     ## [$VERSION] -- YYYY-MM-DD
     ```
     Where `YYYY-MM-DD` is today's date.
   - If there is content under `[Unreleased]`, move it under the new version header
   - If `[Unreleased]` has no content, add the new header with a placeholder:
     ```
     ## [$VERSION] -- YYYY-MM-DD

     ### Added
     - Release $VERSION
     ```

2. **If `CHANGELOG.md` does not exist**, create it:
   ```markdown
   # Changelog

   All notable changes to this project are documented here.

   ## [Unreleased]

   ## [$VERSION] -- YYYY-MM-DD

   ### Added
   - Initial release
   ```

Show the user the CHANGELOG diff and ask for confirmation before proceeding.

## Step 6: Commit the Version Bump

Stage and commit all version changes in the worktree:

```bash
cd "$RELEASE_DIR"
git add -A
git commit -m "chore: bump version to $VERSION"
```

## Step 7: Create Git Tag

Create an annotated tag on the version bump commit:

```bash
cd "$RELEASE_DIR"
git tag -a "$VERSION" -m "Release $VERSION"
```

## Step 8: Push Commit and Tag

Push from the worktree. Since the worktree is on a `release/` branch (not `main`),
lefthook's `block-main-push` guard does not fire.

```bash
cd "$RELEASE_DIR"
git push origin release/$VERSION
git push origin "$VERSION"
```

## Step 9: Create GitHub Release

Extract the changelog section for this version to use as release notes:

```bash
# Extract content between this version header and the next version header
```

Read the CHANGELOG.md, extract everything between `## [$VERSION]` and the next `## [` header
(or end of file). Use this as the release body.

Create the GitHub release:
```bash
gh release create "$VERSION" \
  --title "$VERSION" \
  --notes "$RELEASE_NOTES" \
  --target "release/$VERSION"
```

If `gh` is not available, skip this step and tell the user to create the release manually.

## Step 10: Create PR for Version Bump

Open a PR to merge the release branch into main so the version bump commit is tracked:

```bash
gh pr create --base main --head "release/$VERSION" \
  --title "chore: release $VERSION" \
  --body "## Summary
- Bump version to $VERSION
- Update CHANGELOG.md with release date
- Tag: \`$VERSION\`
- GitHub Release: created

Merge this PR to record the version bump on main."
```

Tell the user the PR URL and suggest they merge it.

## Step 11: Clean Up

Remove the temporary worktree:

```bash
cd -
git worktree remove "$RELEASE_DIR" --force 2>/dev/null
```

## Step 12: Report

Show the user a summary:

```
Release $VERSION complete:
  - Version bumped in: <list of files>
  - CHANGELOG.md: updated with $VERSION header
  - Tag: $VERSION (pushed)
  - GitHub Release: <URL or "skipped">
  - PR: <URL> (merge to record version bump on main)

Next steps:
  1. Review and merge the release PR
  2. Verify the GitHub release at <URL>
```

If any step failed, report what succeeded and what needs manual attention.
</process>
