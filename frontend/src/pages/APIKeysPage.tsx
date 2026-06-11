import React, { useState, useEffect } from 'react';
import { Plus } from 'lucide-react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Modal } from '../components/ui/Modal';
import { Input } from '../components/ui/Input';
import { Select } from '../components/ui/Select';
import { getApiKeys, createApiKey, deleteApiKey } from '../services/api';
import type { ApiKey } from '../types';

const getQuotaColor = (percent: number): string => {
  if (percent < 50) return 'var(--success)';
  if (percent < 80) return 'var(--warn)';
  return 'var(--danger)';
};

const MODEL_OPTIONS = [
  { value: 'all', label: '全部模型' },
  { value: 'gpt-4o', label: 'GPT-4o' },
  { value: 'claude-3-5-sonnet', label: 'Claude 3.5' },
  { value: 'gemini-2.0', label: 'Gemini 2.0' },
];

export const APIKeysPage: React.FC = () => {
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [createModal, setCreateModal] = useState(false);
  const [newKey, setNewKey] = useState({ name: '', models: 'all', quota: 1000000 });
  const [creating, setCreating] = useState(false);
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const fetchKeys = async () => {
    setLoading(true);
    try {
      const data = await getApiKeys();
      setKeys(data);
    } catch (err) {
      console.error('Failed to fetch API keys:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchKeys();
  }, []);

  const handleCreate = async () => {
    if (!newKey.name.trim()) return;
    setCreating(true);
    try {
      const result = await createApiKey({
        name: newKey.name,
        model_permissions: newKey.models === 'all' ? [] : [newKey.models],
        quota_limit: Number(newKey.quota),
      });
      setCreatedKey(result.raw_key);
      fetchKeys();
    } catch (err) {
      console.error('Failed to create API key:', err);
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm('确定要撤销此密钥吗？')) return;
    try {
      await deleteApiKey(id);
      fetchKeys();
    } catch (err) {
      console.error('Failed to delete API key:', err);
    }
  };

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  };

  return (
    <div className="section" id="section-apikeys">
      <div className="section-header">
        <div>
          <h2 className="section-title">API 密钥</h2>
          <p className="section-desc">创建与管理访问密钥</p>
        </div>
        <Button variant="secondary" size="sm" onClick={() => { setCreateModal(true); setCreatedKey(null); }}>
          <Plus size={14} />
          新建密钥
        </Button>
      </div>

      <Card>
        <div className="table-wrap">
          <table className="ds-table">
            <thead>
              <tr>
                <th>名称</th>
                <th>密钥</th>
                <th>模型权限</th>
                <th>剩余配额</th>
                <th>创建时间</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={6} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
                    加载中...
                  </td>
                </tr>
              ) : keys.length === 0 ? (
                <tr>
                  <td colSpan={6} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
                    暂无密钥
                  </td>
                </tr>
              ) : (
                keys.map((key) => (
                  <tr key={key.id}>
                    <td style={{ fontWeight: 500 }}>{key.name}</td>
                    <td>
                      <div className="key-display">
                        <span className="key-masked">{key.key_masked}</span>
                        <button
                          className="copy-btn"
                          onClick={() => copyToClipboard(key.key_prefix)}
                        >
                          复制
                        </button>
                      </div>
                    </td>
                    <td>
                      <Badge>
                        {key.model_permissions.length === 0
                          ? '全部模型'
                          : key.model_permissions.join(' / ')}
                      </Badge>
                    </td>
                    <td className="num-col" style={{ textAlign: 'left' }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <div style={{ flex: 1, height: 4, background: 'var(--border)', borderRadius: 2, overflow: 'hidden' }}>
                          <div style={{ width: `${key.quota_percent}%`, height: '100%', background: getQuotaColor(key.quota_percent), borderRadius: 2 }}></div>
                        </div>
                        <span className="meta">{key.quota_percent.toFixed(0)}%</span>
                      </div>
                    </td>
                    <td className="meta">{new Date(key.created_at * 1000).toLocaleDateString()}</td>
                    <td className="actions">
                      <Button variant="danger" size="sm" onClick={() => handleDelete(key.id)}>
                        撤销
                      </Button>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </Card>

      <Modal
        isOpen={createModal}
        onClose={() => { setCreateModal(false); setCreatedKey(null); }}
        title="新建 API 密钥"
        footer={
          createdKey ? (
            <Button onClick={() => { setCreateModal(false); setCreatedKey(null); }}>完成</Button>
          ) : (
            <>
              <Button variant="secondary" onClick={() => setCreateModal(false)}>取消</Button>
              <Button onClick={handleCreate} disabled={creating || !newKey.name.trim()}>
                {creating ? '创建中...' : '创建'}
              </Button>
            </>
          )
        }
      >
        {createdKey ? (
          <div className="field">
            <label>密钥已创建，请妥善保存</label>
            <div className="key-display" style={{ marginTop: 8 }}>
              <span className="key-value">{createdKey}</span>
              <button
                className="copy-btn"
                onClick={() => copyToClipboard(createdKey)}
                style={{ color: copied ? 'var(--success)' : undefined }}
              >
                {copied ? '已复制' : '复制'}
              </button>
            </div>
          </div>
        ) : (
          <>
            <Input
              label="密钥名称"
              placeholder="例如：生产环境密钥"
              value={newKey.name}
              onChange={(e) => setNewKey({ ...newKey, name: e.target.value })}
            />
            <Select
              label="模型权限"
              options={MODEL_OPTIONS}
              value={newKey.models}
              onChange={(e) => setNewKey({ ...newKey, models: e.target.value })}
            />
            <Input
              label="月度配额上限"
              type="number"
              mono
              placeholder="1000000"
              value={newKey.quota}
              onChange={(e) => setNewKey({ ...newKey, quota: Number(e.target.value) })}
              hint="设置为 0 表示不限制"
            />
          </>
        )}
      </Modal>
    </div>
  );
};