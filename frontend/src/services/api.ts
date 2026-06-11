import axios from 'axios';
import type {
  UsageStats,
  UsageDistribution,
  ModelConfig,
  ModelConfigUpdate,
  ApiKey,
  ApiKeyCreate,
  ApiKeyCreated,
  FallbackPolicy,
  FallbackPolicyCreate,
  FallbackPolicyUpdate,
  TimePeriod,
  ModelsResponse,
  KeysResponse,
  PoliciesResponse,
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

export const updateModel = async (id: string, data: ModelConfigUpdate): Promise<void> => {
  await api.put(`/api/admin/models/${id}`, data);
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

export const deleteApiKey = async (id: string): Promise<void> => {
  await api.delete(`/api/admin/keys/${id}`);
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

export const createFallbackPolicy = async (data: FallbackPolicyCreate): Promise<FallbackPolicy> => {
  const response = await api.post<{ success: boolean; policy: FallbackPolicy }>('/api/admin/fallback/policies', data);
  return response.data.policy;
};

export const updateFallbackPolicy = async (id: string, data: FallbackPolicyUpdate): Promise<FallbackPolicy> => {
  const response = await api.put<{ success: boolean; policy: FallbackPolicy }>(`/api/admin/fallback/policies/${id}`, data);
  return response.data.policy;
};

export const deleteFallbackPolicy = async (id: string): Promise<void> => {
  await api.delete(`/api/admin/fallback/policies/${id}`);
};