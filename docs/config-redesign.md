# Config 重组设计文档

> 状态：v1.1（已确认待编码）· 日期：2026-06-14 · 目标版本：pstep-gateway v0.2

## 0. 决策记录（v1 → v1.1）

| # | 议题 | 决议 |
| --- | --- | --- |
| 1 | 模型状态字段类型 | enum `ModelStatus { Active, RateLimited, Disabled }`，小写字符串持久化 |
| 2 | Fallback 链是否带权重 | **否**，只做有序链 |
| 3 | Upstreams 字段范围 | 只 4 字段：`id / type / base_url / api_key` |
| 4 | 热更新粒度 | **元数据热更新**：名称、状态、单价、fallback 链立即生效；`api_key` 与 `base_url` 改动需重启（影响签名行为，避免运行中密钥被替换导致请求中断） |
| 5 | 网关 API Key ↔ Fallback 链 | **每把 API Key 可独立指定 `fallback_policy`**，覆盖模型默认链（实现「生产 key 走稳链、内部 key 走便宜链」） |
| 6 | Upstreams 资源层级 | **完全合并到 Model**：base_url / api_key 直接写在 model 上，不抽独立 Upstreams 资源（同一密钥被多模型共享时复制填写即可，YAML 结构更扁平） |

> 决策 #6 与 v1 草案差异最大——v1 抽出独立 `upstreams` 顶层 + `/api/admin/upstreams` API。v1.1 取消。

## 1. 背景

当前 `config.yaml.template` 与运行时 `config.yaml` 存在以下问题：

1. **Fallback 链双轨**。`ModelRoute.fallback_chain`（持久化）与 `FallbackPolicyStore`（运行时内存，UUID 管理）并存，前端 Fallback Tab 用的是内存 store，重启即丢，与 config.yaml 不通。
2. **网关 API Key 与 Fallback 链未关联**。`ApiKey.model_permissions` 只能控「哪些模型可用」，无法控「该 Key 走哪条 fallback 链」—— 实际生产中常需要「生产 key 走最稳链、内部 key 走最便宜链」。
3. **字段冗余**。`fallback: '30'`（历史 BUG 残留）、`fallback_chain: []`、`fallback: null`、`metadata.context_window: null` 等同时存在。
4. **状态字段是字符串**。`status: 'active'` 散落在多处字符串字面量里，没有类型约束。

## 2. 目标

- 配置文件由「**模型 + 策略 + 密钥**」三段组成，**与前端三个 Tab 一一对应**：
  - `models` ↔ 「模型配置」Tab
  - `fallback_policies` ↔ 「Fallback 策略」Tab
  - `client_api_keys` ↔ 「API 密钥」Tab（本次把客户端密钥**也持久化**到 config.yaml，与 models 同级）
- 三段均支持**从 config.yaml 加载、从前端读取、PUT 写回**。
- 每把客户端 API Key 可独立指定 `fallback_policy`，覆盖模型默认链。
- `base_url` / `api_key`（**上游签名用**）直接写在 model 上，4 字段扁平结构，不抽 Upstreams 资源。
- 元数据（名称/状态/单价/链）可热更新；`api_key` / `base_url` 改动需重启。
- 保留 env 变量占位符 `${VAR}`。

## 3. 目标配置结构

