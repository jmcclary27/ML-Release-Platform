"""Domain-level errors independent of transport or infrastructure."""


class ReleaseNotFoundError(Exception):
    """Raised when a requested release does not exist."""


class InvalidTransitionError(Exception):
    """Raised when a release lifecycle transition is not legal."""


class PolicyValidationError(ValueError):
    """Raised when a submitted release policy is malformed."""


class OrchestrationError(Exception):
    """Raised when an orchestration adapter cannot complete an operation."""
