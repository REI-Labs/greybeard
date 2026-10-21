variable "name" {
  description = "Name prefix for every resource this module creates."
  type        = string
  default     = "greybeard"
}

variable "namespace" {
  description = "Kubernetes namespace Greybeard runs in."
  type        = string
  default     = "greybeard"
}

variable "service_account" {
  description = "Kubernetes service account the Greybeard pods use."
  type        = string
  default     = "greybeard"
}

variable "cluster_name" {
  description = "EKS cluster name. Used for the Pod Identity association."
  type        = string
}

variable "identity_mode" {
  description = "How pods obtain AWS credentials: \"pod_identity\" (EKS Pod Identity Agent) or \"irsa\" (IAM Roles for Service Accounts)."
  type        = string
  default     = "pod_identity"

  validation {
    condition     = contains(["pod_identity", "irsa"], var.identity_mode)
    error_message = "identity_mode must be \"pod_identity\" or \"irsa\"."
  }
}

variable "oidc_provider_arn" {
  description = "ARN of the cluster's IAM OIDC provider. Required when identity_mode = \"irsa\"."
  type        = string
  default     = null
}

variable "oidc_provider_url" {
  description = "Issuer URL of the cluster's OIDC provider (with or without https://). Required when identity_mode = \"irsa\"."
  type        = string
  default     = null
}

# Greybeard calls cross-region inference profiles (for example
# us.anthropic.*). A profile routes InvokeModel to ANY region in its set, so
# the role must be allowed to invoke the underlying foundation-model ARN in
# every one of those regions, not only the region the client calls. Keep this
# list in sync with the regions of the profile you configure.
variable "bedrock_regions" {
  description = "Regions whose Bedrock foundation models and inference profiles the role may invoke."
  type        = list(string)
  default     = ["us-east-1", "us-east-2", "us-west-2"]

  validation {
    condition     = length(var.bedrock_regions) > 0
    error_message = "bedrock_regions must contain at least one region."
  }
}

variable "bedrock_model_arn_patterns" {
  description = "Optional override for the Bedrock resource ARNs allowed. When null, foundation-model/* and inference-profile/* in every bedrock_regions entry are allowed."
  type        = list(string)
  default     = null
}

variable "create_ssm_parameters" {
  description = "Create SSM parameters for the GitHub App credentials (seeded out-of-band)."
  type        = bool
  default     = true
}

variable "ssm_prefix" {
  description = "Path prefix for the SSM parameters, starting with \"/\" and without a trailing slash."
  type        = string
  default     = "/greybeard"

  validation {
    condition     = can(regex("^/[A-Za-z0-9_./-]*[A-Za-z0-9_.-]$", var.ssm_prefix))
    error_message = "ssm_prefix must start with \"/\" and must not end with \"/\"."
  }
}

variable "create_ecr_repository" {
  description = "Create a private ECR repository for the Greybeard image."
  type        = bool
  default     = false
}

variable "ecr_keep_image_count" {
  description = "Number of images the ECR lifecycle policy keeps. Untagged images expire after 1 day."
  type        = number
  default     = 50
}

variable "tags" {
  description = "Tags applied to every taggable resource."
  type        = map(string)
  default     = {}
}
