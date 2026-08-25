"""Contract for future KServe and Argo Rollouts adapters."""

from __future__ import annotations

from typing import Protocol

from ml_release_platform.domain.models import ModelRelease


class ReleaseOrchestrator(Protocol):
    def deploy_candidate(self, release: ModelRelease) -> None: ...

    def candidate_is_healthy(self, release: ModelRelease) -> bool: ...

    def start_canary(self, release: ModelRelease) -> None: ...

    def promote_candidate(self, release: ModelRelease) -> None: ...

    def rollback_candidate(self, release: ModelRelease) -> None: ...
