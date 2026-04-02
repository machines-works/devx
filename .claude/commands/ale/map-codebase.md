---
name: ale:map-codebase
description: Parallel codebase mapping — produces STACK, ARCHITECTURE, CONVENTIONS, STRUCTURE docs
allowed-tools:
  - Read
  - Bash
  - Task(Explore)
---
<objective>
Spawn 4 parallel Explore agents to map the codebase and produce structured documentation.
Output goes to `.planning/codebase/` (or the path configured in `ale.config.yaml` under
`paths.codebase`). Re-running regenerates all documents (idempotent, not append).

These documents give agents structured knowledge of the project on session start,
eliminating redundant discovery across sessions.
</objective>

<context>
Arguments: $ARGUMENTS
</context>

<process>
## Step 1: Determine Output Directory

```bash
OUTPUT_DIR=$(grep -A1 'codebase:' .claude/ale.config.yaml 2>/dev/null | tail -1 | awk '{print $2}' | tr -d '"' || echo "")
if [ -z "$OUTPUT_DIR" ]; then
  OUTPUT_DIR=".planning/codebase"
fi
echo "Output directory: $OUTPUT_DIR"
```

Create the output directory:
```bash
mkdir -p "$OUTPUT_DIR"
```

## Step 2: Identify Project Root Files

Gather a quick inventory for the Explore agents to reference:

```bash
# List top-level files and directories
ls -1a | head -30

# Check for common config files
ls -1 package.json Cargo.toml go.mod pyproject.toml Gemfile build.gradle pom.xml mix.exs 2>/dev/null

# Show directory structure (2 levels deep, no node_modules/vendor)
find . -maxdepth 2 -type d \
  -not -path '*/node_modules*' \
  -not -path '*/vendor*' \
  -not -path '*/.git*' \
  -not -path '*/dist*' \
  -not -path '*/build*' \
  -not -path '*/.next*' \
  -not -path '*/__pycache__*' \
  -not -path '*/.claude*' \
  | head -50
```

Store the directory listing and config file names — pass them to each agent so they
know where to look without redundant discovery.

## Step 3: Spawn 4 Explore Agents in Parallel

Spawn all 4 agents using `Task(Explore)` with `run_in_background: true`. Each agent
produces one document. Pass the output directory and project inventory to each.

All agents share these instructions:

```
RULES:
- Write ONLY to the specified output file. Do not create other files.
- Keep the document under 150 lines. Be concise — reference file paths, don't inline code.
- Use markdown with clear headers and bullet points.
- Reference specific files and directories by path.
- If you cannot determine something, say "Unknown" rather than guessing.
- Overwrite the file completely (idempotent — this is a regeneration, not an append).
```

### Agent 1: STACK.md

```
You are mapping the technology stack for this project.

Output file: <OUTPUT_DIR>/STACK.md

Investigate and document:

## Languages & Runtime
- Primary language(s) and version requirements
- Runtime environment (Node.js, Go, Python, JVM, etc.)

## Frameworks
- Web framework, ORM, test framework, CLI framework
- Frontend framework if applicable (React, Vue, Svelte, etc.)

## Key Dependencies
- Top 10-15 most important dependencies (not exhaustive)
- What each one does in one line

## Build & Tooling
- Package manager (npm, pnpm, yarn, cargo, go modules, pip, etc.)
- Build tool / bundler
- Linter, formatter, type checker
- Task runner (make, just, mise, scripts)

## Configuration
- Config file format and locations
- Environment variable patterns (.env, config files)
- Feature flags approach if any

Read package.json, go.mod, Cargo.toml, pyproject.toml, or equivalent to identify
dependencies. Check for tsconfig.json, .eslintrc, prettier config, Makefile, etc.
```

### Agent 2: ARCHITECTURE.md

