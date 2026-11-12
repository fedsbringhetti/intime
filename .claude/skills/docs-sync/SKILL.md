---
name: docs-sync
description: Keep intime's documentation in sync with the code. Use whenever code changes in src/, migrations/, tests/, web/, Dockerfile, docker-compose.yml, .env.example, scripts/ or CI — after implementing a feature or fix, when finishing a development step, before committing or opening a PR, and whenever the docs gate hook blocks a commit. Also use when the user asks to check, update, review or fix the docs, even if they do not say "docs" (e.g. "is everything documented?", "update the guide").
---

# Docs sync

The documentation in this repository is two things at once: a reference (how to use the API, configure and deploy the service, how it works) and a step-by-step record of how the code was built (`docs/development/`). Both are only useful if they match the code. This skill is the procedure for bringing them back in line after a change; a hook and CI check the result.

Layout (see `docs/README.md`):

| Path | Content |
|------|---------|
| `docs/user/api.md` | running it, every endpoint, parameters, defaults, responses, errors |
| `docs/user/configuration.md` | environment variables |
| `docs/user/deploy.md` | Docker image, compose, deployment steps and summary table |
| `docs/architecture.md` | what it is, how it works, code map, data, known limitations |
| `docs/development/README.md` | workflow, tests, chapter index and chapter template |
| `docs/development/NN-*.md` | one chapter per development step |
| `docs/plan/phase-N.md` | plans; record deviations here |
| `README.md`, `CLAUDE.md`, `PLAN.md` | overview and quick start; project rules and upstream notes; phases and decisions |

## Workflow

### 1. Find what changed

```bash
python3 .claude/skills/docs-sync/scripts/docs_impact.py
```

It lists the code changed on this branch (commits since `main`, plus staged, unstaged and untracked files), the documents each change may affect, and the development chapters that mention the changed files. Treat it as a starting point: then read the diff itself (`git diff main...HEAD` and `git diff`) to understand what changed in *behavior*, because a one-line change to a default can matter more than a large refactor.

### 2. Update the reference docs

| What changed | Update |
|--------------|--------|
| endpoint, parameter, default, limit, response field, error code | `docs/user/api.md` (tables, examples, error table); README quick start if its commands changed |
| a setting | `docs/user/configuration.md`, `.env.example`, the summary table in `docs/user/deploy.md` |
| design, a new module, a new table or migration | `docs/architecture.md`: "How it works", "Code map", "Data" |
| the web app (`web/`): screens, structure, build | `docs/architecture.md`; `docs/development/README.md` if its commands or checks changed |
| Dockerfile, compose, scripts | `docs/user/deploy.md` (steps and summary table), README |
| test helpers, test layout, CI | `docs/development/README.md` |
| something learned about the upstream API | the "Data source" notes in `CLAUDE.md` |
| a change against an approved plan | the plan in `docs/plan/`, saying what changed and why |
| a new known limitation | "Known limitations" in `docs/architecture.md` |

Write facts once: the README links to `docs/user/` instead of repeating its tables, so there is a single place to update.

### 3. Update the development chapter

Every development step has a chapter in `docs/development/`, written in the same PR as the code.

- **New step** (typically a new `feat/...` branch): create `docs/development/NN-short-name.md`, where `NN` is the next number reported by `docs_impact.py`. Follow the outline under "Writing a chapter" in `docs/development/README.md`: goal, decisions and why, files, how the code works, tests, check it by hand, pitfalls, later changes. Add a row to the chapter table in that README; put the PR number in once the PR exists.
- **Fix or follow-up to an earlier step**: update that step's chapter, and add a line under "Later changes" in any earlier chapter whose code this change touched (the impact report lists them).

What makes a chapter worth reading:

- **Excerpts come from the current files**, not from memory: open the file and copy. Keep each excerpt short (a function or the key part of one). When an excerpt simplifies the real code, put `...` in the shortened line or mark the block as illustrative (```` ```rust illustrative ````), so the checker knows not to compare it.
- **Explain why**, not only what: the alternatives considered and the reason for the choice.
- **Name the Rust features** a newcomer would trip on, in a short "Rust concept" note, once per chapter.
- **Record the real pitfalls** met while developing (errors, surprises, things that had to be redone), and how they were solved.
- **Only document checks that were actually run**: test names and commands, with the observed results.

### 4. Verify

```bash
python3 .claude/skills/docs-sync/scripts/check_docs.py
```

It checks every relative link and anchor, and that every significant line of the ```rust, ```sql, ```dockerfile, ```yaml, ```graphql, ```ts, ```svelte and ```html blocks in `docs/` (plans excluded) still exists in the code, ignoring whitespace. Fix every problem it reports; a missing excerpt line usually means the code changed and the explanation around it needs a look too, not just the snippet.

When you added or changed `curl` examples, run them against a local server (`cargo run`) when possible.

### 5. Report

Tell the user which documents changed and why, in a short list, and keep the documentation changes in the same commit or PR as the code they describe.

## Safety nets

- **Hook** (`.claude/settings.json`): before `git commit` and `gh pr create`, `scripts/docs_gate.py` blocks when code changed on the branch but no documentation did, or when `check_docs.py` fails. It checks the index for a commit (a temporary copy; with `-a`, plus tracked edits) and the committed branch for a PR, never unrelated working-folder edits, so never move docs aside to get past it; if one command stages and commits (`git add ... && git commit`) it checks the working folder. It matches only real commands, not words inside a heredoc or string. Its message lists the documents to review. For a change that truly has no documentation impact (a pure refactor, a dependency bump), add `[skip docs]` to the commit message and say why in the report.
- **CI** runs `check_docs.py` on every PR, so broken links and stale excerpts also fail there.

All documentation is in English, like everything else in the repository.
