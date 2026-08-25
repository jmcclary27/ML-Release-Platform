from __future__ import annotations

import pytest

from ml_release_platform.domain.errors import PolicyValidationError
from ml_release_platform.domain.policy import evaluate_policy, parse_policy


def evaluate(policy: dict, metrics: dict):
    return evaluate_policy(parse_policy(policy), metrics)


@pytest.mark.parametrize(
    ("policy", "metrics"),
    [
        ({"gates": {"accuracy": {"min": 0.9}}}, {"accuracy": 0.9}),
        ({"gates": {"latency_p95": {"max": 200}}}, {"latency_p95": 200}),
    ],
)
def test_threshold_boundaries_pass(policy: dict, metrics: dict) -> None:
    assert evaluate(policy, metrics).passed is True


@pytest.mark.parametrize(
    ("policy", "metrics"),
    [
        ({"gates": {"accuracy": {"min": 0.9}}}, {"accuracy": 0.89}),
        ({"gates": {"latency_p95": {"max": 200}}}, {"latency_p95": 201}),
    ],
)
def test_threshold_failures_are_reported(policy: dict, metrics: dict) -> None:
    result = evaluate(policy, metrics)
    assert result.passed is False
    assert result.results[0].passed is False


def test_multiple_gates_produce_structured_results() -> None:
    result = evaluate(
        {
            "gates": {
                "accuracy": {"min": 0.9},
                "latency_p95": {"max": 200},
                "error_rate": {"max": 0.01},
            }
        },
        {"accuracy": 0.92, "latency_p95": 143, "error_rate": 0.003},
    )
    assert result.passed is True
    assert [gate.metric for gate in result.results] == ["accuracy", "latency_p95", "error_rate"]


def test_missing_metric_is_a_structured_gate_failure() -> None:
    result = evaluate({"gates": {"accuracy": {"min": 0.9}}}, {})
    assert result.passed is False
    assert result.results[0].actual is None
    assert result.results[0].reason == "Required metric was not supplied."


@pytest.mark.parametrize(
    "policy",
    [
        {},
        {"gates": {}},
        {"gates": {"accuracy": {}}},
        {"gates": {"accuracy": {"min": 0.9, "max": 1.0}}},
        {"gates": {"accuracy": {"min": "0.9"}}},
        {"gates": {"accuracy": {"min": float("nan")}}},
        {"gates": {"accuracy": {"min": float("inf")}}},
        {"gates": {"accuracy": {"min": 0.9, "unexpected": 1}}},
        {"gates": {"accuracy": 0.9}},
    ],
)
def test_malformed_policies_are_rejected(policy: dict) -> None:
    with pytest.raises(PolicyValidationError):
        parse_policy(policy)
