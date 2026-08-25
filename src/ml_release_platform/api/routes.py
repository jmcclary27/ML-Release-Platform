"""Thin FastAPI routes delegating release behavior to the service layer."""

from __future__ import annotations

from typing import cast

from fastapi import APIRouter, HTTPException, Request, status

from ml_release_platform.api.schemas import (
    EvaluationRequest,
    EvaluationResponse,
    HealthResponse,
    ReleaseCreateRequest,
    ReleaseResponse,
)
from ml_release_platform.domain.errors import (
    InvalidTransitionError,
    PolicyValidationError,
    ReleaseNotFoundError,
)
from ml_release_platform.services.release_service import ReleaseService

router = APIRouter()


def _service(request: Request) -> ReleaseService:
    return cast(ReleaseService, request.app.state.release_service)


def _http_error(error: Exception) -> HTTPException:
    if isinstance(error, ReleaseNotFoundError):
        return HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=str(error))
    if isinstance(error, InvalidTransitionError):
        return HTTPException(status_code=status.HTTP_409_CONFLICT, detail=str(error))
    if isinstance(error, PolicyValidationError):
        return HTTPException(status_code=status.HTTP_422_UNPROCESSABLE_ENTITY, detail=str(error))
    raise error


@router.get("/health", response_model=HealthResponse)
async def health() -> HealthResponse:
    return HealthResponse()


@router.post("/releases", response_model=ReleaseResponse, status_code=status.HTTP_201_CREATED)
async def create_release(payload: ReleaseCreateRequest, request: Request) -> ReleaseResponse:
    release = _service(request).create_release(
        model_name=payload.model_name,
        version=payload.version,
        image_uri=payload.image_uri,
        artifact_uri=payload.artifact_uri,
        metadata=payload.metadata,
    )
    return ReleaseResponse.from_domain(release)


@router.get("/releases", response_model=list[ReleaseResponse])
async def list_releases(request: Request) -> list[ReleaseResponse]:
    return [ReleaseResponse.from_domain(release) for release in _service(request).list_releases()]


@router.get("/releases/{release_id}", response_model=ReleaseResponse)
async def get_release(release_id: str, request: Request) -> ReleaseResponse:
    try:
        return ReleaseResponse.from_domain(_service(request).get_release(release_id))
    except ReleaseNotFoundError as error:
        raise _http_error(error) from error


@router.post("/releases/{release_id}/evaluate", response_model=EvaluationResponse)
async def evaluate_release(
    release_id: str, payload: EvaluationRequest, request: Request
) -> EvaluationResponse:
    try:
        release = _service(request).evaluate_release(
            release_id, metrics=payload.metrics, raw_policy=payload.policy
        )
    except (ReleaseNotFoundError, InvalidTransitionError, PolicyValidationError) as error:
        raise _http_error(error) from error

    if release.evaluation is None:
        raise RuntimeError("Release evaluation was not persisted.")
    return EvaluationResponse.from_domain(release.evaluation)
