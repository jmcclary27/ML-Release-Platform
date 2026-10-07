//! Kubernetes, `KServe`, and Argo Rollouts implementation of the release boundary.
//!
//! This module deliberately contains all Kubernetes concepts. The application service sees
//! only [`ReleaseOrchestrator`], so the lifecycle and policy engine remain backend agnostic.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::{
    Client,
    api::{
        Api, ApiResource, DeleteParams, DynamicObject, ListParams, Patch, PatchParams, PostParams,
    },
    core::GroupVersionKind,
};
use serde_json::{Map, Value, json};
use tokio::{io::copy_bidirectional, net::TcpListener, time::sleep};

use super::{
    DeploymentHandle, HttpMethod, HttpProbe, HttpRequestError, HttpResponse, LoopbackHttpProbe,
    ReleaseOrchestrator, VERIFICATION_REQUEST, VerificationFailureKind, VerificationResult,
    join_url, validate_health_response, validate_prediction_response,
};
use crate::domain::{errors::OrchestrationError, models::ModelRelease};

const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
const MANAGED_BY_VALUE: &str = "ml-release-platform";
const INFERENCE_SERVICE_API: &str = "serving.kserve.io";
const ROLLOUT_API: &str = "argoproj.io";
const ISTIO_API: &str = "networking.istio.io";

/// Kubernetes adapter configuration. Add-ons are installed separately by the opt-in demo.
#[derive(Clone, Debug)]
pub struct KubernetesOrchestratorConfig {
    pub namespace: String,
    pub container_port: u16,
    pub health_path: String,
    pub inference_path: String,
    pub startup_timeout: Duration,
    pub poll_interval: Duration,
    pub verification_timeout: Duration,
}

/// Result of probing the same detector HTTP contract through Kubernetes.
#[derive(Clone, Debug)]
pub struct KubernetesRuntimeResponses {
    pub health: HttpResponse,
    pub inference: HttpResponse,
}

#[derive(Clone, Copy)]
struct ProbeRequest<'a> {
    method: HttpMethod,
    path: &'a str,
    body: Option<&'a str>,
    timeout: Duration,
}