```yaml
# ===== Server =====
port: 3002
public_url: "http://localhost:3002"

# ===== 1. Fallback 策略 =====
# 一组 fallback 策略 = 一组「模型失败时的自动切换链」
# 节点用 (upstream_name, model_id) 表达。
# 一个策略可被多个 model 或 client_api_key 引用。
fallback_policies:
  high_availability:
    description: "高可用：Claude → Mimo → GPT-4o"
    enabled: true
    chain:
      - upstream: anthropic
        model: "claude-3-5-sonnet-20241022"
      - upstream: mimo
        model: "mimo-v2.5"
      - upstream: openai
        model: "gpt-4o"

  cost_first:
    description: "便宜优先：Mimo → Claude"
    enabled: true
    chain:
      - upstream: mimo
        model: "mimo-v2.5"
      - upstream: anthropic
        model: "claude-3-5-sonnet-20241022"

# ===== 2. Models =====
# 一个 model = 一个对外暴露的 id，绑定 4 字段：
#   type        # openai | anthropic
#   base_url    # 上游 base URL
#   api_key     # 上游签名密钥（env 占位符或明文）
#   model       # 实际发到上游的模型 id
# 可选：
#   fallback_policy       # 该模型默认的 fallback 策略
#   metadata.{name,status,price_per_input,price_per_output}
models:
  claude-sonnet:
    type: anthropic
    base_url: "https://api.anthropic.com/v1"
    api_key: "${ANTHROPIC_API_KEY}"
    model: "claude-3-5-sonnet-20241022"
    fallback_policy: high_availability
    metadata:
      name: "Claude Sonnet"
      status: active               # 枚举小写: active | rate_limited | disabled
      price_per_input: 3.0
      price_per_output: 15.0

  mimo:
    type: anthropic
    base_url: "https://token-plan-cn.xiaomimino.com/anthropic/v1"
    api_key: "${MIMO_API_KEY}"
    model: "mimo-v2.5"
    metadata:
      name: "Mimo"
      status: active
      price_per_input: 0.1
      price_per_output: 0.1

  gpt-4o:
    type: openai
    base_url: "https://api.openai.com/v1"
    api_key: "${OPENAI_API_KEY}"
    model: "gpt-4o"
    fallback_policy: cost_first
    metadata:
      name: "GPT-4o"
      status: active
      price_per_input: 2.5
      price_per_output: 10.0

# ===== 3. 客户端 API 密钥 =====
# 客户端 key = 访问本网关的 key（区别于模型里的 upstream api_key）
# 字段：
#   name                  # 备注名
#   key                   # 明文 key（创建时随机生成，存盘时写明文；列表脱敏展示）
#   model_permissions     # 可用模型 id 列表；空 = 全部
#   fallback_policy       # 可选：该 Key 专用 fallback 链（覆盖模型默认）
#   quota_limit           # 配额上限 token 数，0 = 不限
#   created_at            # 秒时间戳
# 运行期 quota_used 留在内存，不写盘
client_api_keys:
  prod_high_availability:
    name: "生产-高可用"
    key: "sk-gw-prod-xxxxxxxxxxxxxxxxxxxxxxxx"
    model_permissions: []   # 空 = 全部
    fallback_policy: high_availability
    quota_limit: 10000000
    created_at: 1718342400

  internal_cheap:
    name: "内部-便宜"
    key: "sk-gw-internal-yyyyyyyyyyyyyyyyyyyy"
    model_permissions: [claude-sonnet, mimo]
    fallback_policy: cost_first
    quota_limit: 1000000
    created_at: 1718342400

# ===== 4. Server-side runtime state =====
usage_tracking:
  enabled: true
  retention_hours: 24

thaw:
  freeze_duration_minutes: 15
  recovery_threshold: 0.8
  min_requests_to_freeze: 5
  recovering_attempts: 3
```

### 关键变化点（vs 旧 config）

| 旧字段 | 新字段 | 原因 |
| --- | --- | --- |
| 顶层 `upstreams: { id: {type, base_url, api_key} }` | 合并到 `models.<id>.{type, base_url, api_key}` | 决策 #6，扁平化 |
| `models.<id>.fallback_chain: [a, b, c]` | `models.<id>.fallback_policy: <name>` + 顶层 `fallback_policies` | 链可复用 |
| `models.<id>.fallback: '30'` | 删除 | 历史 BUG 字段 |
| `metadata.status` 字符串 | enum `ModelStatus` | 类型安全 |
| `metadata.{reasoning,input,context_window,max_tokens}` | 删 | 本次只保留用得到的字段 |
| `ApiKeyStore` 内存（`/api/admin/keys`） | 顶层 `client_api_keys` 持久化 | 与 config 一致，**支持前端查看/编辑/读取**（决策 #5） |
| `ApiKey.fallback_policy` 不存在 | 新增字段 | 决策 #5 |

