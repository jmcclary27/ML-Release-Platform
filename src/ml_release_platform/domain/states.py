"""Centralized release lifecycle state rules."""

from __future__ import annotations

from enum import StrEnum

from ml_release_platform.domain.errors import InvalidTransitionError


class ReleaseStatus(StrEnum):
    SUBMITTED = "SUBMITTED"
    VALIDATING = "VALIDATING"
    READY = "READY"
    DEPLOYING = "DEPLOYING"
    CANARY = "CANARY"
    PROMOTED = "PROMOTED"
    REJECTED = "REJECTED"
    ROLLED_BACK = "ROLLED_BACK"
    FAILED = "FAILED"


LEGAL_TRANSITIONS: dict[ReleaseStatus, frozenset[ReleaseStatus]] = {
    ReleaseStatus.SUBMITTED: frozenset({ReleaseStatus.VALIDATING}),
    ReleaseStatus.VALIDATING: frozenset(
        {ReleaseStatus.READY, ReleaseStatus.REJECTED, ReleaseStatus.FAILED}
    ),
    ReleaseStatus.READY: frozenset({ReleaseStatus.DEPLOYING}),
    ReleaseStatus.DEPLOYING: frozenset({ReleaseStatus.CANARY, ReleaseStatus.FAILED}),
    ReleaseStatus.CANARY: frozenset(
        {ReleaseStatus.PROMOTED, ReleaseStatus.ROLLED_BACK, ReleaseStatus.FAILED}
    ),
    ReleaseStatus.PROMOTED: frozenset(),
    ReleaseStatus.REJECTED: frozenset(),
    ReleaseStatus.ROLLED_BACK: frozenset(),
    ReleaseStatus.FAILED: frozenset(),
}

TERMINAL_STATES = frozenset(
    {
        ReleaseStatus.PROMOTED,
        ReleaseStatus.REJECTED,
        ReleaseStatus.ROLLED_BACK,
        ReleaseStatus.FAILED,
    }
)


def validate_transition(current: ReleaseStatus, target: ReleaseStatus) -> None:
    """Validate a single lifecycle transition against the shared transition table."""
    if target not in LEGAL_TRANSITIONS[current]:
        raise InvalidTransitionError(
            f"Cannot transition release from {current.value} to {target.value}."
        )
