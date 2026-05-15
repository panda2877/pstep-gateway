// ============================================================================
// Pstep Gateway — 类型定义
// ============================================================================

/** 上游提供商类型 */
export type UpstreamType = 'openai' | 'anthropic';

/** 上游提供商配置 */
export interface UpstreamConfig {
  type: UpstreamType;
  base_url: string;
  api_key: string;
}

/** 模型路由配置 */
export interface ModelRoute {
  upstream: string;
  model: string;
  fallback?: string;
}

/** 用量统计配置 */
export interface UsageConfig {
  enabled: boolean;
  retention_hours: number;
}

/** 完整网关配置 */
export interface GatewayConfig {
  port: number;
  upstreams: Record<string, UpstreamConfig>;
  models: Record<string, ModelRoute>;
  usage_tracking: UsageConfig;
}

/** Token 用量记录 */
export interface UsageRecord {
  model: string;
  upstream: string;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  timestamp: number;
  success: boolean;
  latency_ms: number;
}

/** 用量统计快照 */
export interface UsageStats {
  total_requests: number;
  total_prompt_tokens: number;
  total_completion_tokens: number;
  total_tokens: number;
  by_model: Record<string, {
    requests: number;
    tokens: number;
  }>;
  by_upstream: Record<string, {
    requests: number;
    tokens: number;
  }>;
}

// ============================================================================
// OpenAI 兼容的请求/响应类型
// ============================================================================

export interface OpenAIRequest {
  model: string;
  messages: Array<{
    role: 'system' | 'user' | 'assistant' | 'tool';
    content: string | Array<{ type: string; text?: string; image_url?: { url: string } }>;
    name?: string;
    tool_call_id?: string;
  }>;
  stream?: boolean;
  max_tokens?: number;
  temperature?: number;
  top_p?: number;
  tools?: Array<{
    type: 'function';
    function: {
      name: string;
      description?: string;
      parameters: Record<string, unknown>;
    };
  }>;
  tool_choice?: 'auto' | 'none' | 'required' | { type: 'function'; function: { name: string } };
  stop?: string | string[];
}

export interface OpenAIStreamChunk {
  id: string;
  object: 'chat.completion.chunk';
  created: number;
  model: string;
  choices: Array<{
    index: number;
    delta: {
      role?: string;
      content?: string;
      tool_calls?: Array<{
        index: number;
        id?: string;
        type?: 'function';
        function?: { name?: string; arguments?: string };
      }>;
    };
    finish_reason: string | null;
  }>;
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}

// ============================================================================
// Anthropic 类型
// ============================================================================

export interface AnthropicRequest {
  model: string;
  messages: Array<{
    role: 'user' | 'assistant';
content?: string | Array<Record<string, unknown>>;
  }>;
  system?: string;
  stream?: boolean;
  max_tokens?: number;
  temperature?: number;
  top_p?: number;
  tools?: Array<{
    name: string;
    description?: string;
    input_schema: Record<string, unknown>;
  }>;
  tool_choice?: { type: 'auto' | 'any' | 'tool'; name?: string };
  stop_sequences?: string[];
}

export interface AnthropicStreamEvent {
  type: string;
  message?: { id: string; model: string; usage?: { input_tokens: number; output_tokens: number } };
  index?: number;
  delta?: { text?: string; type?: string; partial_json?: string };
  content_block?: { type: string; text?: string; id?: string; name?: string; input?: unknown };
  content_block_start?: { index: number; content_block: { type: string } };
  content_block_delta?: { index: number; delta: { text?: string; type?: string; partial_json?: string } };
  content_block_stop?: { index: number };
  message_delta?: {
    delta?: { stop_reason?: string; stop_sequence?: string };
    usage?: { output_tokens: number };
  };
  message_start?: {
    message: {
      id: string;
      model: string;
      usage: { input_tokens: number; output_tokens: number };
    };
  };
}