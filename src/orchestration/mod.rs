//! Infrastructure boundary for releasing locally served model containers.
//!
//! The application layer owns lifecycle choices. Implementations only perform requested
//! infrastructure operations, which keeps a future Kubernetes adapter compatible.

use std::{
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::{process::Command, time::sleep};

use crate::domain::{errors::OrchestrationError, models::ModelRelease};

const MANAGED_LABEL: &str = "ml-release-platform.managed=true";
const CONTAINER_PORT: u16 = 8080;
const DETECTOR_CONTRACT_VERSION: &str = "v1";
const VERIFICATION_REQUEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/demo/fixtures/market-data-inference-v1.json"
));

/// Concrete identity and loopback endpoint of a deployed candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentHandle {
    pub container_id: String,
    pub container_name: String,
    pub endpoint: String,
}

/// Why a runtime verification did not pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationFailureKind {
    HttpStatus,
    InvalidResponse,
    Timeout,
    Transport,
}

#[derive(Deserialize)]
struct DetectorHealthResponse {
    contract_version: String,
    status: String,
    model_id: String,
}

#[derive(Deserialize)]
struct DetectorPredictionResponse {
    contract_version: String,
    request_id: String,
    model_id: String,
    predictions: Vec<DetectorPrediction>,
}

#[derive(Deserialize)]
struct DetectorPrediction {
    timestamp: String,
    symbol: String,
    prediction: f64,
}

/// Result of an inference verification request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationResult {
    pub passed: bool,
    pub detail: String,
    pub status: Option<u16>,
    pub body: Option<String>,
    pub failure_kind: Option<VerificationFailureKind>,
}

impl VerificationResult {
    #[must_use]
    pub fn passed(status: u16, body: String) -> Self {
        Self {
            passed: true,
            detail: "Inference verification passed.".to_owned(),
            status: Some(status),
            body: Some(body),
            failure_kind: None,
        }
    }

    #[must_use]
    pub fn failed(
        kind: VerificationFailureKind,
        detail: impl Into<String>,
        status: Option<u16>,
        body: Option<String>,
    ) -> Self {
        Self {
            passed: false,
            detail: detail.into(),
            status,
            body,
            failure_kind: Some(kind),
        }
    }
}

/// Interface shared by local and eventual Kubernetes release adapters.
#[async_trait]
pub trait ReleaseOrchestrator: Send + Sync {
    async fn deploy_candidate(
        &self,
        release: &ModelRelease,
    ) -> Result<DeploymentHandle, OrchestrationError>;
    async fn verify_candidate(
        &self,
        candidate: &DeploymentHandle,
    ) -> Result<VerificationResult, OrchestrationError>;
    async fn promote_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
    ) -> Result<(), OrchestrationError>;
    async fn rollback_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
    ) -> Result<(), OrchestrationError>;
    async fn cleanup_candidate(
        &self,
        candidate: &DeploymentHandle,
    ) -> Result<(), OrchestrationError>;
}

/// Docker adapter configuration. Network operations target loopback endpoints only.
#[derive(Clone, Debug)]
pub struct DockerOrchestratorConfig {
    pub container_port: u16,
    pub health_path: String,
    pub inference_path: String,
    pub startup_timeout: Duration,
    pub poll_interval: Duration,
    pub verification_timeout: Duration,
}

impl Default for DockerOrchestratorConfig {
    fn default() -> Self {
        Self {
            container_port: CONTAINER_PORT,
            health_path: "/health".to_owned(),
            inference_path: "/infer".to_owned(),
            startup_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_millis(250),
            verification_timeout: Duration::from_secs(10),
        }
    }
}

/// Captured result of a Docker CLI invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait]
pub trait DockerCommandRunner: Send + Sync {
    async fn run(&self, arguments: &[String]) -> Result<CommandOutput, OrchestrationError>;
}

/// Real Docker CLI process runner. Docker remains an implementation detail of the adapter.
#[derive(Default)]
pub struct TokioDockerCommandRunner;

