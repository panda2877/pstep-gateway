use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============= Core Config =============

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub port: u16,
    #[serde(default)]
    pub public_url: Option<String>,

    pub models: HashMap<String, ModelRoute>,

    /// 可命名的 fallback 策略，可被 model 或 client_api_key 引用
    #[serde(default)]
    pub fallback_policies: HashMap<String, FallbackPolicyConfig>,

    /// 客户端 API Key（持久化）。运行期 quota_used 在内存中，**不**写盘。
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub client_api_keys: HashMap<String, ClientApiKeyConfig>,

    pub usage_tracking: UsageConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thaw: Option<ThawConfig>,

    /// SQLite 数据库文件路径。None = 保持旧行为（纯内存，重启丢数据）。
    /// 设置后，usage_records 与 quota_usage 会持久化到该文件。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_db: Option<String>,

    /// 兼容旧 config：旧版本顶层有 `upstreams` HashMap；新版本合并到 model 上。
    /// 读取时通过 `load_config` 迁移，结构上不再持有。
    #[serde(default, skip_serializing)]
    pub _legacy_upstreams: Option<serde_yaml::Value>,
}

/// 一个对外暴露的 model id，4 字段（type / base_url / api_key / model）扁平在自身。
///
/// **v0.4 路由语义**:
/// - 客户端请求 `model` 字段**优先**被解释为 fallback policy id（见 `fallback_policies`）。
/// - 命中 policy → 用该 chain 完整路由。
/// - 未命中 policy 但命中 model id → 当作单节点 chain（向后兼容）。
/// - 都不命中 → 400 错误。
///
/// 旧的"client_api_key.fallback_policy = 主失败后追加备选"语义保留。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModelRoute {
    /// `type` / `base_url` / `api_key` / `model` 决定上游签名
    #[serde(rename = "type")]
    pub upstream_type: UpstreamType,
    pub base_url: String,
    pub api_key: String,
    pub model: String,

    #[serde(default)]
    pub metadata: Option<ModelMetadata>,

    /// 兼容旧配置：旧版有 `fallback: '30'` 与 `fallback_chain: [...]`。
    /// 读侧由 `load_config` 转换为新结构，写侧不再输出。
    #[serde(default, skip_serializing)]
    pub _legacy_fallback: Option<String>,
    #[serde(default, skip_serializing)]
    pub _legacy_fallback_chain: Vec<String>,
    #[serde(default, skip_serializing)]
    pub _legacy_fallback_policy: Option<String>,
    #[serde(default, skip_serializing)]
    pub _legacy_upstream: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamType {
    #[default]
    Openai,
    Anthropic,
}

impl UpstreamType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn auth_header(&self) -> &'static str {
        match self {
            Self::Openai => "bearer",
            Self::Anthropic => "x-api-key",
        }
    }
}

/// Providers 层的「投影」：仅供 router/providers 之间传递用，不参与持久化。
/// `ModelRoute` 通过 `as_upstream()` 借用得到。
#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    pub upstream_type: UpstreamType,
    pub base_url: String,
    pub api_key: String,
}

