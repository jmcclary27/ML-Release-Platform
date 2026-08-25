"""Deterministic local orchestration implementation for service-level testing."""

from __future__ import annotations

from ml_release_platform.domain.models import ModelRelease


class LocalReleaseOrchestrator:
    """No-op adapter that models successful local orchestration by default."""

    def __init__(self, candidate_healthy: bool = True) -> None:
        self._candidate_healthy = candidate_healthy

    def deploy_candidate(self, release: ModelRelease) -> None:
        return None

    def candidate_is_healthy(self, release: ModelRelease) -> bool:
        return self._candidate_healthy

    def start_canary(self, release: ModelRelease) -> None:
        return None

    def promote_candidate(self, release: ModelRelease) -> None:
        return None

    def rollback_candidate(self, release: ModelRelease) -> None:
        return None