#[async_trait]
impl DockerCommandRunner for TokioDockerCommandRunner {
    async fn run(&self, arguments: &[String]) -> Result<CommandOutput, OrchestrationError> {
        let output = Command::new("docker")
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| OrchestrationError(format!("Could not run Docker: {error}")))?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpRequestError {
    Timeout,
    Transport(String),
}

#[async_trait]
pub trait HttpProbe: Send + Sync {
    async fn request(
        &self,
        method: HttpMethod,
        endpoint: &str,
        body: Option<&str>,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpRequestError>;
}

/// Minimal HTTP/1.1 loopback client, deliberately independent of the API stack.
#[derive(Default)]
pub struct LoopbackHttpProbe;

#[async_trait]
impl HttpProbe for LoopbackHttpProbe {
    async fn request(
        &self,
        method: HttpMethod,
        endpoint: &str,
        body: Option<&str>,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpRequestError> {
        let endpoint = endpoint.to_owned();
        let body = body.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            request_loopback(method, &endpoint, body.as_deref(), timeout)
        })
        .await
        .map_err(|error| HttpRequestError::Transport(format!("HTTP task failed: {error}")))?
    }
}

/// Docker-backed local release adapter.
pub struct DockerReleaseOrchestrator {
    config: DockerOrchestratorConfig,
    docker: Arc<dyn DockerCommandRunner>,
    http: Arc<dyn HttpProbe>,
}

impl DockerReleaseOrchestrator {
    #[must_use]
    pub fn new(config: DockerOrchestratorConfig) -> Self {
        Self::with_dependencies(
            config,
            Arc::new(TokioDockerCommandRunner),
            Arc::new(LoopbackHttpProbe),
        )
    }

    #[must_use]
    pub fn with_dependencies(
        config: DockerOrchestratorConfig,
        docker: Arc<dyn DockerCommandRunner>,
        http: Arc<dyn HttpProbe>,
    ) -> Self {
        Self {
            config,
            docker,
            http,
        }
    }

    #[must_use]
    pub fn candidate_name(release: &ModelRelease) -> String {
        format!(
            "mlrp-{}-{}-{}",
            sanitize_component(&release.model_name, 24),
            sanitize_component(&release.version, 20),
            sanitize_component(&release.release_id, 12)
        )
    }

    async fn docker_success(
        &self,
        arguments: Vec<String>,
        operation: &str,
    ) -> Result<CommandOutput, OrchestrationError> {
        let output = self.docker.run(&arguments).await?;
        if output.success {
            Ok(output)
        } else {
            Err(OrchestrationError(format!(
                "Docker {operation} failed: {}",
                non_empty_error(&output)
            )))
        }
    }

    async fn remove_container(&self, name_or_id: &str) -> Result<(), OrchestrationError> {
        let output = self
            .docker
            .run(&["rm".to_owned(), "-f".to_owned(), name_or_id.to_owned()])
            .await?;
        if output.success || is_not_found(&output) {
            Ok(())
        } else {
            Err(OrchestrationError(format!(
                "Docker cleanup failed: {}",
                non_empty_error(&output)
            )))
        }
    }

    async fn wait_for_health(&self, endpoint: &str) -> Result<(), OrchestrationError> {
        let url = join_url(endpoint, &self.config.health_path);
        let deadline = Instant::now() + self.config.startup_timeout;
        loop {
            let last_error = match self
                .http
                .request(HttpMethod::Get, &url, None, self.config.poll_interval)
                .await
            {
                Ok(response) if (200..300).contains(&response.status) => {
                    match validate_health_response(&response.body) {
                        Ok(()) => return Ok(()),
                        Err(error) => error,
                    }
                }
                Ok(response) => format!("received HTTP {}", response.status),
                Err(HttpRequestError::Timeout) => "request timed out".to_owned(),
                Err(HttpRequestError::Transport(error)) => error,
            };
            if Instant::now() >= deadline {
                return Err(OrchestrationError(format!(
                    "Candidate did not become healthy within {:?}: {last_error}.",
                    self.config.startup_timeout
                )));
            }
            sleep(self.config.poll_interval).await;
        }
    }

