# AWS/EKS infrastructure foundation

This directory creates the development AWS foundation for the ML Release Platform. It provisions a VPC, EKS cluster and managed node group, ECR repository, and private S3 artifact bucket. It does not deploy the control plane, KServe, Argo Rollouts, or any model-serving workload.

## Prerequisites

- Terraform 1.6 or later
- AWS CLI v2
- `kubectl`
- AWS credentials or a profile with permission to create the described VPC, IAM, EKS, ECR, and S3 resources

Use any configured AWS profile; no profile name is assumed. The credentials need permission to create EKS access entries and to use the IAM principal named in `cluster_admin_principal_arn`.

## Configure and deploy

From the repository root:

```bash
cd infra
cp terraform.tfvars.example terraform.tfvars
```

Edit `terraform.tfvars` before planning:

- Set `cluster_endpoint_public_access_cidrs` to the public `/32` (or approved CIDR) from which you will run `kubectl`.
- Set `cluster_admin_principal_arn` to a static IAM role or user ARN. Do not use an `arn:aws:sts::...:assumed-role/...` session ARN.

Then run:

```bash
terraform init
terraform fmt -check
terraform validate
terraform plan
terraform apply
```

The configuration uses local Terraform state for this individual development environment. Local state is not committed. Use a remote backend with locking before sharing state with a team or using this as a production environment.

## Connect and verify

After apply completes, run the command emitted in the `kubeconfig_command` output, or run:

```bash
aws eks update-kubeconfig \
  --region "$(terraform output -raw aws_region)" \
  --name "$(terraform output -raw cluster_name)"

kubectl get nodes
kubectl get pods --all-namespaces
```

The EKS API has a private endpoint for in-VPC traffic and a public endpoint restricted to `cluster_endpoint_public_access_cidrs`. If `kubectl` cannot connect, confirm that your current public IP matches the configured CIDR and that the AWS identity used by the CLI is the identity represented by `cluster_admin_principal_arn` (or can assume it).

Run the optional workload smoke test:

```bash
kubectl apply -f examples/smoke-test.yaml
kubectl rollout status deployment/nginx-smoke-test --timeout=5m
kubectl get deployment nginx-smoke-test
kubectl get pods -l app=nginx-smoke-test
```

Clean up the smoke test when finished:

```bash
kubectl delete -f examples/smoke-test.yaml
```

## Networking and access design

The VPC spans two availability zones with one public and one private subnet in each. Worker nodes run only in the private subnets and do not receive public IPs. Public subnets are tagged for future internet-facing Kubernetes load balancers; private subnets are tagged for internal load balancers.

A single NAT gateway provides outbound IPv4 access to both private subnets. This is intentional for a portfolio/development environment: it keeps worker nodes private while avoiding the recurring cost of a NAT gateway per AZ. It is not zone-resilient; a production design should use one NAT gateway per AZ or an explicitly evaluated alternative.

EKS authentication uses API-only access entries rather than the legacy `aws-auth` ConfigMap. The configured administrator receives the AWS-managed EKS cluster-admin access policy. Future workload access to AWS services should use EKS Pod Identity or IRSA; this foundation intentionally creates no application/pod IAM permissions.

## Cost considerations

AWS charges accrue while these resources exist. The primary ongoing costs are the EKS control plane, on-demand EC2 worker node, NAT gateway and Elastic IP, ECR image storage, and S3 storage/data transfer. The configuration defaults to one `t3.medium` worker and a single NAT gateway to limit development cost, at the availability tradeoffs described above. The ECR lifecycle policy retains only the newest ten images; S3 keeps current artifact versions but expires noncurrent versions after 30 days.

Review current AWS pricing for the selected region before applying. Do not leave the environment running when it is not needed.

## Destroy

Delete Kubernetes resources that created AWS resources, such as `Service` objects of type `LoadBalancer`, before destroying the foundation. Then run:

```bash
terraform destroy
```

The S3 bucket deliberately has `force_destroy = false` to avoid deleting artifacts accidentally. Empty it intentionally before destroying Terraform infrastructure if it contains objects or versions. ECR images may likewise need to be removed before the repository can be deleted.
