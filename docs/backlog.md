# Backlog

Work not done yet, one item per section, newest last. Defects found while working on something else land here with their evidence instead of being fixed in passing (see CLAUDE.md, How we work). Planned phases and steps live in [plan/](plan/phase-3.md), not here.

Each item: what is wrong or wanted, the evidence (file and line, command and output, date), and why it matters. Remove the item in the PR that resolves it.

## Docs gate reads the working folder

The commit hook (`.claude/skills/docs-sync/scripts/docs_gate.py`) runs `check_docs.py` on the working folder, not on what is staged. Splitting a step into commits forced moving docs aside, and on 2026-11-05 (Phase 3, step 4) that deleted chapter 17 and reverted eight docs by mistake; they were rebuilt from the agent transcript. The hook also matches the words of a commit command anywhere in a shell command, so a heredoc mentioning them triggers it (2026-11-12). Fix: check the index for a commit and the branch for a PR, and match only real commit commands.