    async fn previous_exists(&self, previous: &ModelRelease) -> Result<bool, OrchestrationError> {
        let output = self
            .docker
            .run(&[
                "inspect".to_owned(),
                "--format".to_owned(),
                "{{.State.Running}}".to_owned(),
                Self::candidate_name(previous),
            ])
            .await?;
        Ok(output.success && output.stdout.trim() == "true")
    }
}

#[async_trait]
impl ReleaseOrchestrator for DockerReleaseOrchestrator {
    async fn deploy_candidate(
        &self,
        release: &ModelRelease,
    ) -> Result<DeploymentHandle, OrchestrationError> {
        let name = Self::candidate_name(release);
        let output = self
            .docker_success(
                vec![
                    "run".to_owned(),
                    "-d".to_owned(),
                    "--name".to_owned(),
                    name.clone(),
                    "--label".to_owned(),
                    MANAGED_LABEL.to_owned(),
                    "--label".to_owned(),
                    format!("ml-release-platform.release-id={}", release.release_id),
                    "--label".to_owned(),
                    format!("ml-release-platform.model={}", release.model_name),
                    "-p".to_owned(),
                    format!("127.0.0.1::{}", self.config.container_port),
                    release.image_uri.clone(),
                ],
                "run",
            )
            .await?;
        let container_id = output.stdout;
        if container_id.is_empty() {
            return Err(OrchestrationError(
                "Docker run did not return a container ID.".to_owned(),
            ));
        }
        let port_output = match self
            .docker_success(
                vec![
                    "port".to_owned(),
                    container_id.clone(),
                    self.config.container_port.to_string(),
                ],
                "port lookup",
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                let _ = self.remove_container(&container_id).await;
                return Err(error);
            }
        };
        let endpoint = match parse_loopback_endpoint(&port_output.stdout) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                let _ = self.remove_container(&container_id).await;
                return Err(error);
            }
        };
        let handle = DeploymentHandle {
            container_id,
            container_name: name,
            endpoint,
        };
        if let Err(error) = self.wait_for_health(&handle.endpoint).await {
            let _ = self.cleanup_candidate(&handle).await;
            return Err(error);
        }
        Ok(handle)
    }

    async fn verify_candidate(
        &self,
        candidate: &DeploymentHandle,
    ) -> Result<VerificationResult, OrchestrationError> {
        let url = join_url(&candidate.endpoint, &self.config.inference_path);
        match self
            .http
            .request(
                HttpMethod::Post,
                &url,
                Some(VERIFICATION_REQUEST),
                self.config.verification_timeout,
            )
            .await
        {
            Ok(response) if (200..300).contains(&response.status) => {
                match validate_prediction_response(&response.body) {
                    Ok(()) => Ok(VerificationResult::passed(response.status, response.body)),
                    Err(error) => Ok(VerificationResult::failed(
                        VerificationFailureKind::InvalidResponse,
                        format!(
                            "Inference verification returned an invalid detector response: {error}"
                        ),
                        Some(response.status),
                        Some(response.body),
                    )),
                }
            }
            Ok(response) => Ok(VerificationResult::failed(
                VerificationFailureKind::HttpStatus,
                format!("Inference verification returned HTTP {}.", response.status),
                Some(response.status),
                Some(response.body),
            )),
            Err(HttpRequestError::Timeout) => Ok(VerificationResult::failed(
                VerificationFailureKind::Timeout,
                "Inference verification timed out.",
                None,
                None,
            )),
            Err(HttpRequestError::Transport(error)) => Ok(VerificationResult::failed(
                VerificationFailureKind::Transport,
                format!("Inference verification transport failure: {error}"),
                None,
                None,
            )),
        }
    }

    async fn promote_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
    ) -> Result<(), OrchestrationError> {
        if let Some(previous) = previous_active {
            let previous_name = Self::candidate_name(previous);
            if previous_name != candidate.container_name {
                self.remove_container(&previous_name).await?;
            }
        }
        Ok(())
    }

    async fn rollback_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
    ) -> Result<(), OrchestrationError> {
        self.cleanup_candidate(candidate).await?;
        if let Some(previous) = previous_active
            && !self.previous_exists(previous).await?
        {
            return Err(OrchestrationError(format!(
                "Rollback removed candidate but prior active container '{}' is not running.",
                Self::candidate_name(previous)
            )));
        }
        Ok(())
    }

    async fn cleanup_candidate(
        &self,
        candidate: &DeploymentHandle,
    ) -> Result<(), OrchestrationError> {
        self.remove_container(&candidate.container_id).await
    }
}