/// Kubernetes operations used by the adapter. Tests provide a deterministic fake; the concrete
/// implementation uses the Rust `kube` client and never shells out to `kubectl`.
#[async_trait]
pub trait KubernetesReleaseClient: Send + Sync {
    async fn deploy_candidate(
        &self,
        release: &ModelRelease,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<DeploymentHandle, OrchestrationError>;
    async fn probe_candidate(
        &self,
        candidate: &DeploymentHandle,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<KubernetesRuntimeResponses, OrchestrationError>;
    async fn promote_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<(), OrchestrationError>;
    async fn rollback_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<(), OrchestrationError>;
    async fn cleanup_candidate(
        &self,
        candidate: &DeploymentHandle,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<(), OrchestrationError>;
}

/// KServe/Argo implementation selected by `ML_RELEASE_ORCHESTRATION_BACKEND=kubernetes`.
pub struct KubernetesReleaseOrchestrator {
    config: KubernetesOrchestratorConfig,
    client: Arc<dyn KubernetesReleaseClient>,
}

impl KubernetesReleaseOrchestrator {
    /// Build the production adapter from normal in-cluster or kubeconfig settings.
    ///
    /// # Errors
    ///
    /// Returns an error when Kubernetes client configuration cannot be loaded.
    pub async fn from_default_client(
        config: KubernetesOrchestratorConfig,
    ) -> Result<Self, OrchestrationError> {
        let client = Client::try_default().await.map_err(|error| {
            OrchestrationError(format!("Could not create Kubernetes client: {error}"))
        })?;
        Ok(Self::with_client(
            config,
            Arc::new(KubeReleaseClient::new(client)),
        ))
    }

    #[must_use]
    pub fn with_client(
        config: KubernetesOrchestratorConfig,
        client: Arc<dyn KubernetesReleaseClient>,
    ) -> Self {
        Self { config, client }
    }

    fn verification_failure(responses: KubernetesRuntimeResponses) -> VerificationResult {
        if !(200..300).contains(&responses.health.status) {
            return VerificationResult::failed(
                VerificationFailureKind::HttpStatus,
                format!(
                    "Candidate health verification returned HTTP {}.",
                    responses.health.status
                ),
                Some(responses.health.status),
                Some(responses.health.body),
            );
        }
        if let Err(error) = validate_health_response(&responses.health.body) {
            return VerificationResult::failed(
                VerificationFailureKind::InvalidResponse,
                format!(
                    "Candidate health verification returned an invalid detector response: {error}"
                ),
                Some(responses.health.status),
                Some(responses.health.body),
            );
        }
        if !(200..300).contains(&responses.inference.status) {
            return VerificationResult::failed(
                VerificationFailureKind::HttpStatus,
                format!(
                    "Inference verification returned HTTP {}.",
                    responses.inference.status
                ),
                Some(responses.inference.status),
                Some(responses.inference.body),
            );
        }
        match validate_prediction_response(&responses.inference.body) {
            Ok(()) => {
                VerificationResult::passed(responses.inference.status, responses.inference.body)
            }
            Err(error) => VerificationResult::failed(
                VerificationFailureKind::InvalidResponse,
                format!("Inference verification returned an invalid detector response: {error}"),
                Some(responses.inference.status),
                Some(responses.inference.body),
            ),
        }
    }
}

#[async_trait]
impl ReleaseOrchestrator for KubernetesReleaseOrchestrator {
    async fn deploy_candidate(
        &self,
        release: &ModelRelease,
    ) -> Result<DeploymentHandle, OrchestrationError> {
        self.client.deploy_candidate(release, &self.config).await
    }

    async fn verify_candidate(
        &self,
        candidate: &DeploymentHandle,
    ) -> Result<VerificationResult, OrchestrationError> {
        match self.client.probe_candidate(candidate, &self.config).await {
            Ok(responses) => Ok(Self::verification_failure(responses)),
            Err(error) => Ok(VerificationResult::failed(
                VerificationFailureKind::Transport,
                format!("Could not run Kubernetes inference verification: {error}"),
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
        self.client
            .promote_candidate(candidate, previous_active, &self.config)
            .await
    }

    async fn rollback_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
    ) -> Result<(), OrchestrationError> {
        self.client
            .rollback_candidate(candidate, previous_active, &self.config)
            .await
    }

    async fn cleanup_candidate(
        &self,
        candidate: &DeploymentHandle,
    ) -> Result<(), OrchestrationError> {
        self.client.cleanup_candidate(candidate, &self.config).await
    }
}

/// Production Kubernetes API client. `KServe` owns predictor Pods; Argo changes the router's
/// Istio traffic weights. `MLRelease` observes both before committing state.
pub struct KubeReleaseClient {
    client: Client,
    http: Arc<dyn HttpProbe>,
}

impl KubeReleaseClient {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            http: Arc::new(LoopbackHttpProbe),
        }
    }

    #[must_use]
    pub fn with_http(client: Client, http: Arc<dyn HttpProbe>) -> Self {
        Self { client, http }
    }

    fn resource(group: &str, version: &str, kind: &str, plural: &str) -> ApiResource {
        let mut resource = ApiResource::from_gvk(&GroupVersionKind::gvk(group, version, kind));
        plural.clone_into(&mut resource.plural);
        resource
    }

    fn inference_services(&self, namespace: &str) -> Api<DynamicObject> {
        Api::namespaced_with(
            self.client.clone(),
            namespace,
            &Self::resource(
                INFERENCE_SERVICE_API,
                "v1beta1",
                "InferenceService",
                "inferenceservices",
            ),
        )
    }

    fn rollouts(&self, namespace: &str) -> Api<DynamicObject> {
        Api::namespaced_with(
            self.client.clone(),
            namespace,
            &Self::resource(ROLLOUT_API, "v1alpha1", "Rollout", "rollouts"),
        )
    }

    fn virtual_services(&self, namespace: &str) -> Api<DynamicObject> {
        Api::namespaced_with(
            self.client.clone(),
            namespace,
            &Self::resource(ISTIO_API, "v1", "VirtualService", "virtualservices"),
        )
    }

    fn services(&self, namespace: &str) -> Api<DynamicObject> {
        Api::namespaced_with(
            self.client.clone(),
            namespace,
            &Self::resource("", "v1", "Service", "services"),
        )
    }

    fn name(release: &ModelRelease) -> String {
        format!(
            "mlrp-{}-{}",
            component(&release.model_name, 24),
            component(&release.release_id, 12)
        )
    }

    fn model_name(release: &ModelRelease) -> String {
        format!("mlrp-{}", component(&release.model_name, 35))
    }

    async fn ensure_namespace(&self, namespace: &str) -> Result<(), OrchestrationError> {
        let namespaces: Api<Namespace> = Api::all(self.client.clone());
        let namespace_object: Namespace = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Namespace", "metadata": {"name": namespace, "labels": {MANAGED_BY_LABEL: MANAGED_BY_VALUE}}
        })).map_err(serialization_error)?;
        match namespaces
            .create(&PostParams::default(), &namespace_object)
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(error)) if error.code == 409 => Ok(()),
            Err(error) => Err(kube_error("create namespace", error)),
        }
    }

    async fn apply(
        &self,
        api: &Api<DynamicObject>,
        name: &str,
        value: Value,
    ) -> Result<(), OrchestrationError> {
        let object: DynamicObject = serde_json::from_value(value).map_err(serialization_error)?;
        api.patch(
            name,
            &PatchParams::apply("ml-release-platform").force(),
            &Patch::Apply(&object),
        )
        .await
        .map(|_| ())
        .map_err(|error| kube_error("apply resource", error))
    }

    async fn wait_for_ready(
        &self,
        namespace: &str,
        name: &str,
        timeout: Duration,
        poll: Duration,
    ) -> Result<(), OrchestrationError> {
        let deadline = Instant::now() + timeout;
        let api = self.inference_services(namespace);
        loop {
            if let Some(object) = api
                .get_opt(name)
                .await
                .map_err(|error| kube_error("read InferenceService", error))?
                && condition_true(&object, "Ready")
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(OrchestrationError(format!(
                    "KServe InferenceService '{name}' did not become Ready within {timeout:?}."
                )));
            }
            sleep(poll).await;
        }
    }

    async fn candidate_pod(
        &self,
        namespace: &str,
        service_name: &str,
    ) -> Result<String, OrchestrationError> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let selector = format!("serving.kserve.io/inferenceservice={service_name}");
        let list = pods
            .list(&ListParams::default().labels(&selector))
            .await
            .map_err(|error| kube_error("list predictor Pods", error))?;
        list.items
            .into_iter()
            .find_map(|pod| {
                let ready = pod
                    .status
                    .as_ref()?
                    .conditions
                    .as_ref()?
                    .iter()
                    .any(|condition| condition.type_ == "Ready" && condition.status == "True");
                ready.then_some(pod.metadata.name)
            })
            .flatten()
            .ok_or_else(|| {
                OrchestrationError(format!(
                    "No Ready predictor Pod found for InferenceService '{service_name}'."
                ))
            })
    }

    async fn request_through_port_forward(
        &self,
        namespace: &str,
        pod: &str,
        port: u16,
        request: ProbeRequest<'_>,
    ) -> Result<HttpResponse, OrchestrationError> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let mut forwarder = pods
            .portforward(pod, &[port])
            .await
            .map_err(|error| kube_error("port-forward predictor Pod", error))?;
        let stream = forwarder.take_stream(port).ok_or_else(|| {
            OrchestrationError(format!("Port-forward did not expose detector port {port}."))
        })?;
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|error| {
            OrchestrationError(format!("Could not create verification listener: {error}"))
        })?;
        let address = listener.local_addr().map_err(|error| {
            OrchestrationError(format!(
                "Could not read verification listener address: {error}"
            ))
        })?;
        let bridge = tokio::spawn(async move {
            let (mut local, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut remote = stream;
            copy_bidirectional(&mut local, &mut remote)
                .await
                .map_err(|error| error.to_string())
        });
        let endpoint = join_url(
            &format!("http://127.0.0.1:{}", address.port()),
            request.path,
        );
        let response = self
            .http
            .request(request.method, &endpoint, request.body, request.timeout)
            .await;
        let _ = bridge.await;
        response.map_err(http_error)
    }

    async fn wait_for_traffic(
        &self,
        namespace: &str,
        name: &str,
        candidate_weight: u64,
        timeout: Duration,
        poll: Duration,
    ) -> Result<(), OrchestrationError> {
        let deadline = Instant::now() + timeout;
        let services = self.virtual_services(namespace);
        loop {
            if let Some(object) = services
                .get_opt(name)
                .await
                .map_err(|error| kube_error("read Istio VirtualService", error))?
                && virtual_service_weight(&object, "candidate") == Some(candidate_weight)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(OrchestrationError(format!(
                    "Argo did not set candidate traffic for '{name}' to {candidate_weight}% within {timeout:?}."
                )));
            }
            sleep(poll).await;
        }
    }
}

