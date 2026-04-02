#!/usr/bin/env node
// Claude Code Statusline — mode prefix (tribe/squad/solo) + context + model + active task

const fs = require('fs');
const path = require('path');
const os = require('os');

let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => input += chunk);
process.stdin.on('end', () => {
  try {
    const data = JSON.parse(input);
    const model = data.model?.display_name || 'Claude';
    const workspaceDir = data.workspace?.current_dir || process.cwd();
    const dir = path.basename(workspaceDir);
    const remaining = data.context_window?.remaining_percentage;

    // Repo slug for cross-project team filtering
    const repoSlug = dir.toLowerCase();

    // Context bar (scaled to 80% limit — Claude Code compacts at ~80%)
    let ctx = '';
    if (remaining != null) {
      const rawUsed = Math.max(0, Math.min(100, 100 - Math.round(remaining)));
      const used = Math.min(100, Math.round((rawUsed / 80) * 100));
      const filled = Math.floor(used / 10);
      const bar = '█'.repeat(filled) + '░'.repeat(10 - filled);

      if (used < 60)       ctx = ` \x1b[32m${bar} ${used}%\x1b[0m`;
      else if (used < 80)  ctx = ` \x1b[33m${bar} ${used}%\x1b[0m`;
      else if (used < 95)  ctx = ` \x1b[38;5;208m${bar} ${used}%\x1b[0m`;
      else                 ctx = ` \x1b[5;31m${bar} ${used}%\x1b[0m`;
    }

    // Detect active team and mode (tribe/squad/solo)
    let mode = '';
    let teamName = '';
    let task = '';
    const teamsBase = path.join(os.homedir(), '.claude', 'teams');
    const tasksBase = path.join(os.homedir(), '.claude', 'tasks');
    if (fs.existsSync(teamsBase)) {
      try {
        const teams = fs.readdirSync(teamsBase).filter(t => {
          const p = path.join(teamsBase, t, 'config.json');
          if (!fs.existsSync(p)) return false;
          // Filter teams to current repo — team names contain the repo slug
          return t.toLowerCase().includes(repoSlug);
        });
        for (const t of teams) {
          // Check if this team has in-progress tasks
          const taskDir = path.join(tasksBase, t);
          if (!fs.existsSync(taskDir)) continue;
          const taskFiles = fs.readdirSync(taskDir).filter(f => f.endsWith('.json'));
          for (const f of taskFiles) {
            try {
              const tk = JSON.parse(fs.readFileSync(path.join(taskDir, f), 'utf8'));
              if (tk.status === 'in_progress') {
                teamName = t;
                if (tk.activeForm) task = tk.activeForm;
                break;
              }
            } catch (e) {}
          }
          if (teamName) break;
        }
        // If no in-progress tasks, pick the most recently created team
        if (!teamName && teams.length > 0) {
          teamName = teams.sort((a, b) => {
            try {
              const ca = JSON.parse(fs.readFileSync(path.join(teamsBase, a, 'config.json'), 'utf8'));
              const cb = JSON.parse(fs.readFileSync(path.join(teamsBase, b, 'config.json'), 'utf8'));
              return (cb.createdAt || 0) - (ca.createdAt || 0);
            } catch (e) { return 0; }
          })[0];
        }
        if (teamName) {
          if (teamName.startsWith('tribe-')) mode = 'tribe';
          else {
            // Check member count — single member = solo, multiple = squad
            try {
              const cfg = JSON.parse(fs.readFileSync(path.join(teamsBase, teamName, 'config.json'), 'utf8'));
              mode = (cfg.members || []).length <= 1 ? 'solo' : 'squad';
            } catch (e) { mode = 'squad'; }
          }
        }
        // Detect idle: team exists but no in-progress tasks
        if (teamName && !task) {
          const taskDir = path.join(tasksBase, teamName);
          let hasAnyPending = false;
          if (fs.existsSync(taskDir)) {
            try {
              const files = fs.readdirSync(taskDir).filter(f => f.endsWith('.json'));
              for (const f of files) {
                const tk = JSON.parse(fs.readFileSync(path.join(taskDir, f), 'utf8'));
                if (tk.status === 'pending' || tk.status === 'in_progress') {
                  hasAnyPending = true;
                  break;
                }
              }
            } catch (e) {}
          }
          if (hasAnyPending) task = 'idle — waiting';
        }
      } catch (e) {}
    }

    // PID of the Claude Code process (parent of this script)
    const pid = `\x1b[2mpid:${process.ppid}\x1b[0m`;

    // Output: mode:team (if any) | model | task (if any) | dir | pid | context bar
    const parts = [];
    if (mode && teamName) {
      // Strip repo-slug prefix from team name for brevity (e.g. "ale-workflow-sandbox-fixes" → "sandbox-fixes")
      const short = teamName.replace(/^tribe-/, '').replace(/^[^-]+-/, '');
      const colors = { tribe: '35', squad: '36', solo: '33' }; // magenta, cyan, yellow
      parts.push(`\x1b[1;${colors[mode] || '37'}m${mode}:${short}\x1b[0m`);
    }
    parts.push(`\x1b[2m${model}\x1b[0m`);
    if (task === 'idle — waiting') parts.push(`\x1b[2;33m${task}\x1b[0m`);
    else if (task) parts.push(`\x1b[1m${task}\x1b[0m`);
    parts.push(`\x1b[2m${dir}\x1b[0m`);
    parts.push(pid);
    process.stdout.write(parts.join(' │ ') + ctx);
  } catch (e) {}
});
