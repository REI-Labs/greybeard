output "role_arn" {
  description = "IAM role ARN. With identity_mode = \"irsa\", set it as the service account annotation eks.amazonaws.com/role-arn."
  value       = aws_iam_role.this.arn
}

output "ssm_parameter_names" {
  description = "Map of parameter key to SSM parameter name (empty when create_ssm_parameters = false)."
  value       = { for k, p in aws_ssm_parameter.this : k => p.name }
}

output "ecr_repository_url" {
  description = "ECR repository URL, or null when create_ecr_repository = false."
  value       = var.create_ecr_repository ? aws_ecr_repository.this[0].repository_url : null
}

output "eso_read_policy_json" {
  description = "IAM policy JSON granting read access to the Greybeard SSM parameters. Attach to your External Secrets Operator role."
  value       = data.aws_iam_policy_document.eso_read.json
}
