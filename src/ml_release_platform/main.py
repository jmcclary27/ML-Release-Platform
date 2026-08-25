"""FastAPI application entrypoint."""

from __future__ import annotations

from fastapi import FastAPI

from ml_release_platform.api.routes import router
from ml_release_platform.config import Settings
from ml_release_platform.orchestration.local import LocalReleaseOrchestrator
from ml_release_platform.repositories.sqlalchemy import Database, SqlAlchemyReleaseRepository
from ml_release_platform.services.release_service import ReleaseService


def create_app(database_url: str | None = None) -> FastAPI:
    """Create an independently configurable application instance."""
    resolved_database_url = database_url or Settings.from_environment().database_url
    database = Database(resolved_database_url)
    database.create_tables()

    app = FastAPI(title="ML Release Platform", version="0.1.0")
    app.state.database = database
    app.state.release_service = ReleaseService(
        SqlAlchemyReleaseRepository(database.session_factory), LocalReleaseOrchestrator()
    )
    app.include_router(router)
    return app


app = create_app()
