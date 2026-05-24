# pstep-gateway

> 独立模型网关 — Rust 实现
>
> OpenAI 兼容 API，支持 OpenAI 和 Anthropic 两种输出模式，负责 API Key 管理、路由分发、流式转发

---

## 职责

- 接收 `POST /v1/chat/completions`（OpenAI 兼容）
- 解析 `model` 参数 → 映射到上游 provider
- API Key 服务端管理（前端不暴露密钥）
- 主备故障转移（primary → fallback）
- 流式 SSE 转发
- 用量统计
- 输出格式：`?format=openai` 或 `?format=anthropic`

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

默认监听 `http://localhost:3001`，仅接受本地连接（前面需加反向代理）。

---

## 输出格式

| 参数 | 说明 |
|------|------|
| `?format=openai` | OpenAI SSE 格式（默认） |
| `?format=anthropic` | Anthropic 事件流原始格式 |

非流式响应直接透传上游 JSON，不做格式转换。

---

## API 端点

| 端点 | 方法 | 说明 | 认证 |
|------|------|------|------|
| `/v1/chat/completions` | POST | 聊天补全 | Bearer Token |
| `/v1/models` | GET | 可用模型列表 | 不需要 |
| `/api/models` | GET | Agent 用模型元数据 | 不需要 |
| `/health` | GET | 健康检查 | 不需要 |
| `/stats` | GET | 用量统计聚合 | 不需要 |
| `/stats/recent` | GET | 最近 50 条记录 | 不需要 |

---

## 配置示例

```yaml
port: 3001
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
├── usage.rs          # 用量统计
├── handlers/         # HTTP 处理器
└── providers/        # 上游代理
```