// ============================================================================
// Pstep Gateway — 路由引擎 + 故障转移
// ============================================================================

import type { GatewayConfig, UpstreamConfig, UsageRecord } from './types.js';
import { proxyToOpenAI } from './providers/openai.js';
import { proxyToAnthropic } from './providers/anthropic.js';
import { UsageTracker } from './usage.js';

export interface RouteResult {
  /** 最终的 SSE 文本流（已转换为 OpenAI 格式） */
  stream: ReadableStream<string>;
  /** 最终使用的上游 */
  usedUpstream: string;
  /** 最终使用的模型 */
  usedModel: string;
  /** 是否发生了故障转移 */
  didFailover: boolean;
  /** 用量信息（从响应中解析） */
  usage: { prompt_tokens: number; completion_tokens: number } | null;
}

/**
 * 路由引擎
 */
export class Router {
  private config: GatewayConfig;
  private usageTracker: UsageTracker;

  constructor(config: GatewayConfig) {
    this.config = config;
    this.usageTracker = new UsageTracker(
      config.usage_tracking.enabled,
      config.usage_tracking.retention_hours,
    );
  }

  getUsageTracker(): UsageTracker {
    return this.usageTracker;
  }

  /**
   * 路由一次请求
   * 按优先级尝试：primary → fallback
   */
  async route(
    modelName: string,
    body: string,
    signal?: AbortSignal,
  ): Promise<RouteResult> {
    const route = this.config.models[modelName];
    if (!route) {
      throw new Error(`未知模型: ${modelName}。可用模型: ${Object.keys(this.config.models).join(', ')}`);
    }

    // 尝试主模型
    const primaryResult = await this.tryUpstream(route.upstream, route.model, body, signal);
    if (primaryResult.success && primaryResult.response) {
      return this.makeResult(primaryResult.response, primaryResult.latencyMs ?? 0, route.upstream, route.model, false);
    }

    // 主模型失败，尝试 fallback
    if (route.fallback) {
      const fallbackRoute = this.config.models[route.fallback];
      if (fallbackRoute) {
        console.log(`⚠️  主模型 ${route.model} 失败，切换到 fallback: ${route.fallback}`);
        const fallbackResult = await this.tryUpstream(
          fallbackRoute.upstream,
          fallbackRoute.model,
          body,
          signal,
        );
        if (fallbackResult.success && fallbackResult.response) {
          return this.makeResult(fallbackResult.response, fallbackResult.latencyMs ?? 0, fallbackRoute.upstream, fallbackRoute.model, true);
        }
      }
    }

    // 全部失败
    throw new Error(`所有上游都失败：${route.model}${route.fallback ? ` → ${route.fallback}` : ''}`);
  }

  private async tryUpstream(
    upstreamName: string,
    targetModel: string,
    body: string,
    signal?: AbortSignal,
  ): Promise<{
    success: boolean;
    response?: Response;
    latencyMs?: number;
    error?: string;
  }> {
    const upstream = this.config.upstreams[upstreamName];
    if (!upstream) return { success: false, error: `upstream ${upstreamName} 不存在` };

    try {
      const result = await this.callUpstream(upstream, targetModel, body, signal);
      return {
        success: result.response.ok,
        response: result.response,
        latencyMs: result.latencyMs,
        error: result.response.ok ? undefined : `上游返回 ${result.response.status}: ${result.response.statusText}`,
      };
    } catch (err) {
      return {
        success: false,
        error: err instanceof Error ? err.message : String(err),
      };
    }
  }

  private async callUpstream(
    upstream: UpstreamConfig,
    targetModel: string,
    body: string,
    signal?: AbortSignal,
  ) {
    switch (upstream.type) {
      case 'openai':
        return proxyToOpenAI(upstream, targetModel, body, signal);
      case 'anthropic':
        return proxyToAnthropic(upstream, targetModel, body, signal);
      default:
        throw new Error(`不支持的 upstream 类型: ${upstream.type}`);
    }
  }

  private makeResult(
    response: Response,
    latencyMs: number,
    upstreamName: string,
    modelName: string,
    didFailover: boolean,
  ): RouteResult {
    // 构建 SSE 流
    const stream = new ReadableStream<string>({
      start: async (controller) => {
        try {
          if (!response.body) {
            controller.enqueue(`data: ${JSON.stringify({
              error: '上游返回空响应',
              status: response.status,
            })}\n\n`);
            controller.close();
            return;
          }

          const reader = response.body.getReader();
          const decoder = new TextDecoder();
          let buffer = '';
          let promptTokens = 0;
          let completionTokens = 0;

          while (true) {
            const { done, value } = await reader.read();
            if (done) break;

            buffer += decoder.decode(value, { stream: true });

            // 按行处理（SSE 格式）
            const lines = buffer.split('\n');
            buffer = lines.pop() ?? '';

            for (const line of lines) {
              if (line.trim() === '') continue;
              controller.enqueue(line + '\n');
            }
          }

          // 处理剩余的 buffer
          if (buffer.trim()) {
            controller.enqueue(buffer + '\n');
          }

          controller.enqueue('data: [DONE]\n\n');
          controller.close();

          // 记录用量
          this.usageTracker.record({
            model: modelName,
            upstream: upstreamName,
            prompt_tokens: promptTokens,
            completion_tokens: completionTokens,
            total_tokens: promptTokens + completionTokens,
            timestamp: Date.now(),
            success: true,
            latency_ms: latencyMs,
          });
        } catch (err) {
          controller.error(err);
        }
      },
    });

    return {
      stream,
      usedUpstream: upstreamName,
      usedModel: modelName,
      didFailover,
      usage: null, // usage will be parsed from stream
    };
  }
}