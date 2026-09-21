# Roadmap

## Milestone 0 — Repository Foundation

- Documentation
- Agent guidance
- Project conventions

## Milestone 1 — Local Control Plane (Complete after Sprint 0–1)

- API
- Domain model
- Persistence
- State machine
- Policy engine
- Local orchestration adapter
- Versioned detector HTTP contract and deterministic market-data fixtures
- Docker-backed health, inference-response validation, promotion, and rollback demo
- CI/client-submitted deterministic policy metrics
- Tests

## Milestone 2 — AWS/EKS Foundation (Complete)

- Terraform
- VPC
- EKS
- IAM
- ECR
- S3

The implemented development foundation includes a two-AZ VPC, a single public EKS managed-node worker (to avoid NAT gateway cost), ECR, private versioned S3 storage, Terraform lifecycle commands, and a Kubernetes Job smoke test. It does not deploy the application or install KServe or Argo Rollouts.

## Milestone 3 — KServe

- Install KServe
- Deploy candidate model
- Real orchestration adapter

## Milestone 4 — Argo Rollouts

- Progressive rollout
- Promotion
- Rollback

## Milestone 5 — End-to-End MVP (Remaining)

- Submit candidate
- Validate
- Evaluate policy
- Deploy
- Canary
- Promote or roll back
- Documented demo

The local control plane proves the release decision path against a local
container, but it is not the end-to-end Kubernetes MVP. Remaining work is a
production HTTP adapter and externally supplied artifacts for a detector such
as Market Regime Detection, then KServe candidate serving and Argo Rollouts
progressive traffic control. Automatic metrics collection remains Post-MVP.

## Post-MVP

- Prometheus/CloudWatch automatic metrics collection
- Champion/candidate comparison
- Shadow deployments
- Delayed evaluation
- Model lineage
- Dashboard
- CRD/operator
- Additional deployment strategies
