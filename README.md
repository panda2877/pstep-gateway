# pstep-gateway

> 模型网关 — 零依赖 pi 的独立服务
>
> OpenAI 兼容 API，负责 API Key 管理、路由分发、流式转发

## 职责

- 接收 `POST /v1/chat/completions` (OpenAI 兼容)
- 解析 model → 映射到上游 provider
- API Key 服务端管理（前端不暴露密钥）
- 流式 SSE 转发
- 用量统计（可选）

## 零依赖

本仓库**不依赖** pi 的任何包，可被任何客户端独立使用。

## 开发

```bash
npm install
npm run dev
```

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