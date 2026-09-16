#!/usr/bin/env sh
set -eu

# This is a local-only demo: it builds two images, runs the API, proves rollback,
# then removes only containers labeled as managed by this release platform.
./demo/build-images.sh

ML_RELEASE_DATABASE_URL="sqlite:///./demo/ml_release_demo.sqlite" \
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

cargo run --bin mlrelease -- deploy example:v1 \
  --image mlrp-demo-model:good \
  --metrics-file demo/metrics.json \
  --policy-file demo/policy.json

cargo run --bin mlrelease -- deploy example:v2 \
  --image mlrp-demo-model:bad \
  --metrics-file demo/metrics.json \
  --policy-file demo/policy.json

cargo run --bin mlrelease -- current example
cargo run --bin mlrelease -- history example
