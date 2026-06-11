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

export const ModelsPage: React.FC = () => {
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [loading, setLoading] = useState(true);
  const [editModal, setEditModal] = useState<ModelConfig | null>(null);
  const [formData, setFormData] = useState({ timeout_secs: 30, max_tokens: 128000 });
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
    setFormData({ timeout_secs: model.timeout_secs, max_tokens: model.max_tokens });
  };

  const handleSave = async () => {
    if (!editModal) return;
    setSaving(true);
    try {
      await updateModel(editModal.id, {
        timeout_secs: formData.timeout_secs,
        max_tokens: formData.max_tokens,
      });
      setEditModal(null);
      fetchModels();
    } catch (err) {
      console.error('Failed to update model:', err);
    } finally {
      setSaving(false);
    }
  };

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
                <th>版本</th>
                <th>状态</th>
                <th>超时</th>
                <th>最大 Token</th>
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
                    <td className="meta">{model.version}</td>
                    <td>
                      <Badge variant={getStatusVariant(model.status)}>
                        {getStatusLabel(model.status)}
                      </Badge>
                    </td>
                    <td className="num-col" style={{ textAlign: 'left' }}>{model.timeout_secs}s</td>
                    <td className="num-col" style={{ textAlign: 'left' }}>
                      {model.max_tokens >= 1000 ? `${(model.max_tokens / 1000).toFixed(0)}k` : model.max_tokens}
                    </td>
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
              <Input label="模型名称" value={editModal.name} readOnly />
              <Input label="供应商" value={editModal.provider} readOnly />
            </div>
            <Input label="版本" value={editModal.version} readOnly />
            <div className="field-row">
              <Input
                label="超时 (秒)"
                type="number"
                value={formData.timeout_secs}
                onChange={(e) => setFormData({ ...formData, timeout_secs: Number(e.target.value) })}
              />
              <Input
                label="最大 Token"
                type="number"
                value={formData.max_tokens}
                onChange={(e) => setFormData({ ...formData, max_tokens: Number(e.target.value) })}
              />
            </div>
          </>
        )}
      </Modal>
    </div>
  );
};