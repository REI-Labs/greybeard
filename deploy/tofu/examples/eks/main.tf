terraform {
  required_version = ">= 1.6"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }

  # Local state by default. To use S3 instead (OpenTofu >= 1.10 / Terraform
  # >= 1.10 support native S3 locking via use_lockfile):
  #
  # backend "s3" {
  #   bucket       = "my-tofu-state-bucket"
  #   key          = "greybeard/terraform.tfstate"
  #   region       = "us-east-2"
  #   encrypt      = true
  #   use_lockfile = true
  # }
}

provider "aws" {
  region = var.region

  default_tags {
    tags = var.tags
  }
}

module "greybeard" {
  source = "../../modules/greybeard-aws"

  name            = var.name
  namespace       = var.namespace
  service_account = var.service_account
  cluster_name    = var.cluster_name

  identity_mode     = var.identity_mode
  oidc_provider_arn = var.oidc_provider_arn
  oidc_provider_url = var.oidc_provider_url

  bedrock_regions = var.bedrock_regions

  create_ssm_parameters = true
  ssm_prefix            = var.ssm_prefix
  create_ecr_repository = var.create_ecr_repository
}
