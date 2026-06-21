use crate::providers::streaming::{BoxError, LineSplit};
use crate::providers::OutputFormat;
use crate::types::{
    AnthropicRequest, AnthropicContent, AnthropicMessage, AnthropicContentBlock,
    AnthropicImageSource, AnthropicTool, AnthropicToolChoice, TokenUsage,
    UpstreamConfig, ChatCompletionsRequest, ContentValue, Message as OaiMessage,
    AnthropicMessagesRequest, AnthropicSystem,
    Tool as OaiTool, ToolChoice, ContentPart, FunctionDef, ImageUrl,
};
use bytes::Bytes;
use futures::stream::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::pin::Pin;
use std::time::Duration;

// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_anthropic_to_openai() {
        let body = r#"{
          "model": "minimax",
          "max_tokens": 50,
          "tools": [{"name": "search", "input_schema": {"type":"object"}}],
          "messages": [
            {"role": "user", "content": "query"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_D", "name": "search", "input": {"q":"x"}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_D", "content": "data"}]}
          ]
        }"#;
        let result = anthropic_request_to_openai_json(body, "MiniMax-M2.7").unwrap();
        eprintln!("=== Case A: tool_result-only user ===\n{}\n=== end ===", result);

        let body2 = r#"{
          "model": "minimax",
          "messages": [
            {"role": "user", "content": [
              {"type": "text", "text": "Please call"},
              {"type": "tool_result", "tool_use_id": "toolu_D", "content": "data"}
            ]}
          ]
        }"#;
        let result2 = anthropic_request_to_openai_json(body2, "MiniMax-M2.7").unwrap();
        eprintln!("=== Case B: text + tool_result ===\n{}\n=== end ===", result2);
        panic!("intentional");
    }

    // ----- streaming-path parity test (Anthropic -> OpenAI) -----------------
    //
    // Mirror of the OpenAI->Anthropic parity test in providers/openai.rs.
    // Drives a real Anthropic SSE stream through LineSplit + the
    // AnthropicToOpenAITranslator and asserts the emitted byte stream is
    // identical to the buffered `convert_anthropic_sse_to_openai` output.

    use crate::providers::streaming::LineSplit;
    use bytes::Bytes;
    use futures::stream;

    /// Drive a synthetic Anthropic SSE byte stream through the streaming
    /// pipeline and return the concatenated output. Splits the input into
    /// 1-byte chunks to maximally stress `LineSplit`'s carry buffer.
    fn drive_streaming_anthropic_to_openai(input: &[u8]) -> String {
        let chunks: Vec<Result<Bytes, _>> = input
            .chunks(1)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        let mut ls = LineSplit::new(stream::iter(chunks));
        let mut translator = AnthropicToOpenAITranslator::new();
        let mut out = String::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            while let Some(line_res) = ls.next().await {
                let line = line_res.expect("LineSplit error");
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(rest) = line.strip_prefix("event: ") {
                    translator.current_event = rest.to_string();
                    continue;
                }
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        let evt = translator.current_event.clone();
                        if let Some(extra) = translator.feed_event(&evt, &v) {
                            out.push_str(&extra);
                        }
                    }
                }
            }
        });
        out
    }

    /// Realistic Anthropic→OpenAI translation: message_start carries the id
    /// and model, then content_block_start/delta/stop produces a chat.completion.chunk
    /// with delta.content, then a final message_delta updates stop_reason, then
    /// message_stop emits the finish chunk + [DONE]. The streaming path must
    /// produce the same bytes as the buffered path even with 1-byte chunking.
    #[test]
    fn streaming_anthropic_to_openai_matches_buffered_path() {
        // Build a representative Anthropic SSE event sequence.
        let sse = concat!(
            // 1. message_start — carries id + model
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream_test\",\"model\":\"MiniMax-M3\",\"role\":\"assistant\",\"content\":[]}}\n\n",
            // 2. content_block_start (text)
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            // 3. content_block_delta (text delta with hello)
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            // 4. content_block_delta (more text)
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n",
            // 5. content_block_stop
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            // 6. message_delta — sets stop_reason
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}}\n\n",
            // 7. message_stop — emits the finish chunk + [DONE]
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );

        let non_stream = convert_anthropic_sse_to_openai(sse).expect("non-stream convert ok");
        let streamed = drive_streaming_anthropic_to_openai(sse.as_bytes());
        assert_eq!(
            streamed, non_stream,
            "streaming path diverged from buffered path"
        );

        // Sanity: the streamed text must appear across the chunk deltas, and
        // the final [DONE] sentinel must close the stream. Note the
        // translator emits one OpenAI chunk per Anthropic content_block_delta
        // event — it does not concatenate them — so we look for the
        // individual pieces rather than the joined "hello world" string.
        assert!(streamed.contains("\"hello\""), "missing first text delta: {streamed}");
        assert!(streamed.contains("\" world\""), "missing second text delta: {streamed}");
        assert!(streamed.contains("\"finish_reason\":\"stop\""), "missing final stop: {streamed}");
        assert!(streamed.contains("data: [DONE]"), "missing [DONE] sentinel: {streamed}");
    }
}

