# Roadmap

## Milestone 0 — Repository Foundation

- Documentation
- Agent guidance
- Project conventions

## Milestone 1 — Local Control Plane

- API
- Domain model
- Persistence
- State machine
- Policy engine
- Local orchestration adapter
- Tests

## Milestone 2 — AWS/EKS Foundation (Complete)

- Terraform
- VPC
- EKS
- IAM
- ECR
- S3

The implemented development foundation includes a two-AZ VPC, private EKS managed-node capacity, a cost-conscious single NAT gateway, ECR, private versioned S3 storage, and Terraform documentation/smoke test. It does not deploy the application or install KServe or Argo Rollouts.

## Milestone 3 — KServe

- Install KServe
- Deploy candidate model
- Real orchestration adapter

## Milestone 4 — Argo Rollouts

- Progressive rollout
- Promotion
- Rollback

## Milestone 5 — End-to-End MVP

- Submit candidate
- Validate
- Evaluate policy
- Deploy
- Canary
- Promote or roll back
- Documented demo

## Post-MVP

- Prometheus/CloudWatch metrics
- Champion/candidate comparison
- Shadow deployments
- Delayed evaluation
- Model lineage
- Dashboard
- CRD/operator
- Additional deployment strategies
