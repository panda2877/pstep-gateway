import React, { useState, useEffect } from 'react';
import { Card } from '../components/ui/Card';
import { PieChart } from '../components/charts/PieChart';
import { getUsageStats, getUsageDistribution } from '../services/api';
import type { UsageStats, UsageDistribution, TimePeriod } from '../types';

const formatNumber = (num: number): string => {
  if (num >= 1_000_000) return `${(num / 1_000_000).toFixed(1)}M`;
  if (num >= 1_000) return `${(num / 1_000).toFixed(1)}K`;
  return num.toString();
};

const formatCost = (cost: number): string => {
  return `$${cost.toFixed(2)}`;
};

export const OverviewPage: React.FC = () => {
  const [period, setPeriod] = useState<TimePeriod>('7d');
  const [stats, setStats] = useState<UsageStats | null>(null);
  const [distribution, setDistribution] = useState<UsageDistribution | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const fetchData = async () => {
      setLoading(true);
      try {
        const [statsData, distData] = await Promise.all([
          getUsageStats(period),
          getUsageDistribution(period),
        ]);
        setStats(statsData);
        setDistribution(distData);
      } catch (err) {
        console.error('Failed to fetch overview data:', err);
      } finally {
        setLoading(false);
      }
    };
    fetchData();
  }, [period]);

  return (
    <div className="section" id="section-overview">
      <div className="row-between" style={{ marginBottom: 'var(--gap-md)' }}>
        <div>
          <h1 className="h1">概览</h1>
          <p className="lead" style={{ marginTop: 4 }}>实时监控与配置管理</p>
        </div>
      </div>

      <div className="stats-grid">
        {/* Token 总计 */}
        <div className="stat-card">
          <div className="stat-top">
            <div className="stat-label">Token 总计</div>
            <div className="stat-value num">
              {loading ? '—' : formatNumber(stats?.token_total || 0)}
            </div>
            <div className="stat-meta">
              <div>
                <span className={`stat-change ${(stats?.change_percent || 0) >= 0 ? '' : 'negative'}`}>
                  {(stats?.change_percent || 0) >= 0 ? '↑' : '↓'} {Math.abs(stats?.change_percent || 0).toFixed(1)}%
                </span>{' '}
                较上月
              </div>
              <div>输入 {formatNumber(stats?.token_input || 0)}</div>
              <div>输出 {formatNumber(stats?.token_output || 0)}</div>
            </div>
          </div>
        </div>

        {/* API 成本 */}
        <div className="stat-card">
          <div className="stat-top">
            <div className="stat-label">API 成本</div>
            <div className="stat-value num">
              {loading ? '—' : formatCost(stats?.cost || 0)}
            </div>
            <div className="stat-meta">
              <div>
                <span className={`stat-change ${(stats?.change_percent || 0) >= 0 ? '' : 'negative'}`}>
                  {(stats?.change_percent || 0) >= 0 ? '↑' : '↓'} {Math.abs(stats?.change_percent || 0).toFixed(1)}%
                </span>{' '}
                较上月
              </div>
              <div>付费token计费</div>
            </div>
          </div>
        </div>

        {/* 模型分布 */}
        <div className="stat-card bar-chart-card">
          <Card>
            <div className="bar-chart-header">
              <div>
                <div className="bar-chart-title">模型分布</div>
                <div className="bar-chart-sub">按调用量占比</div>
              </div>
              <div className="time-range-tabs">
                {[1, 7, 30].map((days) => (
                  <button
                    key={days}
                    className={`${days}d` === period ? 'active' : ''}
                    onClick={() => setPeriod(`${days}d` as TimePeriod)}
                  >
                    {days} 天
                  </button>
                ))}
              </div>
            </div>
            {loading ? (
              <div style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>加载中...</div>
            ) : distribution && distribution.models.length > 0 ? (
              <PieChart data={distribution.models} />
            ) : (
              <div style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>暂无数据</div>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
};