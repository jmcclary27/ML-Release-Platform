resource "aws_ecr_repository" "platform" {
  name                 = "${local.name_prefix}-platform"
  image_tag_mutability = "IMMUTABLE"
  force_delete         = true

  image_scanning_configuration {
    scan_on_push = true
  }

  tags = {
    Name = "${local.name_prefix}-platform"
  }
}

resource "aws_ecr_lifecycle_policy" "platform" {
  repository = aws_ecr_repository.platform.name

  policy = jsonencode({
    rules = [
      {
        rulePriority = 1
        description  = "Retain only the most recent ephemeral-demo images."
        selection = {
          tagStatus   = "any"
          countType   = "imageCountMoreThan"
          countNumber = var.ecr_image_retention_count
        }
        action = {
          type = "expire"
        }
      }
    ]
  })
}
