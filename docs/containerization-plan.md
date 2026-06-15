# 容器化迁移方案 — pstep-gateway (+ 未来 agent 平台)

> 目标：把当前 systemd 直跑二进制的工作流，迁移到 Podman + Quadlet；同时重整 CI，解决若干现存隐患。
>
> 起草日期：2026-06-15 · 审阅状态：**已审阅，待实施**

---

## 决策摘要

| 项 | 方案 | 理由 |
|---|---|---|
| 容器运行时 | **Podman + Quadlet** | daemonless，省 ~80 MB RAM；systemd 集成最深 |
| 网络模式 | **host** | gateway 监听 127.0.0.1:3002；省 veth；nginx 配置零改动 |
| 基础镜像 | **distroless/cc-debian12:nonroot** | ~30 MB，无 shell，更安全 |
| 镜像分发 | **GH Action 构建 + 直传服务器**（不经 registry） | 单服务器场景下 registry 是冗余中间环节 |
| 配置管理 | **bind mount 主机目录** | config.yaml 走 `/etc/pstep-gateway/`，env vars 注入 secrets |
| 运行模式 | **rootful**（systemd Quadlet 跑） | 避免 subuid 复杂度，单人单机的合理选择 |
| 主机 nginx | **保留，配置不进仓库** | 解耦部署与运维，CI 不再覆盖运维手调 |
| `pstep-admin.service` | **彻底删除**（僵尸 unit） | 当前 inactive+disabled，端口 3003 由 nginx 提供 |
| 自动更新 | **手动（CI 触发）** | 不引入 Watchtower 类自动重启 |
| 前端 admin | **保留主机 nginx 静态服务** | 不引入新容器 |

---

## 一、现状盘点（现场核查，2026-06-15）

| 项 | 实际情况 | 影响 |
|---|---|---|
| 主机 nginx | 运行中，提供 :80 / :2877 / :3003 | 不动 |
| pstep-gateway.service | active，监听 0.0.0.0:3002 | 容器化替代 |
| pstep-admin.service | **inactive + disabled**，僵尸 unit | 清理删除 |
| pstep-admin 端口 3003 | **由 nginx 提供**（root /opt/pstep/admin/dist） | 不依赖 systemd |
| 备份目录 /opt/pstep/backups/ | 有 1 个 gateway 备份（5/24），未清理 | 容器化后不再需要 |
| frps | 运行中，监听多个端口 | 不动 |

**关键发现**：deploy.yml 第 91 行的 `sudo systemctl stop pstep-admin || true` 就是僵尸 unit 的活证据——它从来没启动过，stop 当然失败，CI 脚本用 `|| true` 吞掉错误。

---

## 二、服务器前置准备（一次性）

```bash
# 1. 安装 podman
apt update && apt install -y podman

# 2. 准备配置目录
sudo mkdir -p /etc/pstep-gateway
sudo cp /opt/pstep/gateway/config.yaml /etc/pstep-gateway/config.yaml
sudo chown -R root:root /etc/pstep-gateway

# 3. Quadlet 目录
sudo mkdir -p /etc/containers/systemd/pstep

# 4. 清理僵尸 unit（迁移开始时执行）
sudo systemctl stop pstep-admin.service    # 失败也无所谓，它本来就 inactive
sudo systemctl disable pstep-admin.service
sudo rm /etc/systemd/system/pstep-admin.service
sudo systemctl daemon-reload
```

---

## 三、项目结构变更

```
pstep-gateway/
├── Containerfile              # 新增：多阶段构建 gateway 镜像
├── .containerignore           # 新增：排除 target, .git, frontend/dist 等
├── quadlet/                   # 新增：Quadlet 单元模板
│   └── pstep-gateway.container
├── .github/workflows/
│   ├── deploy.yml             # 改造：拆 job、加缓存、直传镜像、健康检查加重试
│   └── nginx.conf             # 删除（不再由 CI 管理）
├── docs/
│   └── containerization-plan.md   # 本文档
└── ... 其余不变
```

---

## 四、Containerfile（多阶段构建）

```dockerfile
# ===== Build stage =====
FROM docker.io/library/rust:1-slim AS builder

WORKDIR /build

# 利用 Docker 层缓存：先只拷贝 manifest，构建空骨架，再覆盖真实源码
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && \
    echo 'fn main() { println!("placeholder"); }' > src/main.rs && \
    cargo build --release && \
    rm -rf src target/release/deps/pstep_gateway*

COPY src ./src
RUN cargo build --release

# ===== Runtime stage =====
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/target/release/pstep-gateway /usr/local/bin/pstep-gateway
COPY config.yaml /etc/pstep-gateway/config.yaml

ENV CONFIG_PATH=/etc/pstep-gateway/config.yaml \
    RUST_LOG=info

EXPOSE 3002

# distroless:nonroot 镜像内 UID 65532
ENTRYPOINT ["/usr/local/bin/pstep-gateway"]
```

