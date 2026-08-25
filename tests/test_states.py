from __future__ import annotations

import pytest

from ml_release_platform.domain.errors import InvalidTransitionError
from ml_release_platform.domain.models import ModelRelease, utc_now
from ml_release_platform.domain.states import LEGAL_TRANSITIONS, TERMINAL_STATES, ReleaseStatus


def release_in(status: ReleaseStatus) -> ModelRelease:
    now = utc_now()
    return ModelRelease(
        release_id="release-1",
        model_name="fraud-model",
        version="v2",
        image_uri="registry.example/fraud-model:v2",
        artifact_uri=None,
        status=status,
        created_at=now,
        updated_at=now,
    )


@pytest.mark.parametrize(
    ("current", "target"),
    [(current, target) for current, targets in LEGAL_TRANSITIONS.items() for target in targets],
)
def test_all_defined_transitions_are_allowed(current: ReleaseStatus, target: ReleaseStatus) -> None:
    assert release_in(current).transition_to(target).status is target


def test_invalid_transition_is_rejected() -> None:
    with pytest.raises(InvalidTransitionError, match="SUBMITTED to READY"):
        release_in(ReleaseStatus.SUBMITTED).transition_to(ReleaseStatus.READY)


@pytest.mark.parametrize("terminal", TERMINAL_STATES)
def test_terminal_states_reject_all_transitions(terminal: ReleaseStatus) -> None:
    with pytest.raises(InvalidTransitionError):
        release_in(terminal).transition_to(ReleaseStatus.FAILED)
