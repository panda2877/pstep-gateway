# 模型网关管理控制台 - 开发计划

## 项目概述

使用 TypeScript + React 构建模型网关管理控制台前端，1:1 复刻 `protype/model-gateway-admin.html` 原型图。

## 技术栈

- **框架**: React 18 + TypeScript
- **构建工具**: Vite
- **样式**: 原型图中的 CSS 直接迁移 + Tailwind CSS (可选)
- **HTTP 客户端**: axios / fetch
- **状态管理**: React Context + useReducer (或 Zustand)
- **图表**: 原型图中的 SVG 饼图逻辑迁移 (可选 Recharts/ECharts)

---

## 一、页面结构与 UI 组件

### 1.1 整体布局
- [ ] 顶部导航栏 (TopNav)
  - Logo + 标题 "模型网关"
  - 导航菜单：概览 / 模型 / API 密钥 / Fallback
  - 用户头像 (右上角)
- [ ] 主内容区域 (Container)
  - 路由切换不同 section

### 1.2 概览页 (Overview)
- [ ] 统计数据卡片 (StatCard)
  - Token 总计（数值 + 变化趋势）
  - API 成本（数值 + 变化趋势）
  - 模型分布（饼图 + 图例）
- [ ] 时间范围切换 Tab (1天 / 7天 / 30天)
- [ ] 响应式布局 (stats-grid: 1fr 1fr 2fr)

### 1.3 模型配置页 (Models)
- [ ] 数据表格 (ModelsTable)
  - 列：模型名称、供应商、版本、状态、超时、最大Token、操作
  - 状态 Badge：活跃(绿色)、限流中(黄色)、已禁用(灰色)
  - 编辑按钮打开 Modal
- [ ] 模型编辑 Modal
  - 模型名称、供应商、版本（只读）
  - 超时设置、 最大Token 设置
  - 保存/取消按钮

### 1.4 API 密钥页 (API Keys)
- [ ] 数据表格 (KeysTable)
  - 列：名称、密钥(掩码+复制)、模型权限、剩余配额(进度条)、创建时间、操作
  - 密钥复制功能
  - 配额进度条（颜色根据百分比）
  - 撤销按钮
- [ ] 新建密钥 Modal
  - 密钥名称输入框
  - 模型权限下拉选择
  - 月度配额上限输入
  - 创建/取消按钮

### 1.5 Fallback 策略页 (Fallback)
- [ ] 策略卡片列表
  - 策略名称 + 描述
  - 启用/禁用状态 Badge
  - Fallback 链可视化 (ChainNode + ChainArrow)
  - 编辑按钮
- [ ] 新增策略 Modal
- [ ] 策略编辑 Modal
  - 拖拽排序 Chain 节点
  - 添加/删除节点
  - 启用/禁用开关

---

## 二、后端 API 对接

### 2.1 现有后端 API ✅

| 端点 | 方法 | 状态 | 说明 |
|------|------|------|------|
| `/health` | GET | ✅ | 健康检查 |
| `/stats` | GET | ✅ | 用量统计汇总 |
| `/stats/recent` | GET | ✅ | 最近用量记录 |
| `/api/models` | GET | ✅ | 模型列表 + API Key |
| `/api/health` | GET | ✅ | 模型健康状态 |
| `/v1/models` | GET | ✅ | OpenAI 兼容模型列表 |

### 2.2 Admin API ✅ 已完成

#### 概览页 (Overview)
- [x] `GET /api/admin/usage/stats?period=1d|7d|30d` - 用量统计
  ```json
  {"token_total": 0, "token_input": 0, "token_output": 0, "cost": 0.0, "change_percent": 0.0, "period": "7d"}
  ```
- [x] `GET /api/admin/usage/distribution?period=...` - 模型分布
  ```json
  {"models": [{"name": "gpt-4o", "color": "#3fb950", "percent": 45.0, "tokens": 0}], "period": "7d"}
  ```

#### 模型配置页 (Models)
- [x] `GET /api/admin/models` - 获取模型配置列表
- [x] `GET /api/admin/models/{id}` - 获取单个模型配置
- [x] `PUT /api/admin/models/{id}` - 更新模型配置

#### API 密钥页 (API Keys)
- [x] `GET /api/admin/keys` - 获取密钥列表
- [x] `POST /api/admin/keys` - 创建新密钥
  ```json
  // 请求
  {"name": "生产密钥", "model_permissions": ["gpt-4o"], "quota_limit": 1000000}
  // 响应
  {"success": true, "key": {...}, "raw_key": "sk-gw-xxx-yyy"}
  ```
- [x] `DELETE /api/admin/keys/{id}` - 撤销密钥

#### Fallback 策略页 (Fallback)
- [x] `GET /api/admin/fallback/policies` - 获取 Fallback 策略列表
- [x] `POST /api/admin/fallback/policies` - 创建策略
  ```json
  // 请求
  {"name": "高可用", "description": "GPT-4o → Claude", "enabled": true, "chain": [{"provider": "openai", "model": "gpt-4o"}]}
  ```
