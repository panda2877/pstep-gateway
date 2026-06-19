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

# 装 gdb + 必需的 .so 到 /build/bin-deps/，下一阶段 COPY 进 distroless runtime。
# 目的：deadlock 复发时能用 sidecar 模式 attach pstep-gateway 进程抓栈。
# 用法：
#   podman run --rm -it --pid=container:pstep-gateway-a \
#     --cap-add=SYS_PTRACE --security-opt seccomp=unconfined \
#     --network=none pstep-gateway:latest \
#     /usr/bin/gdb -p 1 /usr/local/bin/pstep-gateway
# 仍保持 cc-debian12:nonroot（无 shell）；gdb 不需要 shell 也能跑。
# ⚠️  gdb 进 distroless runtime 的尝试：当前 builder `rust:1-slim` 已升 trixie
# （glibc 2.40+），runtime `distroless/cc-debian12` 是 bookworm（glibc 2.36）。
# trixie gdb 需要 GLIBC_2.38，跑不起来。fix 见 future-todo。
#
# 当前做法：只把 gdb + gdbserver 二进制装进 builder，**不拷** .so deps，
# 不写进 runtime。等下面 todo 解决了（pin builder 到 bookworm 或 dpkg-deb
# 抽 bookworm gdb）再启 COPY。binary 留 builder 里只为完整保留 build 路径。
RUN apt-get update && \
    apt-get install -y --no-install-recommends gdb gdbserver libreadline8 && \
    rm -rf /var/lib/apt/lists/*

# =============================================================================
# Stage 2: Runtime
# =============================================================================
# distroless: no shell, no apt, no package manager. Reduces attack surface.
# `:nonroot` tag = runs as UID 65532 by default.
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/bin/pstep-gateway /usr/local/bin/pstep-gateway

# gdb + gdbserver + deps 暂不进 runtime（见 builder 阶段注释：GLIBC mismatch）。
# 待 future-todo 解决（pin builder 到 bookworm 或 dpkg-deb 抽 bookworm gdb）后
# 再恢复下面 3 行 COPY。
# COPY --from=builder /build/bin-deps/usr/bin/gdb        /usr/bin/gdb
# COPY --from=builder /build/bin-deps/usr/bin/gdbserver  /usr/bin/gdbserver
# COPY --from=builder /build/bin-deps/usr/lib/x86_64-linux-gnu/ /usr/lib/x86_64-linux-gnu/

# Embed default config from template (real config.yaml is excluded by
# .containerignore; bind-mounted /etc/pstep-gateway/config.yaml at runtime
# overrides this default).
COPY config.yaml.template /etc/pstep-gateway/config.yaml

ENV CONFIG_PATH=/etc/pstep-gateway/config.yaml \
    RUST_LOG=info

EXPOSE 3002

ENTRYPOINT ["/usr/local/bin/pstep-gateway"]