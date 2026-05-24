pub mod anthropic;
pub mod openai;

use crate::types::{UpstreamConfig, UpstreamType};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    OpenAI,
    Anthropic,
}

pub async fn proxy(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    format: OutputFormat,
) -> Result<String, String> {
    match upstream.upstream_type {
        UpstreamType::Openai => openai::proxy(upstream, target_model, body, format).await,
        UpstreamType::Anthropic => anthropic::proxy(upstream, target_model, body, format).await,
    }
}

pub async fn proxy_non_stream(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    format: OutputFormat,
) -> Result<String, String> {
    match upstream.upstream_type {
        UpstreamType::Openai => openai::proxy_non_stream(upstream, target_model, body, format).await,
        UpstreamType::Anthropic => anthropic::proxy_non_stream(upstream, target_model, body, format).await,
    }
}