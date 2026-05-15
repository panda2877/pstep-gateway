// ============================================================================
// Pstep Gateway — Fastify 服务入口
// ============================================================================

import Fastify from 'fastify';
import cors from '@fastify/cors';
import { loadConfig } from './config.js';
import { Router } from './router.js';

async function main() {
  console.log('╔══════════════════════════════════════╗');
  console.log('║        Pstep Gateway v0.1.0         ║');
  console.log('╚══════════════════════════════════════╝');

  // 加载配置
  const config = loadConfig();
  const router = new Router(config);

  // 创建 Fastify 实例
  const app = Fastify({ logger: false });
  await app.register(cors, { origin: true });

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
        const reader = result.stream.getReader();
        let fullResponse = '';
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          fullResponse += value;
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

      const reader = result.stream.getReader();
      const decoder = new TextDecoder();

      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          reply.raw.write(typeof value === 'string' ? value : decoder.decode(value));
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
      version: '0.1.0',
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
      owned_by: route.upstream,
    }));
    return { object: 'list', data: models };
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
    await app.listen({ port, host: '0.0.0.0' });
    console.log(`✅ 网关已启动: http://0.0.0.0:${port}`);
    console.log(`📋 已配置模型: ${Object.keys(config.models).join(', ')}`);
    console.log(`📊 用量统计: ${config.usage_tracking.enabled ? '已启用' : '已禁用'}`);
  } catch (err) {
    console.error('❌ 启动失败:', err);
    process.exit(1);
  }
}

main();