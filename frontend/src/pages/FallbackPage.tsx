import React, { useState, useEffect } from 'react';
import { Plus } from 'lucide-react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Modal } from '../components/ui/Modal';
import { Input } from '../components/ui/Input';
import { FallbackChain } from '../components/fallback/FallbackChain';
import {
  getFallbackPolicies,
  createFallbackPolicy,
  updateFallbackPolicy,
  deleteFallbackPolicy,
  getModels,
} from '../services/api';
import type { FallbackPolicy, ChainNode, ModelConfig } from '../types';

interface FormState {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  chain: ChainNode[];
}

const EMPTY: FormState = {
  id: '',
  name: '',
  description: '',
  enabled: true,
  chain: [],
};

export const FallbackPage: React.FC = () => {
  const [policies, setPolicies] = useState<FallbackPolicy[]>([]);
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [createModal, setCreateModal] = useState(false);
  const [editPolicy, setEditPolicy] = useState<FallbackPolicy | null>(null);
  const [formData, setFormData] = useState<FormState>(EMPTY);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchAll = async () => {
    setLoading(true);
    try {
      const [ps, ms] = await Promise.all([getFallbackPolicies(), getModels()]);
      setPolicies(ps);
      setModels(ms);
    } catch (err) {
      console.error('Failed to fetch fallback data:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchAll();
  }, []);

  const openCreateModal = () => {
    setFormData(EMPTY);
    setError(null);
    setCreateModal(true);
  };

  const openEditModal = (policy: FallbackPolicy) => {
    setEditPolicy(policy);
    setFormData({
      id: policy.id,
      name: policy.name,
      description: policy.description,
      enabled: policy.enabled,
      chain: [...policy.chain],
    });
    setError(null);
  };

  const addChainNode = () => {
    const first = models[0];
    if (!first) return;
    setFormData({
      ...formData,
      // v0.3: 移除 upstream_type 字段；用 model id 作为语义标签
      chain: [...formData.chain, { upstream: first.id, model: first.upstream_model }],
    });
  };

  const removeChainNode = (index: number) => {
    setFormData({
      ...formData,
      chain: formData.chain.filter((_, i) => i !== index),
    });
  };

  const updateChainNode = (index: number, node: ChainNode) => {
    const newChain = [...formData.chain];
    newChain[index] = node;
    setFormData({ ...formData, chain: newChain });
  };

  const handleCreate = async () => {
    if (!formData.id.trim() || !formData.name.trim() || formData.chain.length === 0) {
      setError('id、名称、链节点均不能为空');
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await createFallbackPolicy({
        id: formData.id,
        name: formData.name,
        description: formData.description,
        enabled: formData.enabled,
        chain: formData.chain,
      });
      setCreateModal(false);
      fetchAll();
    } catch (err: any) {
      const msg = err?.response?.data?.message || '创建失败';
      setError(msg);
    } finally {
      setSaving(false);
    }
  };

  const handleUpdate = async () => {
    if (!editPolicy) return;
    if (!formData.name.trim()) {
      setError('名称不能为空');
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await updateFallbackPolicy(editPolicy.id, {
        name: formData.name,
        description: formData.description,
        enabled: formData.enabled,
        chain: formData.chain,
      });
      setEditPolicy(null);
      fetchAll();
    } catch (err: any) {
      const msg = err?.response?.data?.message || '保存失败';
      setError(msg);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm('确定要删除此策略吗？')) return;
    try {
      await deleteFallbackPolicy(id);
      fetchAll();
    } catch (err: any) {
      const msg = err?.response?.data?.message || '删除失败';
      alert(msg);
    }
  };

  return (
    <div className="section" id="section-fallback">
      <div className="section-header">
        <div>
          <h2 className="section-title">Fallback 策略</h2>
          <p className="section-desc">设置模型故障时的自动切换链（与 config.yaml 持久化）</p>
        </div>
        <Button variant="secondary" size="sm" onClick={openCreateModal}>
          <Plus size={14} />
          新增策略
        </Button>
      </div>

      <div className="stack">
        {loading ? (
          <Card><div style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>加载中...</div></Card>
        ) : policies.length === 0 ? (
          <Card><div style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>暂无策略</div></Card>
        ) : (
          policies.map((policy) => (
            <Card key={policy.id}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 'var(--gap-sm)' }}>
                <div>
                  <div style={{ fontWeight: 600, marginBottom: 4 }}>{policy.id} · {policy.name}</div>
                  <div style={{ fontSize: 12, color: 'var(--fg-2)' }}>
                    {policy.description || policy.chain.map(n => n.model).join(' → ')}
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                  <Badge variant={policy.enabled ? 'success' : 'default'}>
                    {policy.enabled ? '已启用' : '已禁用'}
                  </Badge>
                  <Button variant="secondary" size="sm" onClick={() => openEditModal(policy)}>编辑</Button>
                </div>
              </div>
              <FallbackChain chain={policy.chain} showError />
            </Card>
          ))
        )}
      </div>

      {/* Create Modal */}
      <Modal
        isOpen={createModal}
        onClose={() => setCreateModal(false)}
        title="新增策略"
        footer={
          <>
            <Button variant="secondary" onClick={() => setCreateModal(false)}>取消</Button>
            <Button
              onClick={handleCreate}
              disabled={saving || !formData.id.trim() || !formData.name.trim() || formData.chain.length === 0}
            >
              {saving ? '保存中...' : '创建'}
            </Button>
          </>
        }
      >
        <Input
          label="策略 id (英文/数字/下划线)"
          placeholder="例如：high_availability"
          value={formData.id}
          onChange={(e) => setFormData({ ...formData, id: e.target.value })}
        />
        <Input
          label="显示名称"
          placeholder="例如：高可用"
          value={formData.name}
          onChange={(e) => setFormData({ ...formData, name: e.target.value })}
        />
        <Input
          label="描述 (可选)"
          placeholder="描述此策略的用途"
          value={formData.description}
          onChange={(e) => setFormData({ ...formData, description: e.target.value })}
        />
        <div className="field">
          <label>Fallback 链</label>
          <div style={{ marginTop: 8 }}>
            {formData.chain.map((node, i) => (
              <div key={i} style={{ display: 'flex', gap: 8, marginBottom: 8, alignItems: 'center' }}>
                <select
                  className="select"
                  style={{ flex: 1 }}
                  value={node.model}
                  onChange={(e) => {
                    const found = models.find((m) => m.upstream_model === e.target.value);
                    if (found) {
                      // v0.3: 移除 upstream_type 字段；用 model id 作为语义标签
                      updateChainNode(i, { upstream: found.id, model: found.upstream_model });
                    }
                  }}
                >
                  {models.map((m) => (
                    <option key={m.upstream_model} value={m.upstream_model}>
                      {m.upstream_model} ({m.name})
                    </option>
                  ))}
                </select>
                <Button variant="danger" size="sm" onClick={() => removeChainNode(i)}>×</Button>
              </div>
            ))}
            <button className="chain-add" onClick={addChainNode} disabled={models.length === 0}>
              <Plus size={14} /> 添加节点
            </button>
          </div>
        </div>
        {error && <div className="field-hint" style={{ color: '#c00', marginTop: 8 }}>{error}</div>}
      </Modal>

      {/* Edit Modal */}
      <Modal
        isOpen={!!editPolicy}
        onClose={() => setEditPolicy(null)}
        title={`编辑策略：${editPolicy?.id || ''}`}
        footer={
          <>
            <Button variant="danger" size="sm" onClick={() => editPolicy && handleDelete(editPolicy.id)}>
              删除
            </Button>
            <div style={{ flex: 1 }} />
            <Button variant="secondary" onClick={() => setEditPolicy(null)}>取消</Button>
            <Button onClick={handleUpdate} disabled={saving || !formData.name.trim()}>
              {saving ? '保存中...' : '保存'}
            </Button>
          </>
        }
      >
        {editPolicy && (
          <>
            <Input
              label="显示名称"
              value={formData.name}
              onChange={(e) => setFormData({ ...formData, name: e.target.value })}
            />
            <Input
              label="描述 (可选)"
              value={formData.description}
              onChange={(e) => setFormData({ ...formData, description: e.target.value })}
            />
            <div className="field">
              <label>启用</label>
              <label style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <input
                  type="checkbox"
                  checked={formData.enabled}
                  onChange={(e) => setFormData({ ...formData, enabled: e.target.checked })}
                />
                {formData.enabled ? '已启用' : '已禁用'}
              </label>
            </div>
            <div className="field">
              <label>Fallback 链</label>
              <div style={{ marginTop: 8 }}>
                {formData.chain.map((node, i) => (
                  <div key={i} style={{ display: 'flex', gap: 8, marginBottom: 8, alignItems: 'center' }}>
                    <select
                      className="select"
                      style={{ flex: 1 }}
                      value={node.model}
                      onChange={(e) => {
                        const found = models.find((m) => m.upstream_model === e.target.value);
                        if (found) {
                          // v0.3: 移除 upstream_type 字段；用 model id 作为语义标签
                          updateChainNode(i, { upstream: found.id, model: found.upstream_model });
                        }
                      }}
                    >
                      {models.map((m) => (
                        <option key={m.upstream_model} value={m.upstream_model}>
                          {m.upstream_model} ({m.name})
                        </option>
                      ))}
                    </select>
                    <Button variant="danger" size="sm" onClick={() => removeChainNode(i)}>×</Button>
                  </div>
                ))}
                <button className="chain-add" onClick={addChainNode} disabled={models.length === 0}>
                  <Plus size={14} /> 添加节点
                </button>
              </div>
            </div>
            {error && <div className="field-hint" style={{ color: '#c00', marginTop: 8 }}>{error}</div>}
          </>
        )}
      </Modal>
    </div>
  );
};