#[async_trait]
impl KubernetesReleaseClient for KubeReleaseClient {
    async fn deploy_candidate(
        &self,
        release: &ModelRelease,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<DeploymentHandle, OrchestrationError> {
        self.ensure_namespace(&config.namespace).await?;
        let name = Self::name(release);
        let services = self.inference_services(&config.namespace);
        self.apply(
            &services,
            &name,
            inference_service(release, &name, config.container_port),
        )
        .await?;
        self.wait_for_ready(
            &config.namespace,
            &name,
            config.startup_timeout,
            config.poll_interval,
        )
        .await?;
        Ok(DeploymentHandle {
            container_id: name.clone(),
            container_name: name.clone(),
            endpoint: format!(
                "http://{name}-predictor.{}.svc.cluster.local",
                config.namespace
            ),
            metadata: Map::from_iter([
                ("backend".to_owned(), Value::String("kubernetes".to_owned())),
                (
                    "namespace".to_owned(),
                    Value::String(config.namespace.clone()),
                ),
                ("inference_service".to_owned(), Value::String(name)),
                (
                    "model_router".to_owned(),
                    Value::String(Self::model_name(release)),
                ),
            ]),
        })
    }

    async fn probe_candidate(
        &self,
        candidate: &DeploymentHandle,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<KubernetesRuntimeResponses, OrchestrationError> {
        let name = candidate_name(candidate)?;
        let pod = self.candidate_pod(&config.namespace, name).await?;
        let health = self
            .request_through_port_forward(
                &config.namespace,
                &pod,
                config.container_port,
                ProbeRequest {
                    method: HttpMethod::Get,
                    path: &config.health_path,
                    body: None,
                    timeout: config.verification_timeout,
                },
            )
            .await?;
        let inference = self
            .request_through_port_forward(
                &config.namespace,
                &pod,
                config.container_port,
                ProbeRequest {
                    method: HttpMethod::Post,
                    path: &config.inference_path,
                    body: Some(VERIFICATION_REQUEST),
                    timeout: config.verification_timeout,
                },
            )
            .await?;
        Ok(KubernetesRuntimeResponses { health, inference })
    }

    async fn promote_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<(), OrchestrationError> {
        let candidate_name = candidate_name(candidate)?;
        let model = candidate
            .metadata
            .get("model_router")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                OrchestrationError(
                    "Kubernetes deployment handle lacks model_router metadata.".to_owned(),
                )
            })?;
        let rollout_name = format!("{model}-traffic");
        let previous_service = previous_active.map(Self::name);
        let stable_service = format!("{model}-stable");
        let canary_service = format!("{model}-canary");
        let stable_target = previous_service.as_deref().unwrap_or(candidate_name);
        self.apply(
            &self.services(&config.namespace),
            &stable_service,
            external_service(&stable_service, stable_target, &config.namespace),
        )
        .await?;
        self.apply(
            &self.services(&config.namespace),
            &canary_service,
            external_service(&canary_service, candidate_name, &config.namespace),
        )
        .await?;
        self.apply(
            &self.virtual_services(&config.namespace),
            &rollout_name,
            virtual_service(
                &rollout_name,
                &stable_service,
                &canary_service,
                previous_service.is_none(),
            ),
        )
        .await?;
        self.apply(
            &self.rollouts(&config.namespace),
            &rollout_name,
            rollout(
                &rollout_name,
                &stable_service,
                &canary_service,
                candidate_name,
            ),
        )
        .await?;
        self.rollouts(&config.namespace)
            .patch(
                &rollout_name,
                &PatchParams::default(),
                &Patch::Merge(json!({"spec":{"paused":false}})),
            )
            .await
            .map_err(|error| kube_error("resume Argo Rollout", error))?;
        self.wait_for_traffic(
            &config.namespace,
            &rollout_name,
            100,
            config.startup_timeout,
            config.poll_interval,
        )
        .await
    }

