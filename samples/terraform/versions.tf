# The provider versions this workspace is pinned to, and where its state
# lives. Kept apart from the resources so an upgrade is a one-file diff.

terraform {
  required_version = ">= 1.6.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.40"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.6"
    }
  }

  backend "s3" {
    bucket         = "example-terraform-state"
    key            = "repo-sphere-explorer/prod.tfstate"
    region         = "eu-west-2"
    dynamodb_table = "terraform-locks"
    encrypt        = true
  }
}
provider "aws" {
  region = var.region

  default_tags {
    tags = local.common_tags
  }
}
