pub mod anthropic;
pub mod openai;

use crate::types::{UpstreamConfig, UpstreamType};

/// 下游请求格式（即客户端发起的格式）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    OpenAI,
    Anthropic,
}

/// Stream or non-streaming call to upstream.
/// `downstream_format` is the format the client used to call us.
/// We translate as needed to match `upstream.upstream_type`, then translate
/// the response back to `downstream_format`.
pub async fn proxy(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<String, String> {
    match (upstream.upstream_type, downstream_format) {
        // Upstream = OpenAI
        (UpstreamType::Openai, OutputFormat::OpenAI) => {
            openai::proxy_openai_to_openai(upstream, target_model, body, downstream_format).await
        }
        (UpstreamType::Openai, OutputFormat::Anthropic) => {
            openai::proxy_anthropic_to_openai(upstream, target_model, body).await
        }
        // Upstream = Anthropic
        (UpstreamType::Anthropic, OutputFormat::OpenAI) => {
            anthropic::proxy_openai_to_anthropic(upstream, target_model, body, downstream_format).await
        }
        (UpstreamType::Anthropic, OutputFormat::Anthropic) => {
            anthropic::proxy_anthropic_to_anthropic(upstream, target_model, body).await
        }
    }
}

pub async fn proxy_non_stream(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<String, String> {
    match (upstream.upstream_type, downstream_format) {
        (UpstreamType::Openai, OutputFormat::OpenAI) => {
            openai::proxy_non_stream_openai_to_openai(upstream, target_model, body, downstream_format).await
        }
        (UpstreamType::Openai, OutputFormat::Anthropic) => {
            openai::proxy_non_stream_anthropic_to_openai(upstream, target_model, body).await
        }
        (UpstreamType::Anthropic, OutputFormat::OpenAI) => {
            anthropic::proxy_non_stream_openai_to_anthropic(upstream, target_model, body, downstream_format).await
        }
        (UpstreamType::Anthropic, OutputFormat::Anthropic) => {
            anthropic::proxy_non_stream_anthropic_to_anthropic(upstream, target_model, body).await
        }
    }
}
