//! HTTP adapter. Route handlers only validate transport data and delegate to the service.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    application::{ReleaseService, ServiceError},
    domain::{
        models::ModelRelease,
        policy::{GateEvaluation, PolicyEvaluation},
    },
};

#[derive(Clone)]
struct ApiState {
    service: Arc<ReleaseService>,
}

pub fn router(service: Arc<ReleaseService>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/releases", post(create_release).get(list_releases))
        .route("/releases/{release_id}", get(get_release))
        .route("/releases/{release_id}/evaluate", post(evaluate_release))
        .with_state(ApiState { service })
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Deserialize)]
struct ReleaseCreateRequest {
    model_name: String,
    version: String,
    image_uri: String,
    #[serde(default)]
    artifact_uri: Option<String>,
    #[serde(default)]
    metadata: Map<String, Value>,
}

impl ReleaseCreateRequest {
    fn validate_and_trim(mut self) -> Result<Self, ApiError> {
        self.model_name = validate_string(self.model_name, "model_name", 255)?;
        self.version = validate_string(self.version, "version", 255)?;
        self.image_uri = validate_string(self.image_uri, "image_uri", 2048)?;
        self.artifact_uri = self
            .artifact_uri
            .map(|value| validate_string(value, "artifact_uri", 2048))
            .transpose()?;
        Ok(self)
    }
}

fn validate_string(value: String, field: &str, maximum: usize) -> Result<String, ApiError> {
    let trimmed = value.trim().to_owned();
    if trimmed.is_empty() || trimmed.len() > maximum {
        return Err(ApiError::unprocessable(format!("Invalid {field}.")));
    }
    Ok(trimmed)
}

#[derive(Deserialize)]
struct EvaluationRequest {
    metrics: Map<String, Value>,
    policy: Value,
}

#[derive(Serialize)]
struct ReleaseResponse {
    release_id: String,
    model_name: String,
    version: String,
    image_uri: String,
    artifact_uri: Option<String>,
    status: crate::domain::states::ReleaseStatus,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    metadata: Map<String, Value>,
    metrics: Map<String, f64>,
    evaluation: Option<EvaluationResponse>,
    failure_reason: Option<String>,
}

impl From<ModelRelease> for ReleaseResponse {
    fn from(release: ModelRelease) -> Self {
        Self {
            release_id: release.release_id,
            model_name: release.model_name,
            version: release.version,
            image_uri: release.image_uri,
            artifact_uri: release.artifact_uri,
            status: release.status,
            created_at: release.created_at,
            updated_at: release.updated_at,
            metadata: release.metadata,
            metrics: release.metrics,
            evaluation: release.evaluation.map(Into::into),
            failure_reason: release.failure_reason,
        }
    }
}

#[derive(Serialize)]
struct GateEvaluationResponse {
    metric: String,
    operator: String,
    threshold: f64,
    actual: Option<f64>,
    passed: bool,
    reason: Option<String>,
}

impl From<GateEvaluation> for GateEvaluationResponse {
    fn from(value: GateEvaluation) -> Self {
        Self {
            metric: value.metric,
            operator: value.operator.as_str().to_owned(),
            threshold: value.threshold,
            actual: value.actual,
            passed: value.passed,
            reason: value.reason,
        }
    }
}

#[derive(Serialize)]
struct EvaluationResponse {
    passed: bool,
    results: Vec<GateEvaluationResponse>,
}

impl From<PolicyEvaluation> for EvaluationResponse {
    fn from(value: PolicyEvaluation) -> Self {
        Self {
            passed: value.passed,
            results: value.results.into_iter().map(Into::into).collect(),
        }
    }
}

async fn create_release(
    State(state): State<ApiState>,
    Json(payload): Json<ReleaseCreateRequest>,
) -> Result<(StatusCode, Json<ReleaseResponse>), ApiError> {
    let payload = payload.validate_and_trim()?;
    let release = state
        .service
        .create_release(
            payload.model_name,
            payload.version,
            payload.image_uri,
            payload.artifact_uri,
            payload.metadata,
        )
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(release.into())))
}

async fn list_releases(
    State(state): State<ApiState>,
) -> Result<Json<Vec<ReleaseResponse>>, ApiError> {
    state
        .service
        .list_releases()
        .await
        .map(|releases| Json(releases.into_iter().map(Into::into).collect()))
        .map_err(ApiError::from)
}

async fn get_release(
    State(state): State<ApiState>,
    Path(release_id): Path<String>,
) -> Result<Json<ReleaseResponse>, ApiError> {
    state
        .service
        .get_release(&release_id)
        .await
        .map(|release| Json(release.into()))
        .map_err(ApiError::from)
}

async fn evaluate_release(
    State(state): State<ApiState>,
    Path(release_id): Path<String>,
    Json(payload): Json<EvaluationRequest>,
) -> Result<Json<EvaluationResponse>, ApiError> {
    let release = state
        .service
        .evaluate_release(&release_id, payload.metrics, payload.policy)
        .await
        .map_err(ApiError::from)?;
    let evaluation = release
        .evaluation
        .ok_or_else(|| ApiError::internal("Release evaluation was not persisted."))?;
    Ok(Json(evaluation.into()))
}

struct ApiError {
    status: StatusCode,
    detail: String,
}

impl ApiError {
    fn unprocessable(detail: String) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            detail,
        }
    }
    fn internal(detail: &str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            detail: detail.to_owned(),
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::NotFound(error) => Self {
                status: StatusCode::NOT_FOUND,
                detail: error.to_string(),
            },
            ServiceError::InvalidTransition(error) => Self {
                status: StatusCode::CONFLICT,
                detail: error.to_string(),
            },
            ServiceError::PolicyValidation(error) => Self::unprocessable(error.to_string()),
            ServiceError::Repository(error) => Self::internal(&error.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "detail": self.detail })),
        )
            .into_response()
    }
}