## 4. 类型设计（Rust）

```rust
// src/types.rs

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub port: u16,
    #[serde(default)]
    pub public_url: Option<String>,

    pub models: HashMap<String, ModelRoute>,

    #[serde(default)]
    pub fallback_policies: HashMap<String, FallbackPolicyConfig>,

    // 新增：客户端 API Key 持久化
    #[serde(default)]
    pub client_api_keys: HashMap<String, ClientApiKeyConfig>,

    pub usage_tracking: UsageConfig,
    #[serde(default)]
    pub thaw: Option<ThawConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelRoute {
    // 4 字段直接挂 model 上（决策 #6，扁平化）
    #[serde(rename = "type")]
    pub upstream_type: UpstreamType,        // openai | anthropic
    pub base_url: String,
    pub api_key: String,

    pub model: String,                       // 上游实际模型 id

    #[serde(default)]
    pub fallback_policy: Option<String>,     // 引用 fallback_policies key

    #[serde(default)]
    pub metadata: Option<ModelMetadata>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModelMetadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: ModelStatus,                 // ← 改为 enum，小写字符串
    #[serde(default)]
    pub price_per_input: Option<f64>,
    #[serde(default)]
    pub price_per_output: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    #[default]
    Active,
    RateLimited,
    Disabled,
}

impl ModelStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::RateLimited => "rate_limited",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FallbackPolicyConfig {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub chain: Vec<ChainNodeConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChainNodeConfig {
    pub upstream: String,    // 语义标签，仅用于可读性；不再做跨表校验
    pub model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientApiKeyConfig {
    pub name: String,
    pub key: String,                                  // 明文存储；list API 走脱敏
    #[serde(default)]
    pub model_permissions: Vec<String>,               // 空 = 全部
    #[serde(default)]
    pub fallback_policy: Option<String>,              // ← 决策 #5
    pub quota_limit: u64,                             // 0 = 不限
    #[serde(default)]
    pub created_at: u64,                              // 秒时间戳
}

fn default_true() -> bool { true }
```

> 注：删除现有 `UpstreamConfig`（决策 #6）、`ApiKeyStore`（决策 #5）。
> 运行期 `quota_used` 留在 `Arc<RwLock<HashMap<String, u64>>>`，**不**写盘。

## 5. API 设计

### 5.1 Models API

```
GET    /api/admin/models                  # 列表
GET    /api/admin/models/:id              # 详情
PUT    /api/admin/models/:id              # 更新
```

`ModelConfig` 响应：

```json
{
  "id": "claude-sonnet",
  "name": "Claude Sonnet",
  "type": "anthropic",
  "base_url": "https://api.anthropic.com/v1",
  "api_key_masked": "sk-***xyz",                // 脱敏
  "api_key_configured": true,
  "model": "claude-3-5-sonnet-20241022",
  "status": "active",
  "price_per_input": 3.0,
  "price_per_output": 15.0,
  "fallback_policy": "high_availability",
  "fallback_chain": [                            // 服务端展开后给的便利字段
    { "upstream": "anthropic", "model": "claude-3-5-sonnet-20241022" },
    { "upstream": "mimo",      "model": "mimo-v2.5" },
    { "upstream": "openai",    "model": "gpt-4o" }
  ]
}
```

`PUT /api/admin/models/:id` 请求体：

```json
{
  "name": "Claude Sonnet",
  "status": "active",
  "type": "anthropic",
  "base_url": "https://api.anthropic.com/v1",
  "api_key": "********",                          // 占位符 = 不修改
  "model": "claude-3-5-sonnet-20241022",
  "fallback_policy": "high_availability",
  "price_per_input": 3.0,
  "price_per_output": 15.0
}
```

