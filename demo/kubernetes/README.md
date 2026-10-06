# Disposable Kubernetes acceptance demo

`./demo/kubernetes/run.sh` creates a local kind cluster, installs pinned Istio,
KServe, and Argo Rollouts releases, loads the checked-in reference-detector images,
then exercises the ordinary MLRelease HTTP API. It asserts KServe/Argo-backed v1
promotion and schema-invalid v2 rollback before deleting the whole cluster.

This path is opt-in. It requires Docker, kind, kubectl, curl, network access, and
enough local CPU/memory for the add-ons. It does not create AWS resources. Override
the version variables in the script only with a tested compatibility set.
