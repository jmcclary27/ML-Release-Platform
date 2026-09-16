#!/usr/bin/env sh
set -eu

docker build --build-arg MODEL_BEHAVIOR=good -t mlrp-demo-model:good demo/model-server
docker build --build-arg MODEL_BEHAVIOR=bad -t mlrp-demo-model:bad demo/model-server