**特性**：
- 多阶段：构建层 ~1.5 GB → 运行时镜像 **~30 MB**
- `nonroot` 标签：自动以非 root UID 运行
- 不写 `HEALTHCHECK`：distroless 无 shell，由 Quadlet `[Service] Restart=always` + CI 端重试 curl 替代
- config.yaml 嵌入镜像作为默认值；运行时由 bind mount 覆盖

---

## 五、Quadlet 单元

### `quadlet/pstep-gateway.container`

```ini
[Unit]
Description=Pstep Gateway (Rust API gateway)
After=network-online.target
Wants=network-online.target

[Container]
Image=pstep-gateway:latest
ContainerName=pstep-gateway
Network=host

ReadOnly=true
NoNewPrivileges=true
DropCapability=ALL
SecurityLabelDisable=true

Volume=/etc/pstep-gateway:/etc/pstep-gateway:ro

Environment=RUST_LOG=info
Environment=CONFIG_PATH=/etc/pstep-gateway/config.yaml

[Service]
Restart=always
RestartSec=5
TimeoutStartSec=300
TimeoutStopSec=30

[Install]
WantedBy=multi-user.target default.target
```

部署到服务器：
```bash
sed -e "s|%GH_USER%|$GH_USER|g" quadlet/pstep-gateway.container \
  | sudo tee /etc/containers/systemd/pstep/pstep-gateway.container > /dev/null

sudo systemctl daemon-reload
```

部署后**自动生成 systemd 单元名 = `pstep-gateway.service`**。

---

## 六、配置文件管理

**当前**：`/opt/pstep/gateway/config.yaml`（与二进制同目录）  
**目标**：`/etc/pstep-gateway/config.yaml`（标准 Linux 路径）

```
/etc/pstep-gateway/
└── config.yaml          # bind mount 进容器 /etc/pstep-gateway/config.yaml
```

容器内 `CONFIG_PATH` env var 已指向该路径，与你现有的 `${ENV_VAR}` 解析逻辑兼容。

---

## 七、CI/CD 改造（.github/workflows/deploy.yml）

### 7.1 设计原则

1. **构建 job 拆分为并行**：后端构建、前端构建互不依赖，并行执行
2. **镜像直传**：不经过任何 registry，GH Action 构建后 scp 到服务器
3. **缓存**：Cargo 用 `sccache` + GHA cache 后端；npm 已用 GHA cache
4. **健康检查加重试**：替代原来脆弱的 `sleep 5 + curl`
5. **镜像保留策略**：服务器端保留最近 5 个版本，超出自动清理 → 实现原子回滚
6. **nginx.conf 不再进仓库**：服务端独立维护，CI 不覆盖

### 7.2 触发条件（保持现状）

```yaml
on:
  push:
    branches: [main]
    paths:
      - 'src/**'
      - 'Cargo.toml'
      - 'Cargo.lock'
      - 'Containerfile'
      - 'quadlet/**'
      - '.github/workflows/deploy.yml'
  workflow_dispatch:
```

注：`frontend/**` 触发已合并到 `push`（前端和后端仍可分开构建）。  
`config.yaml.template` 改动不触发（合理——它只是模板）。

### 7.3 完整 deploy.yml

