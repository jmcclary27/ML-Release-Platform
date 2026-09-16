//! ML release control-plane library.

pub mod api;
pub mod application;
pub mod config;
pub mod domain;
pub mod orchestration;
pub mod repository;

use std::sync::Arc;

use api::router;
use application::ReleaseService;
use axum::Router;
use config::Settings;
use orchestration::{DockerOrchestratorConfig, DockerReleaseOrchestrator};
use repository::SqliteReleaseRepository;

/// Build an independently configurable application and initialise local persistence.
///
/// # Errors
///
/// Returns [`repository::RepositoryError`] when the configured database cannot be opened or
/// initialised.
pub async fn create_app(database_url: Option<&str>) -> Result<Router, repository::RepositoryError> {
    let settings = Settings::from_environment();
    let database_url = database_url.map_or_else(|| settings.database_url.clone(), str::to_owned);
    let repository = Arc::new(SqliteReleaseRepository::connect(&database_url).await?);
    let service = Arc::new(ReleaseService::new(
        repository,
        Arc::new(DockerReleaseOrchestrator::new(DockerOrchestratorConfig {
            container_port: settings.docker.container_port,
            health_path: settings.docker.health_path,
            inference_path: settings.docker.inference_path,
            startup_timeout: settings.docker.startup_timeout,
            poll_interval: settings.docker.poll_interval,
            verification_timeout: settings.docker.verification_timeout,
        })),
    ));
    Ok(router(service))
}
