// ============================================================================
// Pstep Gateway — Anthropic 格式转换代理
// 接收 OpenAI 格式请求，转换为 Anthropic 格式后发送
// ============================================================================

import type { UpstreamConfig, OpenAIRequest, AnthropicRequest } from '../types.js';

interface ProxyResult {
  response: Response;
  latencyMs: number;
}

/**
 * 将 OpenAI 格式请求转换为 Anthropic 格式
 */
function convertToAnthropic(openaiReq: OpenAIRequest, targetModel: string): AnthropicRequest {
  const anthropicReq: AnthropicRequest = {
    model: targetModel,
    messages: [],
    max_tokens: openaiReq.max_tokens ?? 4096,
    stream: openaiReq.stream,
  };

  // 提取 system 消息（Anthropic 用顶层 system 字段）
  const systemMsg = openaiReq.messages.find(m => m.role === 'system');
  if (systemMsg && typeof systemMsg.content === 'string') {
    anthropicReq.system = systemMsg.content;
  }

  // 转换消息列表（过滤掉 system 消息）
  for (const msg of openaiReq.messages) {
    if (msg.role === 'system') continue;

    if (msg.role === 'tool') {
      // Anthropic 用 tool_result 类型的 content block
      anthropicReq.messages.push({
        role: 'user',
        content: [
          {
            type: 'tool_result' as const,
            tool_use_id: msg.tool_call_id!,
            content: typeof msg.content === 'string' ? msg.content : JSON.stringify(msg.content),
          },
        ],
      });
      continue;
    }

    anthropicReq.messages.push({
      role: msg.role === 'assistant' ? 'assistant' : 'user',
      content: typeof msg.content === 'string' ? msg.content : msg.content,
    });
  }

  // 转换 tools
  if (openaiReq.tools) {
    anthropicReq.tools = openaiReq.tools.map(t => ({
      name: t.function.name,
      description: t.function.description,
      input_schema: t.function.parameters,
    }));
  }

  // 转换 tool_choice
  if (openaiReq.tool_choice) {
    if (openaiReq.tool_choice === 'auto') {
      anthropicReq.tool_choice = { type: 'auto' };
    } else if (openaiReq.tool_choice === 'none') {
      anthropicReq.tool_choice = { type: 'any' }; // Anthropic 没有 none，用 any 近似
    } else if (openaiReq.tool_choice === 'required') {
      anthropicReq.tool_choice = { type: 'any' };
    } else if (typeof openaiReq.tool_choice === 'object') {
      anthropicReq.tool_choice = {
        type: 'tool',
        name: openaiReq.tool_choice.function?.name,
      };
    }
  }

  // 转换 temperature / top_p
  if (openaiReq.temperature !== undefined) anthropicReq.temperature = openaiReq.temperature;
  if (openaiReq.top_p !== undefined) anthropicReq.top_p = openaiReq.top_p;

  return anthropicReq;
}

/**
 * 代理请求到 Anthropic API
 */
export async function proxyToAnthropic(
  upstream: UpstreamConfig,
  targetModel: string,
  body: string,
  signal?: AbortSignal,
): Promise<ProxyResult> {
  const start = performance.now();

  // 解析 OpenAI 格式请求
  const openaiReq: OpenAIRequest = JSON.parse(body);
  const anthropicReq = convertToAnthropic(openaiReq, targetModel);

  const response = await fetch(`${upstream.base_url}/messages`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'x-api-key': upstream.api_key,
      'anthropic-version': '2023-06-01',
    },
    body: JSON.stringify(anthropicReq),
    signal,
  });

  const latencyMs = Math.round(performance.now() - start);
  return { response, latencyMs };
}

/**
 * 从 Anthropic 流式事件中提取 usage
 * Anthropic 在 message_start 和 message_delta 事件中发送用量
 */
