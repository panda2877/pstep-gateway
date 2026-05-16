// ============================================================================
// Pstep Gateway — Fastify 服务入口
// ============================================================================

import Fastify from 'fastify';
import cors from '@fastify/cors';
import { loadConfig } from './config.js';
import { Router } from './router.js';

// API Key 校验中间件
const VALID_API_KEY = 'pstep-gateway-key';

function requireAuth(request: any, reply: any) {
  const auth = request.headers.authorization;
  if (!auth || auth !== `Bearer ${VALID_API_KEY}`) {
    return reply.status(401).send({ error: 'Unauthorized', message: 'Missing or invalid API key' });
  }
}

async function main() {
  console.log('╔══════════════════════════════════════╗');
  console.log('║         Pstep Gateway v0.1.1         ║');
  console.log('╚══════════════════════════════════════╝');

  // 加载配置
  const config = loadConfig();
  const router = new Router(config);

  // 创建 Fastify 实例
  const app = Fastify({ logger: false });
  await app.register(cors, { origin: true });

  // ==================================================================
  // 认证中间件 — 保护 /v1/* 接口，/api/models 除外（用于获取 API Key）
  // ==================================================================
  app.addHook('preHandler', async (request, reply) => {
    // 健康检查、统计、模型列表接口不需要认证
    if (request.url === '/health' || request.url === '/stats' || request.url === '/stats/recent' || request.url === '/v1/models') {
      return;
    }
    requireAuth(request, reply);
  });

  // ==================================================================
  // POST /v1/chat/completions — OpenAI 兼容的聊天补全接口
  // ==================================================================
  app.post('/v1/chat/completions', async (request, reply) => {
    const body = request.body as Record<string, unknown>;
    const modelName = body?.model as string;
    const stream = body?.stream !== false; // 默认流式

    if (!modelName) {
      return reply.status(400).send({ error: '缺少 model 字段' });
    }

    try {
      const bodyStr = JSON.stringify(body);
      const result = await router.route(modelName, bodyStr);

      if (!stream) {
        // 非流式：从 SSE 流中提取 JSON 响应
        let fullResponse = '';
        for await (const chunk of result.stream) {
          fullResponse += typeof chunk === 'string' ? chunk : chunk.toString();
        }
        // 提取 JSON（去掉 SSE 包装）
        const jsonMatch = fullResponse.match(/\{[\s\S]*\}/);
        if (jsonMatch) {
          return reply.type('application/json').send(jsonMatch[0]);
        }
        return reply.status(502).send({ error: '无效的响应格式' });
      }

      // 流式响应
      reply.raw.writeHead(200, {
        'Content-Type': 'text/event-stream',
        'Cache-Control': 'no-cache',
        'Connection': 'keep-alive',
        'X-Accel-Buffering': 'no',
      });

      // 如果发生了故障转移，在响应头中告知
      if (result.didFailover) {
        reply.raw.setHeader('X-Pstep-Failover', 'true');
      }

      const decoder = new TextDecoder();

      try {
        for await (const chunk of result.stream) {
          reply.raw.write(typeof chunk === 'string' ? chunk : decoder.decode(chunk));
        }
      } finally {
        reply.raw.end();
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('❌ 请求失败:', message);

      if (!reply.raw.headersSent) {
        return reply.status(502).send({
          error: 'bad_gateway',
          message,
        });
      }

      // 已经发送了头部，通过 SSE 发送错误
      reply.raw.write(`data: ${JSON.stringify({ error: message })}\n\n`);
      reply.raw.write('data: [DONE]\n\n');
      reply.raw.end();
    }
  });

  // ==================================================================
  // GET /health — 健康检查
  // ==================================================================
  app.get('/health', async () => {
    return {
      status: 'ok',
      version: '0.1.1',
      models: Object.keys(config.models),
      uptime: process.uptime(),
    };
  });

  // ==================================================================
  // GET /v1/models — 列出可用模型
  // ==================================================================
  app.get('/v1/models', async () => {
    const models = Object.entries(config.models).map(([id, route]) => ({
      id,
      object: 'model',
      created: Math.floor(Date.now() / 1000),
      owned_by: (route as any).upstream,
    }));
    return { object: 'list', data: models };
  });

  // ==================================================================
  // GET /api/models — 返回 Agent 可用的完整模型元数据
  // ==================================================================
  app.get('/api/models', async () => {
    const baseUrl = config.public_url || `http://localhost:${config.port}/v1`;
    const models = Object.entries(config.models).map(([id, route]) => {
      const meta = (route as any).metadata || {};
      return {
        id,
        name: meta.name || id,
        api: 'openai-completions',
        provider: 'pstep-gateway',
        baseUrl,
        reasoning: meta.reasoning || false,
        input: meta.input || ['text'],
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
        contextWindow: meta.context_window || 128000,
        maxTokens: meta.max_tokens || 4096,
      };
    });
    return { models, apiKey: VALID_API_KEY };
  });

  // ==================================================================
  // GET /stats — 用量统计
  // ==================================================================
  app.get('/stats', async () => {
    const tracker = router.getUsageTracker();
    return tracker.getStats();
  });

  // ==================================================================
  // GET /stats/recent — 最近请求记录
  // ==================================================================
  app.get('/stats/recent', async () => {
    const tracker = router.getUsageTracker();
    return tracker.getRecent(50);
  });

  // ==================================================================
  // 启动服务
  // ==================================================================
  const port = config.port ?? 3001;
  try {
    // 只监听 localhost，通过代理层对外提供服务
    await app.listen({ port, host: '127.0.0.1' });
    console.log(`✅ 网关已启动: http://127.0.0.1:${port} (仅本地访问)`);
    console.log(`📋 已配置模型: ${Object.keys(config.models).join(', ')}`);
    console.log(`🔒 API Key 校验: 已启用`);
    console.log(`📊 用量统计: ${config.usage_tracking.enabled ? '已启用' : '已禁用'}`);
  } catch (err) {
    console.error('❌ 启动失败:', err);
    process.exit(1);
  }
}

main();