- [x] `GET /api/admin/fallback/policies/{id}` - 获取单个策略
- [x] `PUT /api/admin/fallback/policies/{id}` - 更新策略
- [x] `DELETE /api/admin/fallback/policies/{id}` - 删除策略

---

## 三、开发任务清单

### Phase 1: 项目初始化
- [ ] 初始化 Vite + React + TypeScript 项目
- [ ] 配置 ESLint + Prettier
- [ ] 安装依赖 (axios, react-router-dom, lucide-react 等)
- [ ] 迁移原型图 CSS 变量到全局样式

### Phase 2: 基础组件开发
- [ ] 实现 Button 组件 (primary, secondary, danger, sm)
- [ ] 实现 Badge 组件 (success, warn, danger)
- [ ] 实现 Card 组件
- [ ] 实现 Modal 组件
- [ ] 实现 Table 组件
- [ ] 实现 Form 组件 (Input, Select, Textarea)
- [ ] 实现 FallbackChain 可视化组件

### Phase 3: 页面开发
- [ ] 开发概览页 (OverviewPage)
- [ ] 开发模型配置页 (ModelsPage)
- [ ] 开发 API 密钥页 (APIKeysPage)
- [ ] 开发 Fallback 策略页 (FallbackPage)

### Phase 4: API 对接
- [ ] 配置 API 服务层 (api/service.ts)
- [ ] 实现概览页数据获取
- [ ] 实现模型配置 CRUD
- [ ] 实现 API 密钥 CRUD
- [ ] 实现 Fallback 策略 CRUD

### Phase 5: 完善与优化
- [ ] 添加加载状态 (Loading)
- [ ] 添加错误处理 (ErrorBoundary)
- [ ] 添加 Toast 通知
- [ ] 响应式适配
- [ ] 主题切换 (可选，深色主题已支持)

---

## 四、文件结构建议

```
frontend/
├── src/
│   ├── components/
│   │   ├── ui/
│   │   │   ├── Button.tsx
│   │   │   ├── Badge.tsx
│   │   │   ├── Card.tsx
│   │   │   ├── Modal.tsx
│   │   │   ├── Table.tsx
│   │   │   ├── Input.tsx
│   │   │   └── Select.tsx
│   │   ├── layout/
│   │   │   ├── TopNav.tsx
│   │   │   └── Container.tsx
│   │   ├── charts/
│   │   │   └── PieChart.tsx
│   │   └── fallback/
│   │       └── FallbackChain.tsx
│   ├── pages/
│   │   ├── OverviewPage.tsx
│   │   ├── ModelsPage.tsx
│   │   ├── APIKeysPage.tsx
│   │   └── FallbackPage.tsx
│   ├── services/
│   │   └── api.ts
│   ├── hooks/
│   │   └── useApi.ts
│   ├── types/
│   │   └── index.ts
│   ├── styles/
│   │   └── globals.css
│   ├── App.tsx
│   └── main.tsx
├── index.html
├── package.json
├── tsconfig.json
└── vite.config.ts
```

---

## 五、关键实现细节

### 5.1 API 密钥复制功能
```typescript
const copyKey = async (text: string) => {
  await navigator.clipboard.writeText(text);
  // 显示 Toast 提示
};
```

### 5.2 饼图渲染
- 原型图使用纯 SVG 实现
- 可直接迁移 `renderCharts()` 函数逻辑
- 或使用 Recharts 库简化

### 5.3 时间范围切换
- Tab 按钮触发 `setRange()`
- 重新请求对应时间段数据
- 更新统计卡片和图表

### 5.4 配额进度条颜色
```typescript
const getQuotaColor = (percent: number) => {
  if (percent < 50) return 'var(--success)';
  if (percent < 80) return 'var(--warn)';
  return 'var(--danger)';
};
```

---

## 六、后端改造建议（已实现）

Admin API 已实现于 `src/admin/` 目录：

| 文件 | 说明 |
|------|------|
| `src/admin/mod.rs` | 模块入口 |
| `src/admin/usage.rs` | 用量统计 API |
| `src/admin/models.rs` | 模型配置 API |
| `src/admin/apikeys.rs` | API 密钥 API |
| `src/admin/fallback.rs` | Fallback 策略 API |

**注意**：当前密钥和策略存储在内存中，服务重启后数据丢失。生产环境需接入数据库。

---

## 七、验收标准

1. [ ] 页面 1:1 还原原型图设计
2. [ ] 所有表单交互正常（新增、编辑、删除）
3. [ ] 密钥复制功能正常
4. [ ] 时间范围切换数据更新正常
5. [ ] Modal 弹窗交互正常
6. [ ] 响应式布局正常（桌面/平板/手机）
7. [ ] 错误处理完善（网络错误、请求失败）
8. [ ] 与后端 API 正确对接

---

## 更新日志

- **2026-06-11**: 后端 Admin API 开发完成，所有接口测试通过