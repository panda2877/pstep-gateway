// ============================================================================
// Pstep Gateway — 用量统计
// 内存中的滑动窗口统计器
// ============================================================================

import type { UsageRecord, UsageStats } from './types.js';

export class UsageTracker {
  private records: UsageRecord[] = [];
  private retentionMs: number;
  private enabled: boolean;

  constructor(enabled: boolean, retentionHours: number) {
    this.enabled = enabled;
    this.retentionMs = retentionHours * 60 * 60 * 1000;
  }

  /** 记录一次请求 */
  record(record: UsageRecord): void {
    if (!this.enabled) return;

    this.records.push(record);
    this.cleanup();
  }

  /** 清理过期记录 */
  private cleanup(): void {
    const cutoff = Date.now() - this.retentionMs;
    this.records = this.records.filter(r => r.timestamp > cutoff);
  }

  /** 获取统计快照 */
  getStats(): UsageStats {
    this.cleanup();

    const total = this.records.length;
    let promptTokens = 0;
    let completionTokens = 0;
    const byModel: Record<string, { requests: number; tokens: number }> = {};
    const byUpstream: Record<string, { requests: number; tokens: number }> = {};

    for (const r of this.records) {
      promptTokens += r.prompt_tokens;
      completionTokens += r.completion_tokens;

      if (!byModel[r.model]) byModel[r.model] = { requests: 0, tokens: 0 };
      byModel[r.model].requests++;
      byModel[r.model].tokens += r.total_tokens;

      if (!byUpstream[r.upstream]) byUpstream[r.upstream] = { requests: 0, tokens: 0 };
      byUpstream[r.upstream].requests++;
      byUpstream[r.upstream].tokens += r.total_tokens;
    }

    return {
      total_requests: total,
      total_prompt_tokens: promptTokens,
      total_completion_tokens: completionTokens,
      total_tokens: promptTokens + completionTokens,
      by_model: byModel,
      by_upstream: byUpstream,
    };
  }

  /** 获取最近 N 条记录 */
  getRecent(n: number): UsageRecord[] {
    this.cleanup();
    return this.records.slice(-n);
  }
}