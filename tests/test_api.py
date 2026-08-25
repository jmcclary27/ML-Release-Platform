from __future__ import annotations

import asyncio
from typing import Any

import httpx
import pytest

from ml_release_platform.main import create_app


class ApiClient:
    """Synchronous test helper backed by HTTPX's in-process ASGI transport."""

    def __init__(self, app) -> None:
        self._app = app

    def get(self, path: str) -> httpx.Response:
        return self.request("GET", path)

    def post(self, path: str, **kwargs: Any) -> httpx.Response:
        return self.request("POST", path, **kwargs)

    def request(self, method: str, path: str, **kwargs: Any) -> httpx.Response:
        return asyncio.run(self._request(method, path, **kwargs))

    async def _request(self, method: str, path: str, **kwargs: Any) -> httpx.Response:
        transport = httpx.ASGITransport(app=self._app)
        async with httpx.AsyncClient(transport=transport, base_url="http://testserver") as client:
            return await client.request(method, path, **kwargs)


@pytest.fixture
def client(tmp_path) -> ApiClient:
    return ApiClient(create_app(f"sqlite:///{tmp_path / 'api.sqlite'}"))


def test_health(client: ApiClient) -> None:
    assert client.get("/health").json() == {"status": "ok"}


def test_create_list_and_get_release(client: ApiClient) -> None:
    created = client.post(
        "/releases",
        json={
            "model_name": "fraud-model",
            "version": "v2",
            "image_uri": "registry.example/fraud-model:v2",
            "artifact_uri": "s3://example-models/fraud/v2/model.pkl",
            "metadata": {"owner": "risk"},
        },
    )
    assert created.status_code == 201
    release = created.json()
    assert release["status"] == "SUBMITTED"

    assert client.get("/releases").json()[0]["release_id"] == release["release_id"]
    assert client.get(f"/releases/{release['release_id']}").json()["metadata"] == {"owner": "risk"}


def test_evaluation_passes_to_ready_and_returns_evidence(client: ApiClient) -> None:
    release_id = _create_release(client)
    response = client.post(
        f"/releases/{release_id}/evaluate",
        json={
            "metrics": {"accuracy": 0.92, "latency_p95": 143},
            "policy": {"gates": {"accuracy": {"min": 0.9}, "latency_p95": {"max": 200}}},
        },
    )

    assert response.status_code == 200
    assert response.json()["passed"] is True
    assert client.get(f"/releases/{release_id}").json()["status"] == "READY"


def test_missing_metric_rejects_release(client: ApiClient) -> None:
    release_id = _create_release(client)
    response = client.post(
        f"/releases/{release_id}/evaluate",
        json={"metrics": {}, "policy": {"gates": {"accuracy": {"min": 0.9}}}},
    )

    assert response.status_code == 200
    assert response.json()["passed"] is False
    assert client.get(f"/releases/{release_id}").json()["status"] == "REJECTED"


def test_malformed_policy_returns_422_without_changing_release(client: ApiClient) -> None:
    release_id = _create_release(client)
    response = client.post(
        f"/releases/{release_id}/evaluate",
        json={"metrics": {"accuracy": 0.92}, "policy": {"gates": {}}},
    )

    assert response.status_code == 422
    assert client.get(f"/releases/{release_id}").json()["status"] == "SUBMITTED"


def test_not_found_invalid_create_and_repeated_evaluation_are_reported(client: ApiClient) -> None:
    assert client.get("/releases/unknown").status_code == 404
    assert client.post("/releases", json={}).status_code == 422

    release_id = _create_release(client)
    payload = {
        "metrics": {"accuracy": 0.92},
        "policy": {"gates": {"accuracy": {"min": 0.9}}},
    }
    assert client.post(f"/releases/{release_id}/evaluate", json=payload).status_code == 200
    assert client.post(f"/releases/{release_id}/evaluate", json=payload).status_code == 409


def _create_release(client: ApiClient) -> str:
    response = client.post(
        "/releases",
        json={
            "model_name": "fraud-model",
            "version": "v2",
            "image_uri": "registry.example/fraud-model:v2",
        },
    )
    assert response.status_code == 201
    return str(response.json()["release_id"])