/// Deterministic in-memory adapter for unit and service tests. It deliberately does no I/O.
#[derive(Clone, Debug)]
pub struct LocalReleaseOrchestrator {
    deploy_succeeds: bool,
    verification: VerificationResult,
    promote_succeeds: bool,
    rollback_succeeds: bool,
}

impl Default for LocalReleaseOrchestrator {
    fn default() -> Self {
        Self::new(true)
    }
}

impl LocalReleaseOrchestrator {
    /// Compatibility constructor: `true` creates a verification pass and `false` a failure.
    #[must_use]
    pub fn new(candidate_healthy: bool) -> Self {
        Self {
            deploy_succeeds: true,
            verification: if candidate_healthy {
                VerificationResult::passed(200, "{}".to_owned())
            } else {
                VerificationResult::failed(
                    VerificationFailureKind::HttpStatus,
                    "Candidate did not pass the local health check.",
                    Some(503),
                    None,
                )
            },
            promote_succeeds: true,
            rollback_succeeds: true,
        }
    }

    #[must_use]
    pub fn with_outcomes(
        deploy_succeeds: bool,
        verification: VerificationResult,
        promote_succeeds: bool,
        rollback_succeeds: bool,
    ) -> Self {
        Self {
            deploy_succeeds,
            verification,
            promote_succeeds,
            rollback_succeeds,
        }
    }

    fn handle(release: &ModelRelease) -> DeploymentHandle {
        DeploymentHandle {
            container_id: format!("fake-{}", release.release_id),
            container_name: DockerReleaseOrchestrator::candidate_name(release),
            endpoint: "http://127.0.0.1:0".to_owned(),
        }
    }
}

#[async_trait]
impl ReleaseOrchestrator for LocalReleaseOrchestrator {
    async fn deploy_candidate(
        &self,
        release: &ModelRelease,
    ) -> Result<DeploymentHandle, OrchestrationError> {
        if self.deploy_succeeds {
            Ok(Self::handle(release))
        } else {
            Err(OrchestrationError(
                "Simulated deployment failure.".to_owned(),
            ))
        }
    }

    async fn verify_candidate(
        &self,
        _: &DeploymentHandle,
    ) -> Result<VerificationResult, OrchestrationError> {
        Ok(self.verification.clone())
    }

    async fn promote_candidate(
        &self,
        _: &DeploymentHandle,
        _: Option<&ModelRelease>,
    ) -> Result<(), OrchestrationError> {
        self.promote_succeeds
            .then_some(())
            .ok_or_else(|| OrchestrationError("Simulated promotion failure.".to_owned()))
    }

    async fn rollback_candidate(
        &self,
        _: &DeploymentHandle,
        _: Option<&ModelRelease>,
    ) -> Result<(), OrchestrationError> {
        self.rollback_succeeds
            .then_some(())
            .ok_or_else(|| OrchestrationError("Simulated rollback failure.".to_owned()))
    }

    async fn cleanup_candidate(&self, _: &DeploymentHandle) -> Result<(), OrchestrationError> {
        Ok(())
    }
}

fn validate_health_response(body: &str) -> Result<(), String> {
    let response: DetectorHealthResponse = serde_json::from_str(body)
        .map_err(|error| format!("response is not valid health JSON: {error}"))?;
    if response.contract_version != DETECTOR_CONTRACT_VERSION {
        return Err(format!(
            "contract_version must be '{DETECTOR_CONTRACT_VERSION}'"
        ));
    }
    if response.status != "ready" {
        return Err("status must be 'ready'".to_owned());
    }
    if response.model_id.trim().is_empty() {
        return Err("model_id must not be empty".to_owned());
    }
    Ok(())
}

fn validate_prediction_response(body: &str) -> Result<(), String> {
    let response: DetectorPredictionResponse = serde_json::from_str(body)
        .map_err(|error| format!("response is not valid prediction JSON: {error}"))?;
    if response.contract_version != DETECTOR_CONTRACT_VERSION {
        return Err(format!(
            "contract_version must be '{DETECTOR_CONTRACT_VERSION}'"
        ));
    }
    if response.request_id != verification_request_id()? {
        return Err("request_id does not match the verification request".to_owned());
    }
    if response.model_id.trim().is_empty() {
        return Err("model_id must not be empty".to_owned());
    }
    if response.predictions.is_empty() {
        return Err("predictions must not be empty".to_owned());
    }
    for prediction in response.predictions {
        if prediction.timestamp.trim().is_empty() {
            return Err("prediction timestamp must not be empty".to_owned());
        }
        if prediction.symbol.trim().is_empty() {
            return Err("prediction symbol must not be empty".to_owned());
        }
        if !prediction.prediction.is_finite() {
            return Err("prediction must be finite".to_owned());
        }
    }
    Ok(())
}

