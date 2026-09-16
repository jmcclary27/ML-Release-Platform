#!/usr/bin/env sh
set -eu

# The label is applied only by the local release adapter, so unrelated containers are untouched.
container_ids=$(docker ps -aq --filter label=ml-release-platform.managed=true)
if [ -n "$container_ids" ]; then
    docker rm -f $container_ids
fi
