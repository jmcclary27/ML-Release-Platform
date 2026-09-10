mock_provider "aws" {
  mock_data "aws_availability_zones" {
    defaults = {
      names = ["us-east-1a", "us-east-1b"]
    }
  }

  mock_data "aws_caller_identity" {
    defaults = {
      account_id = "123456789012"
    }
  }

  mock_data "aws_partition" {
    defaults = {
      partition = "aws"
    }
  }
}

override_data {
  target = data.aws_iam_policy_document.eks_cluster_assume_role

  values = {
    json = <<-JSON
      {"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"eks.amazonaws.com"},"Action":"sts:AssumeRole"}]}
    JSON
  }
}

override_data {
  target = data.aws_iam_policy_document.eks_node_assume_role

  values = {
    json = <<-JSON
      {"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}
    JSON
  }
}

override_resource {
  target          = aws_subnet.public[0]
  override_during = plan

  values = {
    id = "subnet-00000000000000001"
  }
}

override_resource {
  target          = aws_subnet.public[1]
  override_during = plan

  values = {
    id = "subnet-00000000000000002"
  }
}

run "plans_an_ephemeral_single_node_demo" {
  command = plan

  variables {
    cluster_endpoint_public_access_cidrs = ["203.0.113.10/32"]
    cluster_admin_principal_arn          = "arn:aws:iam::123456789012:role/PlatformAdministrator"
    # Terraform tests load developer-local terraform.tfvars, so this pins the offline ephemeral-demo fixture to 1/1/1.
    node_desired_size = 1
    node_min_size     = 1
    node_max_size     = 1
  }

  assert {
    condition     = length(aws_subnet.public) == 2
    error_message = "EKS requires two subnets in distinct Availability Zones."
  }

  assert {
    condition     = aws_eks_node_group.development.scaling_config[0].desired_size == 1 && aws_eks_node_group.development.scaling_config[0].min_size == 1 && aws_eks_node_group.development.scaling_config[0].max_size == 1
    error_message = "The default ephemeral demo must provision exactly one worker."
  }

  assert {
    condition     = length(aws_eks_node_group.development.subnet_ids) == 2
    error_message = "The node group must use the two public demo subnets."
  }

  assert {
    condition     = aws_eks_cluster.main.vpc_config[0].endpoint_private_access && aws_eks_cluster.main.vpc_config[0].endpoint_public_access && length(aws_eks_cluster.main.vpc_config[0].public_access_cidrs) > 0
    error_message = "The EKS API must retain private access and restrict public access."
  }

  assert {
    condition     = aws_eks_cluster.main.upgrade_policy[0].support_type == "STANDARD"
    error_message = "The ephemeral cluster must not enter paid EKS extended support."
  }

  assert {
    condition     = aws_ecr_repository.platform.force_delete && aws_s3_bucket.model_artifacts.force_destroy
    error_message = "Explicit Terraform teardown must remove stack-owned images and artifacts."
  }
}