impl ModelRoute {
    /// 投影为 providers 需要的 `UpstreamConfig`（零拷贝借用）。
    pub fn as_upstream(&self) -> UpstreamConfig {
        UpstreamConfig {
            upstream_type: self.upstream_type,
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModelMetadata {
    #[serde(default)]
    pub name: Option<String>,
    /// 枚举小写字符串: `active` | `rate_limited` | `disabled`
    #[serde(default)]
    pub status: ModelStatus,
    #[serde(default)]
    pub price_per_input: Option<f64>,
    #[serde(default)]
    pub price_per_output: Option<f64>,

    #[serde(default, skip_serializing)]
    pub _legacy_reasoning: Option<bool>,
    #[serde(default, skip_serializing)]
    pub _legacy_input: Vec<String>,
    #[serde(default, skip_serializing)]
    pub _legacy_context_window: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    #[default]
    Active,
    RateLimited,
    Disabled,
}

impl ModelStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::RateLimited => "rate_limited",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FallbackPolicyConfig {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub chain: Vec<ChainNodeConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChainNodeConfig {
    /// 语义标签（如 anthropic / mimo / openai），仅用于 UI 展示
    pub upstream: String,
    /// 对应 `models` map 的 key（即对外的 model id）
    pub model: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClientApiKeyConfig {
    pub name: String,
    /// 明文 key；list API 返回 `key_masked` / `key_prefix`
    pub key: String,
    /// 可用模型 id 列表；空 = 全部
    #[serde(default)]
    pub model_permissions: Vec<String>,
    /// 该 Key 专用 fallback 链（覆盖模型默认）
    #[serde(default)]
    pub fallback_policy: Option<String>,
    /// 配额上限 token 数；0 = 不限
    pub quota_limit: u64,
    /// 秒时间戳
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UsageConfig {
    pub enabled: bool,
    pub retention_hours: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThawConfig {
    #[serde(default = "default_freeze_duration")]
    pub freeze_duration_minutes: u32,
    #[serde(default = "default_recovery_threshold")]
    pub recovery_threshold: f32,
    #[serde(default)]
    pub min_requests_to_freeze: u64,
    #[serde(default = "default_recovering_attempts")]
    pub recovering_attempts: u8,
}

fn default_freeze_duration() -> u32 {
    15
}
fn default_recovery_threshold() -> f32 {
    0.8
}
fn default_recovering_attempts() -> u8 {
    3
}

// ============= Runtime types =============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHealthStatus {
    pub state: String,
    pub success_rate: f32,
    pub total_requests: u64,
    pub failed_requests: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frozen_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub model: String,
    pub upstream: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub timestamp: u64,
    pub success: bool,
    pub latency_ms: u64,
}

/// Token usage extracted from upstream response body.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

impl TokenUsage {
    /// Extract token usage from an OpenAI-format response body (non-streaming JSON).
    pub fn from_openai_response(body: &str) -> Self {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
            return Self::default();
        };
        let usage = json.get("usage");
        Self {
            prompt_tokens: usage
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            completion_tokens: usage
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
        }
    }

    /// Extract token usage from an Anthropic-format response body (non-streaming JSON).
    pub fn from_anthropic_response(body: &str) -> Self {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
            return Self::default();
        };
        let usage = json.get("usage");
        Self {
            prompt_tokens: usage
                .and_then(|u| u.get("input_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            completion_tokens: usage
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
        }
    }

    /// Extract token usage from an OpenAI-format SSE stream.
    /// OpenAI sends `usage` in the final chunk when `stream_options.include_usage`
    /// is set; some providers send it unconditionally. We take the last seen
    /// non-zero usage, since later chunks supersede earlier ones.
    pub fn from_openai_sse(body: &str) -> Self {
        let mut usage = Self::default();
        let mut found = false;
        for line in body.lines() {
            let line = line.trim();
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                continue;
            }
            let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };
            if let Some(u) = json.get("usage") {
                let p = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let c = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if p > 0 || c > 0 || found {
                    usage.prompt_tokens = p;
                    usage.completion_tokens = c;
                    found = true;
                }
            }
        }
        usage
    }

    /// Extract token usage from an Anthropic-format SSE stream.
    /// - `message_start.message.usage.input_tokens` carries the input count
    /// - `message_delta.usage.output_tokens` carries the cumulative output count
    pub fn from_anthropic_sse(body: &str) -> Self {
        let mut prompt_tokens: u32 = 0;
        let mut completion_tokens: u32 = 0;
        let mut current_prefix = "";
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with("event: ") {
                current_prefix = &line[7..];
                continue;
            }
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };
            if current_prefix == "message_start" {
                if let Some(u) = json.get("message").and_then(|m| m.get("usage")) {
                    prompt_tokens = u.get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                }
            }
            if current_prefix == "message_delta" {
                if let Some(u) = json.get("usage") {
                    let out = u.get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    if out > completion_tokens {
                        completion_tokens = out;
                    }
                }
            }
        }
        Self {
            prompt_tokens,
            completion_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub total_requests: u32,
    pub total_prompt_tokens: u32,
    pub total_completion_tokens: u32,
    pub total_tokens: u32,
    pub by_model: HashMap<String, ModelStats>,
    pub by_upstream: HashMap<String, UpstreamStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelStats {
    pub requests: u32,
    pub tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpstreamStats {
    pub requests: u32,
    pub tokens: u32,
}

// ============= API request/response types =============

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionsRequest {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub tools: Option<Vec<Tool>>,
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
    #[serde(default)]
    pub stop: Option<StopValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub role: String,
    pub content: ContentValue,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ContentValue {
    String(String),
    Array(Vec<ContentPart>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub part_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageUrl {
    pub url: String,
    /// OpenAI 风格的可选字段。data URL 情况下可自动从前缀推导。
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    #[serde(rename = "type", default)]
    pub tool_type: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolChoice {
    String(String),
    Object(ToolChoiceObject),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolChoiceObject {
    #[serde(rename = "type")]
    pub choice_type: String,
    pub function: Option<FunctionRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionRef {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StopValue {
    String(String),
    Array(Vec<String>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    #[serde(default)]
    pub system: Option<String>,
    pub max_tokens: u32,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub tools: Option<Vec<AnthropicTool>>,
    #[serde(default)]
    pub tool_choice: Option<AnthropicToolChoice>,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: AnthropicContent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AnthropicContent {
    String(String),
    Array(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// v0.3: 多模态图片
    #[serde(default)]
    pub source: Option<AnthropicImageSource>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicImageSource {
    /// "base64" | "url"
    #[serde(rename = "type")]
    pub source_type: String,
    /// "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    #[serde(default)]
    pub media_type: Option<String>,
    /// base64 数据（type=base64 时）
    #[serde(default)]
    pub data: Option<String>,
    /// 远程 URL（type=url 时）
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "input_schema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicToolChoice {
    #[serde(rename = "type")]
    pub choice_type: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessagesMessage>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub system: Option<AnthropicSystem>,
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AnthropicSystem {
    String(String),
    Array(Vec<serde_json::Value>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicMessagesMessage {
    pub role: String,
    pub content: serde_json::Value,
}

// ============= Admin API Types =============

/// Usage statistics for admin dashboard
#[derive(Debug, Clone, Serialize)]
pub struct AdminUsageStats {
    pub token_total: u64,
    pub token_input: u64,
    pub token_output: u64,
    pub cost: f64,
    pub change_percent: f32,
    pub period: String,
}

/// Model distribution for admin dashboard
#[derive(Debug, Clone, Serialize)]
pub struct ModelDistribution {
    pub name: String,
    pub color: String,
    pub percent: f32,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminDistributionResponse {
    pub models: Vec<ModelDistribution>,
    pub period: String,
}

/// 响应给前端的单条 model 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    pub version: String,
    pub status: String,
    pub timeout_secs: u32,
    pub price_per_input: Option<f64>,
    pub price_per_output: Option<f64>,
    /// 引用的 fallback 策略 id 列表（v0.3: 一个 model 可被多个 policy 引用）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub referenced_by_policies: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_masked: Option<String>,
    #[serde(default)]
    pub api_key_configured: bool,
    /// 实际发到上游的 model id
    #[serde(default)]
    pub upstream_model: String,
}

/// PUT /api/admin/models/:id 的请求体
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateModelConfigRequest {
    pub name: Option<String>,
    pub status: Option<String>,
    pub price_per_input: Option<f64>,
    pub price_per_output: Option<f64>,

    // 需重启字段
    pub upstream_type: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,

    /// 编辑上游 api_key：
    /// - None / "" = 不变
    /// - 特殊占位 "********" = 不变
    /// - 其他值 = 覆盖
    #[serde(default)]
    pub api_key: Option<String>,
}

/// POST /api/admin/models 的请求体
#[derive(Debug, Clone, Deserialize)]
pub struct CreateModelRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub upstream_type: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub price_per_input: Option<f64>,
    #[serde(default)]
    pub price_per_output: Option<f64>,
}

/// API Key 列表项（响应给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub key_masked: String,
    pub model_permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_policy: Option<String>,
    pub quota_limit: u64,
    pub quota_used: u64,
    pub quota_percent: f32,
    pub created_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default)]
    pub model_permissions: Vec<String>,
    #[serde(default)]
    pub fallback_policy: Option<String>,
    pub quota_limit: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateApiKeyResponse {
    pub key: ApiKey,
    pub raw_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub model_permissions: Option<Vec<String>>,
    pub fallback_policy: Option<Option<String>>,
    pub quota_limit: Option<u64>,
}

/// Fallback policy 列表项（响应给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackPolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub chain: Vec<ChainNodeConfig>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateFallbackPolicyRequest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub chain: Vec<ChainNodeConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateFallbackPolicyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub chain: Option<Vec<ChainNodeConfig>>,
}
