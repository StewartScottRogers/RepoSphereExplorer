# repo-sphere infrastructure

The infrastructure one environment runs on: instances, logs and the
plumbing between them.

## Using it

```bash
cp terraform.tfvars.example terraform.tfvars   # then edit it
terraform init
terraform plan -out=tfplan
terraform apply tfplan
```

## Notes

- State lives in the S3 backend with a DynamoDB lock table, not on a
  laptop. Local state is state one laptop can destroy and nobody else can
  read.
- `terraform.tfvars` is ignored and only the example is committed. A
  workspace that commits its variables eventually commits a secret.
- The workspace publishes the virtual private cloud identifier, its public
  subnets and its web security group, which is what anything consuming
  this state actually needs.

## Checking

```bash
terraform fmt -check -recursive
terraform validate
tflint
```

---

**This is a fixture.** It lives in `samples/terraform/` so the application has a
Terraform project to open, not just a Terraform file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