// ============================================================================
// openai -> anthropic  (request translation, response based on downstream format)
// ============================================================================

pub async fn proxy_openai_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<(String, TokenUsage), String> {
    let client = build_client()?;
    let openai_req: ChatCompletionsRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let anthropic_req = openai_to_anthropic_request(&openai_req, target_model)?;
    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&anthropic_req).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    // Response translation: if downstream is openai, translate anthropic -> openai
    match downstream_format {
        OutputFormat::Anthropic => {
            let usage = if body_str.contains("event:") || body_str.contains("data:") {
                TokenUsage::from_anthropic_sse(&body_str)
            } else {
                TokenUsage::from_anthropic_response(&body_str)
            };
            Ok((body_str.to_string(), usage))
        }
        OutputFormat::OpenAI => {
            let usage = if body_str.contains("event:") || body_str.contains("data:") {
                TokenUsage::from_anthropic_sse(&body_str)
            } else {
                TokenUsage::from_anthropic_response(&body_str)
            };
            let converted = crate::providers::anthropic::anthropic_response_to_openai(&body_str)?;
            Ok((converted, usage))
        }
    }
}

pub async fn proxy_non_stream_openai_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
    downstream_format: OutputFormat,
) -> Result<(String, TokenUsage), String> {
    let client = build_client()?;
    let openai_req: ChatCompletionsRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let anthropic_req = openai_to_anthropic_request(&openai_req, target_model)?;
    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&anthropic_req).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream (non-stream) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    match downstream_format {
        OutputFormat::Anthropic => {
            let usage = TokenUsage::from_anthropic_response(&body_str);
            Ok((body_str.to_string(), usage))
        }
        OutputFormat::OpenAI => {
            let usage = TokenUsage::from_anthropic_response(&body_str);
            let json: Value = serde_json::from_str(&body_str).map_err(|e| e.to_string())?;
            let resp = convert_anthropic_json_to_openai(&json)?;
            serde_json::to_string(&resp).map(|s| (s, usage)).map_err(|e| e.to_string())
        }
    }
}

// ============================================================================
// anthropic -> anthropic  (passthrough with model rename)
// ============================================================================

pub async fn proxy_anthropic_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<(String, TokenUsage), String> {
    let client = build_client()?;
    let mut req_json: Value = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    // Replace model name with the configured upstream model
    if let Some(obj) = req_json.as_object_mut() {
        obj.insert("model".to_string(), json!(target_model));
    }

    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&req_json).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body_bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    if !status.is_success() {
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream (passthrough) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }
    let usage = if body_str.contains("event:") || body_str.contains("data:") {
        TokenUsage::from_anthropic_sse(&body_str)
    } else {
        TokenUsage::from_anthropic_response(&body_str)
    };
    Ok((body_str.to_string(), usage))
}

pub async fn proxy_non_stream_anthropic_to_anthropic(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<(String, TokenUsage), String> {
    proxy_anthropic_to_anthropic(upstream, target_model, body).await
}

// ============================================================================
// Conversions
// ============================================================================

/// Truncate `s` to at most `max` characters, appending a marker if cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{}…(truncated, total {} chars)", head, s.chars().count())
    }
}

fn build_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(70))
        .build()
        .map_err(|e| e.to_string())
}

