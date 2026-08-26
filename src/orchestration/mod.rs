use async_trait::async_trait;

use crate::domain::{errors::OrchestrationError, models::ModelRelease};

#[async_trait]
pub trait ReleaseOrchestrator: Send + Sync {
    async fn deploy_candidate(&self, release: &ModelRelease) -> Result<(), OrchestrationError>;
    async fn candidate_is_healthy(
        &self,
        release: &ModelRelease,
    ) -> Result<bool, OrchestrationError>;
    async fn start_canary(&self, release: &ModelRelease) -> Result<(), OrchestrationError>;
    async fn promote_candidate(&self, release: &ModelRelease) -> Result<(), OrchestrationError>;
    async fn rollback_candidate(&self, release: &ModelRelease) -> Result<(), OrchestrationError>;
}

pub struct LocalReleaseOrchestrator {
    candidate_healthy: bool,
}

impl Default for LocalReleaseOrchestrator {
    fn default() -> Self {
        Self::new(true)
    }
}

impl LocalReleaseOrchestrator {
    #[must_use]
    pub const fn new(candidate_healthy: bool) -> Self {
        Self { candidate_healthy }
    }
}

#[async_trait]
impl ReleaseOrchestrator for LocalReleaseOrchestrator {
    async fn deploy_candidate(&self, _: &ModelRelease) -> Result<(), OrchestrationError> {
        Ok(())
    }
    async fn candidate_is_healthy(&self, _: &ModelRelease) -> Result<bool, OrchestrationError> {
        Ok(self.candidate_healthy)
    }
    async fn start_canary(&self, _: &ModelRelease) -> Result<(), OrchestrationError> {
        Ok(())
    }
    async fn promote_candidate(&self, _: &ModelRelease) -> Result<(), OrchestrationError> {
        Ok(())
    }
    async fn rollback_candidate(&self, _: &ModelRelease) -> Result<(), OrchestrationError> {
        Ok(())
    }
}
