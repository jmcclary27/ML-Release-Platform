from __future__ import annotations

import pytest

from ml_release_platform.orchestration.local import LocalReleaseOrchestrator
from ml_release_platform.repositories.sqlalchemy import Database, SqlAlchemyReleaseRepository
from ml_release_platform.services.release_service import ReleaseService


@pytest.fixture
def release_service(tmp_path):
    database = Database(f"sqlite:///{tmp_path / 'releases.sqlite'}")
    database.create_tables()
    return ReleaseService(
        SqlAlchemyReleaseRepository(database.session_factory), LocalReleaseOrchestrator()
    )
