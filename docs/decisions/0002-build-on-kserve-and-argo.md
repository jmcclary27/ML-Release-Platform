# Decision: Build on KServe and Argo Rollouts

## Context

Kubernetes-native model serving and progressive delivery are specialized concerns with established tools. Reimplementing either would broaden the platform substantially and duplicate mature capabilities.

## Options Considered

1. Build custom model-serving and progressive-rollout systems.
2. Use KServe for serving and Argo Rollouts for progressive delivery, with a higher-level release control plane.

## Decision

Use KServe for model serving and Argo Rollouts for progressive rollout mechanics. The ML Release Platform will own ML release lifecycle, deterministic release-policy evaluation, candidate/champion concepts, release evidence/history, and higher-level orchestration.

## Consequences

The platform must define adapter boundaries that can be tested without a cluster. It depends on the operational capabilities and contracts of KServe and Argo Rollouts, while avoiding duplicate serving and traffic-management implementations. This keeps differentiation at the release-decision layer.
