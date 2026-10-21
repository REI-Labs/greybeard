output "role_arn" {
  description = "Greybeard IAM role ARN."
  value       = module.greybeard.role_arn
}

output "ssm_parameter_names" {
  description = "SSM parameters to seed out-of-band."
  value       = module.greybeard.ssm_parameter_names
}

output "ecr_repository_url" {
  description = "ECR repository URL (null when not created)."
  value       = module.greybeard.ecr_repository_url
}

output "eso_read_policy_json" {
  description = "Attach to your External Secrets Operator role."
  value       = module.greybeard.eso_read_policy_json
}