```
You are mapping the architecture of this project.

Output file: <OUTPUT_DIR>/ARCHITECTURE.md

Investigate and document:

## Overview
- One paragraph: what this project is and how it works at a high level

## Patterns & Layers
- Architectural pattern (MVC, hexagonal, microservices, monolith, CLI tool, library, etc.)
- Layer separation (handlers/controllers, services/domain, data/repository)
- Dependency injection or service wiring approach

## Data Flow
- How requests/commands enter the system
- How data moves through layers
- How responses/output are produced

## Entry Points
- Main entry point(s) (binary, server start, CLI entry)
- API routes or command definitions if applicable
- Background jobs, workers, or scheduled tasks

## Key Abstractions
- Core interfaces, traits, or abstract classes that define the system's contracts
- Domain models / entities

## External Integrations
- Databases, caches, message queues
- Third-party APIs or services
- File system patterns

Read the main entry point, router/command definitions, and 2-3 representative
handler/service files to understand the architecture. Do not read every file.
```

### Agent 3: CONVENTIONS.md

```
You are mapping the code conventions for this project.

Output file: <OUTPUT_DIR>/CONVENTIONS.md

Investigate and document:

## Code Style
- Naming conventions (camelCase, snake_case, PascalCase — for files, functions, types)
- File naming patterns
- Import/module organization

## Error Handling
- Error type pattern (custom errors, error codes, Result types)
- How errors propagate (throw, return, Result/Option)
- Logging approach and levels

## Testing
- Test framework and runner
- Test file location pattern (co-located, separate test directory)
- Test naming conventions
- Fixture / helper patterns
- Coverage requirements if configured

## Git & Workflow
- Branch naming conventions
- Commit message format (conventional commits, etc.)
- PR process if documented

## Documentation
- Inline documentation style (JSDoc, Go doc comments, docstrings)
- README conventions
- API documentation approach

Read CLAUDE.md, CONTRIBUTING.md, .eslintrc, prettier config, and 2-3 representative
test files to understand conventions. Check CI config for enforced standards.
```

### Agent 4: STRUCTURE.md

```
You are mapping the directory structure of this project.

Output file: <OUTPUT_DIR>/STRUCTURE.md

Investigate and document:

## Directory Layout
- ASCII tree of the top 2-3 levels (exclude node_modules, vendor, dist, build, .git)
- One-line description of each significant directory

## Key Locations
- Source code root
- Test directory
- Configuration files
- Static assets / public files
- Generated / build output
- Documentation
- CI/CD configuration
- Database migrations / schemas

## File Naming
- Source file naming pattern and what determines file boundaries
- Index/barrel files if used
- Generated files and how to identify them

## Module Organization
- How code is organized into modules/packages
- Public API surface (what's exported)
- Internal vs external boundaries

Use `find` or `ls -R` (with exclusions) to map the structure. Read a few directory
index files if they exist. Focus on layout, not content.
```

## Step 4: Wait for Completion

All 4 agents run in parallel. Wait for each to complete. As each finishes, note
success or failure.

If any agent fails, report which document was not generated and suggest re-running
the command.

## Step 5: Verify Output

After all agents complete, verify all 4 documents exist:

```bash
for doc in STACK.md ARCHITECTURE.md CONVENTIONS.md STRUCTURE.md; do
  if [ -f "$OUTPUT_DIR/$doc" ]; then
    lines=$(wc -l < "$OUTPUT_DIR/$doc")
    echo "OK  $doc ($lines lines)"
  else
    echo "MISSING  $doc"
  fi
done
```

## Step 6: Report

```
CODEBASE MAP COMPLETE
=====================
Output: <OUTPUT_DIR>/
  STACK.md         — N lines
  ARCHITECTURE.md  — N lines
  CONVENTIONS.md   — N lines
  STRUCTURE.md     — N lines

These docs are available for session preflight (additionalContext).
Re-run /ale:map-codebase to regenerate.
```

If any documents are missing, note them and suggest re-running.
</process>
