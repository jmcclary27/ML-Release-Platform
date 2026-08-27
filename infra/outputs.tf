output "aws_region" {
  description = "AWS Region containing the environment."
  value       = var.aws_region
}

output "cluster_name" {
  description = "Name of the EKS cluster."
  value       = aws_eks_cluster.main.name
}

output "cluster_endpoint" {
  description = "EKS Kubernetes API server endpoint."
  value       = aws_eks_cluster.main.endpoint
}

output "vpc_id" {
  description = "ID of the EKS VPC."
  value       = aws_vpc.main.id
}

output "ecr_repository_url" {
  description = "URL of the future ML Release Platform container repository."
  value       = aws_ecr_repository.platform.repository_url
}

output "model_artifact_bucket_name" {
  description = "Private S3 bucket reserved for future model and release artifacts."
  value       = aws_s3_bucket.model_artifacts.bucket
}

output "kubeconfig_command" {
  description = "Command to add this EKS cluster to the current kubeconfig."
  value       = "aws eks update-kubeconfig --region ${var.aws_region} --name ${aws_eks_cluster.main.name}"
}