fn verification_request_id() -> Result<String, String> {
    let request: Value = serde_json::from_str(VERIFICATION_REQUEST)
        .map_err(|error| format!("bundled verification fixture is invalid JSON: {error}"))?;
    request
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "bundled verification fixture has no request_id".to_owned())
}

fn sanitize_component(value: &str, limit: usize) -> String {
    let mut sanitized = String::with_capacity(value.len().min(limit));
    let mut previous_separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            sanitized.push(character);
            previous_separator = false;
        } else if !previous_separator {
            sanitized.push('-');
            previous_separator = true;
        }
        if sanitized.len() >= limit {
            break;
        }
    }
    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "release".to_owned()
    } else {
        sanitized.to_owned()
    }
}

fn join_url(endpoint: &str, path: &str) -> String {
    format!(
        "{}/{}",
        endpoint.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn parse_loopback_endpoint(output: &str) -> Result<String, OrchestrationError> {
    let mapping = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| {
            OrchestrationError("Docker did not report a published candidate port.".to_owned())
        })?;
    let host_port = mapping.strip_prefix("127.0.0.1:").ok_or_else(|| {
        OrchestrationError(format!(
            "Docker published candidate on non-loopback address '{mapping}'."
        ))
    })?;
    host_port.parse::<u16>().map_err(|_| {
        OrchestrationError(format!(
            "Docker returned an invalid published port '{mapping}'."
        ))
    })?;
    Ok(format!("http://127.0.0.1:{host_port}"))
}

fn non_empty_error(output: &CommandOutput) -> &str {
    if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    }
}

fn is_not_found(output: &CommandOutput) -> bool {
    let message = format!("{}{}", output.stdout, output.stderr).to_ascii_lowercase();
    message.contains("no such container") || message.contains("not found")
}

fn request_loopback(
    method: HttpMethod,
    endpoint: &str,
    body: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, HttpRequestError> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let authority = endpoint.strip_prefix("http://").ok_or_else(|| {
        HttpRequestError::Transport("Only loopback HTTP endpoints are supported.".to_owned())
    })?;
    let (host, path) = authority.split_once('/').unwrap_or((authority, ""));
    if !host.starts_with("127.0.0.1:") {
        return Err(HttpRequestError::Transport(
            "Only 127.0.0.1 endpoints are supported.".to_owned(),
        ));
    }
    let address = host
        .parse()
        .map_err(|error| HttpRequestError::Transport(format!("Invalid endpoint: {error}")))?;
    let mut stream =
        TcpStream::connect_timeout(&address, timeout).map_err(|error| map_io_error(&error))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| map_io_error(&error))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| map_io_error(&error))?;
    let body = body.unwrap_or("");
    let method = match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
    };
    let request = format!(
        "{method} /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| map_io_error(&error))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| map_io_error(&error))?;
    let (headers, response_body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    let status = headers
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| HttpRequestError::Transport("Malformed HTTP response.".to_owned()))?
        .parse()
        .map_err(|_| HttpRequestError::Transport("Malformed HTTP status.".to_owned()))?;
    Ok(HttpResponse {
        status,
        body: response_body.to_owned(),
    })
}