fn openai_to_anthropic_request(
    openai_req: &ChatCompletionsRequest,
    target_model: &str,
) -> Result<AnthropicRequest, String> {
    let system_msg = openai_req.messages.iter().find(|m| m.role == "system");
    let system = system_msg.and_then(|m| {
        if let ContentValue::String(s) = &m.content {
            Some(s.clone())
        } else {
            None
        }
    });

    let messages: Vec<AnthropicMessage> = openai_req.messages.iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            // OpenAI uses role="tool" with tool_call_id for tool results.
            // Anthropic expects a "user" message with content block type="tool_result".
            if m.role == "tool" {
                let tool_use_id = m.tool_call_id.clone().unwrap_or_default();
                let result_text = match &m.content {
                    ContentValue::String(s) => s.clone(),
                    ContentValue::Array(parts) => parts.iter()
                        .filter_map(|p| p.text.clone())
                        .collect::<Vec<_>>()
                        .join(""),
                };
                let block = AnthropicContentBlock {
                    block_type: "tool_result".to_string(),
                    text: None,
                    id: None,
                    name: None,
                    input: None,
                    tool_use_id: Some(tool_use_id),
                    content: Some(json!(result_text)),
                    source: None,
                };
                return AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Array(vec![block]),
                };
            }
            let role = if m.role == "assistant" { "assistant" } else { "user" };
            let mut blocks: Vec<AnthropicContentBlock> = Vec::new();
            // assistant: prior tool_calls -> tool_use blocks
            if m.role == "assistant" {
                if let Some(tcs) = &m.tool_calls {
                    for tc in tcs {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let name = tc.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        // arguments is a string per openai; parse to JSON for anthropic
                        let input = tc.get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or(json!({}));
                        blocks.push(AnthropicContentBlock {
                            block_type: "tool_use".to_string(),
                            text: None,
                            id: Some(id),
                            name: Some(name),
                            input: Some(input),
                            tool_use_id: None,
                            content: None,
                            source: None,
                        });
                    }
                }
            }
            // text/other content
            match &m.content {
                ContentValue::String(s) if !s.is_empty() => {
                    if !blocks.is_empty() {
                        // mixed text + tool_use: emit as array
                        blocks.insert(0, AnthropicContentBlock {
                            block_type: "text".to_string(),
                            text: Some(s.clone()),
                            id: None, name: None, input: None,
                            tool_use_id: None, content: None,
                            source: None,
                        });
                    } else {
                        // text-only assistant or user string
                    }
                }
                ContentValue::String(s) => {
                    // empty string; no-op
                    let _ = s;
                }
                ContentValue::Array(arr) => {
                    for part in arr {
                        // v0.3: image_url → Anthropic image block
                        if part.part_type == "image_url" {
                            if let Some(img) = &part.image_url {
                                if let Some(block) = openai_image_url_to_anthropic(img) {
                                    blocks.push(block);
                                    continue;
                                }
                            }
                        }
                        let mut obj = serde_json::Map::new();
                        obj.insert("type".to_string(), json!(part.part_type));
                        if let Some(text) = &part.text {
                            obj.insert("text".to_string(), json!(text));
                        }
                        if let Ok(b) = serde_json::from_value::<AnthropicContentBlock>(Value::Object(obj)) {
                            blocks.push(b);
                        }
                    }
                }
            }
            let content = if blocks.is_empty() {
                // fall back to plain string content for empty text cases
                match &m.content {
                    ContentValue::String(s) => AnthropicContent::String(s.clone()),
                    _ => AnthropicContent::String(String::new()),
                }
            } else {
                AnthropicContent::Array(blocks)
            };
            AnthropicMessage { role: role.to_string(), content }
        })
        .collect();

    let max_tokens = openai_req.max_tokens.unwrap_or(4096);

    let tools = openai_req.tools.as_ref().map(|tools| {
        tools.iter().filter_map(|t| {
            let func = t.function.as_ref()?;
            Some(AnthropicTool {
                tool_type: "function".to_string(),
                name: func.name.clone(),
                description: func.description.clone(),
                input_schema: func.parameters.clone(),
            })
        }).collect()
    });

    let tool_choice = openai_req.tool_choice.as_ref().and_then(|tc| {
        match tc {
            ToolChoice::String(s) => {
                match s.as_str() {
                    "auto" => Some(AnthropicToolChoice { choice_type: "auto".to_string(), name: None }),
                    "none" | "required" => Some(AnthropicToolChoice { choice_type: "any".to_string(), name: None }),
                    _ => None,
                }
            }
            ToolChoice::Object(obj) => {
                Some(AnthropicToolChoice {
                    choice_type: "tool".to_string(),
                    name: obj.function.as_ref().map(|f| f.name.clone()),
                })
            }
        }
    });

    Ok(AnthropicRequest {
        model: target_model.to_string(),
        messages,
        system,
        max_tokens,
        stream: openai_req.stream,
        temperature: openai_req.temperature,
        top_p: openai_req.top_p,
        tools,
        tool_choice,
        stop_sequences: None,
    })
}

