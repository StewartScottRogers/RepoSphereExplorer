# A small but complete stack: a VPC with public and private subnets, an
# autoscaling web tier behind a load balancer, and the state/provider
# wiring a real root module carries.

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

variable "region" {
  description = "AWS region to deploy into"
  type        = string
  default     = "eu-west-2"
}

variable "environment" {
  description = "Which environment this stack is"
  type        = string

  validation {
    condition     = contains(["dev", "staging", "prod"], var.environment)
    error_message = "environment must be one of dev, staging, prod."
  }
}

variable "instance_type" {
  description = "EC2 instance type for the web tier"
  type        = string
  default     = "t3.small"
}

variable "desired_capacity" {
  description = "How many web instances to run"
  type        = number
  default     = 2
}

variable "availability_zones" {
  description = "AZs to spread subnets across"
  type        = list(string)
  default     = ["eu-west-2a", "eu-west-2b"]
}

locals {
  name = "rse-${var.environment}"

  common_tags = {
    Project     = "RepoSphereExplorer"
    Environment = var.environment
    ManagedBy   = "terraform"
  }

  public_cidrs  = [for index in range(length(var.availability_zones)) : cidrsubnet("10.20.0.0/16", 8, index)]
  private_cidrs = [for index in range(length(var.availability_zones)) : cidrsubnet("10.20.0.0/16", 8, index + 100)]
}

data "aws_ami" "web" {
  most_recent = true
  owners      = ["amazon"]

  filter {
    name   = "name"
    values = ["al2023-ami-*-x86_64"]
  }
}

resource "random_id" "suffix" {
  byte_length = 4
}

resource "aws_vpc" "main" {
  cidr_block           = "10.20.0.0/16"
  enable_dns_hostnames = true
  enable_dns_support   = true

  tags = merge(local.common_tags, { Name = "${local.name}-vpc" })
}

resource "aws_subnet" "public" {
  count                   = length(var.availability_zones)
  vpc_id                  = aws_vpc.main.id
  cidr_block              = local.public_cidrs[count.index]
  availability_zone       = var.availability_zones[count.index]
  map_public_ip_on_launch = true

  tags = merge(local.common_tags, { Name = "${local.name}-public-${count.index}" })
}

resource "aws_subnet" "private" {
  count             = length(var.availability_zones)
  vpc_id            = aws_vpc.main.id
  cidr_block        = local.private_cidrs[count.index]
  availability_zone = var.availability_zones[count.index]

  tags = merge(local.common_tags, { Name = "${local.name}-private-${count.index}" })
}

resource "aws_security_group" "web" {
  name        = "${local.name}-web-${random_id.suffix.hex}"
  description = "Web tier ingress"
  vpc_id      = aws_vpc.main.id

  ingress {
    description = "HTTPS from anywhere"
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_launch_template" "web" {
  name_prefix   = "${local.name}-web-"
  image_id      = data.aws_ami.web.id
  instance_type = var.instance_type

  network_interfaces {
    security_groups = [aws_security_group.web.id]
  }

  user_data = base64encode(<<-EOT
    #!/bin/bash
    set -euo pipefail
    dnf install -y nginx
    systemctl enable --now nginx
  EOT
  )

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_autoscaling_group" "web" {
  name                = "${local.name}-web"
  min_size            = 1
  max_size            = var.desired_capacity * 2
  desired_capacity    = var.desired_capacity
  vpc_zone_identifier = aws_subnet.private[*].id

  launch_template {
    id      = aws_launch_template.web.id
    version = "$Latest"
  }

  dynamic "tag" {
    for_each = local.common_tags
    content {
      key                 = tag.key
      value               = tag.value
      propagate_at_launch = true
    }
  }
}

module "logging" {
  source = "./modules/logging"

  name           = local.name
  retention_days = var.environment == "prod" ? 90 : 14
  tags           = local.common_tags
}

output "vpc_id" {
  description = "The VPC everything sits in"
  value       = aws_vpc.main.id
}

output "public_subnet_ids" {
  description = "Public subnet ids, one per AZ"
  value       = aws_subnet.public[*].id
}

output "web_security_group" {
  description = "Security group protecting the web tier"
  value       = aws_security_group.web.id
  sensitive   = false
}
