#!/usr/bin/env node
// verify-completion.js — Claude Code PreToolUse hook for TaskUpdate
//
// Blocks task completion (status → "completed") unless the worker's branch
// has commits ahead of origin/main. This enforces the rule that workers
// must push work before marking tasks done.
//
// Checks:
//   1. Is the current branch NOT main/master? (skip check on main)
//   2. Does the branch have commits ahead of origin/main?
//   3. Are those commits pushed to the remote?
//
// Exit 0 = allow, exit 2 = block with message
//
// Environment variables:
//   ALE_VERIFY_COMPLETION — set to "0" to disable (default: enabled)

const { execSync } = require('child_process');

let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => input += chunk);
process.stdin.on('end', () => {
  try {
    // Allow disabling via env var
    if (process.env.ALE_VERIFY_COMPLETION === '0') {
      process.exit(0);
    }

    const data = JSON.parse(input);
    const toolName = data.tool_name || '';
    const toolInput = data.tool_input || {};

    // Only intercept TaskUpdate calls setting status to "completed"
    if (toolName !== 'TaskUpdate') {
      process.exit(0);
    }
    if (toolInput.status !== 'completed') {
      process.exit(0);
    }

    // Get current branch
    let branch;
    try {
      branch = execSync('git symbolic-ref --short HEAD 2>/dev/null', { encoding: 'utf8' }).trim();
    } catch (e) {
      // Detached HEAD or not a git repo — allow through
      process.exit(0);
    }

    // Skip check if on main/master (tribe leads, orchestrators)
    if (branch === 'main' || branch === 'master') {
      process.exit(0);
    }

    // Fetch origin silently to get latest remote state
    try {
      execSync('git fetch origin 2>/dev/null', { encoding: 'utf8', stdio: 'pipe' });
    } catch (e) {
      // Fetch failed — allow through (offline, no remote, etc.)
      process.exit(0);
    }

    // Check if origin/main exists
    let hasOriginMain = true;
    try {
      execSync('git rev-parse --verify origin/main 2>/dev/null', { encoding: 'utf8', stdio: 'pipe' });
    } catch (e) {
      hasOriginMain = false;
    }

    if (!hasOriginMain) {
      // No origin/main — cannot verify, allow through
      process.exit(0);
    }

    // Count commits ahead of origin/main
    let commitsAhead = 0;
    try {
      const count = execSync('git rev-list origin/main..HEAD --count 2>/dev/null', { encoding: 'utf8', stdio: 'pipe' }).trim();
      commitsAhead = parseInt(count, 10) || 0;
    } catch (e) {
      // Cannot count — allow through
      process.exit(0);
    }

    if (commitsAhead === 0) {
      // No commits ahead of origin/main — block completion
      process.stderr.write('[AX_VERIFY_COMPLETION] BLOCKED: Cannot mark task complete — no commits on this branch.\n');
      process.stderr.write(`Branch "${branch}" has 0 commits ahead of origin/main.\n`);
      process.stderr.write('You must commit your changes before marking a task as completed.\n');
      process.stderr.write('Remediation: Commit your work, then push with "git push origin HEAD".\n');
      process.exit(2);
    }

    // Commits exist locally — check if they are pushed
    const remoteBranch = `origin/${branch}`;
    let remoteExists = true;
    try {
      execSync(`git rev-parse --verify ${remoteBranch} 2>/dev/null`, { encoding: 'utf8', stdio: 'pipe' });
    } catch (e) {
      remoteExists = false;
    }

    if (!remoteExists) {
      // Branch not pushed to remote at all — block
      process.stderr.write('[AX_VERIFY_COMPLETION] BLOCKED: Cannot mark task complete — branch not pushed.\n');
      process.stderr.write(`Branch "${branch}" has ${commitsAhead} commit(s) but is not pushed to origin.\n`);
      process.stderr.write('Remediation: Push your branch first with "git push origin HEAD".\n');
      process.exit(2);
    }

    // Check if there are unpushed commits
    let unpushedCount = 0;
    try {
      const count = execSync(`git rev-list ${remoteBranch}..HEAD --count 2>/dev/null`, { encoding: 'utf8', stdio: 'pipe' }).trim();
      unpushedCount = parseInt(count, 10) || 0;
    } catch (e) {
      // Cannot check — allow through (remote branch exists, good enough)
      process.exit(0);
    }

    if (unpushedCount > 0) {
      // Commits exist but not all are pushed — warn but allow
      process.stderr.write(`[AX_VERIFY_COMPLETION] WARNING: ${unpushedCount} unpushed commit(s) on "${branch}".\n`);
      process.stderr.write('Consider running "git push origin HEAD" to push your latest changes.\n');
      // Allow through — commits exist and branch is partially pushed
      process.exit(0);
    }

    // All good — commits exist and are pushed
    process.exit(0);
  } catch (e) {
    // Parse error or unexpected failure — allow through (fail open)
    process.exit(0);
  }
});
