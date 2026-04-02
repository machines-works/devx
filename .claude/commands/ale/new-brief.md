---
name: ale:new-brief
description: Create a team brief and roadmap for a new milestone
argument-hint: "<milestone-name, e.g., 'v1.2-ux'>"
allowed-tools:
  - Read
  - Write
  - Bash
  - Glob
  - Grep
  - Task
  - AskUserQuestion
---
<objective>
Create a team brief (TEAM-BRIEF.md) and roadmap (ROADMAP.md) for a new milestone that can be
used by `/ale:start` to spawn an Agent Team.

This is the planning phase — no code is written, only documentation.
</objective>

<context>
Milestone name: $ARGUMENTS
Output directory: docs/$ARGUMENTS/
</context>

<process>
## Step 1: Gather Context

Read:
1. `CLAUDE.md` — project conventions and architecture
2. `.planning/PROJECT.md` — current project state and milestones (if exists)
3. `.planning/ROADMAP.md` — existing phases and what's shipped (if exists)
4. Any existing research docs the user points to

Ask the user:
1. "What's the goal of this milestone? (1-2 sentences)"
2. "What are the key features/fixes to deliver? (bullet list)"
3. "Are there research docs I should read for context?"
4. "Any hard constraints? (timeline, dependencies on other squads, specific files to avoid)"

## Step 2: Analyze Dependencies

Based on the feature list:
- Identify which files each feature touches
- Check for overlaps with active squad branches (`git branch | grep feat/`)
- Flag any shared files that need coordination (see protocol)
- Group features into phases by dependency (what must come first)

## Step 3: Design Wave Structure

Organize phases into waves:
- **Wave 1**: Foundation work with no inter-phase dependencies (can run in parallel)
- **Wave 2**: Work that depends on Wave 1 outputs
- **Wave 3**: Work that depends on Wave 2 outputs

Principles:
- Maximize parallelism within each wave
- Minimize cross-phase file conflicts
- Each phase should be completable by a single agent
- Phases that touch the same files should be in different waves (so merge resolves first)

**Task dependency mapping**: for each phase, note which other phases must complete first.
This becomes `blockedBy` when `/ale:start` creates the task list.

## Step 4: Write Roadmap

Create `docs/$ARGUMENTS/ROADMAP.md` with:
- Milestone goal
- Phase overview with wave grouping
- Per-phase sections: goal, files touched, success criteria, estimated effort
- Execution order diagram
- Coordination notes (shared files, conflict surface with other squads)
- Task dependency map (which phase IDs block which)

## Step 5: Write Team Brief

Create `docs/$ARGUMENTS/TEAM-BRIEF.md` with:

### Orchestrator Section
Instructions for `/ale:start`:
- Team name and description
- Task creation plan (phases → tasks, with blockedBy relationships)
- Teammate spawn plan (how many per wave, naming convention)
- Shared file coordination notes
- Quality gates

### Per-Phase Agent Context
For each phase:
- Goal and success criteria
- Files to read and edit
- Branch naming convention
- Coordination with other phases (shared files, DM targets)

### Human Operator Notes
- How to start: `/ale:start $ARGUMENTS`
- How to monitor: `/ale:status`
- Merge strategy: wave order, shared file resolution
- How to handle failures: retry, skip, escalate

## Step 6: Confirm with User

Show:
- Phase count and wave structure
- Key coordination concerns
- Task dependency graph

Ask: "Does this look right? Any adjustments before I finalize?"

## Step 7: Create Files

Write both files to `docs/$ARGUMENTS/`.
Confirm: "Team brief ready. Start the squad with `/ale:start $ARGUMENTS`."
</process>
