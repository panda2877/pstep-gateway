# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

pstep-gateway is a standalone model gateway written in Rust. It provides OpenAI-compatible API for routing requests to upstream providers (OpenAI, Anthropic), managing API keys server-side, handling failover, and tracking usage.

## Commands

```bash
make dev      # Development mode (cargo run)
make build    # Build release binary
make start    # Run release binary
make clean    # Clean build artifacts
```

## Architecture

```
src/
├── main.rs           # Entry point, axum server setup
├── config.rs         # YAML config loader with ${ENV_VAR} resolution
├── types.rs          # Type definitions for config and API types
├── router.rs         # Routing logic with failover support
├── usage.rs          # In-memory sliding-window usage tracker
├── handlers/
│   ├── mod.rs        # health, stats, api_models endpoints
│   └── v1.rs        # /v1/chat/completions, /v1/models
└── providers/
    ├── mod.rs        # OutputFormat enum (OpenAI/Anthropic)
    ├── openai.rs     # OpenAI-compatible upstream proxy
    └── anthropic.rs  # Anthropic format conversion + proxy
```

**Request flow**:
1. Fastify receives `POST /v1/chat/completions` (Bearer token auth required)
2. Router.route() looks up model in config
3. tryUpstream() calls primary upstream; on failure, tries fallback
4. Provider proxies request, converting format if needed
5. Response returned (format controlled by `?format=openai|anthropic` query param)

**Key behaviors**:
- Only listens on `127.0.0.1:3001` — expects reverse proxy in front
- `GET /api/models` returns model metadata + API key for agent integration (no auth)
- `GET /v1/models` returns OpenAI-compatible model list (no auth)
- `X-Pstep-Failover` response header set if fallback was used
- Config search order: `CONFIG_PATH` env → `./config.yaml` → `/etc/pstep-gateway/config.yaml`

## Output Formats

- `?format=openai` (default) — OpenAI SSE format
- `?format=anthropic` — Anthropic event stream format
- Non-streaming responses pass through upstream JSON directly

## Deployment

```bash
ssh root@134.175.163.213
```

- 服务部署通过 GitHub Actions 完成
- 服务器已配置免密登录（root 用户）

## Configuration

See `config.yaml.template`. Upstreams require `api_key`. Models reference upstreams and optionally a fallback model for failover.