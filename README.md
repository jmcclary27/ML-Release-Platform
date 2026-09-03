# ML Release Platform

## Project overview

This repository contains a local-first foundation for a Kubernetes-native ML release control plane. It sits above deployment tooling to manage the lifecycle of candidate model releases: registration, policy decisions, orchestration, promotion, rollback, and the evidence behind those decisions.

Responsibilities are intentionally separated:

- **This platform** owns model release lifecycle, policy evaluation, promotion decisions, release history, and orchestration.
- **KServe** owns model serving and inference workloads.
- **Argo Rollouts** owns progressive rollout mechanics.
- **Amazon EKS** provides the Kubernetes runtime.
- **AWS services** provide infrastructure, storage, container registry, and observability where needed.

The platform does not replace KServe or Argo Rollouts. The Rust control plane remains local-first and does **not** connect to Kubernetes, KServe, Argo Rollouts, or external metrics systems. A separately managed, low-cost, ephemeral AWS/EKS foundation is available under [`infra/`](infra/README.md); it does not deploy this application yet.

## Product goal

```text
Candidate Model
      ↓
Register Release
      ↓
Validate
      ↓
Evaluate Policy
      ↓
Deploy Candidate
      ↓
Progressive Rollout
      ↓
Promote or Roll Back
```

Later versions may add automatic metric collection, champion-vs-candidate comparison, shadow deployments, delayed ground-truth evaluation, model lineage, a release dashboard, and Kubernetes CRDs/operator support.

## Current MVP capabilities

- REST API for creating, listing, retrieving, and evaluating model releases.
- SQLite-backed local persistence for release details, optional metadata, metrics, evaluation evidence, and failure reasons.
- Explicit, centrally validated release-state transitions.
- Deterministic minimum/maximum metric gates with auditable structured results.
- A local orchestration adapter and service-level deploy, canary, promotion, and rollback operations; no Kubernetes calls are made.

Future versions will integrate Amazon EKS, KServe, Argo Rollouts, and production observability.

## High-level architecture

```text
Client / CI
    │
    ▼
Release Platform API
    │
    ▼
Release Service / Controller
    │
    ├── Policy Engine
    ├── Persistence
    └── Orchestration Layer
            │
            ├── KServe
            ├── Argo Rollouts
            └── Kubernetes API
                    │
                    ▼
                   EKS
```

See [the architecture document](docs/architecture.md) for component responsibilities and boundaries.

## Local development

Requires Rust 1.98 or later.

```bash
cargo run
```

The API uses `sqlite:///./ml_release_platform.db` by default. Set `ML_RELEASE_DATABASE_URL` to use another SQLAlchemy-compatible database URL; see [.env.example](.env.example).

Run the local checks with:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Delivery milestones

- Local Control Plane — **complete**
- AWS/EKS Foundation — **complete** ([ephemeral demo lifecycle](infra/README.md))
- KServe Integration — **next**
- Argo Rollouts — **future**

## Example workflow

Create a candidate release:

```bash
curl -X POST http://127.0.0.1:8000/releases \
  -H 'content-type: application/json' \
  -d '{
    "model_name": "fraud-model",
    "version": "v2",
    "image_uri": "123456789.dkr.ecr.us-east-1.amazonaws.com/fraud-model:v2",
    "artifact_uri": "s3://example-models/fraud/v2/model.pkl"
  }'
```

Inspect it with `GET /releases/{release_id}`, then submit metrics and a policy:

```bash
curl -X POST http://127.0.0.1:8000/releases/<release_id>/evaluate \
  -H 'content-type: application/json' \
  -d '{
    "metrics": {"accuracy": 0.92, "latency_p95": 143, "error_rate": 0.003},
    "policy": {
      "gates": {
        "accuracy": {"min": 0.90},
        "latency_p95": {"max": 200},
        "error_rate": {"max": 0.01}
      }
    }
  }'
```

A passing evaluation records its gate results and changes the release from `SUBMITTED` through `VALIDATING` to `READY`. A valid policy with a failed or missing required metric records failed evidence and changes the release to `REJECTED`. Malformed policies are rejected with HTTP 422 without changing the release. `GET /health` returns `{ "status": "ok" }`.

## Roadmap

1. Repository and documentation foundation
2. Local API/domain/policy engine
3. AWS/EKS infrastructure
4. KServe integration
5. Argo Rollouts integration
6. End-to-end MVP
7. Automatic observability metrics
8. Champion-vs-candidate evaluation
9. Shadow deployment
10. Registry/lineage
11. Web UI
12. Kubernetes-native CRD/operator

The milestone detail and sequencing live in [docs/roadmap.md](docs/roadmap.md).
