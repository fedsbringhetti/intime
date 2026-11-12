# Documentation

| I want to... | Read |
|--------------|------|
| use the API | [user/api.md](user/api.md) |
| change a setting | [user/configuration.md](user/configuration.md) |
| run it in Docker or deploy it | [user/deploy.md](user/deploy.md) |
| understand how it works | [architecture.md](architecture.md) |
| follow how it was built, step by step | [development/](development/README.md) |
| see what is planned and why | [../PLAN.md](../PLAN.md) and [plan/](plan/phase-1.md) |
| see what is left to do | [backlog.md](backlog.md) |
| see why a structural choice was made | [adr/](adr/README.md) |

## Layout

```
docs/
├── README.md              this page
├── architecture.md        what the service is, how the parts fit together, code map, limits
├── backlog.md             work not done yet, with evidence
├── adr/                   architecture decision records, append-only
├── user/
│   ├── api.md             running it, every endpoint with examples, errors
│   ├── configuration.md   environment variables
│   └── deploy.md          Docker image and deployment on the home server
├── development/
│   ├── README.md          workflow, tests, chapter index and template
│   └── NN-*.md            one chapter per development step, in order
└── plan/
    ├── phase-1.md         detailed plan of Phase 1 (done)
    ├── phase-2.md         Phase 2: web app (done)
    ├── phase-3.md         Phase 3: GTT data, rules, trip tracking, Web Push (approved)
    ├── phase-4.md         Phase 4: authorization and first deployment (decisions confirmed)
    └── steps/             plan and final report of each medium or large item
```

## Keeping it current

- Docs change in the same commit as the code. Every development step adds a chapter in `development/` in the same PR as the code (template in [development/README.md](development/README.md#writing-a-chapter)).
- A change visible to users also updates `user/` and, when the design changes, `architecture.md`.
- Each phase has its plan in `plan/phase-N.md`, written and approved before the phase starts and updated if the plan changes.
- A medium or large item has its plan in `plan/steps/`, written before the code, with a final report. Structural decisions go in `adr/` (append-only); defects found along the way go in `backlog.md`.
- The `docs-sync` skill, a commit hook and a CI check keep this true: see [development/README.md](development/README.md#keeping-the-docs-in-sync).
