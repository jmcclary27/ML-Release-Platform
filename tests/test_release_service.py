from __future__ import annotations

import pytest

from ml_release_platform.domain.errors import (
    InvalidTransitionError,
    PolicyValidationError,
    ReleaseNotFoundError,
)
from ml_release_platform.domain.states import ReleaseStatus
from ml_release_platform.orchestration.local import LocalReleaseOrchestrator
from ml_release_platform.repositories.sqlalchemy import Database, SqlAlchemyReleaseRepository
from ml_release_platform.services.release_service import ReleaseService


def create_release(release_service: ReleaseService):
    return release_service.create_release(
        model_name="fraud-model",
        version="v2",
        image_uri="registry.example/fraud-model:v2",
        artifact_uri="s3://example-models/fraud/v2/model.pkl",
        metadata={"owner": "risk"},
    )


def passing_policy() -> dict:
    return {"gates": {"accuracy": {"min": 0.9}, "latency_p95": {"max": 200}}}


def test_create_get_and_list_releases(release_service: ReleaseService) -> None:
    release = create_release(release_service)

    fetched = release_service.get_release(release.release_id)
    assert fetched.status is ReleaseStatus.SUBMITTED
    assert fetched.metadata == {"owner": "risk"}
    assert [item.release_id for item in release_service.list_releases()] == [release.release_id]


def test_get_missing_release_raises_domain_error(release_service: ReleaseService) -> None:
    with pytest.raises(ReleaseNotFoundError):
        release_service.get_release("missing")


def test_successful_evaluation_persists_evidence_and_readies_release(
    release_service: ReleaseService,
) -> None:
    release = create_release(release_service)

    evaluated = release_service.evaluate_release(
        release.release_id, {"accuracy": 0.92, "latency_p95": 143}, passing_policy()
    )

    assert evaluated.status is ReleaseStatus.READY
    assert evaluated.evaluation is not None and evaluated.evaluation.passed is True
    persisted = release_service.get_release(release.release_id)
    assert persisted.metrics == {"accuracy": 0.92, "latency_p95": 143.0}
    assert persisted.evaluation == evaluated.evaluation


def test_missing_required_metric_rejects_and_persists_evidence(
    release_service: ReleaseService,
) -> None:
    release = create_release(release_service)

    evaluated = release_service.evaluate_release(
        release.release_id, {"accuracy": 0.92}, passing_policy()
    )

    assert evaluated.status is ReleaseStatus.REJECTED
    assert evaluated.evaluation is not None and evaluated.evaluation.results[1].actual is None
    assert evaluated.failure_reason == "Policy evaluation failed for: latency_p95."


def test_malformed_policy_leaves_release_unchanged(release_service: ReleaseService) -> None:
    release = create_release(release_service)

    with pytest.raises(PolicyValidationError):
        release_service.evaluate_release(release.release_id, {"accuracy": 0.92}, {"gates": {}})

    assert release_service.get_release(release.release_id).status is ReleaseStatus.SUBMITTED


def test_evaluation_cannot_repeat_after_final_state(release_service: ReleaseService) -> None:
    release = create_release(release_service)
    release_service.evaluate_release(
        release.release_id, {"accuracy": 0.92, "latency_p95": 143}, passing_policy()
    )

    with pytest.raises(InvalidTransitionError):
        release_service.evaluate_release(
            release.release_id, {"accuracy": 0.92, "latency_p95": 143}, passing_policy()
        )


def test_local_orchestration_drives_ready_canary_promoted_lifecycle(tmp_path) -> None:
    database = Database(f"sqlite:///{tmp_path / 'lifecycle.sqlite'}")
    database.create_tables()
    service = ReleaseService(
        SqlAlchemyReleaseRepository(database.session_factory), LocalReleaseOrchestrator()
    )
    release = create_release(service)
    ready = service.evaluate_release(
        release.release_id, {"accuracy": 0.92, "latency_p95": 143}, passing_policy()
    )

    assert service.deploy_release(ready.release_id).status is ReleaseStatus.CANARY
    assert service.promote_release(ready.release_id).status is ReleaseStatus.PROMOTED


def test_unhealthy_local_candidate_fails_deployment(tmp_path) -> None:
    database = Database(f"sqlite:///{tmp_path / 'unhealthy.sqlite'}")
    database.create_tables()
    service = ReleaseService(
        SqlAlchemyReleaseRepository(database.session_factory),
        LocalReleaseOrchestrator(candidate_healthy=False),
    )
    release = create_release(service)
    ready = service.evaluate_release(
        release.release_id, {"accuracy": 0.92, "latency_p95": 143}, passing_policy()
    )

    failed = service.deploy_release(ready.release_id)
    assert failed.status is ReleaseStatus.FAILED
    assert failed.failure_reason == "Candidate did not pass the local health check."


def test_local_orchestration_rolls_back_canary(tmp_path) -> None:
    database = Database(f"sqlite:///{tmp_path / 'rollback.sqlite'}")
    database.create_tables()
    service = ReleaseService(
        SqlAlchemyReleaseRepository(database.session_factory), LocalReleaseOrchestrator()
    )
    release = create_release(service)
    ready = service.evaluate_release(
        release.release_id, {"accuracy": 0.92, "latency_p95": 143}, passing_policy()
    )

    service.deploy_release(ready.release_id)
    assert service.rollback_release(ready.release_id).status is ReleaseStatus.ROLLED_BACK