export function parseAnthropicUsage(line: string): {
  input_tokens: number;
  output_tokens: number;
} | null {
  try {
    if (!line.startsWith('data: ')) return null;
    const event = JSON.parse(line.slice(6));

    // message_start 包含 input_tokens
    if (event.type === 'message_start' && event.message?.usage) {
      return {
        input_tokens: event.message.usage.input_tokens ?? 0,
        output_tokens: event.message.usage.output_tokens ?? 0,
      };
    }

    // message_delta 包含 output_tokens 增量
    if (event.type === 'message_delta' && event.usage) {
      return {
        input_tokens: 0,
        output_tokens: event.usage.output_tokens ?? 0,
      };
    }

    return null;
  } catch {
    return null;
  }
}

/**
 * 将 Anthropic 流式事件转换为 OpenAI SSE 格式
 */
export function anthropicEventToOpenAI(event: Record<string, unknown>): string | null {
  const type = event.type as string;

  // message_start → 首个 chunk
  if (type === 'message_start') {
    const msg = event.message as Record<string, unknown> | undefined;
    return `data: ${JSON.stringify({
      id: msg?.id ?? `chatcmpl-${Date.now()}`,
      object: 'chat.completion.chunk',
      created: Math.floor(Date.now() / 1000),
      model: msg?.model ?? '',
      choices: [{
        index: 0,
        delta: { role: 'assistant' },
        finish_reason: null,
      }],
    })}\n\n`;
  }

  // content_block_delta (text) → content delta
  if (type === 'content_block_delta') {
    const delta = event.delta as Record<string, unknown> | undefined;
    if (delta?.type === 'text_delta') {
      return `data: ${JSON.stringify({
        id: `chatcmpl-${Date.now()}`,
        object: 'chat.completion.chunk',
        created: Math.floor(Date.now() / 1000),
        model: '',
        choices: [{
          index: 0,
          delta: { content: delta.text },
          finish_reason: null,
        }],
      })}\n\n`;
    }
    // tool_use delta
    if (delta?.type === 'input_json_delta') {
      return `data: ${JSON.stringify({
        id: `chatcmpl-${Date.now()}`,
        object: 'chat.completion.chunk',
        created: Math.floor(Date.now() / 1000),
        model: '',
        choices: [{
          index: 0,
          delta: {
            tool_calls: [{
              index: event.index ?? 0,
              function: { arguments: delta.partial_json },
            }],
          },
          finish_reason: null,
        }],
      })}\n\n`;
    }
    return null;
  }

  // content_block_start (tool_use) → tool call start
  if (type === 'content_block_start') {
    const block = event.content_block as Record<string, unknown> | undefined;
    if (block?.type === 'tool_use') {
      return `data: ${JSON.stringify({
        id: `chatcmpl-${Date.now()}`,
        object: 'chat.completion.chunk',
        created: Math.floor(Date.now() / 1000),
        model: '',
        choices: [{
          index: 0,
          delta: {
            tool_calls: [{
              index: event.index ?? 0,
              id: block.id,
              type: 'function',
              function: { name: block.name, arguments: '' },
            }],
          },
          finish_reason: null,
        }],
      })}\n\n`;
    }
    return null;
  }

  // message_delta → finish reason
  if (type === 'message_delta') {
    const msgDelta = event.delta as Record<string, unknown> | undefined;
    let finishReason: string | null = null;
    if (msgDelta?.stop_reason === 'end_turn') finishReason = 'stop';
    else if (msgDelta?.stop_reason === 'max_tokens') finishReason = 'length';
    else if (msgDelta?.stop_reason === 'tool_use') finishReason = 'tool_calls';

    return `data: ${JSON.stringify({
      id: `chatcmpl-${Date.now()}`,
      object: 'chat.completion.chunk',
      created: Math.floor(Date.now() / 1000),
      model: '',
      choices: [{
        index: 0,
        delta: {},
        finish_reason: finishReason,
      }],
    })}\n\n`;
  }

  return null;
}