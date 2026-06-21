import React, { useState, useEffect } from 'react';
import { Plus } from 'lucide-react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Modal } from '../components/ui/Modal';
import { Input } from '../components/ui/Input';
import {
  getApiKeys,
  createApiKey,
  updateApiKey,
  deleteApiKey,
  revealApiKey,
  getModels,
  getFallbackPoliciesMini,
} from '../services/api';
import type { ApiKey, ModelConfig, FallbackPolicyMini } from '../types';

const getQuotaColor = (percent: number): string => {
  if (percent < 50) return 'var(--success)';
  if (percent < 80) return 'var(--warn)';
  return 'var(--danger)';
};

interface NewKeyForm {
  name: string;
  models: string;
  fallback_policy: string;
  quota: number;
}

interface EditKeyForm {
  name: string;
  models: string;
  fallback_policy: string;
  quota: number;
}

const permToForm = (perms: string[]): string =>
  perms.length === 0 ? 'all' : perms.join(',');

const formToPerm = (s: string): string[] =>
  s === 'all' || !s.trim() ? [] : s.split(',').map((x) => x.trim()).filter(Boolean);

export const APIKeysPage: React.FC = () => {
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [policies, setPolicies] = useState<FallbackPolicyMini[]>([]);
  const [loading, setLoading] = useState(true);
  const [createModal, setCreateModal] = useState(false);
  const [editKey, setEditKey] = useState<ApiKey | null>(null);
  const [newKey, setNewKey] = useState<NewKeyForm>({
    name: '',
    models: 'all',
    fallback_policy: '',
    quota: 1000000,
  });
  const [editForm, setEditForm] = useState<EditKeyForm>({
    name: '',
    models: 'all',
    fallback_policy: '',
    quota: 0,
  });
  const [creating, setCreating] = useState(false);
  const [saving, setSaving] = useState(false);
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const fetchAll = async () => {
    setLoading(true);
    try {
      const [ks, ms, ps] = await Promise.all([
        getApiKeys(),
        getModels(),
        getFallbackPoliciesMini(),
      ]);
      setKeys(ks);
      setModels(ms);
      setPolicies(ps);
    } catch (err) {
      console.error('Failed to fetch data:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchAll();
  }, []);

  const handleCreate = async () => {
    if (!newKey.name.trim()) return;
    setCreating(true);
    try {
      const result = await createApiKey({
        name: newKey.name,
        model_permissions: formToPerm(newKey.models),
        fallback_policy: newKey.fallback_policy || undefined,
        quota_limit: Number(newKey.quota),
      });
      setCreatedKey(result.raw_key);
      fetchAll();
    } catch (err) {
      console.error('Failed to create API key:', err);
    } finally {
      setCreating(false);
    }
  };

  const openEditModal = (key: ApiKey) => {
    setEditKey(key);
    setEditForm({
      name: key.name,
      models: permToForm(key.model_permissions),
      fallback_policy: key.fallback_policy || '',
      quota: key.quota_limit,
    });
  };

  const handleUpdate = async () => {
    if (!editKey) return;
    setSaving(true);
    try {
      await updateApiKey(editKey.id, {
        name: editForm.name,
        model_permissions: formToPerm(editForm.models),
        // 始终更新：传 Some(string|null) — 后端用 None / Some 区分
        fallback_policy: editForm.fallback_policy || null,
        quota_limit: Number(editForm.quota),
      });
      setEditKey(null);
      fetchAll();
    } catch (err) {
      console.error('Failed to update API key:', err);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm('确定要撤销此密钥吗？')) return;
    try {
      await deleteApiKey(id);
      fetchAll();
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

  // List rows can't show the full key (backend only sends the 15-char prefix
  // + masked form for security). When the operator clicks "复制" we call the
  // /reveal endpoint, which returns the plaintext, then put it on the
  // clipboard. Confirm the action so a stray click doesn't leak the key into
  // the clipboard unobserved.
  const copyListedKey = async (key: ApiKey) => {
    if (!window.confirm(`复制「${key.name}」的明文密钥？\n\n此操作会把完整 key 写入剪贴板。`)) {
      return;
    }
    try {
      const plaintext = await revealApiKey(key.id);
      await navigator.clipboard.writeText(plaintext);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (err) {
      console.error('Failed to reveal key:', err);
      window.alert('复制失败：' + (err instanceof Error ? err.message : String(err)));
    }
  };

  return (
    <div className="section" id="section-apikeys">
      <div className="section-header">
        <div>
          <h2 className="section-title">API 密钥</h2>
          <p className="section-desc">创建与管理访问密钥（持久化到 config.yaml）</p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => {
            setCreateModal(true);
            setCreatedKey(null);
          }}
        >
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
                <th>Fallback 链</th>
                <th className="num-col">剩余配额</th>
                <th>创建时间</th>
                <th className="actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={7} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
                    加载中...
                  </td>
                </tr>
              ) : keys.length === 0 ? (
                <tr>
                  <td colSpan={7} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
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
                          onClick={() => copyListedKey(key)}
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
                    <td className="meta">{key.fallback_policy || '—'}</td>
                    <td className="num-col">
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <div
                          style={{
                            flex: 1,
                            height: 4,
                            background: 'var(--border)',
                            borderRadius: 2,
                            overflow: 'hidden',
                          }}
                        >
                          <div
                            style={{
                              width: `${Math.min(100, key.quota_percent)}%`,
                              height: '100%',
                              background: getQuotaColor(key.quota_percent),
                              borderRadius: 2,
                            }}
                          ></div>
                        </div>
                        <span className="meta">{key.quota_percent.toFixed(0)}%</span>
                      </div>
                    </td>
                    <td className="meta">{new Date(key.created_at * 1000).toLocaleDateString()}</td>
                    <td className="actions">
                      <Button variant="secondary" size="sm" onClick={() => openEditModal(key)}>
                        编辑
                      </Button>
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

      {/* Create Modal */}
      <Modal
        isOpen={createModal}
        onClose={() => {
          setCreateModal(false);
          setCreatedKey(null);
        }}
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
            <div className="field">
              <label>模型权限</label>
              <select
                className="select"
                value={newKey.models}
                onChange={(e) => setNewKey({ ...newKey, models: e.target.value })}
              >
                <option value="all">全部模型</option>
                {models.map((m) => (
                  <option key={m.id} value={m.id}>{m.name} ({m.id})</option>
                ))}
              </select>
              <span className="field-hint">如需多选：'mimo,gpt-4o'</span>
            </div>
            <div className="field">
              <label>Fallback 链（覆盖模型默认）</label>
              <select
                className="select"
                value={newKey.fallback_policy}
                onChange={(e) => setNewKey({ ...newKey, fallback_policy: e.target.value })}
              >
                <option value="">（使用模型默认）</option>
                {policies.map((p) => (
                  <option key={p.id} value={p.id}>{p.id}</option>
                ))}
              </select>
            </div>
            <Input
              label="配额上限"
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

      {/* Edit Modal */}
      <Modal
        isOpen={!!editKey}
        onClose={() => setEditKey(null)}
        title={`编辑密钥：${editKey?.name || ''}`}
        footer={
          <>
            <Button variant="secondary" onClick={() => setEditKey(null)}>取消</Button>
            <Button onClick={handleUpdate} disabled={saving}>
              {saving ? '保存中...' : '保存'}
            </Button>
          </>
        }
      >
        {editKey && (
          <>
            <Input
              label="密钥名称"
              value={editForm.name}
              onChange={(e) => setEditForm({ ...editForm, name: e.target.value })}
            />
            <div className="field">
              <label>模型权限</label>
              <select
                className="select"
                value={editForm.models}
                onChange={(e) => setEditForm({ ...editForm, models: e.target.value })}
              >
                <option value="all">全部模型</option>
                {models.map((m) => (
                  <option key={m.id} value={m.id}>{m.name} ({m.id})</option>
                ))}
              </select>
              <span className="field-hint">多选用逗号分隔，例如 'mimo,gpt-4o'</span>
            </div>
            <div className="field">
              <label>Fallback 链（覆盖模型默认）</label>
              <select
                className="select"
                value={editForm.fallback_policy}
                onChange={(e) => setEditForm({ ...editForm, fallback_policy: e.target.value })}
              >
                <option value="">（使用模型默认）</option>
                {policies.map((p) => (
                  <option key={p.id} value={p.id}>{p.id}</option>
                ))}
              </select>
            </div>
            <Input
              label="配额上限"
              type="number"
              mono
              value={editForm.quota}
              onChange={(e) => setEditForm({ ...editForm, quota: Number(e.target.value) })}
              hint="设置为 0 表示不限制"
            />
            <div className="field-hint" style={{ marginTop: 8 }}>
              注：修改 Key 明文需删除后重建
            </div>
          </>
        )}
      </Modal>
    </div>
  );
};
