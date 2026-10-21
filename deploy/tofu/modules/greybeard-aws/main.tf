data "aws_caller_identity" "current" {}
data "aws_partition" "current" {}

locals {
  partition  = data.aws_partition.current.partition
  account_id = data.aws_caller_identity.current.account_id

  is_irsa = var.identity_mode == "irsa"

  oidc_issuer = local.is_irsa && var.oidc_provider_url != null ? trimprefix(var.oidc_provider_url, "https://") : ""

  default_bedrock_arns = concat(
    [for r in var.bedrock_regions : "arn:${local.partition}:bedrock:${r}::foundation-model/*"],
    [for r in var.bedrock_regions : "arn:${local.partition}:bedrock:${r}:${local.account_id}:inference-profile/*"],
  )

  bedrock_arns = var.bedrock_model_arn_patterns != null ? var.bedrock_model_arn_patterns : local.default_bedrock_arns

  # Parameter key => [type, description]. The app-id and installation-id are
  # not secret but live alongside the secrets so one ExternalSecret reads all.
  ssm_parameters = {
    "app-id"          = { type = "String", description = "GitHub App ID" }
    "installation-id" = { type = "String", description = "GitHub App installation ID" }
    "private-key"     = { type = "SecureString", description = "GitHub App RSA private key (PEM)" }
    "webhook-secret"  = { type = "SecureString", description = "GitHub webhook HMAC secret (same value goes in the App settings)" }
  }
}

################################################################################
# IAM role
################################################################################

data "aws_iam_policy_document" "trust" {
  lifecycle {
    precondition {
      condition     = !local.is_irsa || (var.oidc_provider_arn != null && var.oidc_provider_url != null)
      error_message = "oidc_provider_arn and oidc_provider_url are required when identity_mode = \"irsa\"."
    }
  }

  dynamic "statement" {
    for_each = local.is_irsa ? [] : [1]
    content {
      sid     = "EksPodIdentity"
      actions = ["sts:AssumeRole", "sts:TagSession"]
      principals {
        type        = "Service"
        identifiers = ["pods.eks.amazonaws.com"]
      }
    }
  }

  dynamic "statement" {
    for_each = local.is_irsa ? [1] : []
    content {
      sid     = "Irsa"
      actions = ["sts:AssumeRoleWithWebIdentity"]
      principals {
        type        = "Federated"
        identifiers = [var.oidc_provider_arn]
      }
      condition {
        test     = "StringEquals"
        variable = "${local.oidc_issuer}:sub"
        values   = ["system:serviceaccount:${var.namespace}:${var.service_account}"]
      }
      condition {
        test     = "StringEquals"
        variable = "${local.oidc_issuer}:aud"
        values   = ["sts.amazonaws.com"]
      }
    }
  }
}

resource "aws_iam_role" "this" {
  name               = var.name
  description        = "Greybeard PR-review service (${var.identity_mode})"
  assume_role_policy = data.aws_iam_policy_document.trust.json
  tags               = var.tags
}

data "aws_iam_policy_document" "bedrock" {
  statement {
    sid = "BedrockInvoke"
    actions = [
      "bedrock:InvokeModel",
      "bedrock:InvokeModelWithResponseStream",
    ]
    resources = local.bedrock_arns
  }
}

resource "aws_iam_policy" "bedrock" {
  name        = "${var.name}-bedrock"
  description = "Bedrock model invocation for Greybeard"
  policy      = data.aws_iam_policy_document.bedrock.json
  tags        = var.tags
}

resource "aws_iam_role_policy_attachment" "bedrock" {
  role       = aws_iam_role.this.name
  policy_arn = aws_iam_policy.bedrock.arn
}

resource "aws_eks_pod_identity_association" "this" {
  count = local.is_irsa ? 0 : 1

  cluster_name    = var.cluster_name
  namespace       = var.namespace
  service_account = var.service_account
  role_arn        = aws_iam_role.this.arn
  tags            = var.tags
}

################################################################################
# SSM parameters (GitHub App credentials)
#
# Created with a SEED_ME placeholder; real values are written out-of-band and
# ignore_changes keeps tofu from reverting them. Seed WITHOUT a trailing
# newline — a newline in the webhook secret breaks HMAC verification:
#
#   openssl rand -hex 32 > webhook-secret.txt
#   aws ssm put-parameter --overwrite --type SecureString \
#     --name /greybeard/webhook-secret --value "$(cat webhook-secret.txt)"
#
# Use --value "$(cat file)" (command substitution strips the trailing
# newline), NOT --value file://webhook-secret.txt (keeps it).
################################################################################

resource "aws_ssm_parameter" "this" {
  for_each = var.create_ssm_parameters ? local.ssm_parameters : {}

  name        = "${var.ssm_prefix}/${each.key}"
  description = each.value.description
  type        = each.value.type
  value       = "SEED_ME"
  tags        = var.tags

  lifecycle {
    ignore_changes = [value]
  }
}

# Read-only policy for an External Secrets Operator role. Output as JSON only;
# attach it to your ESO role (this module does not create that role).
data "aws_iam_policy_document" "eso_read" {
  statement {
    sid = "ReadGreybeardParameters"
    actions = [
      "ssm:GetParameter",
      "ssm:GetParameters",
      "ssm:GetParametersByPath",
    ]
    resources = [
      "arn:${local.partition}:ssm:*:${local.account_id}:parameter${var.ssm_prefix}",
      "arn:${local.partition}:ssm:*:${local.account_id}:parameter${var.ssm_prefix}/*",
    ]
  }

  # SecureString parameters use the AWS-managed aws/ssm key by default; ESO
  # needs kms:Decrypt through SSM only.
  statement {
    sid       = "DecryptViaSsm"
    actions   = ["kms:Decrypt"]
    resources = ["*"]
    condition {
      test     = "StringLike"
      variable = "kms:ViaService"
      values   = ["ssm.*.amazonaws.com"]
    }
  }
}

################################################################################
# ECR repository (optional)
################################################################################

resource "aws_ecr_repository" "this" {
  count = var.create_ecr_repository ? 1 : 0

  name                 = var.name
  image_tag_mutability = "IMMUTABLE"
  tags                 = var.tags

  image_scanning_configuration {
    scan_on_push = true
  }
}

resource "aws_ecr_lifecycle_policy" "this" {
  count = var.create_ecr_repository ? 1 : 0

  repository = aws_ecr_repository.this[0].name
  policy = jsonencode({
    rules = [
      {
        rulePriority = 1
        description  = "Expire untagged images after 1 day"
        selection = {
          tagStatus   = "untagged"
          countType   = "sinceImagePushed"
          countUnit   = "days"
          countNumber = 1
        }
        action = { type = "expire" }
      },
      {
        rulePriority = 2
        description  = "Keep last ${var.ecr_keep_image_count} images"
        selection = {
          tagStatus   = "any"
          countType   = "imageCountMoreThan"
          countNumber = var.ecr_keep_image_count
        }
        action = { type = "expire" }
      },
    ]
  })
}