/// v0.3 多模态: 把 OpenAI `image_url` 转换为 Anthropic image content block。
///
/// - `data:image/<media_type>;base64,<data>` → `{ type: base64, media_type, data }`
/// - `https://...` → `{ type: url, url }`
fn openai_image_url_to_anthropic(img: &ImageUrl) -> Option<AnthropicContentBlock> {
    let url = img.url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = url.strip_prefix("data:") {
        // data:<media>;base64,<payload>
        if let Some((meta, payload)) = rest.split_once(";base64,") {
            return Some(AnthropicContentBlock {
                block_type: "image".to_string(),
                text: None,
                id: None,
                name: None,
                input: None,
                tool_use_id: None,
                content: None,
                source: Some(AnthropicImageSource {
                    source_type: "base64".to_string(),
                    media_type: Some(meta.to_string()),
                    data: Some(payload.to_string()),
                    url: None,
                }),
            });
        }
        return None;
    }
    // remote URL
    Some(AnthropicContentBlock {
        block_type: "image".to_string(),
        text: None,
        id: None,
        name: None,
        input: None,
        tool_use_id: None,
        content: None,
        source: Some(AnthropicImageSource {
            source_type: "url".to_string(),
            media_type: None,
            data: None,
            url: Some(url.to_string()),
        }),
    })
}

// ============================================================================
// anthropic -> openai  (used by openai::proxy_anthropic_to_openai)
// ============================================================================

/// Convert an anthropic-format request body to an openai-format request body (JSON).
pub fn anthropic_request_to_openai_json(
    body: &str,
    target_model: &str,
) -> Result<String, String> {
    let req: AnthropicMessagesRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;

    let mut messages: Vec<OaiMessage> = Vec::new();

    if let Some(sys) = req.system {
        let sys_text = match sys {
            AnthropicSystem::String(s) => s,
            AnthropicSystem::Array(arr) => arr.iter()
                .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect::<Vec<_>>()
                .join("\n"),
        };
        if !sys_text.is_empty() {
            messages.push(OaiMessage {
                role: "system".to_string(),
                content: ContentValue::String(sys_text),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }

    for m in &req.messages {
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut tool_result_messages: Vec<OaiMessage> = Vec::new();
        let mut text_parts: Vec<ContentPart> = Vec::new();
        // Track whether the source message contained anything other than
        // tool_result blocks. If only tool_result blocks are present, the
        // source message itself was a synthetic carrier — we must not emit a
        // duplicate empty user message that confuses upstream tool-call
        // validators ("tool call result does not follow tool call").
        let mut has_non_tool_result = false;

        let content = match &m.content {
            Value::String(s) => {
                has_non_tool_result = true;
                ContentValue::String(s.clone())
            }
            Value::Array(arr) => {
                for v in arr {
                    let part_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match part_type {
                        "text" => {
                            has_non_tool_result = true;
                            if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                                text_parts.push(ContentPart {
                                    part_type: "text".to_string(),
                                    text: Some(text.to_string()),
                                    image_url: None,
                                });
                            }
                        }
                        "tool_use" => {
                            has_non_tool_result = true;
                            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let input = v.get("input").cloned().unwrap_or(json!({}));
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string())
                                }
                            }));
                        }
                        "tool_result" => {
                            // tool_result blocks are extracted into role="tool"
                            // messages; they do NOT contribute to the source
                            // message's content.
                            let tool_use_id = v.get("tool_use_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let result_text = v.get("content").map(|c| c.to_string()).unwrap_or_default();
                            tool_result_messages.push(OaiMessage {
                                role: "tool".to_string(),
                                content: ContentValue::String(result_text),
                                name: None,
                                tool_call_id: Some(tool_use_id),
                                tool_calls: None,
                            });
                        }
                        "image" => {
                            // v0.3: Anthropic image block → openai image_url part
                            has_non_tool_result = true;
                            if let Some(source) = v.get("source") {
                                let url = match (
                                    source.get("type").and_then(|t| t.as_str()),
                                    source.get("media_type").and_then(|t| t.as_str()),
                                    source.get("data").and_then(|d| d.as_str()),
                                    source.get("url").and_then(|u| u.as_str()),
                                ) {
                                    (Some("base64"), Some(media), Some(data), _) => {
                                        Some(format!("data:{};base64,{}", media, data))
                                    }
                                    (Some("url"), _, _, Some(u)) => Some(u.to_string()),
                                    _ => None,
                                };
                                if let Some(url) = url {
                                    text_parts.push(ContentPart {
                                        part_type: "image_url".to_string(),
                                        text: None,
                                        image_url: Some(ImageUrl { url, detail: None }),
                                    });
                                }
                            }
                        }
                        // skip thinking, etc.
                        _ => {}
                    }
                }
                if text_parts.is_empty() {
                    ContentValue::String(String::new())
                } else if text_parts.len() == 1 && text_parts[0].part_type == "text" {
                    ContentValue::String(text_parts[0].text.clone().unwrap_or_default())
                } else {
                    // 多个 part（含 image_url）必须以 array 形式发出
                    ContentValue::Array(text_parts)
                }
            }
            _ => {
                has_non_tool_result = true;
                ContentValue::String(m.content.to_string())
            }
        };

        // Only emit the source message if it carried something other than
        // tool_result blocks. Otherwise the tool_result extraction below is
        // the only meaningful translation of this source message.
        if has_non_tool_result {
            let role = if m.role == "assistant" { "assistant" } else { "user" };
            messages.push(OaiMessage {
                role: role.to_string(),
                content,
                name: None,
                tool_call_id: None,
                tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
            });
        }
        // tool_result blocks become role="tool" messages
        for tr in tool_result_messages {
            messages.push(tr);
        }
    }

    let openai_tools = req.tools.as_ref().map(|tools| {
        tools.iter().filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            let description = t.get("description").and_then(|d| d.as_str()).unwrap_or("");
            let input_schema = t.get("input_schema").cloned().unwrap_or(json!({}));
            Some(OaiTool {
                tool_type: Some("function".to_string()),
                function: Some(FunctionDef {
                    name: name.to_string(),
                    description: Some(description.to_string()),
                    parameters: input_schema,
                }),
            })
        }).collect()
    });

    let chat_req = ChatCompletionsRequest {
        model: target_model.to_string(),
        messages,
        stream: req.stream.unwrap_or(false),
        max_tokens: req.max_tokens,
        temperature: None,
        top_p: None,
        tools: openai_tools,
        tool_choice: None,
        stop: None,
    };

    serde_json::to_string(&chat_req).map_err(|e| e.to_string())
}

