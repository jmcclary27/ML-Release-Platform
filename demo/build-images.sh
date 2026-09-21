#!/usr/bin/env sh
set -eu

docker build --build-arg MODEL_BEHAVIOR=valid -t mlrp-demo-detector:valid demo/model-server
docker build --build-arg MODEL_BEHAVIOR=invalid -t mlrp-demo-detector:invalid demo/model-server
docker build --build-arg MODEL_BEHAVIOR=unhealthy -t mlrp-demo-detector:unhealthy demo/model-server
