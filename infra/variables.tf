variable "aws_region" {
  description = "AWS Region in which to create the development environment."
  type        = string
  default     = "us-east-1"

  validation {
    condition     = can(regex("^[a-z]{2}-[a-z]+-[0-9]+$", var.aws_region))
    error_message = "aws_region must look like an AWS region identifier, for example us-east-1."
  }
}

variable "project_name" {
  description = "Project prefix used in resource names and tags."
  type        = string
  default     = "ml-release-platform"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{1,30}[a-z0-9]$", var.project_name))
    error_message = "project_name must be 3-32 lowercase letters, digits, or hyphens and start and end with a letter or digit."
  }
}

variable "environment" {
  description = "Environment label used in resource names and tags."
  type        = string
  default     = "dev"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{0,14}$", var.environment))
    error_message = "environment must contain lowercase letters, digits, and hyphens and start with a letter."
  }
}

variable "vpc_cidr" {
  description = "IPv4 CIDR range for the VPC."
  type        = string
  default     = "10.0.0.0/16"

  validation {
    condition     = can(cidrnetmask(var.vpc_cidr))
    error_message = "vpc_cidr must be a valid IPv4 CIDR block."
  }
}

variable "cluster_name" {
  description = "Optional EKS cluster name. Defaults to <project_name>-<environment>-eks."
  type        = string
  default     = null
  nullable    = true

  validation {
    condition     = var.cluster_name == null || can(regex("^[A-Za-z0-9][A-Za-z0-9_-]{0,99}$", var.cluster_name))
    error_message = "cluster_name must be 1-100 characters containing letters, digits, hyphens, or underscores."
  }
}

variable "kubernetes_version" {
  description = "EKS Kubernetes minor version."
  type        = string
  default     = "1.36"

  validation {
    condition     = can(regex("^[0-9]+\\.[0-9]+$", var.kubernetes_version))
    error_message = "kubernetes_version must be a minor version such as 1.36."
  }
}

variable "cluster_endpoint_public_access_cidrs" {
  description = "Trusted CIDRs permitted to reach the public EKS API endpoint. Include the provisioning developer's public IP/CIDR."
  type        = list(string)

  validation {
    condition     = length(var.cluster_endpoint_public_access_cidrs) > 0 && alltrue([for cidr in var.cluster_endpoint_public_access_cidrs : can(cidrnetmask(cidr))])
    error_message = "Provide at least one valid CIDR for cluster_endpoint_public_access_cidrs; do not leave the public EKS API unrestricted."
  }
}

variable "cluster_admin_principal_arn" {
  description = "Static IAM user or role ARN that receives EKS cluster-admin access. Do not use an STS assumed-role ARN."
  type        = string

  validation {
    condition     = can(regex("^arn:[a-z0-9-]+:iam::[0-9]{12}:(user|role)/.+$", var.cluster_admin_principal_arn))
    error_message = "cluster_admin_principal_arn must be a static IAM user or role ARN in the current AWS partition."
  }
}

variable "node_instance_types" {
  description = "EC2 instance types for the development managed node group."
  type        = list(string)
  default     = ["t3.medium"]

  validation {
    condition     = length(var.node_instance_types) > 0
    error_message = "Specify at least one node instance type."
  }
}

variable "node_desired_size" {
  description = "Desired number of development worker nodes."
  type        = number
  default     = 1

  validation {
    condition     = var.node_desired_size >= 1
    error_message = "node_desired_size must be at least one."
  }
}

variable "node_min_size" {
  description = "Minimum number of development worker nodes."
  type        = number
  default     = 1

  validation {
    condition     = var.node_min_size >= 1
    error_message = "node_min_size must be at least one."
  }
}

variable "node_max_size" {
  description = "Maximum number of development worker nodes."
  type        = number
  default     = 2

  validation {
    condition     = var.node_max_size >= var.node_desired_size && var.node_max_size >= var.node_min_size
    error_message = "node_max_size must be at least node_desired_size and node_min_size."
  }
}

variable "ecr_image_retention_count" {
  description = "Number of newest ECR images retained by the development lifecycle policy."
  type        = number
  default     = 10

  validation {
    condition     = var.ecr_image_retention_count >= 1
    error_message = "ecr_image_retention_count must be at least one."
  }
}
