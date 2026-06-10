# API 转发机制

本文档详细说明 pstep-gateway 如何将客户端请求转发到上游大模型提供商，以及如何处理不同提供商之间的格式差异。

## 概述

pstep-gateway 扮演**反向代理**角色，统一接收客户端请求，根据配置的模型映射将请求转发到对应的上游提供商，并处理响应格式的转换。

```
┌─────────────┐      ┌──────────────────┐      ┌─────────────────┐
│   Client    │ ───▶ │  pstep-gateway   │ ───▶ │ Upstream Provider│
│             │ ◀─── │  (format conv)   │ ◀─── │ (OpenAI/Anthropic)│
└─────────────┘      └──────────────────┘      └─────────────────┘
```

## 核心组件

### 1. 路由层 (Router)

**文件**: [src/router.rs](../src/router.rs)

Router 是整个转发的核心，负责：
- 根据 `model` 参数查找对应的 upstream 配置
- 管理 fallback 链
- 调用 provider 进行实际请求

```rust
pub async fn route(
    &self,
    model_name: &str,    // 客户端请求的模型名
    body: &str,          // 原始请求体
    format: OutputFormat, // 期望的输出格式
) -> Result<String, String>
```

### 2. Provider 层

**文件**: [src/providers/mod.rs](../src/providers/mod.rs), [src/providers/openai.rs](../src/providers/openai.rs), [src/providers/anthropic.rs](../src/providers/anthropic.rs)

Provider 负责：
- 构建上游请求（格式转换）
- 发送 HTTP 请求
- 解析响应（提取用量、转换格式）

### 3. 输出格式枚举

**文件**: [src/providers/mod.rs](../src/providers/mod.rs)

```rust
pub enum OutputFormat {
    OpenAI,    // OpenAI SSE 格式
    Anthropic, // Anthropic 事件流格式
}
```

## 转发流程图

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         API 转发流程                                          │
└─────────────────────────────────────────────────────────────────────────────┘

    ┌─────────┐
    │  Client  │
    └────┬────┘
         │ POST /v1/chat/completions
         │ Authorization: Bearer xxx
         ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  handlers/v1.rs: chat_completions()                                          │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ 1. require_auth() — 验证 Bearer Token                               │   │
│  │ 2. 解析 FormatQuery — 确定输出格式 (openai/anthropic)                │   │
│  │ 3. 调用 router.route(model, body, format)                          │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  router.rs: Router::route()                                                  │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ 1. build_chain() — 构建 fallback 链 [primary, fallback1, ...]        │   │
│  │ 2. 遍历 chain，尝试每个上游                                           │   │
│  │ 3. 调用 providers::proxy()                                           │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  providers/mod.rs: proxy()                                                   │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ 根据 upstream.type 选择 provider:                                     │   │
│  │   - Openai → openai.rs::proxy_stream()                              │   │
│  │   - Anthropic → anthropic.rs::proxy_stream()                        │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
         │
         ├──┬────────────────────────┐
         │  │                        │
         ▼  ▼                        ▼
┌─────────────────┐    ┌─────────────────┐
│   openai.rs     │    │  anthropic.rs   │
│                 │    │                 │
│ 直接转发请求    │    │ 格式转换后转发  │
│ 解析 SSE 响应   │    │ 解析 SSE 响应   │
│ 转换格式输出    │    │ 转换格式输出    │
└────────┬────────┘    └────────┬────────┘
         │                       │
         └───────────┬───────────┘
                     ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  格式转换层                                                                  │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ OpenAI → Anthropic 格式:                                            │   │
│  │   delta.content → text_delta                                        │   │
│  │   finish_reason → stop_reason                                       │   │
│  │   usage → 注入到 message_delta                                       │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Anthropic → OpenAI 格式:                                             │   │
│  │   text_delta → delta.content                                         │   │
│  │   stop_reason → finish_reason                                        │   │
│  │   注入 usage 到最终 chunk                                             │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
                     │
                     ▼
              ┌─────────────┐
              │   Client    │
              │ (SSE Stream)│
              └─────────────┘
```

## 请求格式转换

### OpenAI → Anthropic

当客户端使用 OpenAI 格式请求，但 upstream 是 Anthropic 时：

```
OpenAI 请求                          Anthropic 请求
───────────                          ──────────────
{model, messages, ...}    →    {model, messages, ...}
                                    + anthropic-version header
                                    + x-api-key header
