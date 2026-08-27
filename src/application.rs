//! Application service coordinating lifecycle behavior.

use std::sync::Arc;

use chrono::Utc;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::{
        errors::{InvalidTransitionError, PolicyValidationError, ReleaseNotFoundError},
        models::{Metadata, Metrics, ModelRelease},
        policy::{PolicyEvaluation, evaluate_policy, parse_policy},
        states::ReleaseStatus,
    },
    orchestration::ReleaseOrchestrator,
    repository::{ReleaseRepository, RepositoryError},
};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    NotFound(#[from] ReleaseNotFoundError),
    #[error(transparent)]
    InvalidTransition(#[from] InvalidTransitionError),
    #[error(transparent)]
    PolicyValidation(#[from] PolicyValidationError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}

pub struct ReleaseService {
    repository: Arc<dyn ReleaseRepository>,
    orchestrator: Arc<dyn ReleaseOrchestrator>,
}

impl ReleaseService {
    #[must_use]
    pub fn new(
        repository: Arc<dyn ReleaseRepository>,
        orchestrator: Arc<dyn ReleaseOrchestrator>,
    ) -> Self {
        Self {
            repository,
            orchestrator,
        }
    }

    /// # Errors
    ///
    /// Returns [`ServiceError`] when the release cannot be persisted.
    pub async fn create_release(
        &self,
        model_name: String,
        version: String,
        image_uri: String,
        artifact_uri: Option<String>,
        metadata: Metadata,
    ) -> Result<ModelRelease, ServiceError> {
        let now = Utc::now();
        let release = ModelRelease {
            release_id: Uuid::new_v4().to_string(),
            model_name,
            version,
            image_uri,
            artifact_uri,
            status: ReleaseStatus::Submitted,
            created_at: now,
            updated_at: now,
            metadata,
            metrics: Metrics::new(),
            evaluation: None,
            failure_reason: None,
        };
        Ok(self.repository.create(&release).await?)
    }

    /// # Errors
    ///
    /// Returns [`ServiceError`] when persistence fails or the release does not exist.
    pub async fn get_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        self.repository
            .get(release_id)
            .await?
            .ok_or_else(|| ReleaseNotFoundError(release_id.to_owned()).into())
    }

    /// # Errors
    ///
    /// Returns [`ServiceError`] when persistence fails.
    pub async fn list_releases(&self) -> Result<Vec<ModelRelease>, ServiceError> {
        Ok(self.repository.list().await?)
    }

    /// # Errors
    ///
    /// Returns [`ServiceError`] for invalid policies or metrics, missing releases, invalid state
    /// transitions, or persistence failures.
    pub async fn evaluate_release(
        &self,
        release_id: &str,
        raw_metrics: Map<String, Value>,
        raw_policy: Value,
    ) -> Result<ModelRelease, ServiceError> {
        let policy = parse_policy(&raw_policy)?;
        let release = self.get_release(release_id).await?;
        let validating = release.transition_to(ReleaseStatus::Validating)?;
        self.repository.update(&validating).await?;
        let evaluation = evaluate_policy(&policy, &raw_metrics)?;
        let metrics = raw_metrics
            .into_iter()
            .map(
                |(name, value)| -> Result<(String, f64), PolicyValidationError> {
                    let number = value.as_f64().ok_or_else(|| {
                        PolicyValidationError(format!("Metric '{name}' must be a number."))
                    })?;
                    Ok((name, number))
                },
            )
            .collect::<Result<Metrics, _>>()?;
        let failure_reason =
            (!evaluation.passed).then(|| Self::evaluation_failure_reason(&evaluation));
        let evaluated = validating.with_evaluation(metrics, evaluation.clone(), failure_reason);
        let target = if evaluation.passed {
            ReleaseStatus::Ready
        } else {
            ReleaseStatus::Rejected
        };
        Ok(self
            .repository
            .update(&evaluated.transition_to(target)?)
            .await?)
    }

    /// # Errors
    ///
    /// Returns [`ServiceError`] for missing releases, invalid state transitions, or persistence
    /// failures.
    pub async fn deploy_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        let release = self.get_release(release_id).await?;
        let deploying = release.transition_to(ReleaseStatus::Deploying)?;
        self.repository.update(&deploying).await?;
        let result = async {
            self.orchestrator
                .deploy_candidate(&deploying)
                .await
                .map_err(|error| error.to_string())?;
            if !self
                .orchestrator
                .candidate_is_healthy(&deploying)
                .await
                .map_err(|error| error.to_string())?
            {
                return Err("Candidate did not pass the local health check.".to_owned());
            }
            self.orchestrator
                .start_canary(&deploying)
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        match result {
            Ok(()) => Ok(self
                .repository
                .update(&deploying.transition_to(ReleaseStatus::Canary)?)
                .await?),
            Err(error) => self.fail_release(&deploying, error).await,
        }
    }

    /// # Errors
    ///
    /// Returns [`ServiceError`] for missing releases, invalid state transitions, or persistence
    /// failures.
    pub async fn promote_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        let release = self.get_release(release_id).await?;
        match self.orchestrator.promote_candidate(&release).await {
            Ok(()) => Ok(self
                .repository
                .update(&release.transition_to(ReleaseStatus::Promoted)?)
                .await?),
            Err(error) => self.fail_release(&release, error.to_string()).await,
        }
    }

    /// # Errors
    ///
    /// Returns [`ServiceError`] for missing releases, invalid state transitions, or persistence
    /// failures.
    pub async fn rollback_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        let release = self.get_release(release_id).await?;
        match self.orchestrator.rollback_candidate(&release).await {
            Ok(()) => Ok(self
                .repository
                .update(&release.transition_to(ReleaseStatus::RolledBack)?)
                .await?),
            Err(error) => self.fail_release(&release, error.to_string()).await,
        }
    }

    async fn fail_release(
        &self,
        release: &ModelRelease,
        error: String,
    ) -> Result<ModelRelease, ServiceError> {
        Ok(self
            .repository
            .update(
                &release
                    .with_failure(error)
                    .transition_to(ReleaseStatus::Failed)?,
            )
            .await?)
    }
    fn evaluation_failure_reason(evaluation: &PolicyEvaluation) -> String {
        format!(
            "Policy evaluation failed for: {}.",
            evaluation
                .results
                .iter()
                .filter(|result| !result.passed)
                .map(|result| result.metric.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}
