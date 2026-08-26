use thiserror::Error;

#[derive(Debug, Error)]
#[error("Release '{0}' was not found.")]
pub struct ReleaseNotFoundError(pub String);

#[derive(Debug, Error)]
#[error("Cannot transition release from {current} to {target}.")]
pub struct InvalidTransitionError {
    pub current: String,
    pub target: String,
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct PolicyValidationError(pub String);

#[derive(Debug, Error)]
#[error("{0}")]
pub struct OrchestrationError(pub String);
