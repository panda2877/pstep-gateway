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

# Cache manifest deps: copies only Cargo.toml/Cargo.lock first, builds a dummy
# binary to populate target/, then removes the dummy binary. The next `cargo
# build` (with real src) reuses cached deps.
COPY Cargo.toml Cargo.lock ./
RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    mkdir -p src && \
    echo 'fn main() { println!("placeholder"); }' > src/main.rs && \
    cargo build --release && \
    rm -rf src target/release/deps/pstep_gateway*

# Real build with src/ in place.
COPY src ./src
RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/root/.cargo/git,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release

# =============================================================================
# Stage 2: Runtime
# =============================================================================
# distroless: no shell, no apt, no package manager. Reduces attack surface.
# `:nonroot` tag = runs as UID 65532 by default.
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/target/release/pstep-gateway /usr/local/bin/pstep-gateway

# Embed default config; bind-mounted /etc/pstep-gateway/config.yaml at runtime
# overrides this.
COPY config.yaml /etc/pstep-gateway/config.yaml

ENV CONFIG_PATH=/etc/pstep-gateway/config.yaml \
    RUST_LOG=info

EXPOSE 3002

ENTRYPOINT ["/usr/local/bin/pstep-gateway"]