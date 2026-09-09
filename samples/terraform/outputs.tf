# What this workspace publishes to whoever consumes its state.

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
