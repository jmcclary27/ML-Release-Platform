//! Application service coordinating the release lifecycle.

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
    orchestration::{
        DeploymentHandle, ReleaseOrchestrator, VerificationResult as OrchestrationVerification,
    },
    repository::{
        ActiveRelease, DeploymentAttempt, ReleaseEvent, ReleaseRepository, RepositoryError,
        VerificationResult,
    },
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
    #[error("release execution evidence is incomplete: {0}")]
    ExecutionEvidence(String),
}

/// Data needed by clients to explain a release's execution.
#[derive(Clone, Debug)]
pub struct ReleaseExecutionDetail {
    pub release: ModelRelease,
    pub deployment: Option<DeploymentAttempt>,
    pub verification: Option<VerificationResult>,
    pub events: Vec<ReleaseEvent>,
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
    /// Returns an error when the release does not exist or persistence fails.
    pub async fn get_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        self.repository
            .get(release_id)
            .await?
            .ok_or_else(|| ReleaseNotFoundError(release_id.to_owned()).into())
    }

    /// # Errors
    ///
    /// Returns an error when persistence fails.
    pub async fn list_releases(&self) -> Result<Vec<ModelRelease>, ServiceError> {
        Ok(self.repository.list().await?)
    }

    /// Validates request data before transitioning to `VALIDATING`, then persists the policy
    /// evidence before reaching `READY` or terminal `REJECTED`.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid input, an illegal lifecycle command, or persistence failure.
    pub async fn evaluate_release(
        &self,
        release_id: &str,
        raw_metrics: Map<String, Value>,
        raw_policy: Value,
    ) -> Result<ModelRelease, ServiceError> {
        let policy = parse_policy(&raw_policy)?;
        let metrics = parse_metrics(raw_metrics)?;
        let release = self.get_release(release_id).await?;
        let validating = self
            .repository
            .transition(
                &release,
                ReleaseStatus::Validating,
                Some("Policy evaluation started."),
            )
            .await?;
        let raw_evaluation_metrics = metrics
            .iter()
            .map(|(name, value)| (name.clone(), Value::from(*value)))
            .collect();
        let evaluation = evaluate_policy(&policy, &raw_evaluation_metrics)?;
        let failure_reason =
            (!evaluation.passed).then(|| Self::evaluation_failure_reason(&evaluation));
        let evaluated = validating.with_evaluation(metrics, evaluation.clone(), failure_reason);
        self.repository.update(&evaluated).await?;
        self.repository
            .record_event(
                release_id,
                if evaluation.passed {
                    "POLICY_EVALUATION_PASSED"
                } else {
                    "POLICY_EVALUATION_FAILED"
                },
                Some(&Self::evaluation_detail(&evaluation)),
            )
            .await?;
        let target = if evaluation.passed {
            ReleaseStatus::Ready
        } else {
            ReleaseStatus::Rejected
        };
        Ok(self
            .repository
            .transition(&evaluated, target, evaluated.failure_reason.as_deref())
            .await?)
    }

    /// Starts and health-checks a candidate, leaving the current champion active until verify.
    ///
    /// # Errors
    ///
    /// Returns an error for an illegal lifecycle command or persistence failure.
    pub async fn deploy_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        let release = self.get_release(release_id).await?;
        let deploying = self
            .repository
            .transition(
                &release,
                ReleaseStatus::Deploying,
                Some("Candidate deployment started."),
            )
            .await?;
        let started_at = Utc::now();
        match self.orchestrator.deploy_candidate(&deploying).await {
            Ok(handle) => {
                self.repository
                    .record_deployment(deployment_attempt(
                        &deploying,
                        Some(&handle),
                        true,
                        None,
                        started_at,
                    ))
                    .await?;
                Ok(self
                    .repository
                    .transition(
                        &deploying,
                        ReleaseStatus::Verifying,
                        Some("Candidate deployed and passed startup health check."),
                    )
                    .await?)
            }
            Err(error) => {
                let detail = error.to_string();
                self.repository
                    .record_deployment(deployment_attempt(
                        &deploying,
                        None,
                        false,
                        Some(detail.clone()),
                        started_at,
                    ))
                    .await?;
                self.fail_release(&deploying, detail).await
            }
        }
    }

    /// Verifies candidate inference, promoting it on success and rolling it back on failure.
    ///
    /// # Errors
    ///
    /// Returns an error for an illegal lifecycle command, missing deployment evidence, or
    /// persistence failure.
    pub async fn verify_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        let release = self.get_release(release_id).await?;
        Self::require_status(&release, ReleaseStatus::Verifying)?;
        let candidate = self.deployment_handle(&release).await?;
        let previous = self.previous_active(&release).await?;
        let verification = match self.orchestrator.verify_candidate(&candidate).await {
            Ok(result) => result,
            Err(error) => OrchestrationVerification::failed(
                crate::orchestration::VerificationFailureKind::Transport,
                format!("Could not run inference verification: {error}"),
                None,
                None,
            ),
        };
        let detail = verification_detail(&verification);
        self.repository
            .record_verification(VerificationResult {
                result_id: String::new(),
                release_id: release.release_id.clone(),
                passed: verification.passed,
                detail: Some(detail.clone()),
                checked_at: Utc::now(),
            })
            .await?;
        if verification.passed {
            // Commit the durable active pointer before retiring the previous container. A
            // database failure must never remove the only persisted champion deployment.
            let released = self
                .repository
                .promote(
                    &release,
                    Some("Candidate passed verification and is active."),
                )
                .await?;
            match self
                .orchestrator
                .promote_candidate(&candidate, previous.as_ref())
                .await
            {
                Ok(()) => {
                    self.repository
                        .record_event(
                            &release.release_id,
                            "PREVIOUS_DEPLOYMENT_CLEANED_UP",
                            Some("Previous active deployment removed after promotion."),
                        )
                        .await?;
                }
                Err(error) => {
                    self.repository
                        .record_event(
                            &release.release_id,
                            "PREVIOUS_DEPLOYMENT_CLEANUP_FAILED",
                            Some(&error.to_string()),
                        )
                        .await?;
                }
            }
            Ok(released)
        } else {
            self.rollback_after_failure(&release, &candidate, previous.as_ref(), detail)
                .await
        }
    }

    /// Explicitly rolls back a candidate awaiting inference verification.
    ///
    /// # Errors
    ///
    /// Returns an error for an illegal lifecycle command, missing deployment evidence, or
    /// persistence failure.
    pub async fn rollback_release(&self, release_id: &str) -> Result<ModelRelease, ServiceError> {
        let release = self.get_release(release_id).await?;
        Self::require_status(&release, ReleaseStatus::Verifying)?;
        let candidate = self.deployment_handle(&release).await?;
        let previous = self.previous_active(&release).await?;
        self.repository
            .record_event(
                release_id,
                "MANUAL_ROLLBACK_REQUESTED",
                Some("Manual rollback requested before runtime verification."),
            )
            .await?;
        self.rollback_after_failure(
            &release,
            &candidate,
            previous.as_ref(),
            "Candidate was manually rolled back.".to_owned(),
        )
        .await
    }

    /// # Errors
    ///
    /// Returns an error when the release does not exist or persistence fails.
    pub async fn get_history(&self, release_id: &str) -> Result<Vec<ReleaseEvent>, ServiceError> {
        self.get_release(release_id).await?;
        Ok(self.repository.events(release_id).await?)
    }

    /// Returns immutable audit history for all releases of one model.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence fails.
    pub async fn get_model_history(
        &self,
        model_name: &str,
    ) -> Result<Vec<ReleaseEvent>, ServiceError> {
        Ok(self.repository.events_for_model(model_name).await?)
    }

    /// # Errors
    ///
    /// Returns an error when the release does not exist or persistence fails.
    pub async fn get_execution_detail(
        &self,
        release_id: &str,
    ) -> Result<ReleaseExecutionDetail, ServiceError> {
        let release = self.get_release(release_id).await?;
        Ok(ReleaseExecutionDetail {
            deployment: self.repository.latest_deployment(release_id).await?,
            verification: self.repository.latest_verification(release_id).await?,
            events: self.repository.events(release_id).await?,
            release,
        })
    }

    /// # Errors
    ///
    /// Returns an error when persistence fails or the active pointer is invalid.
    pub async fn current_release(
        &self,
        model_name: &str,
    ) -> Result<Option<ModelRelease>, ServiceError> {
        self.release_from_active(self.repository.active(model_name).await?)
            .await
    }

    async fn previous_active(
        &self,
        candidate: &ModelRelease,
    ) -> Result<Option<ModelRelease>, ServiceError> {
        self.release_from_active(self.repository.active(&candidate.model_name).await?)
            .await
    }

    async fn release_from_active(
        &self,
        active: Option<ActiveRelease>,
    ) -> Result<Option<ModelRelease>, ServiceError> {
        match active {
            None => Ok(None),
            Some(active) => self
                .repository
                .get(&active.release_id)
                .await?
                .map(Some)
                .ok_or_else(|| {
                    ServiceError::Repository(RepositoryError::InvalidData(format!(
                        "active release '{}' for model '{}' does not exist",
                        active.release_id, active.model_name,
                    )))
                }),
        }
    }

    async fn deployment_handle(
        &self,
        release: &ModelRelease,
    ) -> Result<DeploymentHandle, ServiceError> {
        let attempt = self
            .repository
            .latest_deployment(&release.release_id)
            .await?
            .filter(|attempt| attempt.succeeded)
            .ok_or_else(|| {
                ServiceError::ExecutionEvidence(
                    "no successful deployment attempt exists".to_owned(),
                )
            })?;
        match (
            attempt.container_id,
            attempt.container_name,
            attempt.endpoint,
        ) {
            (Some(container_id), Some(container_name), Some(endpoint)) => Ok(DeploymentHandle {
                container_id,
                container_name,
                endpoint,
            }),
            _ => Err(ServiceError::ExecutionEvidence(
                "successful deployment is missing its container identity or endpoint".to_owned(),
            )),
        }
    }

    async fn rollback_after_failure(
        &self,
        release: &ModelRelease,
        candidate: &DeploymentHandle,
        previous: Option<&ModelRelease>,
        detail: String,
    ) -> Result<ModelRelease, ServiceError> {
        let rolling_back = self
            .repository
            .transition(release, ReleaseStatus::RollingBack, Some(&detail))
            .await?;
        match self
            .orchestrator
            .rollback_candidate(candidate, previous)
            .await
        {
            Ok(()) => Ok(self
                .repository
                .complete_rollback(
                    &rolling_back,
                    previous.map(|release| release.release_id.as_str()),
                    Some(&detail),
                )
                .await?),
            Err(error) => {
                self.fail_release(&rolling_back, format!("Rollback failed: {error}"))
                    .await
            }
        }
    }

    async fn fail_release(
        &self,
        release: &ModelRelease,
        error: String,
    ) -> Result<ModelRelease, ServiceError> {
        let failed = release.with_failure(error.clone());
        Ok(self
            .repository
            .transition(&failed, ReleaseStatus::Failed, Some(&error))
            .await?)
    }

    fn require_status(release: &ModelRelease, target: ReleaseStatus) -> Result<(), ServiceError> {
        if release.status == target {
            return Ok(());
        }
        Err(release.transition_to(target).unwrap_err().into())
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

    fn evaluation_detail(evaluation: &PolicyEvaluation) -> String {
        if evaluation.passed {
            "All policy gates passed.".to_owned()
        } else {
            Self::evaluation_failure_reason(evaluation)
        }
    }
}

fn parse_metrics(raw_metrics: Map<String, Value>) -> Result<Metrics, PolicyValidationError> {
    raw_metrics
        .into_iter()
        .map(|(name, value)| {
            value
                .as_f64()
                .map(|number| (name.clone(), number))
                .ok_or_else(|| PolicyValidationError(format!("Metric '{name}' must be a number.")))
        })
        .collect()
}

fn deployment_attempt(
    release: &ModelRelease,
    handle: Option<&DeploymentHandle>,
    succeeded: bool,
    detail: Option<String>,
    started_at: chrono::DateTime<Utc>,
) -> DeploymentAttempt {
    DeploymentAttempt {
        attempt_id: String::new(),
        release_id: release.release_id.clone(),
        container_id: handle.map(|value| value.container_id.clone()),
        container_name: handle.map(|value| value.container_name.clone()),
        endpoint: handle.map(|value| value.endpoint.clone()),
        succeeded,
        detail,
        started_at,
        finished_at: Utc::now(),
    }
}

fn verification_detail(result: &OrchestrationVerification) -> String {
    result.status.map_or_else(
        || result.detail.clone(),
        |status| format!("{} HTTP status: {status}.", result.detail),
    )
}
