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
# Previous attempt used a dummy-source-then-replace trick with `cargo clean -p`
# to force a fresh compile, but the interaction between BuildKit's overlay
# cache mount and cargo's incremental metadata produced an essentially-empty
# binary (301 KB, only libc imports). Simpler is more reliable: let cargo
# handle the whole build in one go. Cache mounts give cargo its previous
# target/ between builds so unchanged deps aren't recompiled.
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
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