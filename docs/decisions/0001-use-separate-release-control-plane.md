# Decision: Use a separate ML release control plane

## Context

ML applications need to release models, but their training code, feature logic, and business requirements differ. Embedding release lifecycle control into each application would duplicate release logic and couple the platform to application internals.

## Options Considered

1. Build release behavior into every ML application.
2. Build a reusable release control plane that is separate from ML applications.

## Decision

Build a reusable release control plane as a system separate from any specific ML application. Applications will integrate through stable APIs and artifact references, including model/container URIs, S3 artifacts, ECR images, and release metadata.

## Consequences

The platform can be reused across applications and can maintain a consistent decision and evidence model. It must keep its integration contract stable and cannot assume client training code, feature stores, or application-specific runtime behavior. Client projects retain ownership of their models and application concerns.
