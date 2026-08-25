"""Pydantic schemas for the HTTP API."""

from __future__ import annotations

from datetime import datetime
from typing import Any

from pydantic import BaseModel, ConfigDict, Field

from ml_release_platform.domain.models import ModelRelease
from ml_release_platform.domain.policy import GateEvaluation, PolicyEvaluation
from ml_release_platform.domain.states import ReleaseStatus


class ReleaseCreateRequest(BaseModel):
    model_config = ConfigDict(str_strip_whitespace=True)

    model_name: str = Field(min_length=1, max_length=255)
    version: str = Field(min_length=1, max_length=255)
    image_uri: str = Field(min_length=1, max_length=2048)
    artifact_uri: str | None = Field(default=None, max_length=2048)
    metadata: dict[str, Any] = Field(default_factory=dict)


class EvaluationRequest(BaseModel):
    metrics: dict[str, float]
    policy: dict[str, Any]


class GateEvaluationResponse(BaseModel):
    metric: str
    operator: str
    threshold: float
    actual: float | None
    passed: bool
    reason: str | None

    @classmethod
    def from_domain(cls, evaluation: GateEvaluation) -> GateEvaluationResponse:
        return cls(
            metric=evaluation.metric,
            operator=evaluation.operator,
            threshold=evaluation.threshold,
            actual=evaluation.actual,
            passed=evaluation.passed,
            reason=evaluation.reason,
        )


class EvaluationResponse(BaseModel):
    passed: bool
    results: list[GateEvaluationResponse]

    @classmethod
    def from_domain(cls, evaluation: PolicyEvaluation) -> EvaluationResponse:
        return cls(
            passed=evaluation.passed,
            results=[GateEvaluationResponse.from_domain(result) for result in evaluation.results],
        )


class ReleaseResponse(BaseModel):
    release_id: str
    model_name: str
    version: str
    image_uri: str
    artifact_uri: str | None
    status: ReleaseStatus
    created_at: datetime
    updated_at: datetime
    metadata: dict[str, Any]
    metrics: dict[str, float]
    evaluation: EvaluationResponse | None
    failure_reason: str | None

    @classmethod
    def from_domain(cls, release: ModelRelease) -> ReleaseResponse:
        return cls(
            release_id=release.release_id,
            model_name=release.model_name,
            version=release.version,
            image_uri=release.image_uri,
            artifact_uri=release.artifact_uri,
            status=release.status,
            created_at=release.created_at,
            updated_at=release.updated_at,
            metadata=release.metadata,
            metrics=release.metrics,
            evaluation=(
                EvaluationResponse.from_domain(release.evaluation)
                if release.evaluation is not None
                else None
            ),
            failure_reason=release.failure_reason,
        )


class HealthResponse(BaseModel):
    status: str = "ok"
