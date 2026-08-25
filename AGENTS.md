# Agent Guide

## Project mission

This project will be a reusable ML release control plane for Kubernetes. It coordinates model-release lifecycle and policy decisions while delegating serving to KServe, progressive delivery to Argo Rollouts, and cluster runtime concerns to Amazon EKS.

The repository is currently in Milestone 0: documentation and repository setup. Do not infer that planned components already exist.

## Operating rules

1. Inspect relevant repository files and current configuration before editing.
2. Understand the applicable architecture documentation before adding or changing code.
3. Prefer small, focused, reviewable changes.
4. Do not expand scope beyond the issue or task.
5. Preserve existing behavior unless the task explicitly changes it.
6. Add or update tests whenever behavior changes.
7. Run the relevant tests before finishing.
8. Run configured formatting, linting, and type checking when available.
9. Never commit generated artifacts, credentials, caches, secrets, or local environment files.
10. Clearly report limitations, commands not run, and unverified assumptions.

## Architecture rules

Future implementation must preserve this dependency direction:

```text
API
 ↓
Application / Service Layer
 ↓
Domain Logic
 ↓
Repositories + Infrastructure Adapters
```

- Keep business logic out of route handlers.
- Keep Kubernetes and AWS dependencies out of pure domain logic.
- Put infrastructure integrations behind interfaces/adapters.
- Make policy evaluation deterministic.
- Centralize and explicitly validate release state transitions.
- Keep components testable without AWS or Kubernetes where practical.

Planned application code belongs under `src/ml_release_platform/`; corresponding tests belong under `tests/`. Create these directories only with the first implementation change that needs them. Keep deployment or infrastructure code separate from application and domain code.

## Testing rules

- Add unit tests for domain logic and integration/API tests for endpoints as those layers are introduced.
- Tests must be deterministic and runnable in CI.
- Do not depend on developer-local files, gitignored production data, manual credentials, hidden machine state, or unmarked external network access.
- Use fakes or mocks for cloud and Kubernetes boundaries unless an explicitly designated integration environment is part of the task.

## Scope discipline

Do not opportunistically implement roadmap items. An API task does not authorize Kubernetes work; an EKS task does not authorize UI work; a KServe task does not authorize redesigning the domain model unless necessary for that task.

## Completion checklist

Every final handoff must report:

- what changed and files changed;
- tests added or updated;
- commands executed and their results;
- linting/formatting/type-check results, or why they were unavailable;
- assumptions and known limitations; and
- the recommended next step.

Consult [docs/architecture.md](docs/architecture.md), [docs/development.md](docs/development.md), and [docs/roadmap.md](docs/roadmap.md) before making architectural choices.
