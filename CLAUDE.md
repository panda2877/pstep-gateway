# CLAUDE.md

本文件为 Claude Code（claude.ai/code）提供本仓库的协作指引。
内容为模型可读的中文描述，命令、路径、字段名保留英文原样。

---

## 项目概览

`pstep-gateway` 是一个用 Rust 编写的 LLM API 网关，提供 **OpenAI 兼容**的入站接口，
统一代理到上游厂商（OpenAI、Anthropic、自建 OpenAI 兼容服务等），并承担：

- 路由与 **failover**（命名 fallback 链，可按 model 或 client_api_key 粒度生效）
- **服务端 API Key 管理**（持久化、配额、按 model 授权、动态增删）
- **用量统计**（内存滑动窗口 + SQLite 持久化双层；后台有管理 UI）
- 输出格式转换（OpenAI SSE ↔ Anthropic event stream）
- **自动冻结/解冻**（thaw）：上游连续失败时自动暂停，恢复成功率达标后自动解冻
- **管理后台**（React 前端）：可视化查看用量、配置 model、key、fallback 策略

后端监听 `0.0.0.0:3002`；前端 dist 由主机 nginx 静态托管在 `:3003`，由反向代理统一对外。

---

## 常用命令

```bash
make dev      # 开发模式：cargo run（自动加载 .cargo/env）
make build    # release 编译 → target/release/pstep-gateway
make start    # 跑 release 二进制
make clean    # cargo clean
```

```bash
cargo test                    # 跑单元测试（src/usage_db.rs 等有自带测试）
cargo clippy --all-targets    # lint
```

连接到服务器（**已配免密 SSH**）：

```bash
ssh root@134.175.163.213      # 登录；容器为 pstep-gateway.service（systemd）
```

> 部署由 GitHub Actions 自动完成，**不需要手动 scp**。详见「部署」一节。

---

## 仓库结构

```
.
├── src/                    # Rust 后端
├── frontend/               # React + TS 管理后台（Vite，输出到 frontend/dist）
├── Containerfile           # 多阶段构建：rust:1-slim → distroless/cc-debian12:nonroot
├── quadlet/                # Podman Quadlet 单元（pstep-gateway.container）
├── .github/workflows/      # deploy.yml：CI 编译+推送镜像+远程 systemctl restart
├── config.yaml.template    # 配置模板（v0.2 结构）
└── Makefile
```

---

## 架构

```
src/
├── main.rs           # 入口：AppState、axum 路由注册、SQLite 启动加载
├── config.rs         # YAML 配置加载 + ${ENV_VAR} 解析
├── types.rs          # 配置与 API 类型定义（最大文件，集中维护）
├── router.rs         # 路由 + failover 决策
├── usage.rs          # 内存滑动窗口使用量追踪器（启动时从 DB 回填）
├── usage_db.rs       # SQLite 持久化（rusqlite, bundled, WAL 模式）
├── thaw.rs           # 上游自动冻结/解冻追踪器
├── handlers/
│   ├── mod.rs        # /health, /api/health, /stats, /stats/recent, /api/models
│   └── v1.rs         # /v1/chat/completions, /v1/models, /provider/* 代理入口
├── providers/
│   ├── mod.rs        # OutputFormat 枚举 + 流式/非流式分派
│   ├── openai.rs     # OpenAI 兼容上游代理 + 格式转换
│   └── anthropic.rs  # Anthropic 格式转换 + 代理
└── admin/
    ├── mod.rs
    ├── usage.rs      # /api/admin/usage/{stats,distribution}
    ├── models.rs     # /api/admin/models[/{id}], fallback-policies-mini
    ├── apikeys.rs    # /api/admin/keys[/{id}]
    └── fallback.rs   # /api/admin/fallback/policies[/{id}]
```

**请求流（chat completion）**：

1. axum 接收 `POST /v1/chat/completions`，校验 Bearer token
2. `Router::route()` 按 model id 查找配置，确定上游 + 可选 fallback 链
3. `try_upstream()` 调用主上游；失败则按链试下一个
4. `providers::*` 转换请求/响应格式（OpenAI ↔ Anthropic）
5. 写一条 `usage_record`（内存 + SQLite 双重写入）
6. 流式返回 SSE；非流式直接透传 JSON

---

## 路由清单

### 公开（无需鉴权）

| 路径 | 用途 |
|---|---|
| `GET /health` | 健康检查 + 模型列表 + uptime |
| `GET /api/health` | 各 model 的 thaw 状态 |
| `GET /api/models` | 模型元数据 + **任一** client api key（供 agent 集成一键拿到 key） |
| `GET /v1/models` | OpenAI 兼容模型列表 |
| `GET /stats` | 累计用量统计（JSON 聚合） |
| `GET /stats/recent` | 最近 50 条原始记录 |

### 鉴权（需 `Authorization: Bearer <client_api_key>`）