fn map_io_error(error: &std::io::Error) -> HttpRequestError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        HttpRequestError::Timeout
    } else {
        HttpRequestError::Transport(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use chrono::Utc;
    use serde_json::Map;

    use super::*;

    struct FakeDocker {
        responses: Mutex<VecDeque<CommandOutput>>,
        commands: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl DockerCommandRunner for FakeDocker {
        async fn run(&self, arguments: &[String]) -> Result<CommandOutput, OrchestrationError> {
            self.commands.lock().unwrap().push(arguments.to_vec());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| OrchestrationError("Unexpected Docker command in test.".to_owned()))
        }
    }

    struct FakeHttp {
        responses: Mutex<VecDeque<Result<HttpResponse, HttpRequestError>>>,
        requests: Mutex<Vec<(HttpMethod, String, Option<String>)>>,
    }

    #[async_trait]
    impl HttpProbe for FakeHttp {
        async fn request(
            &self,
            method: HttpMethod,
            endpoint: &str,
            body: Option<&str>,
            _: Duration,
        ) -> Result<HttpResponse, HttpRequestError> {
            self.requests.lock().unwrap().push((
                method,
                endpoint.to_owned(),
                body.map(str::to_owned),
            ));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    Err(HttpRequestError::Transport("unexpected request".to_owned()))
                })
        }
    }

    fn release() -> ModelRelease {
        let now = Utc::now();
        ModelRelease {
            release_id: "release-id-123456".to_owned(),
            model_name: "Risk Model".to_owned(),
            version: "v2".to_owned(),
            image_uri: "mlrp-demo-model:good".to_owned(),
            artifact_uri: None,
            status: crate::domain::states::ReleaseStatus::Deploying,
            created_at: now,
            updated_at: now,
            metadata: Map::new(),
            metrics: std::collections::BTreeMap::default(),
            evaluation: None,
            failure_reason: None,
        }
    }

    #[test]
    fn container_names_are_deterministic_and_safe() {
        assert_eq!(sanitize_component("Risk Model / v2", 24), "risk-model-v2");
        assert_eq!(sanitize_component("***", 24), "release");
    }

    #[test]
    fn loopback_port_parsing_rejects_external_bindings() {
        assert_eq!(
            parse_loopback_endpoint("127.0.0.1:34567\n").unwrap(),
            "http://127.0.0.1:34567"
        );
        assert!(parse_loopback_endpoint("0.0.0.0:34567").is_err());
        assert!(parse_loopback_endpoint("").is_err());
    }

    #[test]
    fn verification_results_preserve_failure_evidence() {
        let result = VerificationResult::failed(
            VerificationFailureKind::HttpStatus,
            "bad inference",
            Some(503),
            Some("unavailable".to_owned()),
        );
        assert!(!result.passed);
        assert_eq!(
            result.failure_kind,
            Some(VerificationFailureKind::HttpStatus)
        );
        assert_eq!(result.status, Some(503));
    }

    #[tokio::test]
    async fn deployment_uses_labeled_loopback_container_and_health_check() {
        let docker = Arc::new(FakeDocker {
            responses: Mutex::new(VecDeque::from([
                CommandOutput {
                    success: true,
                    stdout: "container-123".to_owned(),
                    stderr: String::new(),
                },
                CommandOutput {
                    success: true,
                    stdout: "127.0.0.1:45678".to_owned(),
                    stderr: String::new(),
                },
            ])),
            commands: Mutex::new(Vec::new()),
        });
        let http = Arc::new(FakeHttp {
            responses: Mutex::new(VecDeque::from([Ok(HttpResponse {
                status: 200,
                body: r#"{"contract_version":"v1","status":"ready","model_id":"reference:v1"}"#
                    .to_owned(),
            })])),
            requests: Mutex::new(Vec::new()),
        });
        let adapter = DockerReleaseOrchestrator::with_dependencies(
            DockerOrchestratorConfig {
                poll_interval: Duration::from_millis(1),
                ..DockerOrchestratorConfig::default()
            },
            docker.clone(),
            http.clone(),
        );

        let handle = adapter.deploy_candidate(&release()).await.unwrap();

        assert_eq!(handle.endpoint, "http://127.0.0.1:45678");
        let commands = docker.commands.lock().unwrap();
        assert_eq!(commands[0][0], "run");
        assert!(commands[0].contains(&MANAGED_LABEL.to_owned()));
        assert!(commands[0].contains(&"127.0.0.1::8080".to_owned()));
        assert_eq!(commands[1][0], "port");
        assert_eq!(
            http.requests.lock().unwrap()[0].1,
            "http://127.0.0.1:45678/health"
        );
    }

    #[tokio::test]
    async fn inference_posts_the_market_fixture_and_requires_a_valid_prediction_contract() {
        let http = Arc::new(FakeHttp {
            responses: Mutex::new(VecDeque::from([Ok(HttpResponse {
                status: 200,
                body: r#"{"contract_version":"v1","request_id":"mlrp-market-window-2026-09-18","model_id":"reference:v1","predictions":[{"timestamp":"2026-09-18T20:00:00Z","symbol":"SPY","prediction":0.00098}]}"#.to_owned(),
            })])),
            requests: Mutex::new(Vec::new()),
        });
        let adapter = DockerReleaseOrchestrator::with_dependencies(
            DockerOrchestratorConfig::default(),
            Arc::new(FakeDocker {
                responses: Mutex::new(VecDeque::new()),
                commands: Mutex::new(Vec::new()),
            }),
            http.clone(),
        );

        let result = adapter
            .verify_candidate(&DeploymentHandle {
                container_id: "candidate".to_owned(),
                container_name: "candidate".to_owned(),
                endpoint: "http://127.0.0.1:1234".to_owned(),
            })
            .await
            .unwrap();

        assert!(result.passed);
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests[0].0, HttpMethod::Post);
        assert_eq!(requests[0].1, "http://127.0.0.1:1234/infer");
        assert_eq!(requests[0].2.as_deref(), Some(VERIFICATION_REQUEST));
    }

    #[tokio::test]
    async fn schema_invalid_success_response_is_a_verification_failure() {
        let adapter = DockerReleaseOrchestrator::with_dependencies(
            DockerOrchestratorConfig::default(),
            Arc::new(FakeDocker {
                responses: Mutex::new(VecDeque::new()),
                commands: Mutex::new(Vec::new()),
            }),
            Arc::new(FakeHttp {
                responses: Mutex::new(VecDeque::from([Ok(HttpResponse {
                    status: 200,
                    body: include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/demo/model-server/invalid-response-v1.json"
                    ))
                    .to_owned(),
                })])),
                requests: Mutex::new(Vec::new()),
            }),
        );
        let result = adapter
            .verify_candidate(&DeploymentHandle {
                container_id: "candidate".to_owned(),
                container_name: "candidate".to_owned(),
                endpoint: "http://127.0.0.1:1234".to_owned(),
            })
            .await
            .unwrap();
        assert!(!result.passed);
        assert_eq!(
            result.failure_kind,
            Some(VerificationFailureKind::InvalidResponse)
        );
        assert_eq!(result.status, Some(200));
    }

    #[tokio::test]
    async fn inference_timeout_is_a_verification_failure_not_adapter_error() {
        let adapter = DockerReleaseOrchestrator::with_dependencies(
            DockerOrchestratorConfig::default(),
            Arc::new(FakeDocker {
                responses: Mutex::new(VecDeque::new()),
                commands: Mutex::new(Vec::new()),
            }),
            Arc::new(FakeHttp {
                responses: Mutex::new(VecDeque::from([Err(HttpRequestError::Timeout)])),
                requests: Mutex::new(Vec::new()),
            }),
        );
        let result = adapter
            .verify_candidate(&DeploymentHandle {
                container_id: "candidate".to_owned(),
                container_name: "candidate".to_owned(),
                endpoint: "http://127.0.0.1:1234".to_owned(),
            })
            .await
            .unwrap();
        assert!(!result.passed);
        assert_eq!(result.failure_kind, Some(VerificationFailureKind::Timeout));
    }

    #[tokio::test]
    async fn cleanup_tolerates_already_removed_containers() {
        let adapter = DockerReleaseOrchestrator::with_dependencies(
            DockerOrchestratorConfig::default(),
            Arc::new(FakeDocker {
                responses: Mutex::new(VecDeque::from([CommandOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "Error: No such container: candidate".to_owned(),
                }])),
                commands: Mutex::new(Vec::new()),
            }),
            Arc::new(FakeHttp {
                responses: Mutex::new(VecDeque::new()),
                requests: Mutex::new(Vec::new()),
            }),
        );
        adapter
            .cleanup_candidate(&DeploymentHandle {
                container_id: "candidate".to_owned(),
                container_name: "candidate".to_owned(),
                endpoint: "http://127.0.0.1:1234".to_owned(),
            })
            .await
            .unwrap();
    }
}
