//! Central lifecycle transition rules.

use serde::{Deserialize, Serialize};

use super::errors::InvalidTransitionError;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReleaseStatus {
    Submitted,
    Validating,
    Ready,
    Deploying,
    Verifying,
    RollingBack,
    Released,
    Rejected,
    RolledBack,
    Failed,
}

impl ReleaseStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Released | Self::Rejected | Self::RolledBack | Self::Failed
        )
    }

    #[must_use]
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Submitted, Self::Validating)
                | (
                    Self::Validating,
                    Self::Ready | Self::Rejected | Self::Failed
                )
                | (Self::Ready, Self::Deploying)
                | (Self::Deploying, Self::Verifying | Self::Failed)
                | (Self::Verifying, Self::Released | Self::RollingBack)
                | (Self::RollingBack, Self::RolledBack | Self::Failed)
        )
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submitted => "SUBMITTED",
            Self::Validating => "VALIDATING",
            Self::Ready => "READY",
            Self::Deploying => "DEPLOYING",
            Self::Verifying => "VERIFYING",
            Self::RollingBack => "ROLLING_BACK",
            Self::Released => "RELEASED",
            Self::Rejected => "REJECTED",
            Self::RolledBack => "ROLLED_BACK",
            Self::Failed => "FAILED",
        }
    }
}

impl std::fmt::Display for ReleaseStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReleaseStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "SUBMITTED" => Ok(Self::Submitted),
            "VALIDATING" => Ok(Self::Validating),
            "READY" => Ok(Self::Ready),
            "DEPLOYING" => Ok(Self::Deploying),
            "VERIFYING" => Ok(Self::Verifying),
            "ROLLING_BACK" => Ok(Self::RollingBack),
            "RELEASED" => Ok(Self::Released),
            "REJECTED" => Ok(Self::Rejected),
            "ROLLED_BACK" => Ok(Self::RolledBack),
            "FAILED" => Ok(Self::Failed),
            _ => Err(format!("Unknown release status '{value}'.")),
        }
    }
}

/// # Errors
///
/// Returns [`InvalidTransitionError`] when the target status is not reachable from the current
/// status.
pub fn validate_transition(
    current: ReleaseStatus,
    target: ReleaseStatus,
) -> Result<(), InvalidTransitionError> {
    if current.can_transition_to(target) {
        Ok(())
    } else {
        Err(InvalidTransitionError {
            current: current.to_string(),
            target: target.to_string(),
        })
    }
}
