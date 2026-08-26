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
use orchestration::LocalReleaseOrchestrator;
use repository::SqliteReleaseRepository;

/// Build an independently configurable application and initialise local persistence.
pub async fn create_app(database_url: Option<&str>) -> Result<Router, repository::RepositoryError> {
    let database_url = database_url
        .map(str::to_owned)
        .unwrap_or_else(|| Settings::from_environment().database_url);
    let repository = Arc::new(SqliteReleaseRepository::connect(&database_url).await?);
    let service = Arc::new(ReleaseService::new(
        repository,
        Arc::new(LocalReleaseOrchestrator::default()),
    ));
    Ok(router(service))
}
