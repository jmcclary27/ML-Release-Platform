locals {
  name_prefix  = "${var.project_name}-${var.environment}"
  cluster_name = coalesce(var.cluster_name, "${local.name_prefix}-eks")

  common_tags = {
    Project       = var.project_name
    Environment   = var.environment
    ManagedBy     = "Terraform"
    Lifecycle     = "ephemeral-demo"
    EphemeralDemo = "true"
  }
}