/// Convert an anthropic-format response (JSON or SSE chunk) to openai format.
pub fn anthropic_response_to_openai(anthropic_body: &str) -> Result<String, String> {
    // SSE path
    if anthropic_body.contains("event:") || anthropic_body.contains("data:") {
        return convert_anthropic_sse_to_openai(anthropic_body);
    }
    // JSON path
    let json: Value = serde_json::from_str(anthropic_body)
        .map_err(|e| format!("解析上游响应失败: {}", e))?;
    let openai_resp = convert_anthropic_json_to_openai(&json)?;
    serde_json::to_string(&openai_resp).map_err(|e| e.to_string())
}

fn convert_anthropic_json_to_openai(anthropic_resp: &Value) -> Result<Value, String> {
    let id = anthropic_resp.get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let model = anthropic_resp.get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let blocks = anthropic_resp.get("content")
        .and_then(|v| v.as_array())
        .map(|v| v.clone())
        .unwrap_or_default();

    let mut content_text = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let block_type = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    content_text.push_str(t);
                }
            }
            "tool_use" => {
                let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let input = b.get("input").cloned().unwrap_or(json!({}));
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "index": i,
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string())
                    }
                }));
            }
            // ignore "thinking" and other block types
            _ => {}
        }
    }

    let input_tokens = anthropic_resp.get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = anthropic_resp.get("usage")
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let stop_reason = anthropic_resp.get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("end_turn");
    let finish_reason = if !tool_calls.is_empty() {
        "tool_calls"
    } else {
        match stop_reason {
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            _ => "stop",
        }
    };

    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), json!("assistant"));
    message.insert("content".to_string(), json!(content_text));
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), json!(tool_calls));
    }

    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
            "finish_reason": finish_reason
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    }))
}

fn convert_anthropic_sse_to_openai(sse: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut translator = AnthropicToOpenAITranslator::new();
    for line in sse.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some(rest) = line.strip_prefix("event: ") {
            translator.current_event = rest.to_string();
            continue;
        }
        if line.starts_with("data: ") {
            let data = &line[6..];
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(extra) = translator.feed_event(&translator.current_event.clone(), &v) {
                    out.push_str(&extra);
                }
            }
        }
    }
    Ok(out)
}

