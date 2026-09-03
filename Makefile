.DEFAULT_GOAL := help

INFRA_DIR := infra
SMOKE_MANIFEST := $(INFRA_DIR)/examples/smoke-test.yaml

.PHONY: help infra-init infra-fmt infra-validate infra-test infra-plan infra-up kubeconfig infra-verify infra-smoke infra-smoke-clean infra-down

help:
	@echo "ML Release Platform infrastructure commands:"
	@echo "  make infra-init        Initialize the local Terraform working directory"
	@echo "  make infra-plan        Review the AWS changes Terraform would make"
	@echo "  make infra-up          Apply the reviewed Terraform plan (creates billable resources)"
	@echo "  make kubeconfig        Configure kubectl for the created EKS cluster"
	@echo "  make infra-verify      Verify Kubernetes API connectivity and Ready nodes"
	@echo "  make infra-smoke       Run and remove the tiny Kubernetes scheduling Job"
	@echo "  make infra-down        Interactively run terraform destroy"

infra-init:
	terraform -chdir=$(INFRA_DIR) init

infra-fmt:
	terraform -chdir=$(INFRA_DIR) fmt -check -recursive

infra-validate:
	terraform -chdir=$(INFRA_DIR) init -backend=false
	terraform -chdir=$(INFRA_DIR) validate

infra-test:
	terraform -chdir=$(INFRA_DIR) test

infra-plan:
	terraform -chdir=$(INFRA_DIR) plan

infra-up:
	@echo "This creates billable, ephemeral AWS resources. Destroy them after the demo with: make infra-down"
	terraform -chdir=$(INFRA_DIR) apply

kubeconfig:
	aws eks update-kubeconfig --region "$$(terraform -chdir=$(INFRA_DIR) output -raw aws_region)" --name "$$(terraform -chdir=$(INFRA_DIR) output -raw cluster_name)"

infra-verify: kubeconfig
	kubectl get nodes -o wide
	kubectl wait --for=condition=Ready nodes --all --timeout=5m
	kubectl get pods --all-namespaces

infra-smoke: infra-verify
	@set -eu; cleanup() { kubectl delete --ignore-not-found -f "$(SMOKE_MANIFEST)"; }; trap cleanup EXIT; kubectl apply -f "$(SMOKE_MANIFEST)"; kubectl wait --for=condition=complete job/ml-release-platform-smoke-test --timeout=5m; kubectl logs job/ml-release-platform-smoke-test

infra-smoke-clean:
	kubectl delete --ignore-not-found -f "$(SMOKE_MANIFEST)"

infra-down:
	@echo "This runs an interactive terraform destroy and deletes this stack's ECR images and S3 objects/versions."
	@echo "First delete Kubernetes LoadBalancer Services or other external resources created during your demo."
	terraform -chdir=$(INFRA_DIR) destroy
