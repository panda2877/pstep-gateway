# syntax=docker/dockerfile:1.7
#
# Pstep Gateway — multi-stage Containerfile
#
# Build stage: rust:1-slim with BuildKit cache mounts for cargo registry/git/target
# Runtime stage: distroless/cc-debian12:nonroot (~30 MB, no shell)
#
# Build (local verification):
#   docker buildx build -f Containerfile -t pstep-gateway:dev --load .

# =============================================================================
# Stage 1: Build
# =============================================================================
FROM docker.io/library/rust:1-slim AS builder

WORKDIR /build
RUN mkdir -p /build/bin

# Copy everything in one go, then build with BuildKit cache mounts for cargo.
# IMPORTANT: we deliberately do NOT cache /build/target. A previous version
# did cache target/, and the combination of (a) COPY preserving host mtimes
# on src/ and (b) cache-mount restoration making target/release/deps look
# newer than the source, made cargo incremental skip recompilation when the
# source had actually changed. The result was a stale binary from a prior
# build being tagged with the new commit SHA — silent regression.
#
# The CI/CD build runs once per commit; there is no local-iteration benefit
# to a target cache, so we just rebuild from scratch each time. Registry/git
# caches (downloaded dep sources) are still worth keeping — they avoid
# re-downloading crates from crates.io on every build.
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Belt-and-suspenders: force every .rs file to a fresh mtime before cargo runs.
# Even with no target cache, this guards against any future scenario where
# cache-mount-restored artifacts could out-date the source. Cost: zero (just
# a touch syscall per file).
RUN find /build/src -name '*.rs' -exec touch {} +

RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    cargo build --release && \
    ls -lh target/release/pstep-gateway && \
    # Copy the binary OUTSIDE the cache mount: BuildKit cache mounts are
    # per-stage, so the binary at target/release/pstep-gateway is discarded
    # between stages. /build/bin is on the regular layer filesystem, so
    # the next stage can COPY --from=builder it.
    cp target/release/pstep-gateway /build/bin/pstep-gateway

# =============================================================================
# Stage 2: Runtime
# =============================================================================
# distroless: no shell, no apt, no package manager. Reduces attack surface.
# `:nonroot` tag = runs as UID 65532 by default.
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/bin/pstep-gateway /usr/local/bin/pstep-gateway

# Embed default config from template (real config.yaml is excluded by
# .containerignore; bind-mounted /etc/pstep-gateway/config.yaml at runtime
# overrides this default).
COPY config.yaml.template /etc/pstep-gateway/config.yaml

ENV CONFIG_PATH=/etc/pstep-gateway/config.yaml \
    RUST_LOG=info

EXPOSE 3002

ENTRYPOINT ["/usr/local/bin/pstep-gateway"]