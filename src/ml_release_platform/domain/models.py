"""Core model-release entity."""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from datetime import UTC, datetime
from typing import Any

from ml_release_platform.domain.policy import PolicyEvaluation
from ml_release_platform.domain.states import ReleaseStatus, validate_transition


def utc_now() -> datetime:
    return datetime.now(UTC)


@dataclass(frozen=True, slots=True)
class ModelRelease:
    release_id: str
    model_name: str
    version: str
    image_uri: str
    artifact_uri: str | None
    status: ReleaseStatus
    created_at: datetime
    updated_at: datetime
    metadata: dict[str, Any] = field(default_factory=dict)
    metrics: dict[str, float] = field(default_factory=dict)
    evaluation: PolicyEvaluation | None = None
    failure_reason: str | None = None

    def transition_to(self, target: ReleaseStatus) -> ModelRelease:
        """Return a release in a valid next state with a refreshed timestamp."""
        validate_transition(self.status, target)
        return replace(self, status=target, updated_at=utc_now())

    def with_evaluation(
        self, metrics: dict[str, float], evaluation: PolicyEvaluation, failure_reason: str | None
    ) -> ModelRelease:
        """Attach auditable policy evidence to a release."""
        return replace(
            self,
            metrics=metrics,
            evaluation=evaluation,
            failure_reason=failure_reason,
            updated_at=utc_now(),
        )

    def with_failure(self, reason: str) -> ModelRelease:
        """Attach a failure reason to a lifecycle failure."""
        return replace(self, failure_reason=reason, updated_at=utc_now())
