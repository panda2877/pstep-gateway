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
}

export interface ModelConfigUpdate {
  name?: string;
  timeout_secs?: number;
  price_per_input?: number;
  price_per_output?: number;
  status?: string;
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