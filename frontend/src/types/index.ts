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

// Model Config Types
export interface ModelConfig {
  id: string;
  name: string;
  provider: string;
  version: string;
  status: string;
  timeout_secs: number;
  price_per_input?: number;
  price_per_output?: number;
  upstream: string;
  fallback_chain: string[];
  /** 上游的 base_url（仅编辑界面使用，主列表不展示） */
  base_url?: string;
  /** 上游 api_key 的脱敏展示（如 sk-***abcd） */
  api_key_masked?: string;
  /** 上游 api_key 是否已配置 */
  api_key_configured: boolean;
}

export interface ModelConfigUpdate {
  name?: string;
  timeout_secs?: number;
  price_per_input?: number;
  price_per_output?: number;
  status?: string;
  /** 编辑时使用：覆盖上游的 base_url */
  base_url?: string;
  /** 编辑时使用：覆盖上游的 api_key
   * - 不传 / 空字符串 = 不变
   * - 特殊占位 "********" = 不变（前端展示脱敏值时不变更）
   * - 其他值 = 覆盖
   */
  api_key?: string;
}

// API Key Types
export interface ApiKey {
  id: string;
  name: string;
  key_prefix: string;
  key_masked: string;
  model_permissions: string[];
  quota_limit: number;
  quota_used: number;
  quota_percent: number;
  created_at: number;
}

export interface ApiKeyCreate {
  name: string;
  model_permissions: string[];
  quota_limit: number;
}

export interface ApiKeyCreated {
  success: boolean;
  key: ApiKey;
  raw_key: string;
}

// Fallback Policy Types
export interface ChainNode {
  provider: string;
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

export interface FallbackPolicyCreate {
  name: string;
  description?: string;
  enabled: boolean;
  chain: ChainNode[];
}

export interface FallbackPolicyUpdate {
  name?: string;
  description?: string;
  enabled?: boolean;
  chain?: ChainNode[];
}

// API Key response types
export type TimePeriod = '1d' | '7d' | '30d';

// Response wrapper types
export interface ModelsResponse {
  models: ModelConfig[];
}

export interface KeysResponse {
  keys: ApiKey[];
}

export interface PoliciesResponse {
  policies: FallbackPolicy[];
}