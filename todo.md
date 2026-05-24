# pstep-gateway 重构计划

## 状态：已完成

## 目标

将项目从 TypeScript 重构为 Rust，实现完全独立的模型网关，支持 OpenAI 和 Anthropic 两种输出模式。

## 输出模式

- 默认 `openai` 格式（主流 agent 框架兼容）
- 显式指定：`?format=openai` 或 `?format=anthropic`
- 非流式响应直接透传上游 JSON

---

## 已完成项

### 第一阶段：项目初始化
- [x] 初始化 Rust 项目
- [x] 添加依赖（axum, serde, reqwest, tokio 等）
- [x] 配置 Cargo.toml 和项目结构
- [x] 编写 Makefile（dev, build, start, clean）

### 第二阶段：配置层
- [x] 定义配置结构体
- [x] 实现 YAML 配置文件加载（含 ${ENV_VAR} 解析）
- [x] 配置验证逻辑

### 第三阶段：核心 HTTP 服务
- [x] 搭建 axum 基础框架
- [x] 实现认证中间件（Bearer Token）
- [x] 实现 `POST /v1/chat/completions`
- [x] 实现 `GET /health`
- [x] 实现 `GET /v1/models`
- [x] 实现 `GET /api/models`（含 API Key）
- [x] 实现 `GET /stats` 和 `GET /stats/recent`
- [x] CORS 配置

### 第四阶段：路由与故障转移
- [x] 实现路由逻辑（模型名 → upstream + model）
- [x] 实现主备故障转移（primary → fallback）

### 第五阶段：Provider 代理
- [x] OpenAI Provider：请求转发、流式响应透传
- [x] Anthropic Provider：格式转换、代理

### 第六阶段：输出模式支持
- [x] 实现 `?format` 查询参数
- [x] 流式响应格式转换
- [x] 非流式响应透传

### 第七阶段：统计功能
- [x] 实现内存滑动窗口 UsageTracker
- [x] `/stats` 和 `/stats/recent` 端点

### 第八阶段：测试与文档
- [x] 更新 README.md
- [x] 更新 CLAUDE.md