```yaml
name: 🚀 Deploy Gateway

on:
  workflow_dispatch:
  push:
    branches: [main]
    paths:
      - 'src/**'
      - 'frontend/**'
      - 'Cargo.toml'
      - 'Cargo.lock'
      - 'Makefile'
      - 'Containerfile'
      - 'quadlet/**'
      - '.github/workflows/deploy.yml'

jobs:
  # ===== 1. 后端构建 + 镜像导出 =====
  build-gateway:
    name: 🦀 Build Gateway Image
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v6

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Cache Cargo registry & target
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: cargo-${{ runner.os }}-${{ hashFiles('Cargo.lock') }}
          restore-keys: |
            cargo-${{ runner.os }}-

      - name: Build & export image
        uses: docker/build-push-action@v6
        with:
          context: .
          file: ./Containerfile
          push: false
          tags: pstep-gateway:${{ github.sha }}
          outputs: type=local,dest=/tmp/gateway-image
          cache-from: type=gha
          cache-to: type=gha,mode=max
          load: true   # 让 buildx 加载到本地 docker（其实只是 tar 导出）

      - name: Compress image tar
        run: |
          cd /tmp/gateway-image
          tar -cf pstep-gateway.tar pstep-gateway*.tar
          gzip -1 pstep-gateway.tar
          ls -lh pstep-gateway.tar.gz

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: gateway-image
          path: /tmp/gateway-image/pstep-gateway.tar.gz
          retention-days: 3

  # ===== 2. 前端构建（独立，并行） =====
  build-admin:
    name: 🎨 Build Admin Frontend
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v6

      - uses: actions/setup-node@v4
        with:
          node-version: '20'
          cache: 'npm'
          cache-dependency-path: 'frontend/package-lock.json'

      - working-directory: ./frontend
        run: npm ci

      - working-directory: ./frontend
        run: npm run build

      - uses: actions/upload-artifact@v4
        with:
          name: admin-dist
          path: frontend/dist
          retention-days: 3

  # ===== 3. 部署到服务器 =====
  deploy:
    name: 🚀 Deploy to Server
    needs: [build-gateway, build-admin]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6

      - name: Download artifacts
        uses: actions/download-artifact@v4
        with:
          path: ./artifacts

      - name: Deploy via SSH
        env:
          SSH_KEY_B64: ${{ secrets.SERVER_SSH_KEY }}
          SERVER_HOST: ${{ vars.SERVER_HOST }}
          SERVER_USER: ${{ vars.SERVER_USER }}
          GH_USER: ${{ vars.GH_USER }}
        run: |
          set -euo pipefail
          mkdir -p ~/.ssh
          echo "$SSH_KEY_B64" | base64 -d > ~/.ssh/deploy_key
          chmod 600 ~/.ssh/deploy_key
          ssh-keyscan -H $SERVER_HOST >> ~/.ssh/known_hosts 2>/dev/null
          SSHD="ssh -o StrictHostKeyChecking=accept-new -i ~/.ssh/deploy_key ${SERVER_USER}@${SERVER_HOST}"
          SCP="scp -i ~/.ssh/deploy_key"

          # ---- 部署前端（主机 nginx 静态服务）----
          echo "📦 部署前端..."
          rsync -az --delete \
            -e "$SCP" \
            ./artifacts/admin-dist/ \
            ${SERVER_USER}@${SERVER_HOST}:/tmp/admin-dist/

          $SSHD '
            set -e
            sudo mkdir -p /opt/pstep/admin
            sudo rsync -a --delete /tmp/admin-dist/ /opt/pstep/admin/dist/
            rm -rf /tmp/admin-dist
            # 不 reload nginx——纯静态文件，nginx 自动 pick up
          '

          # ---- 部署 Quadlet 文件 ----
          echo "📜 推送 Quadlet..."
          sed -e "s|%GH_USER%|$GH_USER|g" quadlet/pstep-gateway.container \
            | $SSHD "sudo tee /etc/containers/systemd/pstep/pstep-gateway.container > /dev/null"

          # ---- 直传镜像 ----
          echo "🐳 直传镜像..."
          $SCP ./artifacts/gateway-image/pstep-gateway.tar.gz \
            ${SERVER_USER}@${SERVER_HOST}:/tmp/pstep-gateway.tar.gz

          # ---- 服务器端：load + restart + 健康检查 ----
          $SSHD '
            set -e

            # 1. load 新镜像（tag 为 sha）
            gunzip -f /tmp/pstep-gateway.tar.gz
            sudo podman load -i /tmp/pstep-gateway.tar
            sudo podman tag pstep-gateway:${{ github.sha }} pstep-gateway:latest
            rm -f /tmp/pstep-gateway.tar

            # 2. 让 systemd 感知 Quadlet 变更
            sudo systemctl daemon-reload

            # 3. 重启 gateway（容器化）
            sudo systemctl restart pstep-gateway.service

            # 4. 健康检查：重试 6 次，每次间隔 5s，最多等 30s
            echo "🏥 健康检查..."
            for i in 1 2 3 4 5 6; do
              if curl -sf http://127.0.0.1:3002/health > /dev/null; then
                echo "✅ Gateway healthy (attempt $i)"
                break
              fi
              echo "⏳ attempt $i failed, retrying in 5s..."
              sleep 5
            done | tee /tmp/health.log

            if ! curl -sf http://127.0.0.1:3002/health > /dev/null; then
              echo "❌ Health check failed after 30s"
              echo "---- journalctl tail ----"
              sudo journalctl -u pstep-gateway.service --no-pager -n 50
              exit 1
            fi

            # 5. 验证前端可访问
            if ! curl -sf http://127.0.0.1:3003/ > /dev/null; then
              echo "❌ Frontend check failed"
              exit 1
            fi
            echo "✅ Frontend reachable"

            # 6. 镜像保留策略：保留最近 5 个版本
            echo "🧹 清理旧镜像..."
            KEEP=5
            cd /tmp
            # 旧版本镜像已 tag 为 <sha>，列出按时间排序，删除最旧的
            sudo podman images --format "{{.ID}} {{.CreatedAt}} {{.Repository}}:{{.Tag}}" \
              | grep "pstep-gateway" \
              | grep -v ":latest" \
              | sort -k2 -r \
              | tail -n +$((KEEP+1)) \
              | awk "{print \$1}" \
              | xargs -r sudo podman rmi -f || true

            echo "✅ 部署完成"
          '

      - name: Cleanup
        if: always()
        run: rm -rf ./artifacts
```