可写字段分为「**热更新**」与「**需重启**」两组（决策 #4）：

| 字段 | 是否热更新 |
| --- | --- |
| `name`, `status`, `price_per_input`, `price_per_output` | ✅ |
| `fallback_policy`（模型默认链） | ✅ |
| `model`（上游实际模型 id） | ⚠️ 需重启 |
| `type`, `base_url` | ❌ 需重启 |
| `api_key` | ❌ 需重启（决策 #4） |

**热更新实现**：当 `PUT` 包含热更新字段时立即生效；包含 `api_key`/`base_url`/`type`/`model` 时写盘 + 返回 `{ "success": true, "restart_required": true, "message": "保存成功，api_key/base_url 变更需重启服务" }`。前端据此弹「需要重启」提示。

### 5.2 Fallback 策略 API

```
GET    /api/admin/fallback/policies       # 列表
GET    /api/admin/fallback/policies/:id   # 详情
POST   /api/admin/fallback/policies       # 新建
PUT    /api/admin/fallback/policies/:id   # 更新
DELETE /api/admin/fallback/policies/:id   # 删除（检查 model / client_api_key 引用）
```

请求/响应：

```json
{
  "id": "high_availability",
  "description": "高可用：Claude → Mimo → GPT-4o",
  "enabled": true,
  "chain": [
    { "upstream": "anthropic", "model": "claude-3-5-sonnet-20241022" },
    { "upstream": "mimo",      "model": "mimo-v2.5" },
    { "upstream": "openai",    "model": "gpt-4o" }
  ]
}
```

`upstream` 字段是「语义标签」，仅用于 UI 展示与链节点阅读，**不**做强校验（不同 model 的 `upstream` 可以同名但实际指向不同 base_url，避免跨表引用断链）。

### 5.3 客户端 API Key API

```
GET    /api/admin/keys                       # 列表（key 脱敏）
POST   /api/admin/keys                       # 新建（生成随机 key，返回明文一次）
PUT    /api/admin/keys/:id                   # 更新（name / permissions / fallback_policy / quota）
DELETE /api/admin/keys/:id
```

`ApiKey` 响应：

```json
{
  "id": "prod_high_availability",
  "name": "生产-高可用",
  "key_masked": "sk-gw-prod-***************",
  "key_prefix": "sk-gw-prod",
  "model_permissions": [],
  "fallback_policy": "high_availability",     // ← 决策 #5
  "quota_limit": 10000000,
  "quota_used": 0,
  "quota_percent": 0.0,
  "created_at": 1718342400
}
```

新建响应包含一次性明文 `raw_key`：

```json
{
  "id": "new_id",
  "raw_key": "sk-gw-abc123-xxxxxxxxxxxxxxxx",
  "key": { "id": "new_id", "name": "...", "key_masked": "sk-gw-abc1-*****" }
}
```

`PUT /api/admin/keys/:id` 不接受 `key` 字段——`key` 只能删除重建（避免运行中改 key 导致旧 key 失效应激）。

### 5.4 路由 / 鉴权适配

`require_auth` 现状用 hardcode 的 `VALID_API_KEY`，需改为：从 `client_api_keys` HashMap 查 key，并按 `key` 的 `model_permissions` 和 `fallback_policy` 注入请求上下文。

`src/router.rs::build_chain`：

```rust
fn build_chain(
    model_id: &str,
    config: &GatewayConfig,
    key_fallback_policy: Option<&str>,    // ← 来自 client_api_key，可覆盖模型默认
) -> Vec<(String, String)> {
    let mut chain = Vec::new();
    if let Some(route) = config.models.get(model_id) {
        // 主供应商
        chain.push((route.upstream_type_str(), route.model.clone()));
        // 优先级：key 覆盖 > 模型默认
        let policy_id = key_fallback_policy
            .or(route.fallback_policy.as_deref());
        if let Some(pid) = policy_id {
            if let Some(policy) = config.fallback_policies.get(pid) {
                if policy.enabled {
                    for node in &policy.chain {
                        if !(node.upstream == route.upstream_type_str() && node.model == route.model) {
                            chain.push((node.upstream.clone(), node.model.clone()));
                        }
                    }
                }
            }
        }
    }
    chain
}
```

