---
name: ale:help
description: Show available /ale commands and usage guide
allowed-tools:
  - Read
---
<objective>
Display the complete /ale command reference.

Output ONLY the reference content. Do NOT add project-specific analysis, git status, or next-step suggestions.
</objective>

<process>
## Step 1: Read and display the reference
Read `docs/COMMAND-REFERENCE.md` (relative to the ale-workflow install root) and output its contents exactly as written, formatted as markdown.
</process>
