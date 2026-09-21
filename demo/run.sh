#!/usr/bin/env sh
set -eu

# This local-only acceptance demo proves valid promotion, invalid-response
# rollback, and startup-health rejection against real Docker containers.
./demo/build-images.sh

ML_RELEASE_DATABASE_URL="sqlite:///./demo/ml_release_demo.sqlite" \
ML_RELEASE_STARTUP_TIMEOUT_SECONDS=3 \
  cargo run --bin ml-release-platform >demo/ml_release_demo.log 2>&1 &
server_pid=$!

cleanup() {
  kill "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
  ./demo/cleanup.sh || true
  rm -f demo/ml_release_demo.sqlite demo/ml_release_demo.sqlite-shm \
    demo/ml_release_demo.sqlite-wal demo/ml_release_demo.log
}
trap cleanup EXIT INT TERM

attempt=0
until curl -fsS http://127.0.0.1:8000/health >/dev/null; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then
    echo "API did not become ready; inspect demo/ml_release_demo.log" >&2
    exit 1
  fi
  sleep 1
done

valid_output=$(cargo run --bin mlrelease -- deploy example:v1 \
  --image mlrp-demo-detector:valid \
  --metrics-file demo/metrics.json \
  --policy-file demo/policy.json)
printf '%s\n' "$valid_output"
printf '%s\n' "$valid_output" | grep -F '"status": "RELEASED"' >/dev/null
echo "verified: valid detector promoted v1"

invalid_output=$(cargo run --bin mlrelease -- deploy example:v2 \
  --image mlrp-demo-detector:invalid \
  --metrics-file demo/metrics.json \
  --policy-file demo/policy.json)
printf '%s\n' "$invalid_output"
printf '%s\n' "$invalid_output" | grep -F '"status": "ROLLED_BACK"' >/dev/null
printf '%s\n' "$invalid_output" | grep -F 'invalid detector response' >/dev/null
echo "verified: schema-invalid inference response rolled back v2"

if cargo run --bin mlrelease -- deploy example:v3 \
  --image mlrp-demo-detector:unhealthy \
  --metrics-file demo/metrics.json \
  --policy-file demo/policy.json; then
  echo "unhealthy detector unexpectedly completed deployment" >&2
  exit 1
fi
echo "verified: unhealthy detector was rejected during deployment"

current_output=$(cargo run --bin mlrelease -- current example)
printf '%s\n' "$current_output"
printf '%s\n' "$current_output" | grep -F '"version": "v1"' >/dev/null

history_output=$(cargo run --bin mlrelease -- history example)
printf '%s\n' "$history_output"
printf '%s\n' "$history_output" | grep -F 'DEPLOYMENT_FAILED' >/dev/null
echo "verified: v1 remained active after both failure paths"