> 注：`upstream` 在 chain 节点里也仅作语义标签；fallback 时**不**用其查 `api_key`/`base_url`，而是把目标 model 的 4 字段当主供应商签名。

## 6. 前端改造

### 6.1 三个 Tab（保留现有布局）

- `ModelsPage.tsx`：
  - 列表新增列：`Fallback 链`（显示模型引用的策略名）
  - 编辑 modal 字段：
    - 模型名称（可编辑）
    - 状态（下拉，3 选 1）
    - 输入/输出单价
    - **Fallback 链**（下拉，选项来自 `/api/admin/fallback/policies`）
    - **上游 base_url**（可编辑 → 需重启）
    - **上游 api_key**（密码框 + 脱敏占位 → 需重启）
    - **上游 type**（openai / anthropic 下拉 → 需重启）
    - 主供应商 model id
  - 当用户改了 `api_key` / `base_url` / `type` / `model`，保存成功弹「需要重启服务」提示

- `FallbackPage.tsx`：
  - 数据源切换到 `config.fallback_policies`（持久化，字符串 key 替代 UUID）
  - 「新增策略」按钮创建
  - 「编辑」可改 description / enabled / chain 顺序
  - 「删除」检查引用：若被任何 model 或 client_api_key 引用，禁止删除

- `APIKeysPage.tsx`：
  - 列表新增列：`Fallback 链`（展示该 key 的 `fallback_policy`）
  - 「新建 key」生成随机 `sk-gw-…`，弹一次性明文
  - 「编辑 key」可改：name / 模型权限 / **fallback_policy** / quota_limit
  - 「撤销」= DELETE

### 6.2 删除/下线

- 改造 `src/admin/fallback.rs` 读写 `config.fallback_policies`（替换内存 store）
- `src/admin/apikeys.rs` 改写为读写 `config.client_api_keys`（替换内存 store）
- 鉴权：硬编码 `VALID_API_KEY` 改为从 `config.client_api_keys` 查询

## 7. 迁移路径

1. **读侧兼容**（v0.1 → v0.2）：
   - `load_config` 检测旧结构（顶层 `upstreams`）时，自动把每个 upstream 合并到对应 model（按 `model.upstream == upstreams.id` 匹配）。匹配后 `upstreams` 块整体丢弃。
   - 检测旧 `models.<id>.fallback_chain` 时，自动创建 `fallback_policies.<id>_legacy`，把 chain 拷贝过去；model 字段替换为 `fallback_policy: <id>_legacy`。
   - 检测旧 `ApiKeyStore` 内存数据：首次启动时若内存非空，写到 `config.client_api_keys`。
2. **写侧**：严格按新结构写盘，无冗余 null 字段。
3. **操作脚本**：`make config-migrate` 把旧 config 写出新 config 到 `config.yaml.new`。

## 8. 持久化与并发

- 沿用 `Arc<Mutex<GatewayConfig>>`。
- 写盘前剥除运行期字段（`quota_used` 不在 config 中）。
- `save_config` 序列化顺序固定：fallback_policies → models → client_api_keys → usage_tracking → thaw。
- 所有 Option 字段：`#[serde(default, skip_serializing_if = "Option::is_none")]`。

## 9. 安全

- **上游 `api_key`**：API 响应只返回 `api_key_masked`（前 3 + 后 4）。编辑 modal 占位符 `********` = 不修改。
- **客户端 `key`**：API 响应只返回 `key_masked` 与 `key_prefix`；明文只在创建 POST 响应里返回一次。
- `${ENV_VAR}` 加载时解析，写盘时是明文（已解析值），不再二次解析（避免「写盘时如果是占位符就保留占位符」与「写盘时如果是明文就覆盖原 env 值」二义性）。
- 写盘后设置文件权限 `chmod 600 config.yaml`（部署脚本层面保证）。