| 路径 | 用途 |
|---|---|
| `POST /v1/chat/completions` | 主入口，OpenAI 兼容 chat completion |
| `POST /provider/{model_id}/*` | 备用入口，路径里直接带 model id |

### 管理后台（前端用，目前**未加鉴权**，靠内网+反代保护）

| 路径 | 用途 |
|---|---|
| `GET /api/admin/usage/stats?period=1d\|7d\|30d` | 用量聚合（token 数、成本、变化率） |
| `GET /api/admin/usage/distribution?period=...` | 各 model token 占比 |
| `GET /api/admin/models[/{id}]` | 模型配置 CRUD |
| `GET /api/admin/models/fallback-policies` | 简版策略列表（前端 model 编辑用） |
| `GET/POST/PUT/DELETE /api/admin/keys[/{id}]` | client api key CRUD |
| `GET/POST/PUT/DELETE /api/admin/fallback/policies[/{id}]` | 命名 fallback 链 CRUD |

> ⚠️ 未来要把 `/api/admin/*` 收进鉴权范围。

---

## 输出格式

请求 URL 上的 `?format=` 控制**响应**的流式事件格式：

- `?format=openai`（默认）→ OpenAI SSE（`data: {...}\n\n` + `data: [DONE]`）
- `?format=anthropic` → Anthropic event stream（`event: ...` + `data: {...}\n\n`）

非流式响应（`stream: false`）直接透传上游 JSON，不做格式转换。

---

## 配置（`config.yaml`）

完整结构见 `config.yaml.template`。v0.2 顶层六大块：

| 字段 | 说明 | 修改生效 |
|---|---|---|
| `port` / `public_url` | 监听端口与对外 URL | 重启 |
| `usage_db` | SQLite 文件路径，留空 = 纯内存（重启即丢） | 重启 |
| `fallback_policies` | **命名的** fallback 链（`chain: [{upstream, model}, ...]`） | 热重载 |
| `models` | 对外暴露的 model id；4 字段扁平（`type`/`base_url`/`api_key`/`model`），可选 `fallback_policy` 引用 + `metadata.{name,status,price_per_input,price_per_output}` | `type/base_url/api_key/model` 改需重启；`name/status/price/fallback_policy` 改立即生效 |
| `client_api_keys` | 客户端访问本网关用的 key；`model_permissions` 空数组=全部；可单独指定 `fallback_policy` 覆盖模型默认；`quota_limit` 0=不限 | 热重载 |
| `usage_tracking.{enabled, retention_hours}` | 统计开关与内存保留时长 | 重启 |
| `thaw.{freeze_duration_minutes, recovery_threshold, min_requests_to_freeze, recovering_attempts}` | 自动冻结/解冻参数 | 重启 |

**变量插值**：`api_key: "${ANTHROPIC_API_KEY}"` 在加载时从环境变量解析。

**配置搜索顺序**：`CONFIG_PATH` 环境变量 → `./config.yaml` → `/etc/pstep-gateway/config.yaml`。

**模型状态**：`active` | `rate_limited` | `disabled`（`disabled` 直接拒绝；`rate_limited` 不再触发新请求）。

---

## 用量统计（双层）

**内存层**（`src/usage.rs`）：`UsageTracker` 维护一个 `VecDeque<UsageRecord>`，容量 `RECENT_CAP = 10_000`；
每次 `record()` 同时更新聚合 `UsageStats`。`get_recent()` 调用前会先 `cleanup()`，
把超过 `retention_hours` 的记录剔出。

**持久化层**（`src/usage_db.rs`）：`UsageDb` 包装 rusqlite，**bundled** 特性（无系统 sqlite 依赖）；
WAL 模式。两张表：

```sql
usage_records(id, ts_ms, model, upstream,
              prompt_tokens, completion_tokens, total_tokens,
              success, latency_ms)
quota_usage(key_id PRIMARY KEY, tokens, updated_at_ms)
```

启动时若 `usage_db` 已配置，会从 DB 回填最近 `retention_hours` 的记录到内存，
**重启不丢用量**（但超过 `retention_hours` 的仍会丢，需要备份 DB 才能永久保留）。

文件：`/var/lib/pstep-gateway/usage.db` + `-wal` + `-shm`，权限 0600（distroless 容器内由程序强制）。
**注意**：`/api/admin/usage/stats` 和 `/distribution` 都从**内存**读，不直接查 DB。
`period` 参数支持 `1d` / `7d` / `30d`；未知值静默回退到 `1d`（最小窗口，最保守）。
**前端 `TimePeriod` 类型只声明 `1d | 7d | 30d`**，但后端容忍其他字符串。

---

## 自动冻结（thaw）

`src/thaw.rs` 跟踪每个 model 的失败/成功滑动窗口。当窗口内失败率超过阈值且请求数达标，
把该 model 状态置为 `rate_limited` 一段时间（`freeze_duration_minutes`），
期间不再发新请求。`recovery_threshold` + `recovering_attempts` 决定解冻条件。

