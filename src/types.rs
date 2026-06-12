use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub upstreams: HashMap<String, UpstreamConfig>,
    pub models: HashMap<String, ModelRoute>,
    pub usage_tracking: UsageConfig,
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub thaw: Option<ThawConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpstreamConfig {
    #[serde(rename = "type")]
    pub upstream_type: UpstreamType,
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamType {
    Openai,
    Anthropic,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelRoute {
    pub upstream: String,
    pub model: String,
    #[serde(default)]
    pub fallback: Option<String>,  // 兼容旧配置
    #[serde(default)]
    pub fallback_chain: Vec<String>,  // 新增：链式 fallback
    #[serde(default)]
    pub metadata: Option<ModelMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelMetadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub price_per_input: Option<f64>,   // 价格: 每 1M input tokens 的美元价格
    #[serde(default)]
    pub price_per_output: Option<f64>,  // 价格: 每 1M output tokens 的美元价格
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

fn default_freeze_duration() -> u32 { 15 }
fn default_recovery_threshold() -> f32 { 0.8 }
fn default_recovering_attempts() -> u8 { 3 }

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
            // message_start carries input_tokens
            if current_prefix == "message_start" {
                if let Some(u) = json.get("message").and_then(|m| m.get("usage")) {
                    prompt_tokens = u.get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                }
            }
            // message_delta carries cumulative output_tokens
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
        Self { prompt_tokens, completion_tokens }
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

/// Model configuration for admin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub version: String,
    pub status: String,
    pub timeout_secs: u32,
    pub price_per_input: Option<f64>,
    pub price_per_output: Option<f64>,
    pub upstream: String,
    pub fallback_chain: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateModelConfigRequest {
    pub timeout_secs: Option<u32>,
    pub price_per_input: Option<f64>,
    pub price_per_output: Option<f64>,
    pub status: Option<String>,
}

/// API Key for admin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub key_masked: String,
    pub model_permissions: Vec<String>,
    pub quota_limit: u64,
    pub quota_used: u64,
    pub quota_percent: f32,
    pub created_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub model_permissions: Vec<String>,
    pub quota_limit: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateApiKeyResponse {
    pub key: ApiKey,
    pub raw_key: String,
}

/// Fallback policy for admin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackPolicy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub chain: Vec<ChainNode>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainNode {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateFallbackPolicyRequest {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub chain: Vec<ChainNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateFallbackPolicyRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub chain: Option<Vec<ChainNode>>,
}