use std::{path::PathBuf, str::FromStr, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ml_release_platform::{
    application::ReleaseService,
    create_app,
    domain::{
        models::{Metadata, ModelRelease},
        policy::{evaluate_policy, parse_policy},
        states::ReleaseStatus,
    },
    orchestration::LocalReleaseOrchestrator,
    repository::{ReleaseRepository, SqliteReleaseRepository},
};
use serde_json::{Map, Value, json};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt;

fn database_url(name: &str) -> (String, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "ml-release-platform-{name}-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    (format!("sqlite:///{}", path.display()), path)
}

async fn service(name: &str) -> (ReleaseService, PathBuf) {
    let (url, path) = database_url(name);
    let repository = Arc::new(SqliteReleaseRepository::connect(&url).await.unwrap());
    (
        ReleaseService::new(repository, Arc::new(LocalReleaseOrchestrator::default())),
        path,
    )
}

async fn create_release(service: &ReleaseService) -> ModelRelease {
    service
        .create_release(
            "fraud-model".to_owned(),
            "v2".to_owned(),
            "registry.example/fraud-model:v2".to_owned(),
            Some("s3://example-models/fraud/v2/model.pkl".to_owned()),
            Metadata::from_iter([(String::from("owner"), json!("risk"))]),
        )
        .await
        .unwrap()
}

fn policy() -> Value {
    json!({"gates": {"accuracy": {"min": 0.9}, "latency_p95": {"max": 200}}})
}
fn metrics() -> Map<String, Value> {
    Map::from_iter([
        (String::from("accuracy"), json!(0.92)),
        (String::from("latency_p95"), json!(143)),
    ])
}

#[test]
fn all_defined_transitions_are_allowed() {
    let transitions = [
        (ReleaseStatus::Submitted, ReleaseStatus::Validating),
        (ReleaseStatus::Validating, ReleaseStatus::Ready),
        (ReleaseStatus::Validating, ReleaseStatus::Rejected),
        (ReleaseStatus::Validating, ReleaseStatus::Failed),
        (ReleaseStatus::Ready, ReleaseStatus::Deploying),
        (ReleaseStatus::Deploying, ReleaseStatus::Canary),
        (ReleaseStatus::Deploying, ReleaseStatus::Failed),
        (ReleaseStatus::Canary, ReleaseStatus::Promoted),
        (ReleaseStatus::Canary, ReleaseStatus::RolledBack),
        (ReleaseStatus::Canary, ReleaseStatus::Failed),
    ];
    for (current, target) in transitions {
        assert!(current.can_transition_to(target));
    }
}

#[test]
fn terminal_states_and_invalid_transition_are_rejected() {
    assert!(!ReleaseStatus::Submitted.can_transition_to(ReleaseStatus::Ready));
    for status in [
        ReleaseStatus::Promoted,
        ReleaseStatus::Rejected,
        ReleaseStatus::RolledBack,
        ReleaseStatus::Failed,
    ] {
        assert!(status.is_terminal());
        assert!(!status.can_transition_to(ReleaseStatus::Failed));
    }
}

#[test]
fn policy_boundaries_results_and_missing_metrics_match_contract() {
    let parsed =
        parse_policy(&json!({"gates": {"accuracy": {"min": 0.9}, "latency_p95": {"max": 200}}}))
            .unwrap();
    assert!(
        evaluate_policy(
            &parsed,
            &Map::from_iter([
                (String::from("accuracy"), json!(0.9)),
                (String::from("latency_p95"), json!(200))
            ])
        )
        .unwrap()
        .passed
    );
    let failed = evaluate_policy(
        &parsed,
        &Map::from_iter([(String::from("accuracy"), json!(0.89))]),
    )
    .unwrap();
    assert!(!failed.passed);
    assert!(!failed.results[0].passed);
    assert_eq!(
        failed.results[1].reason.as_deref(),
        Some("Required metric was not supplied.")
    );
}

#[test]
fn malformed_policies_are_rejected() {
    for policy in [
        json!({}),
        json!({"gates": {}}),
        json!({"gates": {"accuracy": {}}}),
        json!({"gates": {"accuracy": {"min": 0.9, "max": 1.0}}}),
        json!({"gates": {"accuracy": {"min": "0.9"}}}),
        json!({"gates": {"accuracy": {"min": 0.9, "unexpected": 1}}}),
        json!({"gates": {"accuracy": 0.9}}),
    ] {
        assert!(parse_policy(&policy).is_err());
    }
}

#[tokio::test]
async fn create_get_list_and_not_found_work() {
    let (service, path) = service("basic").await;
    let release = create_release(&service).await;
    assert_eq!(
        service
            .get_release(&release.release_id)
            .await
            .unwrap()
            .metadata["owner"],
        "risk"
    );
    assert_eq!(
        service.list_releases().await.unwrap()[0].release_id,
        release.release_id
    );
    assert!(service.get_release("missing").await.is_err());
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn evaluation_persists_evidence_and_enforces_transitions() {
    let (service, path) = service("evaluation").await;
    let release = create_release(&service).await;
    let evaluated = service
        .evaluate_release(&release.release_id, metrics(), policy())
        .await
        .unwrap();
    assert_eq!(evaluated.status, ReleaseStatus::Ready);
    assert!(evaluated.evaluation.unwrap().passed);
    let latency_p95 = service
        .get_release(&release.release_id)
        .await
        .unwrap()
        .metrics["latency_p95"];
    assert!((latency_p95 - 143.0).abs() < f64::EPSILON);
    assert!(
        service
            .evaluate_release(&release.release_id, metrics(), policy())
            .await
            .is_err()
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn failed_or_malformed_evaluations_have_python_behavior() {
    let (service, path) = service("evaluation-failure").await;
    let release = create_release(&service).await;
    let rejected = service
        .evaluate_release(
            &release.release_id,
            Map::from_iter([(String::from("accuracy"), json!(0.92))]),
            policy(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status, ReleaseStatus::Rejected);
    assert_eq!(
        rejected.failure_reason.as_deref(),
        Some("Policy evaluation failed for: latency_p95.")
    );
    let fresh = create_release(&service).await;
    assert!(
        service
            .evaluate_release(&fresh.release_id, metrics(), json!({"gates": {}}))
            .await
            .is_err()
    );
    assert_eq!(
        service.get_release(&fresh.release_id).await.unwrap().status,
        ReleaseStatus::Submitted
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn local_orchestration_promotes_rolls_back_and_fails_unhealthy_candidates() {
    let (service, path) = service("lifecycle").await;
    let release = create_release(&service).await;
    service
        .evaluate_release(&release.release_id, metrics(), policy())
        .await
        .unwrap();
    assert_eq!(
        service
            .deploy_release(&release.release_id)
            .await
            .unwrap()
            .status,
        ReleaseStatus::Canary
    );
    assert_eq!(
        service
            .promote_release(&release.release_id)
            .await
            .unwrap()
            .status,
        ReleaseStatus::Promoted
    );
    let rollback = create_release(&service).await;
    service
        .evaluate_release(&rollback.release_id, metrics(), policy())
        .await
        .unwrap();
    service.deploy_release(&rollback.release_id).await.unwrap();
    assert_eq!(
        service
            .rollback_release(&rollback.release_id)
            .await
            .unwrap()
            .status,
        ReleaseStatus::RolledBack
    );
    let (url, unhealthy_path) = database_url("unhealthy");
    let repository = Arc::new(SqliteReleaseRepository::connect(&url).await.unwrap());
    let unhealthy = ReleaseService::new(repository, Arc::new(LocalReleaseOrchestrator::new(false)));
    let candidate = create_release(&unhealthy).await;
    unhealthy
        .evaluate_release(&candidate.release_id, metrics(), policy())
        .await
        .unwrap();
    let failed = unhealthy
        .deploy_release(&candidate.release_id)
        .await
        .unwrap();
    assert_eq!(failed.status, ReleaseStatus::Failed);
    assert_eq!(
        failed.failure_reason.as_deref(),
        Some("Candidate did not pass the local health check.")
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(unhealthy_path).unwrap();
}

#[tokio::test]
async fn legacy_python_schema_and_rows_remain_readable_and_updatable() {
    let (url, path) = database_url("legacy");
    let raw_url = ml_release_platform::repository::normalize_sqlite_url(&url);
    let options = SqliteConnectOptions::from_str(&raw_url)
        .unwrap()
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE releases (release_id VARCHAR(36) PRIMARY KEY NOT NULL, model_name VARCHAR(255) NOT NULL, version VARCHAR(255) NOT NULL, image_uri VARCHAR(2048) NOT NULL, artifact_uri VARCHAR(2048), status VARCHAR(32) NOT NULL, created_at DATETIME NOT NULL, updated_at DATETIME NOT NULL, metadata JSON NOT NULL, metrics JSON NOT NULL, evaluation JSON, failure_reason VARCHAR(2048))").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO releases VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind("legacy-release")
        .bind("legacy")
        .bind("v1")
        .bind("registry/legacy:v1")
        .bind(Option::<String>::None)
        .bind("SUBMITTED")
        .bind("2026-01-02 03:04:05.000000")
        .bind("2026-01-02 03:04:05.000000")
        .bind("{\"owner\": \"risk\"}")
        .bind("{}")
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .execute(&pool)
        .await
        .unwrap();
    drop(pool);
    let repository = SqliteReleaseRepository::connect(&url).await.unwrap();
    let release = repository.get("legacy-release").await.unwrap().unwrap();
    assert_eq!(release.status, ReleaseStatus::Submitted);
    assert_eq!(release.metadata["owner"], "risk");
    let updated = release.transition_to(ReleaseStatus::Validating).unwrap();
    repository.update(&updated).await.unwrap();
    assert_eq!(
        repository
            .get("legacy-release")
            .await
            .unwrap()
            .unwrap()
            .status,
        ReleaseStatus::Validating
    );
    std::fs::remove_file(path).unwrap();
}

async fn response_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn http_contract_covers_health_create_get_list_evaluate_and_errors() {
    let (url, path) = database_url("http");
    let app = create_app(Some(&url)).await.unwrap();
    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(response_json(health).await, json!({"status": "ok"}));
    let create = app.clone().oneshot(Request::builder().method("POST").uri("/releases").header("content-type", "application/json").body(Body::from(json!({"model_name":" fraud-model ","version":"v2","image_uri":"registry.example/fraud-model:v2","metadata":{"owner":"risk"}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let release_id = response_json(create).await["release_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/releases")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response_json(list).await[0]["release_id"], release_id);
    let evaluation = app.clone().oneshot(Request::builder().method("POST").uri(format!("/releases/{release_id}/evaluate")).header("content-type", "application/json").body(Body::from(json!({"metrics":{"accuracy":0.92,"latency_p95":143},"policy":{"gates":{"accuracy":{"min":0.9},"latency_p95":{"max":200}}}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(evaluation.status(), StatusCode::OK);
    assert_eq!(response_json(evaluation).await["passed"], true);
    let repeated = app.clone().oneshot(Request::builder().method("POST").uri(format!("/releases/{release_id}/evaluate")).header("content-type", "application/json").body(Body::from(json!({"metrics":{"accuracy":0.92},"policy":{"gates":{"accuracy":{"min":0.9}}}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(repeated.status(), StatusCode::CONFLICT);
    let missing = app
        .oneshot(
            Request::builder()
                .uri("/releases/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    std::fs::remove_file(path).unwrap();
}
