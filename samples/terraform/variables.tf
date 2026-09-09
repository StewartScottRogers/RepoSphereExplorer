# Every input this workspace takes. One file, so `terraform plan -var-file`
# has one place to be checked against.

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