/// Streaming variant of `convert_anthropic_sse_to_openai`. Holds the
/// per-line state machine (tool block tracking, stop_reason) so a caller
/// feeding one SSE event (event-name + data JSON) at a time gets back
/// exactly the bytes to flush downstream at each step.
///
/// Wire ordering: anthropic SSE interleaves `event:` and `data:` lines for
/// the same logical event. Callers should call `feed_event(name, json)` for
/// each `data:` line, passing the most recently seen `event:` token as
/// `name`. The translator filters `event: message_start` and
/// `event: content_block_stop` internally — they produce no openai output —
/// and on `event: message_stop` emits the final finish chunk + `[DONE]`.
pub struct AnthropicToOpenAITranslator {
    pub id: String,
    pub model: String,
    /// tool_use block metadata keyed by anthropic content_block index
    pub tool_blocks: std::collections::HashMap<u64, (String, String)>,
    /// ordered list of tool call indices seen, to assign openai tool_calls index
    pub tool_order: Vec<u64>,
    pub final_stop_reason: Option<String>,
    /// Most recently seen `event: <name>` token. Reset by callers on each
    /// upstream event header; consumed by the next `data:` line.
    pub current_event: String,
    pub done_emitted: bool,
}

impl Default for AnthropicToOpenAITranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicToOpenAITranslator {
    pub fn new() -> Self {
        Self {
            id: "chatcmpl-unknown".to_string(),
            model: String::new(),
            tool_blocks: std::collections::HashMap::new(),
            tool_order: Vec::new(),
            final_stop_reason: None,
            current_event: String::new(),
            done_emitted: false,
        }
    }

    /// Feed one Anthropic SSE event (already parsed from the `data:` line)
    /// along with the event-name token that preceded it. Returns `Some(out)`
    /// with bytes to flush downstream, or `None` if the event produced no
    /// openai output.
    pub fn feed_event(&mut self, event: &str, v: &Value) -> Option<String> {
        if event == "message_start" || event == "content_block_stop" {
            return None;
        }

        let mut out = String::new();

        if event == "message_stop" {
            if self.done_emitted {
                return None;
            }
            let finish = if !self.tool_order.is_empty() {
                "tool_calls"
            } else {
                match self.final_stop_reason.as_deref() {
                    Some("max_tokens") => "length",
                    Some("tool_use") => "tool_calls",
                    _ => "stop",
                }
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let chunk = json!({
                "id": self.id,
                "object": "chat.completion.chunk",
                "created": now,
                "model": self.model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": finish
                }]
            });
            out.push_str(&format!("data: {}\n\n", chunk));
            out.push_str("data: [DONE]\n\n");
            self.done_emitted = true;
            return Some(out);
        }

        // Pull id/model out of any payload that carries them (typically
        // message_start; harmless to re-extract on later events).
        if let Some(mid) = v.get("message").and_then(|m| m.get("id")).and_then(|x| x.as_str()) {
            self.id = mid.to_string();
        }
        if let Some(m) = v.get("message").and_then(|m| m.get("model")).and_then(|x| x.as_str()) {
            self.model = m.to_string();
        }

        let now = || std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        match v.get("type").and_then(|x| x.as_str()) {
            Some("content_block_start") => {
                let block = v.get("content_block");
                let block_type = block.and_then(|b| b.get("type")).and_then(|x| x.as_str()).unwrap_or("");
                if block_type == "tool_use" {
                    let block_index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0);
                    let tool_id = block.and_then(|b| b.get("id")).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let tool_name = block.and_then(|b| b.get("name")).and_then(|x| x.as_str()).unwrap_or("").to_string();
                    self.tool_blocks.insert(block_index, (tool_id.clone(), tool_name.clone()));
                    if !self.tool_order.contains(&block_index) {
                        self.tool_order.push(block_index);
                    }
                    let oa_index = self.tool_order.iter().position(|i| *i == block_index).unwrap_or(0);
                    let chunk = json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "created": now(),
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": oa_index,
                                    "id": tool_id,
                                    "type": "function",
                                    "function": {"name": tool_name, "arguments": ""}
                                }]
                            },
                            "finish_reason": null
                        }]
                    });
                    out.push_str(&format!("data: {}\n\n", chunk));
                }
            }
            Some("content_block_delta") => {
                let block_index = v.get("index").and_then(|x| x.as_u64()).unwrap_or(0);
                if self.tool_blocks.contains_key(&block_index) {
                    // tool_use argument delta
                    if let Some(partial) = v.get("delta").and_then(|d| d.get("partial_json")).and_then(|x| x.as_str()) {
                        let oa_index = self.tool_order.iter().position(|i| *i == block_index).unwrap_or(0);
                        let chunk = json!({
                            "id": self.id,
                            "object": "chat.completion.chunk",
                            "created": now(),
                            "model": self.model,
                            "choices": [{
                                "index": 0,
                                "delta": {
                                    "tool_calls": [{
                                        "index": oa_index,
                                        "function": {"arguments": partial}
                                    }]
                                },
                                "finish_reason": null
                            }]
                        });
                        out.push_str(&format!("data: {}\n\n", chunk));
                    }
                } else if let Some(text) = v.get("delta").and_then(|d| d.get("text")).and_then(|x| x.as_str()) {
                    let chunk = json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "created": now(),
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": {"content": text},
                            "finish_reason": null
                        }]
                    });
                    out.push_str(&format!("data: {}\n\n", chunk));
                }
            }
            Some("message_delta") => {
                if let Some(sr) = v.get("delta").and_then(|d| d.get("stop_reason")).and_then(|x| x.as_str()) {
                    self.final_stop_reason = Some(sr.to_string());
                }
            }
            _ => {}
        }

        if out.is_empty() { None } else { Some(out) }
    }

    /// Call after the input stream ends. If the upstream truncated before
    /// emitting `message_stop`, this synthesizes the final finish chunk +
    /// `[DONE]` so openai clients don't hang waiting for `[DONE]`.
    pub fn finish(&mut self) -> Option<String> {
        if self.done_emitted {
            return None;
        }
        let finish = if !self.tool_order.is_empty() {
            "tool_calls"
        } else {
            match self.final_stop_reason.as_deref() {
                Some("max_tokens") => "length",
                Some("tool_use") => "tool_calls",
                _ => "stop",
            }
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut out = String::new();
        let chunk = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": now,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": finish
            }]
        });
        out.push_str(&format!("data: {}\n\n", chunk));
        out.push_str("data: [DONE]\n\n");
        self.done_emitted = true;
        Some(out)
    }
}

