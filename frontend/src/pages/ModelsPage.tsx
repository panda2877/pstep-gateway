import React, { useState, useEffect } from 'react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Modal } from '../components/ui/Modal';
import { Input } from '../components/ui/Input';
import { getModels, updateModel } from '../services/api';
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

export const ModelsPage: React.FC = () => {
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [editModal, setEditModal] = useState<ModelConfig | null>(null);
  const [formData, setFormData] = useState<FormState | null>(null);
  const [saving, setSaving] = useState(false);
  const [resultMsg, setResultMsg] = useState<string | null>(null);

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

  const formatPrice = (price?: number) => (price ? `$${price.toFixed(3)}` : '-');

  return (
    <div className="section" id="section-models">
      <div className="section-header">
        <div>
          <h2 className="section-title">模型配置</h2>
          <p className="section-desc">管理模型参数与上游连接</p>
        </div>
      </div>

      <Card>
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
                        : '—'}
                    </td>
                    <td className="num-col" style={{ textAlign: 'left' }}>{formatPrice(model.price_per_input)}</td>
                    <td className="num-col" style={{ textAlign: 'left' }}>{formatPrice(model.price_per_output)}</td>
                    <td className="actions">
                      <Button variant="secondary" size="sm" onClick={() => openEditModal(model)}>
                        编辑
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

            <div className="section-divider"><span>上游配置（变更需重启服务）</span></div>

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
    </div>
  );
};
