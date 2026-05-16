# pstep-gateway

> 模型网关 — 零依赖 pi 的独立服务
>
> OpenAI 兼容 API，负责 API Key 管理、路由分发、流式转发

属于 [**Pstep Platform**](https://github.com/panda2877/pstep-engine) 的模型网关组件。

---

## 职责

- 接收 `POST /v1/chat/completions` (OpenAI 兼容)
- 解析 `model` 参数 → 映射到上游 provider
- API Key 服务端管理（前端不暴露密钥）
- 流式 SSE 转发
- 用量统计（可选）

## 零依赖

本仓库**不依赖** pi 或 pstep-engine 的任何包，可被任何 OpenAI 兼容客户端独立使用。

---

## 开发

```bash
npm install
npm run dev
```

默认监听 `http://localhost:3001`。

## 部署

```bash
npm run build
npm start
```

或使用 Docker：

```bash
docker build -t pstep-gateway .
docker run -p 3001:3001 pstep-gateway
```

生产环境建议使用 systemd 管理进程，参见 [pstep-engine 部署文档](https://github.com/panda2877/pstep-engine#%E9%83%A8%E7%BD%B2)。

---

## 与 pstep-engine 的关系

```
pstep-engine (Agent 逻辑引擎)
       │
       │ HTTP /v1/chat/completions
       ▼
pstep-gateway (模型网关)
       │
       ├─ Anthropic
       ├─ OpenAI
       └─ 其他上游
```

pstep-engine 通过 `streamFn` 将 LLM 请求发往 pstep-gateway，gateway 负责路由和密钥注入。