## 10. 实施拆解

| 步骤 | 内容 | 涉及文件 | 预计行数 |
| --- | --- | --- | --- |
| S1 | `types.rs`：新增 `FallbackPolicyConfig`/`ChainNodeConfig`/`ClientApiKeyConfig`/`ModelStatus` enum；`ModelRoute` 改为 4 字段扁平 | `src/types.rs` | +80 / -25 |
| S2 | `config.rs::load_config` 写迁移逻辑 + 校验 | `src/config.rs` | +60 |
| S3 | `router.rs::build_chain` 接受 `key_fallback_policy` 覆盖参数 | `src/router.rs` | +20 / -10 |
| S4 | `admin/models.rs`：`UpdateModelConfigRequest` 接收 4 字段；区分「热更新」与「需重启」字段，返回 `restart_required` 标记 | `src/admin/models.rs` | +40 / -10 |
| S5 | `admin/fallback.rs` 改为读写 `config.fallback_policies`，删除 `FallbackPolicyStore` | `src/admin/fallback.rs` + `src/main.rs` | +60 / -50 |
| S6 | `admin/apikeys.rs` 改为读写 `config.client_api_keys`，删除 `ApiKeyStore` | `src/admin/apikeys.rs` + `src/main.rs` | +80 / -50 |
| S7 | 鉴权：硬编码 `VALID_API_KEY` 改为从 `config.client_api_keys` 查询，并把查到的 `fallback_policy` 注入请求上下文 | `src/handlers/v1.rs` | +30 / -10 |
| S8 | 前端 `types/index.ts` 同步扩展类型 | `frontend/src/types/index.ts` | +20 / -5 |
| S9 | 前端 `ModelsPage` 编辑 modal 改造：新增 fallback 链下拉、type/base_url/api_key 字段、状态枚举、重启提示 | `frontend/src/pages/ModelsPage.tsx` | +60 / -10 |
| S10 | 前端 `FallbackPage` 数据源切换 + 引用检查提示 | `frontend/src/pages/FallbackPage.tsx` | +20 / -10 |
| S11 | 前端 `APIKeysPage` 新增 fallback_policy 字段、显示 key 链 | `frontend/src/pages/APIKeysPage.tsx` | +40 / -5 |
| S12 | `config.yaml.template` 全部重写 | `config.yaml.template` | 全部 |
| S13 | `scripts/config-migrate.py`（可选，CI/手动） | `scripts/config-migrate.py` | +60 |
| S14 | 部署 + 验证：模型列表 / fallback 编辑 / API key 链 / 热更新 vs 重启提示 | — | — |

## 11. 风险与回滚

- 风险 1：客户端 API Key 落盘后明文 `key` 写文件——如服务器被入侵，攻击者直接拿到所有调用方 key。缓解：文件 `chmod 600`、运维限制；后续可引入 `key_hash` 字段 + 启动时通过 env 注入明文。
- 风险 2：路由层用 `key.fallback_policy` 覆盖模型默认链时，router.rs 当前签名只接受 model_id，需扩展请求上下文传递「已鉴 key 的元数据」。缓解：S7 与 S3 同步提交。
- 风险 3：热更新与「需重启」字段混在同一 PUT 请求里语义模糊。缓解：响应体显式 `restart_required: true/false`，前端按字段显示提示。
- 风险 4：删除 `ApiKeyStore`/`FallbackPolicyStore` 后，前端如果还有代码引用会 500。缓解：S6/S5 + S11/S10 同时提交。
- 回滚策略：保留 v0.1 旧字段为可选（`#[serde(default)]`），旧 release 仍可读新结构（缺字段视为空）。
