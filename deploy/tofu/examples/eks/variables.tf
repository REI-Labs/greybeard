variable "region" {
  description = "AWS region for the EKS cluster, SSM parameters and ECR repository."
  type        = string
}

variable "cluster_name" {
  description = "Existing EKS cluster name."
  type        = string
}

variable "name" {
  description = "Name prefix for created resources."
  type        = string
  default     = "greybeard"
}

variable "namespace" {
  description = "Kubernetes namespace."
  type        = string
  default     = "greybeard"
}

variable "service_account" {
  description = "Kubernetes service account."
  type        = string
  default     = "greybeard"
}

variable "identity_mode" {
  description = "\"pod_identity\" or \"irsa\"."
  type        = string
  default     = "pod_identity"
}

variable "oidc_provider_arn" {
  description = "Cluster OIDC provider ARN (irsa only)."
  type        = string
  default     = null
}

variable "oidc_provider_url" {
  description = "Cluster OIDC issuer URL (irsa only)."
  type        = string
  default     = null
}

variable "bedrock_regions" {
  description = "Regions covered by the Bedrock cross-region inference profile in use."
  type        = list(string)
  default     = ["us-east-1", "us-east-2", "us-west-2"]
}

variable "ssm_prefix" {
  description = "SSM parameter path prefix."
  type        = string
  default     = "/greybeard"
}

variable "create_ecr_repository" {
  description = "Create a private ECR repository for the image."
  type        = bool
  default     = false
}

variable "tags" {
  description = "Default tags for every resource."
  type        = map(string)
  default     = {}
}
