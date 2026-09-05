terraform {
  required_version = ">= 1.0"
}

variable "region" {
  default = "us-east-1"
}

resource "aws_instance" "example" {
  ami           = "ami-123456"
  instance_type = "t3.micro"
}
