use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use super::{
    errors::InvalidTransitionError,
    policy::PolicyEvaluation,
    states::{ReleaseStatus, validate_transition},
};

pub type Metadata = Map<String, Value>;
pub type Metrics = BTreeMap<String, f64>;

#[derive(Clone, Debug, PartialEq)]
pub struct ModelRelease {
    pub release_id: String,
    pub model_name: String,
    pub version: String,
    pub image_uri: String,
    pub artifact_uri: Option<String>,
    pub status: ReleaseStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: Metadata,
    pub metrics: Metrics,
    pub evaluation: Option<PolicyEvaluation>,
    pub failure_reason: Option<String>,
}

impl ModelRelease {
    /// # Errors
    ///
    /// Returns [`InvalidTransitionError`] when the target status is not reachable from the
    /// release's current status.
    pub fn transition_to(&self, target: ReleaseStatus) -> Result<Self, InvalidTransitionError> {
        validate_transition(self.status, target)?;
        let mut transitioned = self.clone();
        transitioned.status = target;
        transitioned.updated_at = Utc::now();
        Ok(transitioned)
    }

    #[must_use]
    pub fn with_evaluation(
        &self,
        metrics: Metrics,
        evaluation: PolicyEvaluation,
        failure_reason: Option<String>,
    ) -> Self {
        let mut updated = self.clone();
        updated.metrics = metrics;
        updated.evaluation = Some(evaluation);
        updated.failure_reason = failure_reason;
        updated.updated_at = Utc::now();
        updated
    }

    #[must_use]
    pub fn with_failure(&self, reason: String) -> Self {
        let mut updated = self.clone();
        updated.failure_reason = Some(reason);
        updated.updated_at = Utc::now();
        updated
    }
}
