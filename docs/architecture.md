# Architecture

## System purpose

The ML Release Platform is a reusable release control plane for ML models. It receives candidate releases, makes deterministic and auditable release decisions, and coordinates deployment progression. It is not a model-training system and is deliberately independent of any one ML application.

## Responsibilities

### API

The API will accept release submissions, retrieve releases, accept lifecycle commands, and perform client-facing validation. It must delegate business decisions to the application/service layer.

### Release service

The release service will coordinate lifecycle work: state transitions, policy evaluation, and orchestration decisions. It is the primary application-level boundary.

### Domain layer

The domain layer will define `ModelRelease`, release states, legal state transitions, policies, and evaluation results. It must remain deterministic and free of AWS, Kubernetes, and HTTP dependencies.

### Repository layer

Repositories will persist releases, release history, and policy-evaluation evidence. The initial local implementation is planned to use SQLite.

### Orchestration layer

The orchestration layer will abstract deployment of a candidate, candidate-health checks, canary initiation, candidate promotion, and rollback. Infrastructure-specific implementations belong behind this boundary.

### KServe

KServe will run model-serving and inference workloads as Kubernetes-native deployments. This platform will request and coordinate serving changes; it will not implement a serving runtime.

### Argo Rollouts

Argo Rollouts will execute progressive traffic shifts, rollout progression, and rollback mechanics. This platform will decide when those actions should occur; it will not reimplement them.

### EKS

Amazon EKS will provide the Kubernetes runtime, workload scheduling, and cluster-level infrastructure.

## Initial architecture

The planned local MVP has no cloud dependency in its core path:

```text
FastAPI
   ↓
Release Service
   ↓
Domain
   ├── State Machine
   └── Policy Engine
   ↓
Repositories
   └── SQLite initially

Orchestration
   └── Local/mock implementation initially
```

The target cloud architecture is:

```text
Client / CI
      ↓
Platform API
      ↓
Release Controller
      ↓
Policy Engine
      ↓
KServe / Argo
      ↓
EKS
      ↓
AWS Infrastructure
```

## State model

Planned release states are:

```text
SUBMITTED
VALIDATING
READY
DEPLOYING
CANARY
PROMOTED
REJECTED
ROLLED_BACK
FAILED
```

Examples of intended legal transitions include `SUBMITTED → VALIDATING`, `VALIDATING → READY | REJECTED | FAILED`, `READY → DEPLOYING`, `DEPLOYING → CANARY | FAILED`, and `CANARY → PROMOTED | ROLLED_BACK | FAILED`. The exact transition table will be defined centrally with the domain model. No adapter or route handler may bypass that validation.

## Integration boundaries

External ML applications integrate through stable boundaries rather than application-specific code. Expected boundaries include a REST API, model or container URIs, S3 artifact references, ECR image references, and release metadata. The platform must not import client application code or assume an application's internal training workflow.

## Non-goals

This platform is not intended to provide:

- model training;
- feature engineering;
- a custom model-serving runtime;
- a replacement for KServe;
- a replacement for Argo Rollouts;
- arbitrary workflow orchestration; or
- a generic Kubernetes management platform.

## Implementation guidance

Maintain the dependency direction `API → Application/Service → Domain → Repositories + Infrastructure Adapters`. New source and tests will be introduced with Milestone 1 under `src/ml_release_platform/` and `tests/`, respectively. Infrastructure configuration should remain separate from those layers.
