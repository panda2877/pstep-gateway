# pstep-gateway

> 独立模型网关 — Rust 实现
>
> OpenAI 兼容 API，支持 OpenAI 和 Anthropic 两种输出模式，负责 API Key 管理、路由分发、流式转发

---

## 目录

- [快速开始](#快速开始)
- [API 接口清单](#api-接口清单)
- [输出格式](#输出格式)
- [配置示例](#配置示例)
- [核心机制](#核心机制)
- [目录结构](#目录结构)

---

## 快速开始

```bash
# 配置
cp config.yaml.template config.yaml
# 编辑 config.yaml 填入你的 API Key

# 开发
make dev

# 部署
make build
make start
```

默认监听 `http://localhost:3002`，仅接受本地连接（前面需加反向代理）。

---

## API 接口清单

### 1. 聊天补全 `/v1/chat/completions`

**OpenAI 兼容接口**，支持流式和非流式响应。

#### 请求

```http
POST /v1/chat/completions
Authorization: Bearer pstep-gateway-key
Content-Type: application/json

{
  "model": "claude-sonnet",
  "messages": [
    {"role": "user", "content": "你好，请介绍一下你自己"}
  ],
  "stream": true,
  "max_tokens": 1024
}
```

#### 流式响应示例

```bash
curl -X POST http://localhost:3002/v1/chat/completions \
  -H "Authorization: Bearer pstep-gateway-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet",
    "messages": [{"role": "user", "content": "Hello"}],
    "stream": true
  }'
```

返回 SSE 格式：

```
data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","created":1234567890,"model":"claude-3-5-sonnet-20241022","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","created":1234567890,"model":"claude-3-5-sonnet-20241022","choices":[{"index":0,"delta":{"content":"! I am"},"finish_reason":null}]}

data: [DONE]
```

#### 非流式响应示例

```bash
curl -X POST http://localhost:3002/v1/chat/completions \
  -H "Authorization: Bearer pstep-gateway-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}],
    "stream": false
  }'
```

返回 JSON：

```json
{
  "id": "chatcmpl-xxx",
  "object": "chat.completion",
  "created": 1234567890,
  "model": "gpt-4o",
  "choices": [{
    "index": 0,
    "message": {
      "role": "assistant",
      "content": "Hello! How can I help you today?"
    },
    "finish_reason": "stop"
  }],
  "usage": {
    "prompt_tokens": 10,
    "completion_tokens": 20,
    "total_tokens": 30
  }
}
```

---

### 2. Anthropic 消息接口 `/v1/messages`

**Anthropic 原生格式**接口。

#### 请求

```http
POST /v1/messages
Authorization: Bearer pstep-gateway-key
Content-Type: application/json
anthropic-version: 2023-06-01

{
  "model": "claude-sonnet",
  "messages": [
    {"role": "user", "content": "你好"}
  ],
  "max_tokens": 1024
}
```

#### 流式响应示例

```bash
curl -X POST http://localhost:3002/v1/messages \
  -H "Authorization: Bearer pstep-gateway-key" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-sonnet",
    "messages": [{"role": "user", "content": "Hello"}],
    "stream": true,
    "max_tokens": 1024
  }'
```

返回 Anthropic SSE 格式：

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_xxx","type":"message","role":"assistant","content":[],"model":"claude-3-5-sonnet-20241022","stop_reason":null}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: ping
data: {"type":"ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","usage":{"output_tokens":10}},"usage":{"output_tokens":10}}

event: message_stop
data: {"type":"message_stop"}
```

---

### 3. 模型列表 `/v1/models`

**无需认证**，返回 OpenAI 兼容的模型列表。

#### 请求

```http
GET /v1/models
```

#### 响应

```bash
curl http://localhost:3002/v1/models
```

```json
{
  "object": "list",
  "data": [
    {"id": "gpt-4o", "object": "model", "created": 1234567890, "owned_by": "openai-main"},
    {"id": "claude-sonnet", "object": "model", "created": 1234567890, "owned_by": "anthropic-main"}
  ]
}
```

---

### 4. Agent 模型元数据 `/api/models`

**无需认证**，返回用于 Agent 集成的模型元数据（包含 API Key）。

#### 请求

```http
GET /api/models
```

#### 响应

```json
{
  "models": [
    {
      "id": "gpt-4o",
      "provider": "openai",
      "upstream": "openai-main",
      "api_key_env": "OPENAI_API_KEY"
    },
    {
      "id": "claude-sonnet",
      "provider": "anthropic",
      "upstream": "anthropic-main",
      "api_key_env": "ANTHROPIC_API_KEY"
    }
  ]
}
```

---

### 5. 健康检查 `/health`

**无需认证**。

```bash
curl http://localhost:3002/health
```

返回：

```json
{"status": "ok"}
```

---

### 6. 用量统计 `/stats`

**无需认证**，返回聚合用量数据。

```bash
curl http://localhost:3002/stats
```

响应：

```json
{
  "total_requests": 1000,
  "total_tokens": 5000000,
  "total_prompt_tokens": 3000000,
  "total_completion_tokens": 2000000,
  "by_model": {
    "gpt-4o": {"requests": 600, "tokens": 3000000},
    "claude-sonnet": {"requests": 400, "tokens": 2000000}
  }
}
```

---

### 7. 最近请求记录 `/stats/recent`

**无需认证**，返回最近 50 条请求记录。

```bash
curl http://localhost:3002/stats/recent
```

响应：

```json
{
  "records": [
    {
      "model": "claude-sonnet",
      "upstream": "anthropic-main",
      "prompt_tokens": 100,
      "completion_tokens": 200,
      "total_tokens": 300,
      "success": true,
      "latency_ms": 1500,
      "timestamp": 1234567890000
    }
  ]
}
```

---

### 8. Provider 代理 `/provider/{provider}/{*path}`

**需要认证**，将请求直接转发到指定 provider。

#### Anthropic 转发示例

```bash
curl -X POST http://localhost:3002/provider/anthropic/v1/messages \
  -H "Authorization: Bearer pstep-gateway-key" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"claude-sonnet-20241022","messages":[{"role":"user","content":"hi"}],"max_tokens":100}'
```

#### OpenAI 转发示例

```bash
curl -X POST http://localhost:3002/provider/openai/v1/chat/completions \
  -H "Authorization: Bearer pstep-gateway-key" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}'
```

---

## 输出格式

| 参数 | 说明 |
|------|------|
| `?format=openai` | OpenAI SSE 格式（默认） |
| `?format=anthropic` | Anthropic 事件流原始格式 |

非流式响应直接透传上游 JSON，不做格式转换。

**示例**：使用 Anthropic 格式输出

```bash
curl -X POST "http://localhost:3002/v1/chat/completions?format=anthropic" \
  -H "Authorization: Bearer pstep-gateway-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet",
    "messages": [{"role": "user", "content": "Hello"}],
    "stream": true
  }'
```

---

## 响应头

| 响应头 | 说明 |
|--------|------|
| `X-Pstep-Failover` | 当使用 fallback 模型时返回 `true` |
| `Content-Type` | `text/event-stream`（流式）或 `application/json`（非流式） |
| `Cache-Control` | `no-cache` |

---

## 核心机制

了解更多关于本项目核心机制的工作原理：

- [API 转发机制](./docs/api-forwarding.md) — 了解请求如何路由到上游提供商并转换格式
- [模型 Fallback 机制](./docs/model-fallback.md) — 了解主备故障转移和自动恢复策略

---

## 配置示例

```yaml
port: 3002
public_url: "https://your-gateway.example.com"

upstreams:
  openai-main:
    type: openai
    base_url: "https://api.openai.com/v1"
    api_key: "${OPENAI_API_KEY}"

  anthropic-main:
    type: anthropic
    base_url: "https://api.anthropic.com"
    api_key: "${ANTHROPIC_API_KEY}"

models:
  gpt-4o:
    upstream: openai-main
    model: "gpt-4o"

  claude-sonnet:
    upstream: anthropic-main
    model: "claude-3-5-sonnet-20241022"
    fallback: gpt-4o

usage_tracking:
  enabled: true
  retention_hours: 24
```

---

## 目录结构

```
src/
├── main.rs           # 入口
├── config.rs         # 配置加载
├── types.rs          # 类型定义
├── router.rs         # 路由 + 故障转移
├── thaw.rs           # 模型冻结/解冻跟踪
├── usage.rs          # 用量统计
├── handlers/         # HTTP 处理器
│   ├── mod.rs        # health, stats, api_models
│   └── v1.rs         # /v1/chat/completions, /v1/messages
└── providers/        # 上游代理
    ├── mod.rs        # 代理入口
    ├── openai.rs     # OpenAI 格式处理
    └── anthropic.rs  # Anthropic 格式转换
```