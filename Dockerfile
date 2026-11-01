# syntax=docker/dockerfile:1
FROM rust:latest AS builder

WORKDIR /src
COPY . .
RUN cargo build --release --workspace

FROM ubuntu:24.04

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates \
    xvfb \
    openbox \
    mangohud \
    vulkan-tools \
    pciutils \
    procps \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/helix /usr/local/bin/helix
COPY --from=builder /src/target/release/helix-helper /usr/local/bin/helix-helper

# Pre-created runner/cache roots (populated by image bake or volume mount).
RUN mkdir -p /opt/helix/runners /opt/helix/prefixes /opt/helix/cache
ENV HELIX_RUNNERS_ROOT=/opt/helix/runners

# Helper policy is installed by the host; in-container helper runs as root for tests.
CMD ["helix", "--help"]
