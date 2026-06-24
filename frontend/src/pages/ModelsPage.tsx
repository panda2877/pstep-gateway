import React, { useState, useEffect } from 'react';
import { Plus } from 'lucide-react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Modal } from '../components/ui/Modal';
import { Input } from '../components/ui/Input';
import { getModels, updateModel, createModel, deleteModel } from '../services/api';
import type { ModelConfig } from '../types';

const KEEP_PLACEHOLDER = '********';

const STATUS_LABEL: Record<string, string> = {
  active: '活跃',
  rate_limited: '限流中',
  disabled: '已禁用',
};

interface FormState {
  name: string;
  status: 'active' | 'rate_limited' | 'disabled';
  price_per_input: number;
  price_per_output: number;
  base_url: string;
  model: string;
  api_key: string;
}

const buildFormState = (m: ModelConfig): FormState => ({
  name: m.name,
  status: (m.status as FormState['status']) || 'active',
  price_per_input: m.price_per_input || 0,
  price_per_output: m.price_per_output || 0,
  base_url: m.base_url || '',
  model: m.upstream_model || '',
  api_key: KEEP_PLACEHOLDER,
});

interface CreateFormState {
  id: string;
  type: 'openai' | 'anthropic';
  name: string;
  base_url: string;
  model: string;
  api_key: string;
  status: 'active' | 'rate_limited' | 'disabled';
  price_per_input: number;
  price_per_output: number;
}

const EMPTY_CREATE: CreateFormState = {
  id: '',
  type: 'openai',
  name: '',
  base_url: '',
  model: '',
  api_key: '',
  status: 'active',
  price_per_input: 0,
  price_per_output: 0,
};

