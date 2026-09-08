# A multi-stage build for a Rust service: cached dependency layer, a
# static-ish release binary, then a small runtime image that drops root.

# syntax=docker/dockerfile:1.7

ARG RUST_VERSION=1.84
ARG DEBIAN_RELEASE=bookworm

FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS planner

WORKDIR /build

RUN cargo install cargo-chef --locked --version 0.1.68

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo chef prepare --recipe-path recipe.json

FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS builder

WORKDIR /build

RUN apt-get update \
    && apt-get install --no-install-recommends -y pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef --locked --version 0.1.68

COPY --from=planner /build/recipe.json recipe.json

# The dependency layer changes only when the manifests do.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef cook --release --recipe-path recipe.json

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --bin service \
    && strip target/release/service

FROM debian:${DEBIAN_RELEASE}-slim AS runtime

LABEL org.opencontainers.image.title="repo-sphere-explorer-service" \
      org.opencontainers.image.source="https://github.com/StewartScottRogers/RepoSphereExplorer" \
      org.opencontainers.image.licenses="MIT"

ENV RUST_LOG=info \
    RSE_DATA_DIR=/var/lib/repo-sphere-explorer \
    TZ=Etc/UTC

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 explorer \
    && useradd --system --uid 10001 --gid explorer --home-dir /home/explorer --create-home explorer \
    && mkdir -p ${RSE_DATA_DIR} \
    && chown explorer:explorer ${RSE_DATA_DIR}

WORKDIR /home/explorer

COPY --from=builder --chown=explorer:explorer /build/target/release/service /usr/local/bin/service
COPY --chown=explorer:explorer samples ./samples

VOLUME ["/var/lib/repo-sphere-explorer"]

EXPOSE 8080 9090

USER explorer

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

STOPSIGNAL SIGTERM

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/service"]
CMD ["--listen", "0.0.0.0:8080", "--metrics", "0.0.0.0:9090"]
