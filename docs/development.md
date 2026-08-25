# Development Conventions

## Branching

Do not work directly on `main`. Prefer short-lived branches named:

```text
feature/<short-description>
fix/<short-description>
docs/<short-description>
```

## Commits

Prefer small commits with one clear intent. Do not mix unrelated changes in a commit. Use commit messages that describe the change rather than the process used to make it.

## Pull requests

Each pull request should explain the problem being solved, implementation summary, testing performed, known limitations, relevant screenshots or logs, and intentionally deferred follow-up work. Keep PRs focused enough for reviewers and coding agents to validate quickly.

## Testing expectations

As implementation is introduced, include domain unit tests, service tests, API tests, adapter tests, and infrastructure validation where appropriate. Tests must not depend on local-only files, gitignored datasets, manual credentials, or hidden developer-machine state.

Tests should be deterministic and run in CI. Cloud/Kubernetes behavior should normally be exercised through fakes or mocks; explicitly scoped integration tests may use a designated environment and must state that requirement.

## Verification and future CI

Before submitting work, run the relevant tests and all configured formatting, linting, and type-checking tools. Review the diff for unrelated or generated changes.

When tooling is added, CI should enforce:

- tests;
- formatting;
- linting;
- type checking;
- Terraform validation where Terraform exists; and
- credential-leak prevention.

This repository has no executable tooling or CI workflow yet, so there are currently no project commands to run. Add documented commands only alongside working tooling.
