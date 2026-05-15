// ============================================================================
// Pstep Gateway — OpenAI 兼容上游代理
// 接收 OpenAI 格式请求，转发到 OpenAI 兼容 API
// ============================================================================

import type { UpstreamConfig } from '../types.js';

interface ProxyResult {
  response: Response;
  latencyMs: number;
}

/**
 * 代理请求到 OpenAI 兼容 API
 * 直接转发，只替换 model 名和注入 API Key
 */
export async function proxyToOpenAI(
  upstream: UpstreamConfig,
  targetModel: string,
  body: string,
  signal?: AbortSignal,
): Promise<ProxyResult> {
  const start = performance.now();

  const response = await fetch(`${upstream.base_url}/chat/completions`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${upstream.api_key}`,
    },
    body: body.replace(
      // 替换 body 中的 model 名为上游实际模型名
      /"model"\s*:\s*"[^"]*"/,
      `"model": "${targetModel}"`,
    ),
    signal,
  });

  const latencyMs = Math.round(performance.now() - start);
  return { response, latencyMs };
}

/**
 * 从 OpenAI 流式响应中解析最终 usage
 * OpenAI 在最后一个 chunk 中发送 usage 字段
 */
export function parseOpenAIUsage(chunk: string): {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
} | null {
  try {
    // 跳过 "data: [DONE]" 标记
    if (chunk === 'data: [DONE]' || !chunk.startsWith('data: ')) return null;

    const data = JSON.parse(chunk.slice(6));
    if (data.usage) {
      return {
        prompt_tokens: data.usage.prompt_tokens ?? 0,
        completion_tokens: data.usage.completion_tokens ?? 0,
        total_tokens: data.usage.total_tokens ?? 0,
      };
    }
    return null;
  } catch {
    return null;
  }
}