`/api/health` 返回各 model 当前 thaw 状态。`thaw` 段为可选项，缺失则禁用自动冻结。

---

## 前端（`frontend/`）

- React 18 + TypeScript + Vite，依赖管理用 `npm`
- 页面：`OverviewPage`（用量 + 饼图）、`ModelsPage`、`APIKeysPage`、`FallbackPage`
- API 客户端集中在 `frontend/src/services/api.ts`
- 构建产物：`frontend/dist/`，**由主机 nginx 静态托管在 :3003**
- 部署：CI 把 `dist` rsync 到服务器的 `/opt/pstep/admin/dist/`

```bash
cd frontend
npm ci
npm run build      # 产物在 dist/
```

---

## 部署

**完全由 GitHub Actions 自动化**（`.github/workflows/deploy.yml`）：

1. push 到 `main` 触发（或手动 `workflow_dispatch`）
2. `build-admin` job：npm ci + npm run build → 上传 dist artifact
3. `deploy` job：cargo 编译 → `docker build` 镜像（BuildKit + GHA cache，**`load: true`** 加载到 runner）
4. **双路分发**：
   - **`docker save` + scp 直传** → 服务器 `podman load` → `systemctl restart`
     （主路径；**国内拉 registry 慢**，所以走 scp）
   - **`docker push` 到 GHCR**（`ghcr.io/panda2877/pstep-gateway`）
     （**异地备份**；全量保留，不清理）
5. 6 次重试健康检查 → 清理 5 个版本之前的本地旧镜像（GHCR 端不动）

> 主路径用 **scp**（中国服务器拉 GHCR 慢，实测 ~10KB/s），所以服务器侧 `podman load` 而不是 `podman pull`。
> 备份用 **GHCR**（`ghcr.io/panda2877/pstep-gateway`），全量保留历史版本；用于异地容灾 / 跨服务器分发。
> 想要从 GHCR 恢复某版本：`podman pull ghcr.io/panda2877/pstep-gateway:<sha>` 然后 `podman tag ... pstep-gateway:latest && systemctl restart`。

**服务器**：`134.175.163.213`（root，容器 systemd unit `pstep-gateway.service` 由 Quadlet 生成）。

**触发条件**（以下任一文件改动 push 到 main 即触发）：

```
src/**  frontend/**  Cargo.toml  Cargo.lock  Makefile
Containerfile  .containerignore  quadlet/**  .github/workflows/deploy.yml
```

**容器形态**（[quadlet/pstep-gateway.container](quadlet/pstep-gateway.container)）：

- 基础镜像：`gcr.io/distroless/cc-debian12:nonroot`（无 shell，UID 65532）
- `Image=pstep-gateway:latest`（**本地镜像**，由 deploy 脚本 load 后 tag 为 latest；不会自动 pull）
- 加固：`ReadOnly=true` / `NoNewPrivileges=true` / `DropCapability=ALL`
- 挂载：
  - `/etc/pstep-gateway:/etc/pstep-gateway:ro`（config；需 `chmod 644`，因为 distroless 无 /etc/passwd）
  - `/var/lib/pstep-gateway:/var/lib/pstep-gateway:rw`（SQLite；需 `chown 65532:65532`）
- `Network=host`（直接监听 3002，简化反代）
- `Restart=always`，5 秒重试

**手动滚动/回滚**（一般不需要）：

```bash
ssh root@134.175.163.213
sudo podman images | grep pstep-gateway        # 本地最近 5 个版本
sudo podman tag pstep-gateway:<旧sha> pstep-gateway:latest
sudo systemctl restart pstep-gateway.service
```

**从 GHCR 备份恢复**（新服务器 / 本地 5 个版本已全清的情况）：

```bash
# 1. 服务器上登录 GHCR（PAT 需要 read:packages 权限）
sudo podman login ghcr.io -u <github-user>

# 2. 拉指定版本
sudo podman pull ghcr.io/panda2877/pstep-gateway:<sha>

# 3. tag + restart
sudo podman tag ghcr.io/panda2877/pstep-gateway:<sha> pstep-gateway:latest
sudo systemctl restart pstep-gateway.service
```

---

## 开发注意

- **改 `base_url` / `api_key` / `type` / `model`**：需重启服务才生效（这些是上游契约，运行时改会不一致）
- **改 `name` / `status` / `price_*` / `fallback_policy`**：热重载，下一次请求立即生效
- **管理后台 `/api/admin/*` 目前无鉴权**：靠内网 + 反代保护；上线公网前必须加认证
- **WAL 模式的 SQLite**：备份时建议用 `sqlite3 .backup` 或先停服（不要直接 `cp`）
- **distroless 镜像调试**：`podman exec` 进不去（无 shell），看日志用 `journalctl -u pstep-gateway.service`
- **${ENV_VAR} 解析在启动时一次性完成**，运行中改环境变量需要重启
