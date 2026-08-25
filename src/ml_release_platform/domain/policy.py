"""Deterministic policy parsing and metric-gate evaluation."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from math import isfinite
from typing import Any, Literal

from ml_release_platform.domain.errors import PolicyValidationError

GateOperator = Literal["min", "max"]


@dataclass(frozen=True, slots=True)
class PolicyGate:
    metric: str
    operator: GateOperator
    threshold: float


@dataclass(frozen=True, slots=True)
class ReleasePolicy:
    gates: tuple[PolicyGate, ...]


@dataclass(frozen=True, slots=True)
class GateEvaluation:
    metric: str
    operator: GateOperator
    threshold: float
    actual: float | None
    passed: bool
    reason: str | None = None


@dataclass(frozen=True, slots=True)
class PolicyEvaluation:
    passed: bool
    results: tuple[GateEvaluation, ...]


def _as_number(value: Any, field_name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise PolicyValidationError(f"{field_name} must be a number.")
    numeric_value = float(value)
    if not isfinite(numeric_value):
        raise PolicyValidationError(f"{field_name} must be finite.")
    return numeric_value


def parse_policy(raw_policy: Mapping[str, Any]) -> ReleasePolicy:
    """Validate an external policy representation and return its deterministic form."""
    if not isinstance(raw_policy, Mapping):
        raise PolicyValidationError("Policy must be an object.")
    if set(raw_policy) != {"gates"}:
        raise PolicyValidationError("Policy must contain exactly one 'gates' field.")

    raw_gates = raw_policy["gates"]
    if not isinstance(raw_gates, Mapping) or not raw_gates:
        raise PolicyValidationError("Policy gates must be a non-empty object.")

    gates: list[PolicyGate] = []
    for metric, raw_gate in raw_gates.items():
        if not isinstance(metric, str) or not metric.strip():
            raise PolicyValidationError("Policy gate names must be non-empty strings.")
        if not isinstance(raw_gate, Mapping):
            raise PolicyValidationError(f"Gate '{metric}' must be an object.")
        if set(raw_gate) not in ({"min"}, {"max"}):
            raise PolicyValidationError(
                f"Gate '{metric}' must contain exactly one of 'min' or 'max'."
            )
        operator: GateOperator = "min" if "min" in raw_gate else "max"
        gates.append(
            PolicyGate(
                metric=metric,
                operator=operator,
                threshold=_as_number(raw_gate[operator], f"Gate '{metric}' threshold"),
            )
        )
    return ReleasePolicy(gates=tuple(gates))


def evaluate_policy(policy: ReleasePolicy, metrics: Mapping[str, Any]) -> PolicyEvaluation:
    """Evaluate policy gates in input order without external side effects."""
    results: list[GateEvaluation] = []
    for gate in policy.gates:
        if gate.metric not in metrics:
            results.append(
                GateEvaluation(
                    metric=gate.metric,
                    operator=gate.operator,
                    threshold=gate.threshold,
                    actual=None,
                    passed=False,
                    reason="Required metric was not supplied.",
                )
            )
            continue

        actual = _as_number(metrics[gate.metric], f"Metric '{gate.metric}'")
        passed = actual >= gate.threshold if gate.operator == "min" else actual <= gate.threshold
        results.append(
            GateEvaluation(
                metric=gate.metric,
                operator=gate.operator,
                threshold=gate.threshold,
                actual=actual,
                passed=passed,
            )
        )
    return PolicyEvaluation(passed=all(result.passed for result in results), results=tuple(results))