// ============================================================================
// Streaming proxy variants
// ============================================================================
//
// Same architecture as the openai.rs streaming section:
//   upstream reqwest stream
//     └─ tokio::spawn driver task:
//          ├─ LineSplit bytes into lines
//          ├─ feed each line+event into AnthropicToOpenAITranslator
//          │  (or pass through verbatim for anthropic passthrough)
//          └─ forward output bytes to body_tx
//     └─ tokio::spawn_blocking usage tail:
//          └─ drain lines_rx, run TokenUsage::from_anthropic_sse at EOF

fn reqwest_err_to_box(e: reqwest::Error) -> BoxError {
    Box::new(e) as BoxError
}

fn mpsc_to_body_stream(
    rx: tokio::sync::mpsc::Receiver<Result<Bytes, BoxError>>,
) -> Pin<Box<dyn futures::stream::Stream<Item = Result<Bytes, BoxError>> + Send>> {
    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
}

fn spawn_anthropic_usage_tail(
    mut lines_rx: tokio::sync::mpsc::Receiver<String>,
) -> tokio::task::JoinHandle<TokenUsage> {
    tokio::task::spawn_blocking(move || {
        const TAIL_CAP: usize = 32 * 1024;
        let mut buf = String::new();
        while let Some(line) = lines_rx.blocking_recv() {
            if buf.len() + line.len() + 1 > TAIL_CAP {
                let overflow = (buf.len() + line.len() + 1) - TAIL_CAP;
                let drop_n = buf
                    .char_indices()
                    .nth(overflow)
                    .map(|(i, _)| i)
                    .unwrap_or(buf.len());
                buf.drain(..drop_n);
            }
            buf.push_str(&line);
            buf.push('\n');
        }
        TokenUsage::from_anthropic_sse(&buf)
    })
}

