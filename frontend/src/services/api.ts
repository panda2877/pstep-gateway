import axios from 'axios';
import type {
  UsageStats,
  UsageDistribution,
  ModelConfig,
  ModelConfigUpdate,
  ApiKey,
  ApiKeyCreate,
  ApiKeyUpdate,
  ApiKeyCreated,
  FallbackPolicy,
  CreateFallbackPolicyRequest,
  UpdateFallbackPolicyRequest,
  FallbackPolicyMini,
  TimePeriod,
  ModelsResponse,
  KeysResponse,
  PoliciesResponse,
  FallbackPolicyMiniResponse,
} from '../types';

const API_BASE = import.meta.env.VITE_API_BASE || 'http://127.0.0.1:3002';

const api = axios.create({
  baseURL: API_BASE,
  headers: {
    'Content-Type': 'application/json',
  },
});

// Usage APIs
export const getUsageStats = async (period: TimePeriod): Promise<UsageStats> => {
  const response = await api.get<UsageStats>(`/api/admin/usage/stats?period=${period}`);
  return response.data;
};

export const getUsageDistribution = async (period: TimePeriod): Promise<UsageDistribution> => {
  const response = await api.get<UsageDistribution>(`/api/admin/usage/distribution?period=${period}`);
  return response.data;
};

// Model Config APIs
export const getModels = async (): Promise<ModelConfig[]> => {
  const response = await api.get<ModelsResponse>('/api/admin/models');
  return response.data.models;
};

export const getModel = async (id: string): Promise<ModelConfig> => {
  const response = await api.get<ModelConfig>(`/api/admin/models/${id}`);
  return response.data;
};

export interface UpdateModelResponse {
  success: boolean;
  message: string;
  model_id: string;
  restart_required: boolean;
  changes: Record<string, unknown>;
  model: ModelConfig;
}

export const updateModel = async (
  id: string,
  data: ModelConfigUpdate,
): Promise<UpdateModelResponse> => {
  const response = await api.put<UpdateModelResponse>(`/api/admin/models/${id}`, data);
  return response.data;
};

export const getFallbackPoliciesMini = async (): Promise<FallbackPolicyMini[]> => {
  const response = await api.get<FallbackPolicyMiniResponse>('/api/admin/models/fallback-policies');
  return response.data.policies;
};

// API Key APIs
export const getApiKeys = async (): Promise<ApiKey[]> => {
  const response = await api.get<KeysResponse>('/api/admin/keys');
  return response.data.keys;
};

export const createApiKey = async (data: ApiKeyCreate): Promise<ApiKeyCreated> => {
  const response = await api.post<ApiKeyCreated>('/api/admin/keys', data);
  return response.data;
};

export const updateApiKey = async (id: string, data: ApiKeyUpdate): Promise<ApiKey> => {
  const response = await api.put<{ success: boolean; key: ApiKey }>(`/api/admin/keys/${id}`, data);
  return response.data.key;
};

export const deleteApiKey = async (id: string): Promise<void> => {
  await api.delete(`/api/admin/keys/${id}`);
};

export const revealApiKey = async (id: string): Promise<string> => {
  const response = await api.post<{ id: string; name: string; key: string }>(
    `/api/admin/keys/${id}/reveal`
  );
  return response.data.key;
};

// Fallback Policy APIs
export const getFallbackPolicies = async (): Promise<FallbackPolicy[]> => {
  const response = await api.get<PoliciesResponse>('/api/admin/fallback/policies');
  return response.data.policies;
};

export const getFallbackPolicy = async (id: string): Promise<FallbackPolicy> => {
  const response = await api.get<FallbackPolicy>(`/api/admin/fallback/policies/${id}`);
  return response.data;
};

export const createFallbackPolicy = async (
  data: CreateFallbackPolicyRequest,
): Promise<FallbackPolicy> => {
  const response = await api.post<{ success: boolean; policy: FallbackPolicy }>(
    '/api/admin/fallback/policies',
    data,
  );
  return response.data.policy;
};

export const updateFallbackPolicy = async (
  id: string,
  data: UpdateFallbackPolicyRequest,
): Promise<FallbackPolicy> => {
  const response = await api.put<{ success: boolean; policy: FallbackPolicy }>(
    `/api/admin/fallback/policies/${id}`,
    data,
  );
  return response.data.policy;
};

export const deleteFallbackPolicy = async (id: string): Promise<void> => {
  await api.delete(`/api/admin/fallback/policies/${id}`);
};
