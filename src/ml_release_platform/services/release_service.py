"""Application service for model-release behavior."""

from __future__ import annotations

from typing import Any
from uuid import uuid4

from ml_release_platform.domain.errors import OrchestrationError, ReleaseNotFoundError
from ml_release_platform.domain.models import ModelRelease, utc_now
from ml_release_platform.domain.policy import PolicyEvaluation, evaluate_policy, parse_policy
from ml_release_platform.domain.states import ReleaseStatus
from ml_release_platform.orchestration.base import ReleaseOrchestrator
from ml_release_platform.repositories.base import ReleaseRepository


class ReleaseService:
    """Coordinates release lifecycle decisions without HTTP or cloud dependencies."""

    def __init__(self, repository: ReleaseRepository, orchestrator: ReleaseOrchestrator) -> None:
        self._repository = repository
        self._orchestrator = orchestrator

    def create_release(
        self,
        *,
        model_name: str,
        version: str,
        image_uri: str,
        artifact_uri: str | None,
        metadata: dict[str, Any],
    ) -> ModelRelease:
        now = utc_now()
        release = ModelRelease(
            release_id=str(uuid4()),
            model_name=model_name,
            version=version,
            image_uri=image_uri,
            artifact_uri=artifact_uri,
            status=ReleaseStatus.SUBMITTED,
            created_at=now,
            updated_at=now,
            metadata=metadata,
        )
        return self._repository.create(release)

    def get_release(self, release_id: str) -> ModelRelease:
        release = self._repository.get(release_id)
        if release is None:
            raise ReleaseNotFoundError(f"Release '{release_id}' was not found.")
        return release

    def list_releases(self) -> list[ModelRelease]:
        return self._repository.list()

    def evaluate_release(
        self, release_id: str, metrics: dict[str, float], raw_policy: dict[str, Any]
    ) -> ModelRelease:
        """Persist deterministic evaluation evidence and advance a submitted release."""
        policy = parse_policy(raw_policy)
        release = self.get_release(release_id)
        validating = release.transition_to(ReleaseStatus.VALIDATING)
        self._repository.update(validating)

        evaluation = evaluate_policy(policy, metrics)
        failure_reason = None if evaluation.passed else self._evaluation_failure_reason(evaluation)
        evaluated = validating.with_evaluation(metrics, evaluation, failure_reason)
        target = ReleaseStatus.READY if evaluation.passed else ReleaseStatus.REJECTED
        finalized = evaluated.transition_to(target)
        return self._repository.update(finalized)

    def deploy_release(self, release_id: str) -> ModelRelease:
        """Deploy a ready release through the configured orchestration adapter."""
        release = self.get_release(release_id)
        deploying = release.transition_to(ReleaseStatus.DEPLOYING)
        self._repository.update(deploying)
        try:
            self._orchestrator.deploy_candidate(deploying)
            if not self._orchestrator.candidate_is_healthy(deploying):
                raise OrchestrationError("Candidate did not pass the local health check.")
            self._orchestrator.start_canary(deploying)
        except Exception as error:
            failed = deploying.with_failure(str(error)).transition_to(ReleaseStatus.FAILED)
            return self._repository.update(failed)
        return self._repository.update(deploying.transition_to(ReleaseStatus.CANARY))

    def promote_release(self, release_id: str) -> ModelRelease:
        """Promote a canary release through the configured orchestration adapter."""
        release = self.get_release(release_id)
        try:
            self._orchestrator.promote_candidate(release)
        except Exception as error:
            return self._fail_canary(release, error)
        return self._repository.update(release.transition_to(ReleaseStatus.PROMOTED))

    def rollback_release(self, release_id: str) -> ModelRelease:
        """Roll back a canary release through the configured orchestration adapter."""
        release = self.get_release(release_id)
        try:
            self._orchestrator.rollback_candidate(release)
        except Exception as error:
            return self._fail_canary(release, error)
        return self._repository.update(release.transition_to(ReleaseStatus.ROLLED_BACK))

    def _fail_canary(self, release: ModelRelease, error: Exception) -> ModelRelease:
        failed = release.with_failure(str(error)).transition_to(ReleaseStatus.FAILED)
        return self._repository.update(failed)

    @staticmethod
    def _evaluation_failure_reason(evaluation: PolicyEvaluation) -> str:
        failed_metrics = [result.metric for result in evaluation.results if not result.passed]
        return f"Policy evaluation failed for: {', '.join(failed_metrics)}."
