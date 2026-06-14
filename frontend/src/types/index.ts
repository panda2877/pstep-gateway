// API Response Types
export interface UsageStats {
  token_total: number;
  token_input: number;
  token_output: number;
  cost: number;
  change_percent: number;
  period: string;
}

export interface ModelDistribution {
  name: string;
  color: string;
  percent: number;
  tokens: number;
}

export interface UsageDistribution {
  models: ModelDistribution[];
  period: string;
}

// ============== Model Config ==============

/** 服务端响应的单条 model 配置 */
export interface ModelConfig {
  id: string;
  name: string;
  version: string;
  status: 'active' | 'rate_limited' | 'disabled' | string;
  timeout_secs: number;
  price_per_input?: number;
  price_per_output?: number;
  /** v0.3: 引用此 model 的所有 fallback 策略 id */
  referenced_by_policies?: string[];
  /** 上游 base_url（仅编辑界面使用） */
  base_url?: string;
  /** 上游 api_key 的脱敏显示 */
  api_key_masked?: string;
  /** 上游 api_key 是否已配置 */
  api_key_configured: boolean;
  /** 实际发到上游的 model id */
  upstream_model: string;
}

/** PUT /api/admin/models/:id 的请求体 */
export interface ModelConfigUpdate {
  // 热更新字段
  name?: string;
  status?: 'active' | 'rate_limited' | 'disabled';
  price_per_input?: number;
  price_per_output?: number;
  // 需重启字段
  base_url?: string;
  model?: string;
  /** None / "" = 不变；"********" = 不变；其他 = 覆盖 */
  api_key?: string;
}

export interface ModelsResponse {
  models: ModelConfig[];
}

export interface FallbackPolicyMini {
  id: string;
}

export interface FallbackPolicyMiniResponse {
  policies: FallbackPolicyMini[];
}

// ============== Fallback Policy ==============

export interface ChainNode {
  /** 语义标签，如 'anthropic' / 'mimo' / 'openai' */
  upstream: string;
  model: string;
}

export interface FallbackPolicy {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  chain: ChainNode[];
  created_at: number;
}

export interface CreateFallbackPolicyRequest {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  chain: ChainNode[];
}

export interface UpdateFallbackPolicyRequest {
  name?: string;
  description?: string;
  enabled?: boolean;
  chain?: ChainNode[];
}

export interface PoliciesResponse {
  policies: FallbackPolicy[];
}

// ============== API Key ==============

export interface ApiKey {
  id: string;
  name: string;
  key_prefix: string;
  key_masked: string;
  model_permissions: string[];
  /** 该 Key 专用 fallback 链（覆盖模型默认） */
  fallback_policy?: string | null;
  quota_limit: number;
  quota_used: number;
  quota_percent: number;
  created_at: number;
}

export interface ApiKeyCreate {
  name: string;
  model_permissions: string[];
  fallback_policy?: string;
  quota_limit: number;
}

export interface ApiKeyUpdate {
  name?: string;
  model_permissions?: string[];
  /** 外层 Some 表示「要更新」；内层 None 表示「置空」 */
  fallback_policy?: string | null;
  quota_limit?: number;
}

export interface ApiKeyCreated {
  success: boolean;
  key: ApiKey;
  raw_key: string;
}

export interface KeysResponse {
  keys: ApiKey[];
}

// ============== Misc ==============

export type TimePeriod = '1d' | '7d' | '30d';
