"""SQLAlchemy-backed local release persistence."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from sqlalchemy import JSON, DateTime, String, create_engine, select
from sqlalchemy.engine import Engine
from sqlalchemy.orm import DeclarativeBase, Mapped, Session, mapped_column, sessionmaker

from ml_release_platform.domain.models import ModelRelease
from ml_release_platform.domain.policy import GateEvaluation, PolicyEvaluation
from ml_release_platform.domain.states import ReleaseStatus


class Base(DeclarativeBase):
    pass


class ReleaseRecord(Base):
    __tablename__ = "releases"

    release_id: Mapped[str] = mapped_column(String(36), primary_key=True)
    model_name: Mapped[str] = mapped_column(String(255), nullable=False)
    version: Mapped[str] = mapped_column(String(255), nullable=False)
    image_uri: Mapped[str] = mapped_column(String(2048), nullable=False)
    artifact_uri: Mapped[str | None] = mapped_column(String(2048), nullable=True)
    status: Mapped[str] = mapped_column(String(32), nullable=False, index=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    metadata_json: Mapped[dict[str, Any]] = mapped_column("metadata", JSON, nullable=False)
    metrics_json: Mapped[dict[str, float]] = mapped_column("metrics", JSON, nullable=False)
    evaluation_json: Mapped[dict[str, Any] | None] = mapped_column(
        "evaluation", JSON, nullable=True
    )
    failure_reason: Mapped[str | None] = mapped_column(String(2048), nullable=True)


class Database:
    """Owns the SQLAlchemy engine and creates local tables on startup."""

    def __init__(self, database_url: str) -> None:
        connect_args = {"check_same_thread": False} if database_url.startswith("sqlite") else {}
        self.engine: Engine = create_engine(database_url, connect_args=connect_args)
        self.session_factory = sessionmaker(self.engine, expire_on_commit=False)

    def create_tables(self) -> None:
        Base.metadata.create_all(self.engine)


def _as_utc(timestamp: datetime) -> datetime:
    return timestamp.replace(tzinfo=UTC) if timestamp.tzinfo is None else timestamp


def _evaluation_to_json(evaluation: PolicyEvaluation | None) -> dict[str, Any] | None:
    if evaluation is None:
        return None
    return {
        "passed": evaluation.passed,
        "results": [
            {
                "metric": result.metric,
                "operator": result.operator,
                "threshold": result.threshold,
                "actual": result.actual,
                "passed": result.passed,
                "reason": result.reason,
            }
            for result in evaluation.results
        ],
    }


def _evaluation_from_json(value: dict[str, Any] | None) -> PolicyEvaluation | None:
    if value is None:
        return None
    return PolicyEvaluation(
        passed=bool(value["passed"]),
        results=tuple(
            GateEvaluation(
                metric=str(result["metric"]),
                operator=result["operator"],
                threshold=float(result["threshold"]),
                actual=float(result["actual"]) if result["actual"] is not None else None,
                passed=bool(result["passed"]),
                reason=result.get("reason"),
            )
            for result in value["results"]
        ),
    )


def _to_domain(record: ReleaseRecord) -> ModelRelease:
    return ModelRelease(
        release_id=record.release_id,
        model_name=record.model_name,
        version=record.version,
        image_uri=record.image_uri,
        artifact_uri=record.artifact_uri,
        status=ReleaseStatus(record.status),
        created_at=_as_utc(record.created_at),
        updated_at=_as_utc(record.updated_at),
        metadata=dict(record.metadata_json),
        metrics={name: float(value) for name, value in record.metrics_json.items()},
        evaluation=_evaluation_from_json(record.evaluation_json),
        failure_reason=record.failure_reason,
    )


def _apply(record: ReleaseRecord, release: ModelRelease) -> None:
    record.model_name = release.model_name
    record.version = release.version
    record.image_uri = release.image_uri
    record.artifact_uri = release.artifact_uri
    record.status = release.status.value
    record.created_at = release.created_at
    record.updated_at = release.updated_at
    record.metadata_json = release.metadata
    record.metrics_json = release.metrics
    record.evaluation_json = _evaluation_to_json(release.evaluation)
    record.failure_reason = release.failure_reason


class SqlAlchemyReleaseRepository:
    """Repository implementation that keeps SQLAlchemy outside the domain layer."""

    def __init__(self, session_factory: sessionmaker[Session]) -> None:
        self._session_factory = session_factory

    def create(self, release: ModelRelease) -> ModelRelease:
        with self._session_factory.begin() as session:
            record = ReleaseRecord(release_id=release.release_id)
            _apply(record, release)
            session.add(record)
        return release

    def get(self, release_id: str) -> ModelRelease | None:
        with self._session_factory() as session:
            record = session.get(ReleaseRecord, release_id)
            return _to_domain(record) if record is not None else None

    def list(self) -> list[ModelRelease]:
        with self._session_factory() as session:
            records = session.scalars(
                select(ReleaseRecord).order_by(ReleaseRecord.created_at.desc())
            ).all()
            return [_to_domain(record) for record in records]

    def update(self, release: ModelRelease) -> ModelRelease:
        with self._session_factory.begin() as session:
            record = session.get(ReleaseRecord, release.release_id)
            if record is None:
                raise KeyError(release.release_id)
            _apply(record, release)
        return release
