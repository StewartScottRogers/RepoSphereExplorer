# repo-sphere container

A multi-stage build for the application, and a compose file that runs it
against a database.

## Using it

```bash
make build          # just the image
make run            # image plus a database
make lint           # hadolint over the Dockerfile
```

## Notes

- Multi-stage, so the toolchain that compiles the binary is not in the
  image that ships it. A build image and a runtime image have different
  jobs and very different sizes.
- `.dockerignore` excludes `.git`, `target/` and `node_modules/`. A smaller
  context is a faster build and a smaller chance of a key ending up in a
  layer.
- The runtime container is `read_only` with a `tmpfs` for `/tmp` and
  `no-new-privileges`. A container that can write to its own filesystem is
  a container an attacker can modify.
- Health checks on both services, and `depends_on: condition:
  service_healthy`, so the application does not start against a database
  that is not accepting connections yet.

---

**This is a fixture.** It lives in `samples/dockerfile/` so the application has a
Docker project to open, not just a Docker file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
