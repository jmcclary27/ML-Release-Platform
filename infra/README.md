# Ephemeral AWS/EKS demo foundation

This directory provisions only the AWS foundation for a short-lived ML Release Platform demo: VPC networking, an EKS control plane, one managed worker, a future container repository, and a private future-artifact bucket. It does **not** deploy the control plane, KServe, Argo Rollouts, model workloads, databases, load balancers, persistent volumes, or observability services.

The intended lifecycle is **CREATE → VERIFY → DEMO → DESTROY**. The environment is for hours, not days. Run `make infra-down` as soon as the demo is complete.

## Architecture and cost choices

EKS requires subnets in at least two Availability Zones, so this stack creates two public subnets, an internet gateway, and public routing. Its single `t3.medium` managed node runs in those subnets and has a public IPv4 address so it can retrieve EKS and container images without a NAT gateway. There is no SSH configuration and no public workload or load-balancer service in this foundation. EKS-managed security groups do not add Internet-wide ingress; the public EKS API is restricted to the CIDRs supplied in `cluster_endpoint_public_access_cidrs` while private endpoint access remains enabled.

Using public workers is a deliberate demo-only cost tradeoff. It removes the recurring NAT gateway and Elastic IP cost while preserving a real EKS cluster with restricted control-plane access. A production deployment should use private workers and appropriate egress design.

The primary charges while the stack exists are:

- EKS control-plane hourly charges;
- one on-demand `t3.medium` worker and its public IPv4 address;
- ECR image storage and S3 object storage, if used; and
- normal Internet/AWS data-transfer charges.

There is intentionally no NAT gateway, load balancer, database, persistent disk, or monitoring service. ECR retains only three images; S3 expires artifacts after seven days and noncurrent versions after one day. Current AWS pricing varies by region, so review it before applying.

All resources receive `Project`, `Environment`, `ManagedBy=Terraform`, `Lifecycle=ephemeral-demo`, and `EphemeralDemo=true` tags. Explicit Terraform destruction also deletes contents of only this stack's ECR repository and S3 bucket.

## Prerequisites and authentication

- Terraform 1.7 or later
- AWS CLI v2
- `kubectl`
- GNU Make
- AWS credentials for the target account and region; no keys belong in this repository

Authenticate with your normal AWS SSO, profile, environment credentials, or instance/role credentials. For example, with an SSO profile:

```bash
aws sso login --profile <profile>
export AWS_PROFILE=<profile>
aws sts get-caller-identity
```

The provisioning identity needs permissions to create, tag, describe, and delete the VPC/EC2 networking resources, EKS cluster/node group/access entries, required IAM roles and managed-policy attachments, ECR repository/lifecycle policy, and S3 bucket/configuration. It also needs permission to pass the created EKS IAM roles. The static IAM user or role configured as `cluster_admin_principal_arn` needs EKS access-entry administration and becomes Kubernetes cluster admin. Do not use an STS assumed-role session ARN for that variable.

## Create and verify

From the repository root, create an ignored local configuration file:

```bash
cp infra/terraform.tfvars.example infra/terraform.tfvars
```

Edit both required values before any plan:

- Set `cluster_endpoint_public_access_cidrs` to your current public `/32` or another approved CIDR. Never use `0.0.0.0/0`.
- Set `cluster_admin_principal_arn` to your static IAM role or user ARN.

Then run:

```bash
make infra-init
make infra-plan
make infra-up
make kubeconfig
make infra-verify
make infra-smoke
```

`infra-up` visibly runs interactive `terraform apply` and creates billable resources. `infra-verify` confirms kubeconfig/API connectivity and that every worker is Ready. `infra-smoke` creates a tiny, resource-bounded Job, waits for successful scheduling and completion, prints its log, and removes it even if the wait fails. Run `make infra-smoke-clean` if a local interruption prevents that cleanup.

For the equivalent direct Terraform workflow, use `terraform -chdir=infra init`, `plan`, and `apply`; `make kubeconfig` runs the `aws eks update-kubeconfig` command emitted by the Terraform output.

## Demo and teardown

Run the future platform demo only after verification. Before teardown, remove every Kubernetes object created by the demo, particularly `Service` objects of type `LoadBalancer`, persistent-volume claims, and anything that provisions AWS resources. Check:

```bash
kubectl get services --all-namespaces
kubectl get pvc --all-namespaces
kubectl get ingress --all-namespaces
terraform -chdir=infra output -raw teardown_verification_commands
```

Save the last output before teardown; it contains account/region-specific AWS checks to run afterward. Then destroy deliberately:

```bash
make infra-down
```

The target prints its behavior and invokes interactive `terraform destroy`; it does not schedule or hide deletion. ECR images and every S3 object/version in the stack bucket are automatically removed because this is an explicitly ephemeral demo stack. No unrelated resource is selected by those settings.

After successful destroy, run the saved EKS, ECR, and S3 lookup commands: each should return a not-found error. Also verify that no EC2 instances, load balancers, or ENIs remain with `Project=ml-release-platform` and `EphemeralDemo=true` in the target region, and that `terraform -chdir=infra state list` returns no managed infrastructure.

If destroy is blocked, wait for Kubernetes-created load balancers/ENIs to finish deleting, delete their originating Kubernetes resources, and retry. If an external actor has attached an ENI or security group to another resource, identify and remove that external attachment rather than force-deleting Terraform resources. Do not run `terraform destroy` with a different account, region, or state file than the one used to create the demo.

## Local validation

These commands do not create AWS resources:

```bash
make infra-fmt
make infra-validate
make infra-test
```

`infra-test` uses Terraform's mocked AWS provider and checks the two-subnet EKS topology, default single-node capacity, restricted API configuration, standard EKS support policy, and force-destroy settings without AWS credentials.