    async fn rollback_candidate(
        &self,
        candidate: &DeploymentHandle,
        previous_active: Option<&ModelRelease>,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<(), OrchestrationError> {
        let model = candidate
            .metadata
            .get("model_router")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                OrchestrationError(
                    "Kubernetes deployment handle lacks model_router metadata.".to_owned(),
                )
            })?;
        let rollout_name = format!("{model}-traffic");
        if previous_active.is_none() {
            return self.cleanup_candidate(candidate, config).await;
        }
        self.rollouts(&config.namespace)
            .patch(
                &rollout_name,
                &PatchParams::default(),
                &Patch::Merge(json!({"spec":{"paused":true}})),
            )
            .await
            .map_err(|error| kube_error("pause Argo Rollout", error))?;
        let stable_name = Self::name(previous_active.expect("checked above"));
        let stable_service = format!("{model}-stable");
        let canary_service = format!("{model}-canary");
        self.apply(
            &self.services(&config.namespace),
            &stable_service,
            external_service(&stable_service, &stable_name, &config.namespace),
        )
        .await?;
        self.apply(
            &self.virtual_services(&config.namespace),
            &rollout_name,
            virtual_service(&rollout_name, &stable_service, &canary_service, false),
        )
        .await?;
        self.wait_for_traffic(
            &config.namespace,
            &rollout_name,
            0,
            config.startup_timeout,
            config.poll_interval,
        )
        .await?;
        self.cleanup_candidate(candidate, config).await
    }

    async fn cleanup_candidate(
        &self,
        candidate: &DeploymentHandle,
        config: &KubernetesOrchestratorConfig,
    ) -> Result<(), OrchestrationError> {
        let name = candidate_name(candidate)?;
        match self
            .inference_services(&config.namespace)
            .delete(name, &DeleteParams::default())
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(error)) if error.code == 404 => Ok(()),
            Err(error) => Err(kube_error("delete candidate InferenceService", error)),
        }
    }
}