### 7.4 关键设计取舍

| 设计点 | 取舍 | 理由 |
|---|---|---|
| 拆 build 任务并行 | + 30-60s 节省 | 后端编译和前端 npm 互不依赖 |
| cargo 缓存 | - 60-90% 编译时间 | 命中时 cargo build 只编自家代码 |
| buildx GHA cache | 镜像层复用 | deps 层几乎不变，秒级复用 |
| 不重 reload nginx | 简化 | 静态文件 nginx 自动 pick up；reload 仅在 nginx.conf 变更时需要 |
| 健康检查 6×5s 重试 | 替代脆弱 sleep 5 | 给 Rust 启动 + 健康检查 endpoint 充分时间 |
| 保留 5 个镜像版本 | 实现原子回滚 | `podman load <old.tar>` 秒级回退 |
| 不引入 healthcheck 容器 | distroless 无 shell | 主机 systemd + CI curl 已够 |

---

## 八、迁移步骤（手动一次性）

```bash
# === 在服务器上（推荐先在测试机器演练）===

# 1. 备份现状
sudo cp -r /opt/pstep /opt/pstep.backup.$(date +%Y%m%d)

# 2. 安装 podman
sudo apt install -y podman

# 3. 准备配置目录
sudo mkdir -p /etc/pstep-gateway
sudo cp /opt/pstep/gateway/config.yaml /etc/pstep-gateway/

# 4. 清理僵尸 unit
sudo systemctl stop pstep-admin.service    # 即使失败也无所谓
sudo systemctl disable pstep-admin.service
sudo rm /etc/systemd/system/pstep-admin.service
sudo systemctl daemon-reload

# 5. 停掉旧后端（保留二进制作为最后回退）
sudo systemctl stop pstep-gateway.service
sudo systemctl disable pstep-gateway.service

# 6. 部署 Quadlet 文件
sudo mkdir -p /etc/containers/systemd/pstep
# 从仓库 quadlet/pstep-gateway.container 拷贝到此目录
sudo sed -i "s|%GH_USER%|yourname|g" /etc/containers/systemd/pstep/pstep-gateway.container
sudo systemctl daemon-reload

# 7. 触发首次 CI 部署（或手动跑一遍流程）
# 镜像 + frontend dist 会被推过来，systemd 重启容器

# 8. 验证
sudo systemctl status pstep-gateway.service
podman ps
curl -sf http://127.0.0.1:3002/health
curl -sf http://127.0.0.1:3003/

# 9. 观察 3-7 天，确认稳定后清理备份
# sudo rm -rf /opt/pstep.backup.YYYYMMDD
```

---

## 九、回滚预案

### 场景 A：容器化 gateway 起不来
```bash
# 服务器端操作
sudo systemctl stop pstep-gateway.service
sudo systemctl disable pstep-gateway.service
sudo rm /etc/containers/systemd/pstep/pstep-gateway.container
sudo systemctl daemon-reload

# 恢复旧 systemd unit
sudo cp /opt/pstep.backup.YYYYMMDD/systemd/pstep-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl start pstep-gateway.service
curl -sf http://127.0.0.1:3002/health
```

### 场景 B：新版本 gateway 有 bug
```bash
# 服务器端操作（无需重装旧二进制！）
sudo podman tag pstep-gateway:<last-known-good-sha> pstep-gateway:latest
sudo systemctl restart pstep-gateway.service
```
镜像层在容器里，回滚 = 改 tag + restart，**秒级**。