/// Drive an anthropic-upstream stream: split into lines, feed each line
/// into `translator` (which decides whether to emit output), and forward
/// both the output bytes to `body_tx` and the raw upstream lines to
/// `lines_tx` for usage extraction. For the passthrough case (`translate`
/// is false) the upstream bytes are forwarded verbatim with `'\n'`
/// separators reinserted.
async fn drive_anthropic_sse_to_body(
    upstream: impl futures::stream::Stream<Item = Result<Bytes, BoxError>> + Unpin + Send + 'static,
    body_tx: tokio::sync::mpsc::Sender<Result<Bytes, BoxError>>,
    lines_tx: tokio::sync::mpsc::Sender<String>,
    translate: bool,
) {
    let mut line_splitter = LineSplit::new(upstream);
    let mut translator = AnthropicToOpenAITranslator::new();
    let mut current_event: String = String::new();

    while let Some(line_res) = line_splitter.next().await {
        let line = match line_res {
            Ok(l) => l,
            Err(e) => {
                let _ = body_tx.send(Err(e)).await;
                return;
            }
        };

        // Forward raw upstream line for usage extraction.
        let _ = lines_tx.try_send(line.clone());

        if !translate {
            // Anthropic → Anthropic passthrough: forward verbatim.
            let mut bytes = line.into_bytes();
            bytes.push(b'\n');
            if body_tx.send(Ok(Bytes::from(bytes))).await.is_err() {
                return;
            }
            continue;
        }

        // Anthropic → OpenAI: track current event name; feed data lines.
        if let Some(rest) = line.strip_prefix("event: ") {
            current_event = rest.to_string();
            continue;
        }
        if line.starts_with("data: ") {
            let data = &line[6..];
            if let Ok(json) = serde_json::from_str::<Value>(data) {
                if let Some(extra) = translator.feed_event(&current_event, &json) {
                    if body_tx.send(Ok(Bytes::from(extra))).await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    if translate {
        if let Some(extra) = translator.finish() {
            let _ = body_tx.send(Ok(Bytes::from(extra))).await;
        }
    }
}

/// Stream variant: anthropic upstream → anthropic downstream passthrough.
/// (Used when the client used `/v1/messages` directly.)
pub async fn proxy_anthropic_to_anthropic_stream(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<
    (
        Pin<Box<dyn futures::stream::Stream<Item = Result<Bytes, BoxError>> + Send>>,
        tokio::task::JoinHandle<TokenUsage>,
    ),
    String,
> {
    let client = build_client()?;
    let mut req_json: Value = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;
    if let Some(obj) = req_json.as_object_mut() {
        obj.insert("model".to_string(), json!(target_model));
    }

    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&req_json).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body_bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let body_str = String::from_utf8_lossy(&body_bytes);
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream (stream passthrough) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    let upstream_stream = resp
        .bytes_stream()
        .map(|r| r.map_err(reqwest_err_to_box));

    let (body_tx, body_rx) = tokio::sync::mpsc::channel::<Result<Bytes, BoxError>>(32);
    let (lines_tx, lines_rx) = tokio::sync::mpsc::channel::<String>(64);

    tokio::spawn(drive_anthropic_sse_to_body(
        upstream_stream,
        body_tx,
        lines_tx,
        false, // passthrough
    ));

    let usage_handle = spawn_anthropic_usage_tail(lines_rx);
    let body_stream = mpsc_to_body_stream(body_rx);
    Ok((body_stream, usage_handle))
}

/// Stream variant: anthropic upstream → openai downstream (translator).
pub async fn proxy_openai_to_anthropic_stream(
    upstream: &UpstreamConfig,
    target_model: &str,
    body: &str,
) -> Result<
    (
        Pin<Box<dyn futures::stream::Stream<Item = Result<Bytes, BoxError>> + Send>>,
        tokio::task::JoinHandle<TokenUsage>,
    ),
    String,
> {
    let client = build_client()?;
    let openai_req: ChatCompletionsRequest = serde_json::from_str(body)
        .map_err(|e| format!("请求体解析失败: {}", e))?;
    let anthropic_req = openai_to_anthropic_request(&openai_req, target_model)?;

    let resp = client
        .post(format!("{}/messages", upstream.base_url))
        .header("Content-Type", "application/json")
        .header("x-api-key", &upstream.api_key)
        .header("anthropic-version", "2023-06-01")
        .body(serde_json::to_string(&anthropic_req).map_err(|e| e.to_string())?)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let body_bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        let body_str = String::from_utf8_lossy(&body_bytes);
        tracing::error!(
            target: "upstream",
            upstream = "anthropic",
            target_model,
            base_url = %upstream.base_url,
            status = status.as_u16(),
            body = %truncate(&body_str, 2048),
            "anthropic upstream (openai->anthropic stream) returned non-success"
        );
        return Err(format!("上游返回 {}: {}", status.as_u16(), body_str));
    }

    let upstream_stream = resp
        .bytes_stream()
        .map(|r| r.map_err(reqwest_err_to_box));

    let (body_tx, body_rx) = tokio::sync::mpsc::channel::<Result<Bytes, BoxError>>(32);
    let (lines_tx, lines_rx) = tokio::sync::mpsc::channel::<String>(64);

    tokio::spawn(drive_anthropic_sse_to_body(
        upstream_stream,
        body_tx,
        lines_tx,
        true, // translate
    ));

    let usage_handle = spawn_anthropic_usage_tail(lines_rx);
    let body_stream = mpsc_to_body_stream(body_rx);
    Ok((body_stream, usage_handle))
}
