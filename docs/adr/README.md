# Architecture decision records

Structural or hard-to-reverse decisions, one file each: `NNNN-short-title.md`, numbered in order.

ADRs are append-only. A decision that no longer holds is not edited: a new ADR replaces it and says so, and the old one gets only a line at the top pointing to the new one.

Decisions made before this folder existed are in the decision tables of [../plan/](../plan/phase-3.md) and in [../../PLAN.md](../../PLAN.md); they stay there.

## Template

```markdown
# NNNN. Title

Date: YYYY-MM-DD · Status: accepted (or: superseded by NNNN)

## Context
What forces the decision, with the facts and where they were verified.

## Decision
What was decided.

## Consequences
What becomes easier or harder; what was rejected and why.
```

## Records

| # | Decision | Date |
|---|----------|------|
| 1 | [Plans, backlog and ADRs in the repository](0001-plans-backlog-and-adrs-in-the-repository.md) | 2026-11-12 |
