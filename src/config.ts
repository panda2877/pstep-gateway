// ============================================================================
// Pstep Gateway — YAML 配置加载器
// ============================================================================

import { readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'yaml';
import type { GatewayConfig } from './types.js';

const __dirname = dirname(fileURLToPath(import.meta.url));

/** 解析 ${ENV_VAR} 引用为实际环境变量值 */
function resolveEnvVars(value: string): string {
  return value.replace(/\$\{([^}]+)\}/g, (_, envVar) => {
    const envValue = process.env[envVar];
    if (!envValue) {
      console.warn(`⚠️  环境变量 ${envVar} 未设置，使用空字符串`);
      return '';
    }
    return envValue;
  });
}

/** 递归解析配置中所有 ${ENV_VAR} 引用 */
function resolveEnvRefs(obj: unknown): unknown {
  if (typeof obj === 'string') {
    return resolveEnvVars(obj);
  }
  if (Array.isArray(obj)) {
    return obj.map(resolveEnvRefs);
  }
  if (obj && typeof obj === 'object') {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(obj)) {
      result[key] = resolveEnvRefs(value);
    }
    return result;
  }
  return obj;
}

/**
 * 加载网关配置
 * 查找顺序：
 *   1. config.yaml（本地开发，已 gitignore）
 *   2. /etc/pstep-gateway/config.yaml（Docker 挂载）
 *   3. ${CONFIG_PATH} 环境变量指定路径
 */
export function loadConfig(): GatewayConfig {
  const searchPaths = [
    process.env.CONFIG_PATH,
    join(process.cwd(), 'config.yaml'),
    join(__dirname, '..', 'config.yaml'),
    '/etc/pstep-gateway/config.yaml',
  ].filter(Boolean) as string[];

  let configPath = '';
  for (const path of searchPaths) {
    if (path && existsSync(path)) {
      configPath = path;
      break;
    }
  }

  if (!configPath) {
    console.error('❌ 未找到配置文件！');
    console.error('   请创建 config.yaml（参考 config.yaml.template）');
    console.error('   或设置 CONFIG_PATH 环境变量');
    process.exit(1);
  }

  console.log(`📄 加载配置: ${configPath}`);
  const raw = readFileSync(configPath, 'utf-8');
  const parsed = parse(raw);
  const resolved = resolveEnvRefs(parsed) as GatewayConfig;

  // 验证配置
  validateConfig(resolved);

  return resolved;
}

function validateConfig(config: GatewayConfig): void {
  if (!config.port) config.port = 3001;

  if (!config.upstreams || Object.keys(config.upstreams).length === 0) {
    console.error('❌ 配置错误：至少需要一个 upstream');
    process.exit(1);
  }

  if (!config.models || Object.keys(config.models).length === 0) {
    console.error('❌ 配置错误：至少需要一个 model');
    process.exit(1);
  }

  // 验证每个模型的 upstream 存在
  for (const [name, route] of Object.entries(config.models)) {
    if (!config.upstreams[route.upstream]) {
      console.error(`❌ 模型 "${name}" 引用了不存在的 upstream "${route.upstream}"`);
      process.exit(1);
    }
    if (route.fallback && !config.models[route.fallback]) {
      console.error(`❌ 模型 "${name}" 的 fallback "${route.fallback}" 不存在`);
      process.exit(1);
    }
  }

  // 验证每个 upstream 有 api_key
  for (const [name, upstream] of Object.entries(config.upstreams)) {
    if (!upstream.api_key) {
      console.error(`❌ upstream "${name}" 未设置 api_key`);
      process.exit(1);
    }
  }

  // 默认用量统计
  if (!config.usage_tracking) {
    config.usage_tracking = { enabled: true, retention_hours: 24 };
  }
}

/** 列出所有已配置的模型 */
export function listModels(config: GatewayConfig): string[] {
  return Object.keys(config.models);
}