fn inference_service(release: &ModelRelease, name: &str, port: u16) -> Value {
    json!({
        "apiVersion": "serving.kserve.io/v1beta1", "kind": "InferenceService",
        "metadata": {"name": name, "labels": {MANAGED_BY_LABEL: MANAGED_BY_VALUE, "ml-release-platform.model": release.model_name, "ml-release-platform.release-id": release.release_id}},
        "spec": {"predictor": {"containers": [{"name": "detector", "image": release.image_uri, "ports": [{"containerPort": port, "protocol": "TCP"}], "resources": {"requests": {"cpu": "50m", "memory": "64Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}}}]}}
    })
}

fn external_service(name: &str, target: &str, namespace: &str) -> Value {
    json!({"apiVersion":"v1", "kind":"Service", "metadata":{"name":name,"labels":{MANAGED_BY_LABEL:MANAGED_BY_VALUE}}, "spec":{"type":"ExternalName", "externalName":format!("{target}-predictor.{namespace}.svc.cluster.local")}})
}

fn virtual_service(name: &str, stable: &str, candidate: &str, candidate_active: bool) -> Value {
    let candidate_weight = if candidate_active { 100 } else { 0 };
    json!({
        "apiVersion": "networking.istio.io/v1", "kind": "VirtualService", "metadata": {"name": name, "labels": {MANAGED_BY_LABEL: MANAGED_BY_VALUE}},
        "spec": {"hosts": [name], "http": [{"name": "primary", "route": [
            {"destination": {"host": stable}, "weight": 100 - candidate_weight},
            {"destination": {"host": candidate}, "weight": candidate_weight}
        ]}]}
    })
}

fn rollout(name: &str, stable_service: &str, canary_service: &str, candidate: &str) -> Value {
    json!({
        "apiVersion": "argoproj.io/v1alpha1", "kind": "Rollout", "metadata": {"name": name, "labels": {MANAGED_BY_LABEL: MANAGED_BY_VALUE}},
        "spec": {"replicas": 1, "selector": {"matchLabels": {"app": name}}, "template": {"metadata": {"labels": {"app": name}, "annotations":{"ml-release-platform.candidate":candidate}}, "spec": {"containers": [{"name": "traffic-controller", "image": "registry.k8s.io/pause:3.10", "resources": {"requests": {"cpu": "5m", "memory": "8Mi"}, "limits": {"cpu": "20m", "memory": "16Mi"}}}]}}, "strategy": {"canary": {"stableService":stable_service, "canaryService":canary_service, "steps": [{"setWeight": 100}, {"pause": {}}], "trafficRouting": {"istio": {"virtualService": {"name": name, "routes": ["primary"]}}}}}, "paused": true}
    })
}

fn candidate_name(candidate: &DeploymentHandle) -> Result<&str, OrchestrationError> {
    candidate
        .metadata
        .get("inference_service")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            OrchestrationError(
                "Kubernetes deployment handle lacks inference_service metadata.".to_owned(),
            )
        })
}

fn condition_true(object: &DynamicObject, expected: &str) -> bool {
    object
        .data
        .get("status")
        .and_then(|status| status.get("conditions"))
        .and_then(Value::as_array)
        .is_some_and(|conditions| {
            conditions.iter().any(|condition| {
                condition.get("type") == Some(&Value::String(expected.to_owned()))
                    && condition.get("status") == Some(&Value::String("True".to_owned()))
            })
        })
}

fn virtual_service_weight(object: &DynamicObject, _route_name: &str) -> Option<u64> {
    object
        .data
        .get("spec")?
        .get("http")?
        .as_array()?
        .iter()
        .find(|route| route.get("name").and_then(Value::as_str) == Some("primary"))?
        .get("route")?
        .as_array()?
        .iter()
        .last()?
        .get("weight")?
        .as_u64()
}

fn component(value: &str, limit: usize) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            result.push(character);
            separator = false;
        } else if !separator {
            result.push('-');
            separator = true;
        }
        if result.len() >= limit {
            break;
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "release".to_owned()
    } else {
        result.to_owned()
    }
}

#[allow(clippy::needless_pass_by_value)]
fn serialization_error(error: serde_json::Error) -> OrchestrationError {
    OrchestrationError(format!("Could not serialize Kubernetes resource: {error}"))
}
#[allow(clippy::needless_pass_by_value)]
fn kube_error(operation: &str, error: kube::Error) -> OrchestrationError {
    OrchestrationError(format!("Kubernetes {operation} failed: {error}"))
}
fn http_error(error: HttpRequestError) -> OrchestrationError {
    match error {
        HttpRequestError::Timeout => {
            OrchestrationError("Kubernetes runtime verification timed out.".to_owned())
        }
        HttpRequestError::Transport(detail) => OrchestrationError(format!(
            "Kubernetes runtime verification transport failure: {detail}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use chrono::Utc;

    use super::*;
    use crate::domain::{models::ModelRelease, states::ReleaseStatus};

    struct FakeKubernetes {
        responses: KubernetesRuntimeResponses,
        promotions: Mutex<usize>,
        rollbacks: Mutex<usize>,
    }

    #[async_trait]
    impl KubernetesReleaseClient for FakeKubernetes {
        async fn deploy_candidate(
            &self,
            release: &ModelRelease,
            _: &KubernetesOrchestratorConfig,
        ) -> Result<DeploymentHandle, OrchestrationError> {
            Ok(DeploymentHandle {
                container_id: release.release_id.clone(),
                container_name: "mlrp-example".to_owned(),
                endpoint: "http://cluster.local".to_owned(),
                metadata: Map::from_iter([(
                    "inference_service".to_owned(),
                    Value::String("mlrp-example".to_owned()),
                )]),
            })
        }
        async fn probe_candidate(
            &self,
            _: &DeploymentHandle,
            _: &KubernetesOrchestratorConfig,
        ) -> Result<KubernetesRuntimeResponses, OrchestrationError> {
            Ok(self.responses.clone())
        }
        async fn promote_candidate(
            &self,
            _: &DeploymentHandle,
            _: Option<&ModelRelease>,
            _: &KubernetesOrchestratorConfig,
        ) -> Result<(), OrchestrationError> {
            *self.promotions.lock().unwrap() += 1;
            Ok(())
        }
        async fn rollback_candidate(
            &self,
            _: &DeploymentHandle,
            _: Option<&ModelRelease>,
            _: &KubernetesOrchestratorConfig,
        ) -> Result<(), OrchestrationError> {
            *self.rollbacks.lock().unwrap() += 1;
            Ok(())
        }
        async fn cleanup_candidate(
            &self,
            _: &DeploymentHandle,
            _: &KubernetesOrchestratorConfig,
        ) -> Result<(), OrchestrationError> {
            Ok(())
        }
    }

    fn config() -> KubernetesOrchestratorConfig {
        KubernetesOrchestratorConfig {
            namespace: "test".to_owned(),
            container_port: 8080,
            health_path: "/health".to_owned(),
            inference_path: "/infer".to_owned(),
            startup_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(1),
            verification_timeout: Duration::from_secs(1),
        }
    }

    fn release() -> ModelRelease {
        ModelRelease {
            release_id: "release-1".to_owned(),
            model_name: "example".to_owned(),
            version: "v1".to_owned(),
            image_uri: "example:v1".to_owned(),
            artifact_uri: None,
            status: ReleaseStatus::Ready,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: Map::new(),
            metrics: BTreeMap::default(),
            evaluation: None,
            failure_reason: None,
        }
    }

    fn health() -> HttpResponse {
        HttpResponse {
            status: 200,
            body: r#"{"contract_version":"v1","status":"ready","model_id":"reference:v1"}"#
                .to_owned(),
        }
    }

    #[tokio::test]
    async fn kubernetes_adapter_reuses_schema_validation_for_a_malformed_success() {
        let fake = Arc::new(FakeKubernetes {
            responses: KubernetesRuntimeResponses {
                health: health(),
                inference: HttpResponse {
                    status: 200,
                    body: include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/demo/model-server/invalid-response-v1.json"
                    ))
                    .to_owned(),
                },
            },
            promotions: Mutex::new(0),
            rollbacks: Mutex::new(0),
        });
        let adapter = KubernetesReleaseOrchestrator::with_client(config(), fake);
        let handle = adapter.deploy_candidate(&release()).await.unwrap();
        let result = adapter.verify_candidate(&handle).await.unwrap();
        assert!(!result.passed);
        assert_eq!(
            result.failure_kind,
            Some(VerificationFailureKind::InvalidResponse)
        );
    }

    #[tokio::test]
    async fn kubernetes_adapter_delegates_promotion_and_rollback() {
        let fake = Arc::new(FakeKubernetes {
            responses: KubernetesRuntimeResponses {
                health: health(),
                inference: HttpResponse {
                    status: 504,
                    body: String::new(),
                },
            },
            promotions: Mutex::new(0),
            rollbacks: Mutex::new(0),
        });
        let adapter = KubernetesReleaseOrchestrator::with_client(config(), fake.clone());
        let handle = adapter.deploy_candidate(&release()).await.unwrap();
        adapter.promote_candidate(&handle, None).await.unwrap();
        adapter.rollback_candidate(&handle, None).await.unwrap();
        assert_eq!(*fake.promotions.lock().unwrap(), 1);
        assert_eq!(*fake.rollbacks.lock().unwrap(), 1);
    }
}