### 场景 C：CI 整个流程出问题
- GH Actions 上一次成功的 run 可以 Re-run
- 镜像保留 3 天 artifact，可以从 GH 下载后手动 scp

---

## 十、监控与日志

| 维度 | 命令 |
|---|---|
| 状态 | `systemctl status pstep-gateway.service`（名字与旧一致！） |
| 实时日志 | `journalctl -u pstep-gateway.service -f` |
| 容器列表 | `podman ps -a` |
| 资源占用 | `podman stats` 或 `systemd-cgtop` |
| 镜像列表 | `podman images \| grep pstep` |
| 重启历史 | `journalctl -u pstep-gateway.service --since "1 day ago"` |

**运维心智模型冲击最小化**：`systemctl` 和 `journalctl` 命令完全保留。

---

## 十一、现存问题解决清单

| # | 现状问题 | 新方案对应措施 |
|---|---|---|
| 1 | 无原子回滚（二进制直接覆盖） | 保留 5 个镜像 tag，`podman tag` 即可回退 |
| 2 | 脆弱的 `sleep 5 + curl` | 6×5s 重试，失败时 dump journal |
| 3 | `pstep-admin.service` 僵尸 unit | 显式删除 unit 文件 + 取消 enable |
| 4 | `/opt/pstep/backups/` 无限累积 | 改用镜像 tag 保留，备份目录可彻底废弃 |
| 5 | 前后端耦合在同 job | 拆 `build-gateway` 和 `build-admin` 并行 job |
| 6 | config.yaml 改动不触发 | 不变（合理设计） |
| 7 | nginx.conf 被 CI 覆盖 | **删除**仓库里的 nginx.conf，由服务器运维独立维护 |
| 8 | 无 cargo 缓存 | GHA `actions/cache` + buildx GHA cache 双层缓存 |
| 9 | `gateway-bin` 污染 workspace | 改为 `/tmp/gateway-image/` + artifact，不落 workspace |
| 10 | 单元测试无门禁 | （未在本次范围，可后续加 `cargo test` job） |

---

## 十二、未来 agent 平台（占位，等技术栈确定后补）

预计架构：

```
quadlet/
├── pi-agent-api.container        # 主服务（端口 3004）
├── pi-agent-worker.container     # 后台 worker
└── pi-agent-db.container         # Postgres / Redis（视技术栈）

config/
├── /etc/pi-agent/api.yaml
├── /etc/pi-agent/worker.yaml
└── /etc/pi-agent/db.env
```

CI 扩展：在 `build-gateway` 旁加 `build-pi-agent` job，复用同样的"构建→artifact→deploy"模板。

---

## 十三、决策确认

| # | 项目 | 决策 | 日期 |
|---|---|---|---|
| 1 | 镜像保留版本数 | **5** | 2026-06-15 |
| 2 | 清理 `/opt/pstep/backups/` 历史备份 | **同意**（稳定运行一周后清理） | 2026-06-15 |
| 3 | agent 平台相关 | **本轮不做**，待技术栈确定后另起一份计划 | 2026-06-15 |
| 4 | 清理 `pstep-admin.service` 僵尸 unit | **同意** | 2026-06-15 |
| 5 | 删除仓库里的 `.github/workflows/nginx.conf` | **同意**（服务器 nginx 配置独立维护） | 2026-06-15 |
| 6 | distroless vs alpine | **distroless**（默认） | 2026-06-15 |
| 7 | `cargo test` 是否作为部署门禁 | **暂不加**（后续 PR 中再考虑） | 2026-06-15 |

### 仍待你提供

| # | 项目 | 备注 |
|---|---|---|
| A | **GH 用户名** | 镜像命名空间 `ghcr.io/<这里>/pstep-gateway`；请提供后替换所有 `<GH_USER>` 占位符 |
| B | （未来）agent 平台技术栈 | 等你定 |

---

## 附录 A：环境变量 / Secrets 清单

| 名称 | 用途 | 位置 |
|---|---|---|
| `SERVER_SSH_KEY` | SSH 私钥（base64） | GH repo secrets |
| `SERVER_HOST` | 服务器域名/IP | GH repo variables |
| `SERVER_USER` | SSH 用户 | GH repo variables |
| `GH_USER` | GH 用户名（镜像命名空间） | GH repo variables（**新增**） |
| `GITHUB_TOKEN` | GH Actions 内置 | 自动 |