export const ModelsPage: React.FC = () => {
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [editModal, setEditModal] = useState<ModelConfig | null>(null);
  const [formData, setFormData] = useState<FormState | null>(null);
  const [saving, setSaving] = useState(false);
  const [resultMsg, setResultMsg] = useState<string | null>(null);

  // Create modal state
  const [createModal, setCreateModal] = useState(false);
  const [createForm, setCreateForm] = useState<CreateFormState>(EMPTY_CREATE);
  const [createError, setCreateError] = useState<string | null>(null);

  const fetchModels = async () => {
    setLoading(true);
    try {
      const ms = await getModels();
      setModels(ms);
    } catch (err) {
      console.error('Failed to fetch models:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchModels();
  }, []);

  const openEditModal = (model: ModelConfig) => {
    setEditModal(model);
    setFormData(buildFormState(model));
    setResultMsg(null);
  };

  const handleSave = async () => {
    if (!editModal || !formData) return;
    setSaving(true);
    setResultMsg(null);
    try {
      const payload: Record<string, unknown> = {
        name: formData.name,
        status: formData.status,
        price_per_input: formData.price_per_input,
        price_per_output: formData.price_per_output,
        base_url: formData.base_url,
        model: formData.model,
      };
      if (formData.api_key !== KEEP_PLACEHOLDER && formData.api_key !== '') {
        payload.api_key = formData.api_key;
      }
      const resp = await updateModel(editModal.id, payload as Parameters<typeof updateModel>[1]);
      setResultMsg(resp.message);
      fetchModels();
    } catch (err) {
      console.error('Failed to update model:', err);
      setResultMsg('保存失败');
    } finally {
      setSaving(false);
    }
  };

  const handleCreate = async () => {
    if (!createForm.id.trim() || !createForm.base_url.trim() || !createForm.model.trim()) {
      setCreateError('id、base_url、model 不能为空');
      return;
    }
    setSaving(true);
    setCreateError(null);
    try {
      await createModel({
        id: createForm.id,
        type: createForm.type,
        base_url: createForm.base_url,
        api_key: createForm.api_key,
        model: createForm.model,
        name: createForm.name || undefined,
        status: createForm.status,
        price_per_input: createForm.price_per_input || undefined,
        price_per_output: createForm.price_per_output || undefined,
      });
      setCreateModal(false);
      setCreateForm(EMPTY_CREATE);
      fetchModels();
    } catch (err: any) {
      const msg = err?.response?.data?.message || '创建失败';
      setCreateError(msg);
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm(`确定要删除模型 "${id}" 吗？`)) return;
    try {
      await deleteModel(id);
      fetchModels();
    } catch (err: any) {
      const msg = err?.response?.data?.message || '删除失败';
      alert(msg);
    }
  };

  const formatPrice = (price?: number) => {
    if (!price) return '-';
    const fixed = Number(price.toFixed(4));
    const str = fixed.toString();
    return `$${str}`;
  };

  return (
    <div className="section" id="section-models">
      <div className="section-header">
        <div>
          <h2 className="section-title">模型配置</h2>
          <p className="section-desc">管理模型参数与上游连接（持久化到 SQLite）</p>
        </div>
        <Button variant="secondary" size="sm" onClick={() => { setCreateForm(EMPTY_CREATE); setCreateError(null); setCreateModal(true); }}>
          <Plus size={14} />
          新增模型
        </Button>
      </div>

      <Card className="tight">
        <div className="table-wrap">
          <table className="ds-table">
            <thead>
              <tr>
                <th>模型</th>
                <th>状态</th>
                <th>被引用策略</th>
                <th className="num-col">输入单价</th>
                <th className="num-col">输出单价</th>
                <th className="actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={6} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
                    加载中...
                  </td>
                </tr>
              ) : models.length === 0 ? (
                <tr>
                  <td colSpan={6} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
                    暂无数据
                  </td>
                </tr>
              ) : (
                models.map((model) => (
                  <tr key={model.id}>
                    <td style={{ fontWeight: 500 }}>{model.name}</td>
                    <td>
                      <Badge variant={model.status === 'active' ? 'success' : model.status === 'rate_limited' ? 'warn' : 'default'}>
                        {STATUS_LABEL[model.status] || model.status}
                      </Badge>
                    </td>
                    <td className="meta">
                      {model.referenced_by_policies && model.referenced_by_policies.length > 0
                        ? model.referenced_by_policies.join(', ')
                        : <span style={{ color: 'var(--muted)' }}>—</span>}
                    </td>
                    <td className="num-col">{formatPrice(model.price_per_input)}</td>
                    <td className="num-col">{formatPrice(model.price_per_output)}</td>
                    <td className="actions">
                      <div style={{ display: 'flex', gap: 4 }}>
                        <Button variant="ghost" size="sm" onClick={() => openEditModal(model)}>
                          编辑
                        </Button>
                        <Button variant="danger" size="sm" onClick={() => handleDelete(model.id)}>
                          删除
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </Card>

      {/* Edit Modal */}
      <Modal
        isOpen={!!editModal}
        onClose={() => setEditModal(null)}
        title="编辑模型配置"
        footer={
          <>
            <Button variant="secondary" onClick={() => setEditModal(null)}>取消</Button>
            <Button onClick={handleSave} disabled={saving || !formData}>
              {saving ? '保存中...' : '保存'}
            </Button>
          </>
        }
      >
        {editModal && formData && (
          <>
            <Input
              label="模型名称"
              value={formData.name}
              onChange={(e) => setFormData({ ...formData, name: e.target.value })}
            />

            <div className="field">
              <label>状态</label>
              <select
                className="select"
                value={formData.status}
                onChange={(e) =>
                  setFormData({ ...formData, status: e.target.value as FormState['status'] })
                }
              >
                <option value="active">活跃</option>
                <option value="rate_limited">限流中</option>
                <option value="disabled">已禁用</option>
              </select>
            </div>

            <div className="field-row">
              <Input
                label="输入单价 ($/1M tokens)"
                type="number"
                step="0.001"
                value={formData.price_per_input}
                onChange={(e) => setFormData({ ...formData, price_per_input: Number(e.target.value) })}
              />
              <Input
                label="输出单价 ($/1M tokens)"
                type="number"
                step="0.001"
                value={formData.price_per_output}
                onChange={(e) => setFormData({ ...formData, price_per_output: Number(e.target.value) })}
              />
            </div>

            <div className="section-divider"><span>上游配置</span></div>

            <Input
              label="上游模型 id"
              value={formData.model}
              placeholder="例如：claude-3-5-sonnet-20241022"
              onChange={(e) => setFormData({ ...formData, model: e.target.value })}
            />

            <Input
              label="Base URL"
              value={formData.base_url}
              placeholder="https://api.openai.com/v1"
              onChange={(e) => setFormData({ ...formData, base_url: e.target.value })}
              hint="上游服务的 API 地址"
            />

            <Input
              label="API Key"
              type="password"
              mono
              value={formData.api_key}
              placeholder={KEEP_PLACEHOLDER}
              onChange={(e) => setFormData({ ...formData, api_key: e.target.value })}
              hint={
                editModal.api_key_configured
                  ? `当前已配置（${editModal.api_key_masked || '****'}），留空或保持占位符表示不修改`
                  : '尚未配置'
              }
            />

            {resultMsg && (
              <div className="field-hint" style={{ marginTop: 8, color: 'var(--fg-2)' }}>
                {resultMsg}
              </div>
            )}
          </>
        )}
      </Modal>

      {/* Create Modal */}
      <Modal
        isOpen={createModal}
        onClose={() => setCreateModal(false)}
        title="新增模型"
        footer={
          <>
            <Button variant="secondary" onClick={() => setCreateModal(false)}>取消</Button>
            <Button
              onClick={handleCreate}
              disabled={saving || !createForm.id.trim() || !createForm.base_url.trim() || !createForm.model.trim()}
            >
              {saving ? '创建中...' : '创建'}
            </Button>
          </>
        }
      >
        <Input
          label="模型 id（英文/数字/下划线/短横线）"
          placeholder="例如：mimo-v2.5"
          value={createForm.id}
          onChange={(e) => setCreateForm({ ...createForm, id: e.target.value })}
        />
        <div className="field">
          <label>上游类型</label>
          <select
            className="select"
            value={createForm.type}
            onChange={(e) => setCreateForm({ ...createForm, type: e.target.value as 'openai' | 'anthropic' })}
          >
            <option value="openai">OpenAI 兼容</option>
            <option value="anthropic">Anthropic</option>
          </select>
        </div>
        <Input
          label="模型名称（显示用）"
          placeholder="例如：Mimo"
          value={createForm.name}
          onChange={(e) => setCreateForm({ ...createForm, name: e.target.value })}
        />
        <Input
          label="上游模型 id"
          placeholder="例如：mimo-v2.5"
          value={createForm.model}
          onChange={(e) => setCreateForm({ ...createForm, model: e.target.value })}
        />
        <Input
          label="Base URL"
          placeholder="https://api.openai.com/v1"
          value={createForm.base_url}
          onChange={(e) => setCreateForm({ ...createForm, base_url: e.target.value })}
        />
        <Input
          label="API Key"
          type="password"
          mono
          value={createForm.api_key}
          placeholder="sk-..."
          onChange={(e) => setCreateForm({ ...createForm, api_key: e.target.value })}
        />
        <div className="field">
          <label>状态</label>
          <select
            className="select"
            value={createForm.status}
            onChange={(e) => setCreateForm({ ...createForm, status: e.target.value as 'active' | 'rate_limited' | 'disabled' })}
          >
            <option value="active">活跃</option>
            <option value="rate_limited">限流中</option>
            <option value="disabled">已禁用</option>
          </select>
        </div>
        <div className="field-row">
          <Input
            label="输入单价 ($/1M tokens)"
            type="number"
            step="0.001"
            value={createForm.price_per_input}
            onChange={(e) => setCreateForm({ ...createForm, price_per_input: Number(e.target.value) })}
          />
          <Input
            label="输出单价 ($/1M tokens)"
            type="number"
            step="0.001"
            value={createForm.price_per_output}
            onChange={(e) => setCreateForm({ ...createForm, price_per_output: Number(e.target.value) })}
          />
        </div>
        {createError && <div className="field-hint" style={{ color: '#c00', marginTop: 8 }}>{createError}</div>}
      </Modal>
    </div>
  );
};
