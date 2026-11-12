# 0001. Plans, backlog and ADRs in the repository

Date: 2026-11-12 · Status: accepted

## Context

Until Phase 3, step 4, the plan of a step lived in the conversation and in a scratch file outside the repository, decisions in the phase plans' tables, and defects found along the way were fixed in the same PR or forgotten. Step 4 cost about 6.8M effective tokens, mostly one long session re-reading itself; the plan had to be re-explained to every agent and could not be picked up by a new session.

## Decision

- The plan of a medium or large item is a file in `docs/plan/steps/` (at most about 150 lines), written before any code and ending with a one-page report of what was built and where the plan was wrong.
- Defects a change did not introduce go to `docs/backlog.md` with their evidence.
- Structural or hard-to-reverse decisions go to `docs/adr/`, append-only.
- The working rules (sizing, one review, verification, commits and pushes approved separately, PRs opened by the user, Conventional Commits) are in `CLAUDE.md`, How we work.

## Consequences

A new session starts from a file instead of a conversation, so phases can run in separate, short sessions. Plans and reports stay in history. The `.claude/handoff/` folder proposed the same day is dropped. Phase plans keep their decision tables; new structural decisions go to ADRs instead.
