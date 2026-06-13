import React, { useState, useEffect } from 'react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Modal } from '../components/ui/Modal';
import { Input } from '../components/ui/Input';
import { getModels, updateModel } from '../services/api';
import type { ModelConfig } from '../types';

const getStatusVariant = (status: string): 'success' | 'warn' | 'default' => {
  switch (status) {
    case 'active':
      return 'success';
    case 'rate_limited':
      return 'warn';
    default:
      return 'default';
  }
};

const getStatusLabel = (status: string): string => {
  switch (status) {
    case 'active':
      return '活跃';
    case 'rate_limited':
      return '限流中';
    default:
      return '已禁用';
  }
};

interface FormState {
  name: string;
  timeout_secs: number;
  price_per_input: number;
  price_per_output: number;
  base_url: string;
  /** 占位 "********" 表示「不修改」；非占位表示要写入的新值 */
  api_key: string;
}

const KEEP_PLACEHOLDER = '********';

export const ModelsPage: React.FC = () => {
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [editModal, setEditModal] = useState<ModelConfig | null>(null);
  const [formData, setFormData] = useState<FormState>({
    name: '',
    timeout_secs: 30,
    price_per_input: 0,
    price_per_output: 0,
    base_url: '',
    api_key: KEEP_PLACEHOLDER,
  });
  const [saving, setSaving] = useState(false);

  const fetchModels = async () => {
    setLoading(true);
    try {
      const data = await getModels();
      setModels(data);
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
    setFormData({
      name: model.name,
      timeout_secs: model.timeout_secs,
      price_per_input: model.price_per_input || 0,
      price_per_output: model.price_per_output || 0,
      base_url: model.base_url || '',
      // 主列表不返回明文 api_key，编辑界面以占位符 "********" 显示
      api_key: KEEP_PLACEHOLDER,
    });
  };

  const handleSave = async () => {
    if (!editModal) return;
    setSaving(true);
    try {
      const payload: Record<string, unknown> = {
        name: formData.name,
        timeout_secs: formData.timeout_secs,
        price_per_input: formData.price_per_input,
        price_per_output: formData.price_per_output,
        base_url: formData.base_url,
      };
      // 仅当用户实际输入了新值时，才提交 api_key
      if (formData.api_key !== KEEP_PLACEHOLDER && formData.api_key !== '') {
        payload.api_key = formData.api_key;
      }
      await updateModel(editModal.id, payload as Parameters<typeof updateModel>[1]);
      setEditModal(null);
      fetchModels();
    } catch (err) {
      console.error('Failed to update model:', err);
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
          <p className="section-desc">管理模型参数与 fallback 策略</p>
        </div>
      </div>

      <Card>
        <div className="table-wrap">
          <table className="ds-table">
            <thead>
              <tr>
                <th>模型</th>
                <th>供应商</th>
                <th>状态</th>
                <th>超时</th>
                <th>输入单价</th>
                <th>输出单价</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td colSpan={7} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
                    加载中...
                  </td>
                </tr>
              ) : models.length === 0 ? (
                <tr>
                  <td colSpan={7} style={{ textAlign: 'center', padding: '20px', color: 'var(--fg-2)' }}>
                    暂无数据
                  </td>
                </tr>
              ) : (
                models.map((model) => (
                  <tr key={model.id}>
                    <td style={{ fontWeight: 500 }}>{model.name}</td>
                    <td className="meta">{model.provider}</td>
                    <td>
                      <Badge variant={getStatusVariant(model.status)}>
                        {getStatusLabel(model.status)}
                      </Badge>
                    </td>
                    <td className="num-col" style={{ textAlign: 'left' }}>{model.timeout_secs}s</td>
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
            <Button onClick={handleSave} disabled={saving}>
              {saving ? '保存中...' : '保存'}
            </Button>
          </>
        }
      >
        {editModal && (
          <>
            <div className="field-row">
              <Input label="模型名称" value={formData.name} onChange={(e) => setFormData({ ...formData, name: e.target.value })} />
              <Input label="供应商" value={editModal.provider} readOnly />
            </div>
            <div className="field-row">
              <Input
                label="超时 (秒)"
                type="number"
                value={formData.timeout_secs}
                onChange={(e) => setFormData({ ...formData, timeout_secs: Number(e.target.value) })}
              />
              <Input label="状态" value={editModal.status} readOnly />
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
            <div className="field-row">
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
                hint={editModal.api_key_configured ? `当前已配置（${editModal.api_key_masked || '****'}），留空或保持占位符表示不修改` : '尚未配置'}
              />
            </div>
          </>
        )}
      </Modal>
    </div>
  );
};
