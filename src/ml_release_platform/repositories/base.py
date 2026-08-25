"""Repository interface consumed by application services."""

from __future__ import annotations

from typing import Protocol

from ml_release_platform.domain.models import ModelRelease


class ReleaseRepository(Protocol):
    def create(self, release: ModelRelease) -> ModelRelease: ...

    def get(self, release_id: str) -> ModelRelease | None: ...

    def list(self) -> list[ModelRelease]: ...

    def update(self, release: ModelRelease) -> ModelRelease: ...