```

**转换规则**:
| OpenAI 字段 | Anthropic 处理 |
|-------------|----------------|
| `messages` | 直接使用，role 映射 user/assistant |
| `system` | 转换为 `system` 字段 |
| `tools` | 转换为 `tools` 数组 |
| `stream` | 直接传递 |
| `max_tokens` | 直接传递 |

### Anthropic → OpenAI

当客户端使用 Anthropic 格式请求，但 upstream 是 OpenAI 时：

```
Anthropic 请求                      OpenAI 请求
─────────────                       ──────────────
{model, messages, ...}    →    {model, messages, ...}
+ anthropic-version       →    (移除)
+ x-api-key               →    Authorization: Bearer
```

**转换规则**:
| Anthropic 字段 | OpenAI 处理 |
|----------------|-------------|
| `messages` | 直接使用 |
| `system` | 转换为 `messages` 中的 system 消息 |
| `tools` | 转换为 `tools` 数组 |

## 响应格式转换

### SSE 事件类型映射

#### OpenAI SSE 格式

```
event: chat.completion.chunk
data: {"id":"...","choices":[{"delta":{"content":"..."}}]}

event: chat.completion.chunk  
data: {"id":"...","choices":[{"delta":{"content":"..."}}]}

event: chat.completion
data: {"id":"...","choices":[{}],"usage":{...}}
```

#### Anthropic SSE 格式

```
event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"..."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"..."}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","usage":{"output_tokens":10}}}
```

### Token 用量提取

从上游响应中提取 token 用量并注入到输出：

```rust
// 从 SSE 中提取 usage 信息
let usage = extract_usage_from_sse_events(&sse_text);

// 在 OpenAI 格式中，最终 chunk 包含:
{
  "choices": [{
    "finish_reason": "stop",
    "index": 0
  }],
  "usage": {
    "prompt_tokens": 100,
    "completion_tokens": 50,
    "total_tokens": 150
  },
  "model": "claude-3-5-sonnet-20241022"
}
```

## Provider 路由

### 内置 Provider

| Provider | Base URL | 认证方式 |
|----------|----------|----------|
| `openai` | `https://api.openai.com/v1` | Bearer Token |
| `anthropic` | `https://api.anthropic.com/v1` | x-api-key |
| `deepseek` | `https://api.deepseek.com/v1` | Bearer Token |

### 配置驱动 Provider

在 `config.yaml` 中定义自定义 upstream：

```yaml
upstreams:
  my-custom-provider:
    type: openai  # 或 anthropic
    base_url: "https://custom-api.example.com/v1"
    api_key: "${CUSTOM_API_KEY}"
```

## 错误处理

### 上游错误传播

```rust
match result {
    Ok((response, usage)) => {
        // 记录用量
        self.usage_tracker.record(UsageRecord {
            model: model_name.to_string(),
            upstream: current_upstream.clone(),
            success: true,
            // ...
        });
        Ok(response)
    }
    Err(e) => {
        tracing::error!("upstream request failed: {}", e);
        // 尝试 fallback
        continue;
    }
}
```

### 常见错误码

| 错误码 | 含义 | 处理方式 |
|--------|------|----------|
| 401 | 认证失败 | 不重试，返回错误 |
| 429 | 限流 | 等待后重试（通过 fallback） |
| 500 | 上游内部错误 | 尝试 fallback |
| 502 | 网关错误 | 尝试 fallback |
| 503 | 服务不可用 | 尝试 fallback |

## 非流式请求

非流式请求走简化路径：

```rust
pub async fn route_non_stream(
    &self,
    model_name: &str,
    body: &str,
    format: OutputFormat,
) -> Result<String, String> {
    // 直接透传上游 JSON 响应，不做格式转换
    match providers::proxy_non_stream(upstream, &model, body, format) {
        Ok((response, _)) => Ok(response),
        Err(e) => Err(e),
    }
}
```

## 性能考虑

1. **连接复用**: 使用 `reqwest::Client` 复用 HTTP 连接
2. **超时控制**: 请求超时设为 120 秒
3. **流式转发**: 实时转发 SSE，不做缓冲
4. **用量跟踪**: 异步记录，不阻塞响应

## 相关文档

- [模型 Fallback 机制](./model-fallback.md) — 了解故障转移策略
- [配置参考](../config.yaml.template) — 完整的配置项说明