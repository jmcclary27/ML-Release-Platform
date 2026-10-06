#!/usr/bin/env sh
set -eu

# Opt-in, disposable Kubernetes acceptance path. It never touches AWS/EKS.
# Prerequisites: kind, docker, kubectl, curl, and the Rust toolchain.
CLUSTER_NAME=${ML_RELEASE_KIND_CLUSTER:-ml-release-platform-demo}
NAMESPACE=${ML_RELEASE_KUBERNETES_NAMESPACE:-ml-release-platform}
ISTIO_VERSION=${ML_RELEASE_ISTIO_VERSION:-1.24.2}
KSERVE_VERSION=${ML_RELEASE_KSERVE_VERSION:-v0.20.0}
ARGO_ROLLOUTS_VERSION=${ML_RELEASE_ARGO_ROLLOUTS_VERSION:-v1.8.0}

cleanup() {
  kill "${server_pid:-}" 2>/dev/null || true
  wait "${server_pid:-}" 2>/dev/null || true
  kind delete cluster --name "$CLUSTER_NAME" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

kind create cluster --name "$CLUSTER_NAME"
curl -fsSL "https://istio.io/downloadIstio" | ISTIO_VERSION="$ISTIO_VERSION" sh -
"istio-$ISTIO_VERSION/bin/istioctl" install --set profile=demo -y
kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f "https://github.com/argoproj/argo-rollouts/releases/download/$ARGO_ROLLOUTS_VERSION/install.yaml"
kubectl apply -f "https://github.com/kserve/kserve/releases/download/$KSERVE_VERSION/kserve.yaml"
kubectl wait --for=condition=Available deployment/argo-rollouts -n argo-rollouts --timeout=5m

./demo/build-images.sh
kind load docker-image --name "$CLUSTER_NAME" mlrp-demo-detector:valid mlrp-demo-detector:invalid

ML_RELEASE_DATABASE_URL="sqlite:///./demo/ml_release_kubernetes.sqlite" \
ML_RELEASE_ORCHESTRATION_BACKEND=kubernetes \
ML_RELEASE_KUBERNETES_NAMESPACE="$NAMESPACE" \
  cargo run --bin ml-release-platform >demo/ml_release_kubernetes.log 2>&1 &
server_pid=$!

attempt=0
until curl -fsS http://127.0.0.1:8000/health >/dev/null; do
  attempt=$((attempt + 1))
  [ "$attempt" -lt 30 ] || { cat demo/ml_release_kubernetes.log >&2; exit 1; }
  sleep 1
done

valid=$(cargo run --bin mlrelease -- deploy example:v1 --image mlrp-demo-detector:valid --metrics-file demo/metrics.json --policy-file demo/policy.json)
printf '%s\n' "$valid" | grep -F '"status": "RELEASED"' >/dev/null
invalid=$(cargo run --bin mlrelease -- deploy example:v2 --image mlrp-demo-detector:invalid --metrics-file demo/metrics.json --policy-file demo/policy.json)
printf '%s\n' "$invalid" | grep -F '"status": "ROLLED_BACK"' >/dev/null
printf '%s\n' "verified: API-driven KServe/Argo promotion and rollback"
