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
} from '../services/api';
import type { FallbackPolicy, FallbackPolicyCreate, ChainNode } from '../types';

const AVAILABLE_MODELS = [
  { provider: 'OpenAI', model: 'gpt-4o' },
  { provider: 'Anthropic', model: 'claude-3-5-sonnet' },
  { provider: 'Google', model: 'gemini-2.0' },
  { provider: 'DeepSeek', model: 'deepseek-v3' },
];

export const FallbackPage: React.FC = () => {
  const [policies, setPolicies] = useState<FallbackPolicy[]>([]);
  const [loading, setLoading] = useState(true);
  const [createModal, setCreateModal] = useState(false);
  const [editPolicy, setEditPolicy] = useState<FallbackPolicy | null>(null);
  const [formData, setFormData] = useState<FallbackPolicyCreate>({
    name: '',
    description: '',
    enabled: true,
    chain: [],
  });
  const [saving, setSaving] = useState(false);

  const fetchPolicies = async () => {
    setLoading(true);
    try {
      const data = await getFallbackPolicies();
      setPolicies(data);
    } catch (err) {
      console.error('Failed to fetch fallback policies:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchPolicies();
  }, []);

  const openCreateModal = () => {
    setFormData({ name: '', description: '', enabled: true, chain: [] });
    setCreateModal(true);
  };

  const openEditModal = (policy: FallbackPolicy) => {
    setEditPolicy(policy);
    setFormData({
      name: policy.name,
      description: policy.description,
      enabled: policy.enabled,
      chain: [...policy.chain],
    });
  };

  const addChainNode = () => {
    setFormData({
      ...formData,
      chain: [...formData.chain, AVAILABLE_MODELS[0]],
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
    if (!formData.name.trim() || formData.chain.length === 0) return;
    setSaving(true);
    try {
      await createFallbackPolicy(formData);
      setCreateModal(false);
      fetchPolicies();
    } catch (err) {
      console.error('Failed to create fallback policy:', err);
    } finally {
      setSaving(false);
    }
  };

  const handleUpdate = async () => {
    if (!editPolicy || !formData.name.trim()) return;
    setSaving(true);
    try {
      await updateFallbackPolicy(editPolicy.id, formData);
      setEditPolicy(null);
      fetchPolicies();
    } catch (err) {
      console.error('Failed to update fallback policy:', err);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm('确定要删除此策略吗？')) return;
    try {
      await deleteFallbackPolicy(id);
      fetchPolicies();
    } catch (err) {
      console.error('Failed to delete fallback policy:', err);
    }
  };

  return (
    <div className="section" id="section-fallback">
      <div className="section-header">
        <div>
          <h2 className="section-title">Fallback 策略</h2>
          <p className="section-desc">设置模型故障时的自动切换链</p>
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
                  <div style={{ fontWeight: 600, marginBottom: 4 }}>{policy.name}</div>
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
            <Button onClick={handleCreate} disabled={saving || !formData.name.trim() || formData.chain.length === 0}>
              {saving ? '保存中...' : '创建'}
            </Button>
          </>
        }
      >
        <Input
          label="策略名称"
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
                  value={`${node.provider}:${node.model}`}
                  onChange={(e) => {
                    const [provider, model] = e.target.value.split(':');
                    updateChainNode(i, { provider, model });
                  }}
                >
                  {AVAILABLE_MODELS.map((m) => (
                    <option key={`${m.provider}:${m.model}`} value={`${m.provider}:${m.model}`}>
                      {m.provider} - {m.model}
                    </option>
                  ))}
                </select>
                <Button variant="danger" size="sm" onClick={() => removeChainNode(i)}>×</Button>
              </div>
            ))}
            <button className="chain-add" onClick={addChainNode}>
              <Plus size={14} /> 添加节点
            </button>
          </div>
        </div>
      </Modal>

      {/* Edit Modal */}
      <Modal
        isOpen={!!editPolicy}
        onClose={() => setEditPolicy(null)}
        title="编辑策略"
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
              label="策略名称"
              value={formData.name}
              onChange={(e) => setFormData({ ...formData, name: e.target.value })}
            />
            <Input
              label="描述 (可选)"
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
                      value={`${node.provider}:${node.model}`}
                      onChange={(e) => {
                        const [provider, model] = e.target.value.split(':');
                        updateChainNode(i, { provider, model });
                      }}
                    >
                      {AVAILABLE_MODELS.map((m) => (
                        <option key={`${m.provider}:${m.model}`} value={`${m.provider}:${m.model}`}>
                          {m.provider} - {m.model}
                        </option>
                      ))}
                    </select>
                    <Button variant="danger" size="sm" onClick={() => removeChainNode(i)}>×</Button>
                  </div>
                ))}
                <button className="chain-add" onClick={addChainNode}>
                  <Plus size={14} /> 添加节点
                </button>
              </div>
            </div>
          </>
        )}
      </Modal>
    </div>
  );
};