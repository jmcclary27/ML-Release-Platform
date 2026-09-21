use std::{path::PathBuf, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ml_release_platform::{
    api,
    application::ReleaseService,
    domain::{
        models::Metadata,
        policy::{evaluate_policy, parse_policy},
        states::ReleaseStatus,
    },
    orchestration::{LocalReleaseOrchestrator, VerificationFailureKind, VerificationResult},
    repository::{ReleaseRepository, SqliteReleaseRepository},
};
use serde_json::{Map, Value, json};
use tower::ServiceExt;

fn database_url(name: &str) -> (String, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "ml-release-platform-{name}-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    (format!("sqlite:///{}", path.display()), path)
}

async fn repository(name: &str) -> (Arc<SqliteReleaseRepository>, PathBuf) {
    let (url, path) = database_url(name);
    (
        Arc::new(SqliteReleaseRepository::connect(&url).await.unwrap()),
        path,
    )
}

async fn create_release(service: &ReleaseService, version: &str) -> String {
    service
        .create_release(
            "example".to_owned(),
            version.to_owned(),
            format!("demo:{version}"),
            None,
            Metadata::new(),
        )
        .await
        .unwrap()
        .release_id
}

fn policy() -> Value {
    json!({"gates":{"accuracy":{"min":0.9}}})
}
fn metrics() -> Map<String, Value> {
    Map::from_iter([(String::from("accuracy"), json!(0.95))])
}

#[test]
fn explicit_lifecycle_transitions_reject_skipped_or_terminal_states() {
    for (current, target) in [
        (ReleaseStatus::Submitted, ReleaseStatus::Validating),
        (ReleaseStatus::Validating, ReleaseStatus::Ready),
        (ReleaseStatus::Validating, ReleaseStatus::Rejected),
        (ReleaseStatus::Ready, ReleaseStatus::Deploying),
        (ReleaseStatus::Deploying, ReleaseStatus::Verifying),
        (ReleaseStatus::Verifying, ReleaseStatus::Released),
        (ReleaseStatus::Verifying, ReleaseStatus::RollingBack),
        (ReleaseStatus::RollingBack, ReleaseStatus::RolledBack),
        (ReleaseStatus::RollingBack, ReleaseStatus::Failed),
    ] {
        assert!(current.can_transition_to(target));
    }
    assert!(!ReleaseStatus::Submitted.can_transition_to(ReleaseStatus::Ready));
    assert!(!ReleaseStatus::Released.can_transition_to(ReleaseStatus::Failed));
    assert!(ReleaseStatus::Released.is_terminal());
}

#[test]
fn policy_evaluation_preserves_gate_boundaries_and_rejects_malformed_contracts() {
    let parsed = parse_policy(&policy()).unwrap();
    assert!(
        evaluate_policy(
            &parsed,
            &Map::from_iter([(String::from("accuracy"), json!(0.9))]),
        )
        .unwrap()
        .passed
    );
    assert!(
        !evaluate_policy(
            &parsed,
            &Map::from_iter([(String::from("accuracy"), json!(0.89))]),
        )
        .unwrap()
        .passed
    );
    for malformed in [
        json!({}),
        json!({"gates": {}}),
        json!({"gates": {"accuracy": {"min": 0.9, "max": 1.0}}}),
        json!({"gates": {"accuracy": {"min": "0.9"}}}),
    ] {
        assert!(parse_policy(&malformed).is_err());
    }
}

async fn close_and_remove(repository: Arc<SqliteReleaseRepository>, path: PathBuf) {
    repository.close().await;
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn invalid_runtime_response_restores_the_previous_active_release_and_audits_it() {
    let (repository, path) = repository("critical-rollback").await;
    let good = ReleaseService::new(
        repository.clone(),
        Arc::new(LocalReleaseOrchestrator::default()),
    );
    let v1 = create_release(&good, "v1").await;
    good.evaluate_release(&v1, metrics(), policy())
        .await
        .unwrap();
    good.deploy_release(&v1).await.unwrap();
    assert_eq!(
        good.verify_release(&v1).await.unwrap().status,
        ReleaseStatus::Released
    );
    assert_eq!(
        good.current_release("example")
            .await
            .unwrap()
            .unwrap()
            .release_id,
        v1
    );

    let bad_orchestrator = LocalReleaseOrchestrator::with_outcomes(
        true,
        VerificationResult::failed(
            VerificationFailureKind::InvalidResponse,
            "demo detector returned a schema-invalid inference response",
            Some(200),
            Some(include_str!("../demo/model-server/invalid-response-v1.json").to_owned()),
        ),
        true,
        true,
    );
    let bad = ReleaseService::new(repository.clone(), Arc::new(bad_orchestrator));
    let v2 = create_release(&bad, "v2").await;
    bad.evaluate_release(&v2, metrics(), policy())
        .await
        .unwrap();
    assert_eq!(
        bad.deploy_release(&v2).await.unwrap().status,
        ReleaseStatus::Verifying
    );
    let rolled_back = bad.verify_release(&v2).await.unwrap();
    assert_eq!(rolled_back.status, ReleaseStatus::RolledBack);
    assert_eq!(
        bad.current_release("example")
            .await
            .unwrap()
            .unwrap()
            .release_id,
        v1
    );
    let event_types: Vec<_> = bad
        .get_history(&v2)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.event_type)
        .collect();
    assert!(event_types.contains(&"VERIFICATION_FAILED".to_owned()));
    assert!(event_types.contains(&"ACTIVE_VERSION_RESTORED".to_owned()));

    drop(good);
    drop(bad);
    close_and_remove(repository, path).await;
}

#[tokio::test]
async fn policy_rejection_never_reaches_deployment() {
    let (repository, path) = repository("rejected").await;
    let service = ReleaseService::new(
        repository.clone(),
        Arc::new(LocalReleaseOrchestrator::default()),
    );
    let release_id = create_release(&service, "v1").await;
    let rejected = service
        .evaluate_release(
            &release_id,
            Map::from_iter([(String::from("accuracy"), json!(0.1))]),
            policy(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status, ReleaseStatus::Rejected);
    assert!(service.deploy_release(&release_id).await.is_err());
    assert!(
        repository
            .latest_deployment(&release_id)
            .await
            .unwrap()
            .is_none()
    );
    drop(service);
    close_and_remove(repository, path).await;
}

#[tokio::test]
async fn deployment_failure_keeps_existing_active_release() {
    let (repository, path) = repository("deployment-failure").await;
    let good = ReleaseService::new(
        repository.clone(),
        Arc::new(LocalReleaseOrchestrator::default()),
    );
    let v1 = create_release(&good, "v1").await;
    good.evaluate_release(&v1, metrics(), policy())
        .await
        .unwrap();
    good.deploy_release(&v1).await.unwrap();
    good.verify_release(&v1).await.unwrap();
    let failed = ReleaseService::new(
        repository.clone(),
        Arc::new(LocalReleaseOrchestrator::with_outcomes(
            false,
            VerificationResult::passed(200, "{}".to_owned()),
            true,
            true,
        )),
    );
    let v2 = create_release(&failed, "v2").await;
    failed
        .evaluate_release(&v2, metrics(), policy())
        .await
        .unwrap();
    assert_eq!(
        failed.deploy_release(&v2).await.unwrap().status,
        ReleaseStatus::Failed
    );
    assert_eq!(
        failed
            .current_release("example")
            .await
            .unwrap()
            .unwrap()
            .release_id,
        v1
    );
    drop(good);
    drop(failed);
    close_and_remove(repository, path).await;
}

async fn response_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}

#[tokio::test]
async fn api_exposes_execution_history_current_and_command_errors() {
    let (repository, path) = repository("api").await;
    let service = Arc::new(ReleaseService::new(
        repository.clone(),
        Arc::new(LocalReleaseOrchestrator::default()),
    ));
    let app = api::router(service);
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/releases")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"model_name":"example","version":"v1","image_uri":"demo:v1"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let release_id = response_json(create).await["release_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let malformed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/releases/{release_id}/evaluate"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(malformed.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let history = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/releases/{release_id}/history"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(history.status(), StatusCode::OK);
    let current = app
        .oneshot(
            Request::builder()
                .uri("/models/example/current")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response_json(current).await["active"], Value::Null);
    drop(repository.clone());
    repository.close().await;
    std::fs::remove_file(path).unwrap();
}
